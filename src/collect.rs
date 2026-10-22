use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::alerts::{AlertProcessor, Check, MetricsState, RpcObs, WarnInput, check_warnings, firing};
use crate::bft_log::{RoundTracker, start_bft_log_tail};
use crate::config::{METRICS_URL, RPC_URL, settings};
use crate::event_history::{get_history, push_history};
use crate::exec_log::{ExecSnapshot, ExecWindow, start_exec_log_tail};
use crate::frame::{CollectorState, TickInput, build_tick_frame, error_frame, record_block, status_of};
use crate::http;
use crate::metric_snapshot::read_metrics_snapshot;
use crate::notify::{NullNotifier, start_telegram_outbox};
use crate::otlp::{Labels, OtlpReceiver, start_otlp_receiver};
use crate::poll::{fetch_block_number, fetch_metrics_text};
use crate::types::{Frame, FrameError, RoundSummary, Snapshot};

// bft exports every second; no export for this long means the node isn't pushing.
const OTLP_STALE_MS: i64 = 15_000;
const LIVE_RPC_TIMEOUT: Duration = Duration::from_millis(500);

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

async fn start_metrics_source() -> Option<OtlpReceiver> {
    let s = settings();
    if s.otlp_listen.is_empty() {
        eprintln!("metrics: scraping {METRICS_URL}");
        return None;
    }
    let extra: Labels = s.node_key.iter().map(|k| ("secp_key".to_string(), k.clone())).collect();
    let r = start_otlp_receiver(&s.otlp_listen, extra).await;
    match &r {
        Some(_) => eprintln!("metrics: OTLP receiver on {}", s.otlp_listen),
        None => eprintln!("metrics: {} is busy (otelcol or a monadar daemon running?), scraping {METRICS_URL}", s.otlp_listen),
    }
    r
}

// What the journals say right now, whatever the state of RPC and metrics.
struct JournalView {
    exec: ExecSnapshot,
    rounds: Option<RoundSummary>,
    bft_silent_ms: Option<i64>,
}

// The data half of a tick: polls RPC + metrics, holds the previous-tick state
// needed for deltas, runs the alert rules, and returns a Frame.
// Rendering is done elsewhere (render.rs) from the returned Frame.
pub struct Collector {
    state: CollectorState,
    alerts: AlertProcessor,
    exec_log: Arc<Mutex<ExecWindow>>,
    bft_log: Arc<Mutex<RoundTracker>>,
    otlp: Option<OtlpReceiver>,
    started_at: i64,
    last_heartbeat: Option<Instant>,
}

impl Collector {
    pub async fn new() -> Collector {
        let s = settings();
        let telegram = s.config.telegram.as_ref().filter(|t| !t.token.is_empty() && !t.chat_id.is_empty());
        let notifier = match telegram {
            Some(t) => start_telegram_outbox(t, s.outbox_file.clone()),
            None => Box::new(NullNotifier),
        };
        let alerts = AlertProcessor::new(
            telegram.and_then(|t| t.delay_ticks).unwrap_or(3),
            (telegram.and_then(|t| t.cooldown_min).unwrap_or(5.0) * 60_000.0) as i64,
            Box::new(push_history),
            notifier,
            Box::new(now_ms),
        );
        let bft_log = start_bft_log_tail();
        bft_log.lock().unwrap().configure_self(s.node_key.as_deref());
        Collector {
            state: CollectorState::default(),
            alerts,
            exec_log: start_exec_log_tail(),
            bft_log,
            otlp: start_metrics_source().await,
            started_at: now_ms(),
            last_heartbeat: None,
        }
    }

    // Ok(None): the OTLP receiver just started and has no export yet — not an outage.
    async fn metrics_text(&self) -> Result<Option<String>, String> {
        let Some(otlp) = &self.otlp else {
            return fetch_metrics_text(METRICS_URL, Duration::from_secs(3)).await.map(Some);
        };
        match otlp.last_export() {
            None if now_ms() - self.started_at < OTLP_STALE_MS => Ok(None),
            Some(last) if now_ms() - last <= OTLP_STALE_MS => Ok(Some(otlp.text())),
            _ => Err("no recent OTLP exports".into()),
        }
    }

    fn journals(&self) -> JournalView {
        let now = now_ms();
        let exec = self.exec_log.lock().unwrap().snapshot(now as f64);
        let bft = self.bft_log.lock().unwrap();
        JournalView { exec, rounds: bft.summary(now), bft_silent_ms: bft.silent_ms(now) }
    }

    // For viewers between ticks: the current block from RPC and the executed
    // block from the execution journal. None without a live journal: with the
    // metric's execution height, 1s old, the lag would look wrong.
    pub async fn live_blocks(&self) -> Option<(i64, i64)> {
        let exec = self.exec_log.lock().unwrap().last_block(now_ms() as f64)?;
        let (block, _) = fetch_block_number(RPC_URL, LIVE_RPC_TIMEOUT).await.ok()?;
        Some((block, exec))
    }

    // Dead man's switch ping, daemon only: fire and forget, the far end alerts on silence.
    fn heartbeat(&mut self) {
        let s = settings();
        let Some(url) = s.config.heartbeat_url.clone().filter(|_| s.args.daemon) else { return };
        let every = Duration::from_secs_f64(s.config.heartbeat_sec.unwrap_or(60.0).max(1.0));
        if self.last_heartbeat.is_some_and(|t| t.elapsed() < every) {
            return;
        }
        self.last_heartbeat = Some(Instant::now());
        tokio::spawn(async move {
            match http::get(&url, Duration::from_secs(10)).await {
                Ok(r) if r.ok() => {}
                Ok(r) => eprintln!("heartbeat: HTTP {}", r.status),
                Err(e) => eprintln!("heartbeat: {e}"),
            }
        });
    }

    // RPC or metrics unavailable: run the rules on what is known, show an error frame.
    fn degraded(
        &mut self,
        ts: String,
        error: FrameError,
        rpc: Option<(i64, RpcObs, String)>,
        metrics: MetricsState,
        j: JournalView,
    ) -> Frame {
        let lag = rpc.as_ref().zip(j.exec.last_block).map(|((block, ..), eb)| (block - eb).max(0));
        let checks: Vec<Check> = check_warnings(
            &WarnInput {
                rpc: rpc.as_ref().map(|r| r.1),
                metrics,
                lag,
                exec_p99: j.exec.stats.map(|e| e.p99),
                rounds: j.rounds.as_ref(),
                bft_silent_ms: j.bft_silent_ms,
            },
            &settings().thresholds,
        );
        let snap = match &rpc {
            Some((block, obs, bps)) => Snapshot {
                block: Some(*block),
                block_delta: obs.block_delta,
                bps: bps.clone(),
                lag,
                exec_p99: j.exec.stats.map(|e| e.p99),
                rpc_ms: Some(obs.rpc_ms),
                ..Snapshot::default()
            },
            None => Snapshot::default(),
        };
        let new_crit = self.alerts.process(&ts, &checks, &snap);
        let mut f = error_frame(ts, get_history(), error, rpc.map(|(block, obs, bps)| (block, obs.block_delta, bps)));
        f.warnings = firing(&checks);
        f.status = status_of(&f.warnings);
        f.new_crit = new_crit;
        f
    }

    pub async fn tick(&mut self) -> Frame {
        self.heartbeat();
        let s = settings();
        let ts = chrono::Local::now().format("%H:%M:%S").to_string();

        let Ok((block, rpc_ms)) = fetch_block_number(RPC_URL, Duration::from_secs(5)).await else {
            // Node/RPC unreachable: RPC_DOWN fires; rules that need RPC data are
            // Unknown and keep their state instead of resolving.
            let j = self.journals();
            return self.degraded(ts, FrameError::Rpc, None, MetricsState::Pending, j);
        };

        let now = now_ms();
        let sample = record_block(&mut self.state, block, now);
        let obs = RpcObs { block_delta: sample.block_delta, stalled_ms: sample.stalled_ms, rpc_ms };
        let metrics = self.metrics_text().await;
        let j = self.journals();
        let mt = match metrics {
            Ok(Some(mt)) => mt,
            Ok(None) => return self.degraded(ts, FrameError::Metrics, Some((block, obs, sample.bps)), MetricsState::Pending, j),
            Err(_) => return self.degraded(ts, FrameError::Metrics, Some((block, obs, sample.bps)), MetricsState::Down, j),
        };

        let metrics = read_metrics_snapshot(&mt);
        // Without node_key in the config and before the journal told us, fall back to the secp_key label.
        self.bft_log.lock().unwrap().suggest_self(Some(&metrics.node_key));
        let built = build_tick_frame(
            &mut self.state,
            TickInput {
                ts: ts.clone(),
                block,
                block_delta: sample.block_delta,
                bps: sample.bps,
                rpc_ms,
                metrics: &metrics,
                thresholds: &s.thresholds,
                exec: j.exec,
                rounds: j.rounds,
                stalled_ms: sample.stalled_ms,
                bft_silent_ms: j.bft_silent_ms,
                now_ms: now,
            },
        );
        let new_crit = self.alerts.process(&ts, &built.checks, &built.snap);
        if built.restarted {
            self.alerts.notify_restart(&ts, &built.uptime, &built.snap);
        }
        let mut frame = built.frame;
        frame.new_crit = new_crit;
        frame.history = get_history();
        frame
    }
}
