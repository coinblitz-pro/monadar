use std::collections::HashMap;

use indexmap::IndexMap;

use crate::format::{js_round, num};
use crate::notify::{Notice, NoticeKind, Notifier};
use crate::telegram::snapshot_rows;
use crate::types::{EventLevel, HistoryEntry, Level, RoundSummary, Snapshot, Thresholds, Warning};

// Below this many rounds in 10 min (normally ~1500) the TC share is not meaningful.
const NET_MIN_ROUNDS: u64 = 200;
// monad-bft at DEBUG logs ~100 lines/s; this long without a line means the tail is broken.
const BFT_SILENT_MS: i64 = 60_000;

// Rounded to the nearest second: ticks are 1s apart, so a two-tick condition
// lasts 1.99s or 2.01s depending on jitter, and should read "2s" either way.
pub fn fmt_duration(ms: i64) -> String {
    let s = (ms + 500).div_euclid(1000);
    if s < 60 {
        return format!("{s}s");
    }
    let m = s / 60;
    if m < 60 {
        return format!("{m}m{}s", s % 60);
    }
    format!("{}h{}m", m / 60, m % 60)
}

// Outcome of one rule on one tick. Unknown means the rule's data source is
// unavailable: the rule's state is frozen — it neither fires nor resolves.
// A source going away (RPC down, execution journal silent, a metric missing)
// must never look like recovery.
#[derive(Clone, Debug, PartialEq)]
pub enum Check {
    Firing(Warning),
    Clear(&'static str),
    Unknown(&'static str),
}

#[cfg(test)]
impl Check {
    pub fn key(&self) -> &str {
        match self {
            Check::Firing(w) => &w.key,
            Check::Clear(k) | Check::Unknown(k) => k,
        }
    }
}

pub fn firing(checks: &[Check]) -> Vec<Warning> {
    checks
        .iter()
        .filter_map(|c| match c {
            Check::Firing(w) => Some(w.clone()),
            _ => None,
        })
        .collect()
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RpcObs {
    pub block_delta: Option<i64>,
    pub stalled_ms: i64,
    pub rpc_ms: i64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct MetricObs {
    pub dv: Option<f64>,
    pub dt: Option<f64>,
    pub dtc: Option<f64>,
    pub syncing: Option<f64>,
    pub peers: Option<f64>,
    pub lat_p99: Option<f64>,
}

#[derive(Clone, Copy, Debug)]
pub enum MetricsState {
    Up(MetricObs),
    Down,
    // Not known yet (OTLP receiver waiting for the first export).
    Pending,
}

pub struct WarnInput<'a> {
    // None: RPC is down.
    pub rpc: Option<RpcObs>,
    pub metrics: MetricsState,
    pub lag: Option<i64>,
    pub exec_p99: Option<i64>,
    pub rounds: Option<&'a RoundSummary>,
    // Time since the last monad-bft journal line; None if none was ever seen.
    pub bft_silent_ms: Option<i64>,
}

fn warning(level: Level, key: &'static str, short: String, detail: String) -> Check {
    Check::Firing(Warning { level, key: key.into(), short, detail })
}

// Threshold rule over an optional value: Unknown without data.
fn rule(key: &'static str, value: Option<f64>, fires: impl Fn(f64) -> bool, make: impl Fn(f64) -> Check) -> Check {
    match value {
        None => Check::Unknown(key),
        Some(v) if fires(v) => make(v),
        Some(_) => Check::Clear(key),
    }
}

pub fn check_warnings(i: &WarnInput, t: &Thresholds) -> Vec<Check> {
    let m = match i.metrics {
        MetricsState::Up(m) if i.rpc.is_some() => Some(m),
        _ => None,
    };
    let mut w = Vec::new();

    w.push(match i.rpc {
        None => warning(Level::Crit, "RPC_DOWN", "RPC DOWN".into(), "RPC endpoint not responding — node may be down or unreachable".into()),
        Some(_) => Check::Clear("RPC_DOWN"),
    });
    w.push(match (i.rpc, i.metrics) {
        (Some(_), MetricsState::Down) => {
            warning(Level::Warn, "METRICS_DOWN", "METRICS DOWN".into(), "Metrics endpoint unavailable — node process may be down".into())
        }
        (Some(_), MetricsState::Up(_)) => Check::Clear("METRICS_DOWN"),
        _ => Check::Unknown("METRICS_DOWN"),
    });

    w.push(rule(
        "SYNCING",
        m.and_then(|m| m.syncing),
        |s| s == 1.0,
        |_| warning(Level::Crit, "SYNCING", "SYNCING".into(), "Node is state-syncing, not fully caught up with the network".into()),
    ));

    // Time-based: a timed-out round alone pauses blocks for ~1-1.5s.
    w.push(rule(
        "STUCK",
        i.rpc.map(|r| r.stalled_ms as f64),
        |ms| ms >= t.stuck_sec * 1000.0,
        |ms| {
            warning(
                Level::Crit,
                "STUCK",
                "STUCK".into(),
                format!("Block number has not advanced for {}s — node may be stuck or disconnected", js_round(ms / 1000.0)),
            )
        },
    ));

    let voting = match (i.rpc.and_then(|r| r.block_delta), m.and_then(|m| m.dv)) {
        (Some(bd), Some(dv)) => Some(if dv == 0.0 && bd > 0 { 1.0 } else { 0.0 }),
        _ => None,
    };
    w.push(rule(
        "NOT_VOTING",
        voting,
        |v| v == 1.0,
        |_| {
            warning(
                Level::Crit,
                "NOT_VOTING",
                "NOT VOTING".into(),
                "Node is not creating votes while blocks advance — consensus participation lost".into(),
            )
        },
    ));

    // Round-level data from the bft journal: alert only on what's our fault.
    // Timeouts/TCs caused by other leaders' misses are network noise (see net_tc).
    let r = i.rounds;
    w.push(rule("MISSED_ROUND", r.map(|r| r.missed10m as f64), |n| n > 0.0, |n| {
        warning(
            Level::Crit,
            "MISSED_ROUND",
            format!("MISSED ROUND ×{n}"),
            format!("Our node was leader and its block did not make it in {n} round(s) during the last 10 min (round ended by timeout certificate)"),
        )
    }));
    w.push(rule("LATE", r.map(|r| r.late_tmo10m as f64), |n| n > 0.0, |n| {
        warning(
            Level::Warn,
            "LATE",
            format!("LATE ×{n}"),
            format!("Our node timed out {n} round(s) in the last 10 min that the network completed normally — proposals reach us late (network/CPU)"),
        )
    }));

    // Network-wide degradation, explicitly not our node. Single TC rounds are
    // routine (~0.7%, mostly a few offline validators), so only abnormal rates
    // or back-to-back TCs (the whole chain stands still for seconds) count.
    w.push(match r {
        None => Check::Unknown("NET_DEGRADED"),
        Some(r) => {
            let n = r.net10m;
            let tc_pct = if n.rounds >= NET_MIN_ROUNDS { n.tc as f64 / n.rounds as f64 * 100.0 } else { 0.0 };
            let mut reasons = Vec::new();
            if tc_pct > t.net_tc_pct_max {
                reasons.push(format!("{tc_pct:.1}% of rounds ended by TC"));
            }
            if n.max_tc_run as f64 > t.net_tc_run_max {
                reasons.push(format!("{} TC rounds in a row", n.max_tc_run));
            }
            if reasons.is_empty() {
                Check::Clear("NET_DEGRADED")
            } else {
                let short =
                    if tc_pct > t.net_tc_pct_max { format!("NET TC {tc_pct:.1}%") } else { format!("NET TC ×{} in a row", n.max_tc_run) };
                warning(Level::Warn, "NET_DEGRADED", short, format!("Network (not our node), last 10 min: {}", reasons.join(", ")))
            }
        }
    });

    // No journal data (bft not logging at DEBUG): fall back to metric deltas,
    // which can't tell our timeouts from other leaders' misses.
    let fallback = |v: Option<f64>| if r.is_some() { Some(0.0) } else { v };
    w.push(rule(
        "TMO",
        fallback(m.and_then(|m| m.dt)),
        |d| d > 0.0,
        |d| {
            let d = num(d);
            warning(Level::Warn, "TMO", format!("TMO Δ{d}"), format!("{d} new local timeout(s) — node failed to receive proposal in time"))
        },
    ));
    w.push(rule(
        "TC",
        fallback(m.and_then(|m| m.dtc)),
        |d| d > t.tc_max,
        |d| {
            let d = num(d);
            warning(
                Level::Warn,
                "TC",
                format!("TC Δ{d}"),
                format!("{d} new timeout certificate(s) — round(s) completed via timeout instead of QC"),
            )
        },
    ));

    w.push(rule(
        "LAG",
        i.lag.map(|l| l as f64),
        |l| l > t.lag_max,
        |l| warning(Level::Warn, "LAG", format!("LAG {l}"), format!("Execution behind consensus by {l} blocks")),
    ));
    w.push(rule(
        "EXEC_SLOW",
        i.exec_p99.map(|p| p as f64),
        |p| p > t.exec_ms_max,
        |p| {
            warning(
                Level::Warn,
                "EXEC_SLOW",
                format!("EXEC SLOW {p}ms"),
                format!("Execution latency p99 is {p}ms (proposal → executed, last 60s)"),
            )
        },
    ));
    w.push(rule(
        "LOW_PEERS",
        m.and_then(|m| m.peers),
        |p| p < t.peers_min,
        |p| {
            let p = num(p);
            warning(
                Level::Warn,
                "LOW_PEERS",
                format!("LOW PEERS {p}"),
                format!("Only {p} peers discovered — network connectivity may be degraded"),
            )
        },
    ));
    w.push(rule(
        "HIGH_P99",
        m.and_then(|m| m.lat_p99),
        |l| l > t.p99_max,
        |l| {
            let l = num(l);
            warning(
                Level::Warn,
                "HIGH_P99",
                format!("HIGH P99 {l}ms"),
                format!("Broadcast latency p99 is {l}ms — may miss round deadlines"),
            )
        },
    ));
    w.push(rule(
        "SLOW_RPC",
        i.rpc.map(|r| r.rpc_ms as f64),
        |ms| ms > t.rpc_max,
        |ms| warning(Level::Warn, "SLOW_RPC", format!("SLOW RPC {ms}ms"), format!("RPC response took {ms}ms — node may be overloaded")),
    ));

    // The bft journal fed us before and went quiet: round tracking is blind.
    let silent = if i.rpc.is_some() { Some(i.bft_silent_ms.unwrap_or(0) as f64) } else { None };
    w.push(rule("BFT_LOG_SILENT", silent, |ms| ms > BFT_SILENT_MS as f64, |ms| {
        warning(
            Level::Warn,
            "BFT_LOG_SILENT",
            "BFT LOG SILENT".into(),
            format!(
                "No monad-bft journal lines for {} — round tracking is blind (journalctl stopped, no journal access, or bft no longer logs at DEBUG)",
                fmt_duration(ms as i64)
            ),
        )
    }));

    w
}

struct Active {
    warning: Warning,
    since: i64,
    ticks: u64,
    // None until the delay has passed; then whether a notice went out (false:
    // suppressed by cooldown, so no "resolved" either).
    announced: Option<bool>,
    // Consecutive Clear ticks; >0 means already shown as resolved, the notice
    // waits out the grace period in case the condition flaps back.
    cleared: u64,
    cleared_at: i64,
}

pub struct AlertProcessor {
    delay_ticks: u64,
    cooldown_ms: i64,
    history: Box<dyn FnMut(HistoryEntry) + Send + Sync>,
    notifier: Box<dyn Notifier>,
    now: Box<dyn Fn() -> i64 + Send + Sync>,
    active: IndexMap<String, Active>,
    last_notice: HashMap<String, i64>,
    suppressed: HashMap<String, u32>,
}

impl AlertProcessor {
    pub fn new(
        delay_ticks: u64,
        cooldown_ms: i64,
        history: Box<dyn FnMut(HistoryEntry) + Send + Sync>,
        notifier: Box<dyn Notifier>,
        now: Box<dyn Fn() -> i64 + Send + Sync>,
    ) -> AlertProcessor {
        AlertProcessor {
            delay_ticks: delay_ticks.max(1),
            cooldown_ms,
            history,
            notifier,
            now,
            active: IndexMap::new(),
            last_notice: HashMap::new(),
            suppressed: HashMap::new(),
        }
    }

    // Cooldown per key: a repeat within cooldown_ms is not sent but counted,
    // and the next message for that key says how many were skipped.
    fn admit(&mut self, key: &str, now: i64) -> Option<u32> {
        if self.last_notice.get(key).is_some_and(|t| now - t < self.cooldown_ms) {
            *self.suppressed.entry(key.to_string()).or_default() += 1;
            return None;
        }
        self.last_notice.insert(key.to_string(), now);
        Some(self.suppressed.remove(key).unwrap_or(0))
    }

    // Returns whether a new critical warning appeared (for the terminal bell).
    pub fn process(&mut self, ts: &str, checks: &[Check], snap: &Snapshot) -> bool {
        let now = (self.now)();
        let mut new_crit = false;

        for check in checks {
            match check {
                Check::Firing(w) => {
                    let a = self.active.entry(w.key.clone()).or_insert_with(|| Active {
                        warning: w.clone(),
                        since: now,
                        ticks: 0,
                        announced: None,
                        cleared: 0,
                        cleared_at: 0,
                    });
                    if a.ticks == 0 || a.cleared > 0 {
                        (self.history)(HistoryEntry { time: ts.into(), level: w.level.into(), text: w.detail.clone() });
                        if a.cleared > 0 {
                            a.since = now;
                            a.ticks = 0;
                            a.cleared = 0;
                        }
                        new_crit |= w.level == Level::Crit;
                    }
                    a.ticks += 1;
                    a.warning = w.clone();
                    if w.level == Level::Crit && a.ticks >= self.delay_ticks && a.announced.is_none() {
                        let (since, key) = (a.since, w.key.clone());
                        let suppressed = self.admit(&key, now);
                        self.active[&key].announced = Some(suppressed.is_some());
                        if let Some(suppressed) = suppressed {
                            self.notifier.push(Notice {
                                key,
                                kind: NoticeKind::Fired,
                                at: since,
                                title: w.short.clone(),
                                detail: w.detail.clone(),
                                snap: snapshot_rows(snap),
                                since: None,
                                suppressed,
                            });
                        }
                    }
                }
                Check::Clear(key) => {
                    let Some(a) = self.active.get_mut(*key) else { continue };
                    a.cleared += 1;
                    if a.cleared == 1 {
                        a.cleared_at = now;
                        let dur = fmt_duration(now - a.since);
                        (self.history)(HistoryEntry {
                            time: ts.into(),
                            level: EventLevel::Resolved,
                            text: format!("Resolved: {} ({dur})", a.warning.short),
                        });
                    }
                    if a.announced != Some(true) {
                        self.active.shift_remove(*key);
                    } else if a.cleared >= self.delay_ticks {
                        let a = self.active.shift_remove(*key).unwrap();
                        self.notifier.push(Notice {
                            key: key.to_string(),
                            kind: NoticeKind::Resolved,
                            at: a.cleared_at,
                            title: a.warning.short,
                            detail: String::new(),
                            snap: snapshot_rows(snap),
                            since: Some(a.since),
                            suppressed: 0,
                        });
                    }
                }
                // Data source unavailable: keep whatever state the rule had.
                Check::Unknown(_) => {}
            }
        }
        new_crit
    }

    pub fn notify_restart(&mut self, ts: &str, uptime: &str, snap: &Snapshot) {
        (self.history)(HistoryEntry { time: ts.into(), level: EventLevel::Warn, text: format!("Node restarted (uptime {uptime})") });
        let now = (self.now)();
        if let Some(suppressed) = self.admit("RESTART", now) {
            self.notifier.push(Notice {
                key: "RESTART".into(),
                kind: NoticeKind::Info,
                at: now,
                title: "NODE RESTART".into(),
                detail: format!("uptime {uptime}"),
                snap: snapshot_rows(snap),
                since: None,
                suppressed,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct Recorder(Arc<Mutex<Vec<Notice>>>);

    impl Notifier for Recorder {
        fn push(&self, n: Notice) {
            self.0.lock().unwrap().push(n);
        }
    }

    struct Harness {
        proc: AlertProcessor,
        sent: Arc<Mutex<Vec<Notice>>>,
        history: Arc<Mutex<Vec<HistoryEntry>>>,
        clock: Arc<Mutex<i64>>,
    }

    impl Harness {
        fn new(delay_ticks: u64, cooldown_ms: i64) -> Harness {
            let sent = Arc::new(Mutex::new(Vec::new()));
            let history = Arc::new(Mutex::new(Vec::new()));
            let clock = Arc::new(Mutex::new(0));
            let (h, c) = (history.clone(), clock.clone());
            let proc = AlertProcessor::new(
                delay_ticks,
                cooldown_ms,
                Box::new(move |e| h.lock().unwrap().push(e)),
                Box::new(Recorder(sent.clone())),
                Box::new(move || *c.lock().unwrap()),
            );
            Harness { proc, sent, history, clock }
        }

        // One tick, one second later than the previous one.
        fn tick(&mut self, checks: &[Check]) {
            *self.clock.lock().unwrap() += 1000;
            self.proc.process("t", checks, &Snapshot::default());
        }

        fn sent(&self) -> Vec<(String, NoticeKind)> {
            self.sent.lock().unwrap().iter().map(|n| (n.key.clone(), n.kind)).collect()
        }

        fn resolved(&self) -> Vec<String> {
            self.history.lock().unwrap().iter().filter(|e| e.level == EventLevel::Resolved).map(|e| e.text.clone()).collect()
        }
    }

    fn crit(key: &'static str) -> Check {
        warning(Level::Crit, key, key.replace('_', " "), "d".into())
    }

    #[test]
    fn crit_notifies_after_delay_ticks_once() {
        let mut h = Harness::new(3, 0);
        h.tick(&[crit("RPC_DOWN")]);
        h.tick(&[crit("RPC_DOWN")]);
        assert!(h.sent().is_empty(), "no alert before delay_ticks");
        h.tick(&[crit("RPC_DOWN")]);
        assert_eq!(h.sent(), vec![("RPC_DOWN".into(), NoticeKind::Fired)]);
        h.tick(&[crit("RPC_DOWN")]);
        assert_eq!(h.sent().len(), 1, "no repeat while still firing");
        assert_eq!(h.sent.lock().unwrap()[0].at, 1000, "notice carries the time the condition started");
    }

    #[test]
    fn unknown_freezes_the_state_instead_of_resolving() {
        let mut h = Harness::new(1, 0);
        h.tick(&[crit("EXEC_SLOW")]);
        // The execution journal went silent: no data is not recovery.
        h.tick(&[Check::Unknown("EXEC_SLOW")]);
        h.tick(&[Check::Unknown("EXEC_SLOW")]);
        assert!(h.resolved().is_empty());
        assert_eq!(h.sent(), vec![("EXEC_SLOW".into(), NoticeKind::Fired)]);
        h.tick(&[Check::Clear("EXEC_SLOW")]);
        assert_eq!(h.resolved(), vec!["Resolved: EXEC SLOW (3s)"]);
        assert_eq!(h.sent().last().unwrap(), &("EXEC_SLOW".into(), NoticeKind::Resolved));
    }

    #[test]
    fn unknown_does_not_count_towards_the_delay() {
        let mut h = Harness::new(2, 0);
        h.tick(&[crit("STUCK")]);
        h.tick(&[Check::Unknown("STUCK")]);
        assert!(h.sent().is_empty());
        h.tick(&[crit("STUCK")]);
        assert_eq!(h.sent().len(), 1);
    }

    #[test]
    fn resolved_waits_the_grace_period_and_flaps_are_not_resent() {
        let mut h = Harness::new(2, 0);
        h.tick(&[crit("STUCK")]);
        h.tick(&[crit("STUCK")]);
        assert_eq!(h.sent().len(), 1);
        h.tick(&[Check::Clear("STUCK")]);
        assert_eq!(h.resolved().len(), 1, "history shows the resolve at once");
        // Flaps back within the grace period: still the same incident.
        h.tick(&[crit("STUCK")]);
        h.tick(&[crit("STUCK")]);
        assert_eq!(h.sent().len(), 1, "no second alert for a flap");
        h.tick(&[Check::Clear("STUCK")]);
        h.tick(&[Check::Clear("STUCK")]);
        assert_eq!(h.sent(), vec![("STUCK".into(), NoticeKind::Fired), ("STUCK".into(), NoticeKind::Resolved)]);
    }

    #[test]
    fn warn_level_and_short_blips_are_not_sent() {
        let mut h = Harness::new(3, 0);
        h.tick(&[warning(Level::Warn, "LAG", "LAG 9".into(), "d".into())]);
        h.tick(&[crit("STUCK")]);
        h.tick(&[Check::Clear("STUCK")]);
        assert!(h.sent().is_empty());
        assert_eq!(h.resolved(), vec!["Resolved: STUCK (1s)"]);
    }

    #[test]
    fn cooldown_counts_suppressed_repeats_and_skips_their_resolves() {
        let mut h = Harness::new(1, 60_000);
        h.tick(&[crit("STUCK")]);
        h.tick(&[Check::Clear("STUCK")]);
        h.tick(&[crit("STUCK")]); // within cooldown: suppressed
        h.tick(&[Check::Clear("STUCK")]); // its resolve is not sent either
        assert_eq!(h.sent(), vec![("STUCK".into(), NoticeKind::Fired), ("STUCK".into(), NoticeKind::Resolved)]);
        *h.clock.lock().unwrap() += 60_000;
        h.tick(&[crit("STUCK")]);
        let sent = h.sent.lock().unwrap();
        assert_eq!(sent.len(), 3);
        assert_eq!(sent[2].suppressed, 1, "the next message mentions the skipped repeat");
    }

    fn input<'a>(rpc: Option<RpcObs>, metrics: MetricsState) -> WarnInput<'a> {
        WarnInput { rpc, metrics, lag: None, exec_p99: None, rounds: None, bft_silent_ms: None }
    }

    fn state<'a>(checks: &'a [Check], key: &str) -> &'a Check {
        checks.iter().find(|c| c.key() == key).unwrap()
    }

    #[test]
    fn rules_are_unknown_without_their_source() {
        let t = crate::config::DEFAULT_THRESHOLDS;
        let down = check_warnings(&input(None, MetricsState::Pending), &t);
        assert!(matches!(state(&down, "RPC_DOWN"), Check::Firing(_)));
        for key in ["METRICS_DOWN", "STUCK", "LOW_PEERS", "SLOW_RPC", "EXEC_SLOW", "MISSED_ROUND", "BFT_LOG_SILENT"] {
            assert_eq!(state(&down, key), &Check::Unknown(key), "{key} while RPC is down");
        }

        // Metrics down, RPC fine: block-based rules still work.
        let rpc = RpcObs { block_delta: Some(0), stalled_ms: 9000, rpc_ms: 5 };
        let m_down = check_warnings(&input(Some(rpc), MetricsState::Down), &t);
        assert!(matches!(state(&m_down, "METRICS_DOWN"), Check::Firing(_)));
        assert!(matches!(state(&m_down, "STUCK"), Check::Firing(_)));
        assert_eq!(state(&m_down, "LOW_PEERS"), &Check::Unknown("LOW_PEERS"));
        assert_eq!(state(&m_down, "NOT_VOTING"), &Check::Unknown("NOT_VOTING"));
    }

    #[test]
    fn bft_log_silence_is_reported_once_the_journal_was_seen() {
        let t = crate::config::DEFAULT_THRESHOLDS;
        let rpc = Some(RpcObs::default());
        let mut i = input(rpc, MetricsState::Pending);
        assert_eq!(state(&check_warnings(&i, &t), "BFT_LOG_SILENT"), &Check::Clear("BFT_LOG_SILENT"), "never seen: DEBUG is just off");
        i.bft_silent_ms = Some(5_000);
        assert_eq!(state(&check_warnings(&i, &t), "BFT_LOG_SILENT"), &Check::Clear("BFT_LOG_SILENT"));
        i.bft_silent_ms = Some(120_000);
        assert!(matches!(state(&check_warnings(&i, &t), "BFT_LOG_SILENT"), Check::Firing(_)));
    }
}
