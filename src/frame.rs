use std::collections::VecDeque;

use crate::alerts::{Check, MetricObs, MetricsState, RpcObs, WarnInput, check_warnings, firing};
use crate::bft_log::size_ratio;
use crate::config::BUILD_VERSION;
use crate::exec_log::ExecSnapshot;
use crate::format::{delta, format_uptime, num, opt_num};
use crate::metric_snapshot::MetricsSnapshot;
use crate::types::{CounterCell, Frame, FrameError, HistoryEntry, Level, RoundSummary, Snapshot, Status, Thresholds};

const RATE_WINDOW: usize = 30;

#[derive(Default)]
pub struct CollectorState {
    prev_block: Option<i64>,
    prev_commits: Option<f64>,
    prev_votes: Option<f64>,
    prev_timeouts: Option<f64>,
    prev_fwd: Option<f64>,
    prev_lagging: Option<f64>,
    prev_tc: Option<f64>,
    prev_tx_commits: Option<f64>,
    prev_proposals: Option<f64>,
    prev_uptime_us: Option<f64>,
    exec_lag: i64,
    block_samples: VecDeque<(i64, i64)>, // (time ms, block)
    last_advance_ms: Option<i64>,
}

pub struct BlockSample {
    pub block_delta: Option<i64>,
    pub bps: String,
    // How long the block number has not changed. A single timed-out round
    // already pauses blocks for ~1-1.5s, so one equal tick is not "stuck".
    pub stalled_ms: i64,
}

pub struct BuiltFrame {
    pub frame: Frame,
    pub checks: Vec<Check>,
    pub snap: Snapshot,
    pub restarted: bool,
    pub uptime: String,
}

pub struct TickInput<'a> {
    pub ts: String,
    pub block: i64,
    pub block_delta: Option<i64>,
    pub bps: String,
    pub rpc_ms: i64,
    pub metrics: &'a MetricsSnapshot,
    pub thresholds: &'a Thresholds,
    pub exec: ExecSnapshot,
    pub rounds: Option<RoundSummary>,
    pub stalled_ms: i64,
    pub bft_silent_ms: Option<i64>,
}

pub fn status_of(warnings: &[crate::types::Warning]) -> Status {
    if warnings.iter().any(|w| w.level == Level::Crit) {
        Status::Crit
    } else if warnings.is_empty() {
        Status::Ok
    } else {
        Status::Warn
    }
}

fn counter(value: Option<f64>, prev: Option<f64>) -> CounterCell {
    CounterCell { value, delta: delta(value, prev) }
}

pub fn short_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 12 {
        return key.to_string();
    }
    let head: String = chars[..6].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}..{tail}")
}

pub fn record_block(state: &mut CollectorState, block: i64, now_ms: i64) -> BlockSample {
    let block_delta = state.prev_block.map(|p| block - p);
    state.prev_block = Some(block);
    if block_delta != Some(0) || state.last_advance_ms.is_none() {
        state.last_advance_ms = Some(now_ms);
    }
    let stalled_ms = now_ms - state.last_advance_ms.unwrap_or(now_ms);

    state.block_samples.push_back((now_ms, block));
    if state.block_samples.len() > RATE_WINDOW {
        state.block_samples.pop_front();
    }
    let (&(t0, b0), &(t1, b1)) = (state.block_samples.front().unwrap(), state.block_samples.back().unwrap());
    let dt = (t1 - t0) as f64 / 1000.0;
    if state.block_samples.len() < 2 || dt == 0.0 {
        return BlockSample { block_delta, bps: "?".into(), stalled_ms };
    }
    BlockSample { block_delta, bps: format!("{:.2}", (b1 - b0) as f64 / dt), stalled_ms }
}

impl CollectorState {
    fn reset_node_counters(&mut self) {
        self.prev_commits = None;
        self.prev_votes = None;
        self.prev_timeouts = None;
        self.prev_tc = None;
        self.prev_lagging = None;
        self.prev_tx_commits = None;
        self.prev_proposals = None;
        self.prev_fwd = None;
    }
}

struct LogInput<'a> {
    block: i64,
    bps: &'a str,
    rpc_ms: i64,
    metrics: &'a MetricsSnapshot,
    uptime: &'a str,
    stake_pct: &'a str,
    sync_pct: Option<&'a str>,
    exec_lag: i64,
    exec_block: Option<i64>,
    exec: &'a ExecSnapshot,
    rounds: Option<&'a RoundSummary>,
    leader_pct: &'a str,
    voted_pct: Option<f64>,
    warning_shorts: Vec<&'a str>,
}

fn log_line(i: LogInput) -> String {
    let m = i.metrics;
    let mut parts = vec![
        chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
        format!("block={}", i.block),
        format!("bps={}", i.bps),
        format!("ver={}", m.ver),
        format!("up={}", i.uptime),
        format!("peers={}", opt_num(m.peers)),
        format!("stake={}", i.stake_pct),
        format!("sessions={}", opt_num(m.sessions)),
        format!("sync={}", if m.syncing == Some(1.0) { "yes" } else { "no" }),
    ];
    if let Some(p) = i.sync_pct {
        parts.push(format!("sync_pct={p}%"));
    }
    parts.extend([
        format!("commits={}", opt_num(m.commits)),
        format!("votes={}", opt_num(m.votes)),
        format!("tmo={}", opt_num(m.timeouts)),
        format!("tc={}", opt_num(m.tc)),
        format!("lag={}", i.exec_lag),
        format!("lag_evt={}", num(m.lagging.unwrap_or(0.0))),
        format!("exec={}", i.exec_block.map(|b| b.to_string()).unwrap_or("?".into())),
    ]);
    if let Some(e) = i.exec.stats {
        parts.extend([
            format!("exec_p50={}ms", e.p50),
            format!("exec_p99={}ms", e.p99),
            format!("exec_max={}ms", e.max),
            format!("exec_n={}", e.count),
        ]);
    }
    parts.push(format!("tx_commits={}", opt_num(m.tx_commits)));
    if let Some(d) = i.rounds.map(|r| &r.h24) {
        parts.extend([
            format!("led24={}", d.led),
            format!("ok24={}", d.ok),
            format!("missed24={}", d.missed),
            format!("failed24={}", d.failed),
            format!("unseen24={}", d.not_received),
            format!("blocks24={}", d.committed),
            format!("txs24={}", d.txs),
            format!("net_tc24={}/{}", d.net_missed, d.net_rounds),
            format!("late24={}", d.late_tmo),
            format!("size_vs_net24={}", size_ratio(d).map(|r| format!("{r:.2}")).unwrap_or("?".into())),
            format!("fwd24={}", d.fwd_txs),
        ]);
    }
    parts.extend([
        format!("txpool={}", opt_num(m.txpool)),
        format!("fwd={}", opt_num(m.fwd)),
        format!("p99={}ms", opt_num(m.lat_p99)),
        format!("rpc={}ms", i.rpc_ms),
        format!("key={}", short_key(&m.node_key)),
        format!("net={}", m.network),
    ]);
    if let Some(p) = m.proposals {
        parts.push(format!("prop={}", num(p)));
    }
    if let Some(h) = m.handled {
        parts.push(format!("handled={}", num(h)));
    }
    parts.push(format!("leader={}", i.leader_pct));
    parts.push(format!("voted={}", i.voted_pct.map(|v| format!("{v:.1}%")).unwrap_or("?".into())));
    let warn_tags = if i.warning_shorts.is_empty() { String::new() } else { format!(" WARN={}", i.warning_shorts.join(",")) };
    parts.join(" ") + &warn_tags
}

pub fn build_tick_frame(state: &mut CollectorState, input: TickInput) -> BuiltFrame {
    let m = input.metrics;
    let uptime = m.uptime_us.map(format_uptime).unwrap_or("?".into());
    let stake_pct = m.stake_bps.map(|b| format!("{:.2}%", b / 100.0)).unwrap_or("?".into());

    let restarted = matches!((state.prev_uptime_us, m.uptime_us), (Some(prev), Some(cur)) if cur < prev);
    state.prev_uptime_us = m.uptime_us;
    if restarted {
        state.reset_node_counters();
    }

    let dv = delta(m.votes, state.prev_votes);
    let dt = delta(m.timeouts, state.prev_timeouts);
    let dtc = delta(m.tc, state.prev_tc);
    let dl = delta(m.lagging, state.prev_lagging);
    let dc = delta(m.commits, state.prev_commits);

    // Prefer the execution journal: it's real-time, whereas the metric lags by
    // the OTLP export interval (~2-3 blocks at 0.4s/block).
    let exec = input.exec.stats;
    let exec_block = input.exec.last_block.or(m.exec_block.map(|b| crate::format::js_round(b) as i64));
    let exec_lag = match (exec_block, dc, input.block_delta) {
        (Some(eb), _, _) => (input.block - eb).max(0),
        (None, Some(dc), Some(bd)) => ((state.exec_lag + bd) as f64 - dc).max(0.0) as i64,
        _ => state.exec_lag,
    };
    state.exec_lag = exec_lag;

    let sync_pct = match (m.syncing, m.sync_progress, m.sync_target) {
        (Some(s), Some(p), Some(t)) if s == 1.0 && t > 0.0 => Some(format!("{:.2}", p / t * 100.0)),
        _ => None,
    };

    let rounds = input.rounds;
    let checks = check_warnings(
        &WarnInput {
            rpc: Some(RpcObs { block_delta: input.block_delta, stalled_ms: input.stalled_ms, rpc_ms: input.rpc_ms }),
            metrics: MetricsState::Up(MetricObs { dv, dt, dtc, syncing: m.syncing, peers: m.peers, lat_p99: m.lat_p99 }),
            lag: Some(exec_lag),
            exec_p99: exec.map(|e| e.p99),
            rounds: rounds.as_ref(),
            bft_silent_ms: input.bft_silent_ms,
        },
        input.thresholds,
    );
    let warnings = firing(&checks);
    let status = status_of(&warnings);

    let snap = Snapshot {
        block: Some(input.block),
        block_delta: input.block_delta,
        bps: input.bps.clone(),
        lag: Some(exec_lag),
        exec_p99: exec.map(|e| e.p99),
        led24h: rounds.as_ref().map(|r| r.h24.led),
        missed24h: rounds.as_ref().map(|r| r.h24.missed + r.h24.failed),
        peers: m.peers,
        p99: m.lat_p99,
        rpc_ms: Some(input.rpc_ms),
        syncing: m.syncing,
        sync_pct: sync_pct.clone(),
        uptime: m.uptime_us.map(|_| uptime.clone()),
        ver: Some(m.ver.clone()).filter(|v| v != "?"),
    };

    let leader_pct = match (m.proposals, m.handled) {
        (Some(p), Some(h)) if h > 0.0 => format!("{:.2}%", p / h * 100.0),
        _ => "?".into(),
    };
    let voted_pct = match (m.votes, m.handled) {
        (Some(v), Some(h)) if h > 0.0 => Some(v / h * 100.0),
        _ => None,
    };

    let log = log_line(LogInput {
        block: input.block,
        bps: &input.bps,
        rpc_ms: input.rpc_ms,
        metrics: m,
        uptime: &uptime,
        stake_pct: &stake_pct,
        sync_pct: sync_pct.as_deref(),
        exec_lag,
        exec_block,
        exec: &input.exec,
        rounds: rounds.as_ref(),
        leader_pct: &leader_pct,
        voted_pct,
        warning_shorts: warnings.iter().map(|w| w.short.as_str()).collect(),
    });

    let frame = Frame {
        ts: input.ts,
        error: None,
        block: input.block,
        block_delta: input.block_delta,
        bps: input.bps,
        ver: m.ver.clone(),
        uptime: uptime.clone(),
        syncing: m.syncing,
        sync_pct,
        peers: m.peers,
        sessions: m.sessions,
        commits: counter(m.commits, state.prev_commits),
        votes: counter(m.votes, state.prev_votes),
        timeouts: counter(m.timeouts, state.prev_timeouts),
        tc: counter(m.tc, state.prev_tc),
        exec_lag,
        lag_evt_delta: dl,
        exec_block,
        exec,
        rounds,
        tx: counter(m.tx_commits, state.prev_tx_commits),
        txpool: m.txpool,
        fwd: counter(m.fwd, state.prev_fwd),
        p99: m.lat_p99,
        rpc_ms: input.rpc_ms,
        key: short_key(&m.node_key),
        network: m.network.clone(),
        stake_pct,
        proposals: counter(m.proposals, state.prev_proposals),
        leader_pct,
        voted_pct,
        build_version: BUILD_VERSION.into(),
        status,
        new_crit: false,
        warnings,
        history: Vec::new(),
        log_line: log,
    };

    state.prev_commits = m.commits.or(state.prev_commits);
    state.prev_votes = m.votes.or(state.prev_votes);
    state.prev_timeouts = m.timeouts.or(state.prev_timeouts);
    state.prev_fwd = m.fwd.or(state.prev_fwd);
    state.prev_lagging = m.lagging.or(state.prev_lagging);
    state.prev_tc = m.tc.or(state.prev_tc);
    state.prev_tx_commits = m.tx_commits.or(state.prev_tx_commits);
    state.prev_proposals = m.proposals.or(state.prev_proposals);

    BuiltFrame { frame, checks, snap, restarted, uptime }
}

pub fn error_frame(ts: String, history: Vec<HistoryEntry>, error: FrameError, block: Option<(i64, Option<i64>, String)>) -> Frame {
    let (block, block_delta, bps) = block.unwrap_or((0, None, "?".into()));
    Frame {
        ts,
        error: Some(error),
        block,
        block_delta,
        bps,
        ver: "?".into(),
        uptime: "?".into(),
        syncing: None,
        sync_pct: None,
        peers: None,
        sessions: None,
        commits: CounterCell::default(),
        votes: CounterCell::default(),
        timeouts: CounterCell::default(),
        tc: CounterCell::default(),
        exec_lag: 0,
        lag_evt_delta: None,
        exec_block: None,
        exec: None,
        rounds: None,
        tx: CounterCell::default(),
        txpool: None,
        fwd: CounterCell::default(),
        p99: None,
        rpc_ms: 0,
        key: "?".into(),
        network: "?".into(),
        stake_pct: "?".into(),
        proposals: CounterCell::default(),
        leader_pct: "?".into(),
        voted_pct: None,
        build_version: BUILD_VERSION.into(),
        status: Status::Ok,
        new_crit: false,
        warnings: Vec::new(),
        history,
        log_line: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DEFAULT_THRESHOLDS;
    use crate::types::{ExecStats, MissedRound, NetWindow, RoundAgg};

    const NET_OK: NetWindow = NetWindow { rounds: 1500, tc: 10, max_tc_run: 1 };
    const T: Thresholds = DEFAULT_THRESHOLDS;

    fn metrics() -> MetricsSnapshot {
        MetricsSnapshot {
            ver: "0.14.4".into(),
            peers: Some(100.0),
            stake_bps: Some(43.0),
            sessions: Some(10.0),
            syncing: Some(0.0),
            uptime_us: Some(10_000_000.0),
            commits: Some(100.0),
            votes: Some(100.0),
            timeouts: Some(0.0),
            tc: Some(0.0),
            lagging: Some(0.0),
            proposals: Some(1.0),
            handled: Some(100.0),
            exec_block: Some(100.0),
            tx_commits: Some(1000.0),
            txpool: Some(0.0),
            fwd: Some(0.0),
            lat_p99: Some(100.0),
            sync_progress: None,
            sync_target: None,
            node_key: "0x1234567890abcdef".into(),
            network: "testnet".into(),
        }
    }

    fn tick(
        state: &mut CollectorState,
        block: i64,
        now_ms: i64,
        m: &MetricsSnapshot,
        exec: ExecSnapshot,
        rounds: Option<RoundSummary>,
    ) -> BuiltFrame {
        let s = record_block(state, block, now_ms);
        build_tick_frame(
            state,
            TickInput {
                ts: "12:00:00".into(),
                block,
                block_delta: s.block_delta,
                bps: s.bps,
                rpc_ms: 10,
                metrics: m,
                thresholds: &T,
                exec,
                rounds,
                stalled_ms: s.stalled_ms,
                bft_silent_ms: None,
            },
        )
    }

    fn keys(f: &Frame) -> Vec<&str> {
        f.warnings.iter().map(|w| w.key.as_str()).collect()
    }

    fn summary(a: RoundAgg, missed10m: u64, late_tmo10m: u64, net10m: NetWindow, last_missed: Option<MissedRound>) -> RoundSummary {
        RoundSummary { h1: a, h24: a, missed10m, late_tmo10m, net10m, last_missed, rounds_per_sec: 2.5 }
    }

    fn net_agg(net_rounds: u64, net_missed: u64) -> RoundAgg {
        RoundAgg { net_rounds, net_missed, ..RoundAgg::default() }
    }

    #[test]
    fn execution_lag_is_consensus_minus_execution_block() {
        let mut st = CollectorState::default();
        let b = tick(&mut st, 110, 0, &MetricsSnapshot { exec_block: Some(100.0), ..metrics() }, ExecSnapshot::default(), None);
        assert_eq!(b.frame.exec_lag, 10);
        assert_eq!(b.frame.status, Status::Warn);
        assert_eq!(keys(&b.frame), vec!["LAG"]);
    }

    #[test]
    fn fallback_lag_grows_when_block_delta_exceeds_commit_delta() {
        let mut st = CollectorState::default();
        tick(&mut st, 100, 0, &MetricsSnapshot { exec_block: None, commits: Some(100.0), ..metrics() }, ExecSnapshot::default(), None);
        let b = tick(
            &mut st,
            105,
            1000,
            &MetricsSnapshot { exec_block: None, commits: Some(102.0), ..metrics() },
            ExecSnapshot::default(),
            None,
        );
        assert_eq!(b.frame.exec_lag, 3);
        assert_eq!(b.frame.bps, "5.00");
    }

    #[test]
    fn prefers_the_execution_journal_over_the_lagging_metric() {
        let mut st = CollectorState::default();
        let exec = ExecSnapshot { stats: Some(ExecStats { count: 150, p50: 5, p99: 49, max: 136 }), last_block: Some(110) };
        let b = tick(&mut st, 110, 0, &MetricsSnapshot { exec_block: Some(100.0), ..metrics() }, exec, None);
        assert_eq!(b.frame.exec_block, Some(110));
        assert_eq!(b.frame.exec_lag, 0);
        assert_eq!(b.frame.exec, exec.stats);
        assert_eq!(b.snap.exec_p99, Some(49));
        assert_eq!(b.frame.status, Status::Ok);
        assert!(b.frame.log_line.contains("exec_p50=5ms exec_p99=49ms exec_max=136ms exec_n=150"));
    }

    #[test]
    fn raises_critical_missed_round_when_our_rounds_timed_out_recently() {
        let mut st = CollectorState::default();
        let a = RoundAgg { led: 3, ok: 2, missed: 1, committed: 2, txs: 20, net_rounds: 1000, net_missed: 5, ..RoundAgg::default() };
        let rounds = summary(a, 1, 0, NET_OK, Some(MissedRound { round: 5, t: 0 }));
        let b = tick(&mut st, 110, 0, &MetricsSnapshot { exec_block: Some(110.0), ..metrics() }, ExecSnapshot::default(), Some(rounds));
        assert_eq!(keys(&b.frame), vec!["MISSED_ROUND"]);
        assert_eq!(b.frame.status, Status::Crit);
        assert_eq!(b.snap.led24h, Some(3));
        assert_eq!(b.snap.missed24h, Some(1));
        assert!(b.frame.log_line.contains("led24=3 ok24=2 missed24=1 failed24=0 unseen24=0 blocks24=2 txs24=20 net_tc24=5/1000"));
    }

    #[test]
    fn with_round_data_network_timeouts_are_not_warnings_but_late_ones_are() {
        let check = |late_tmo10m: u64, with_rounds: bool| -> Vec<String> {
            let mut st = CollectorState::default();
            tick(&mut st, 110, 0, &MetricsSnapshot { exec_block: Some(110.0), ..metrics() }, ExecSnapshot::default(), None);
            // another leader missed: both counters grew by one
            let m = MetricsSnapshot { exec_block: Some(112.0), timeouts: Some(1.0), tc: Some(1.0), votes: Some(101.0), ..metrics() };
            let rounds = with_rounds.then(|| summary(net_agg(1000, 5), 0, late_tmo10m, NET_OK, None));
            let b = tick(&mut st, 112, 1000, &m, ExecSnapshot::default(), rounds);
            keys(&b.frame).iter().map(|s| s.to_string()).collect()
        };
        assert_eq!(check(0, false), vec!["TMO", "TC"], "metric fallback without journal data");
        assert!(check(0, true).is_empty(), "network misses are not our problem");
        assert_eq!(check(2, true), vec!["LATE"]);
    }

    #[test]
    fn warns_when_execution_latency_p99_exceeds_exec_ms_max() {
        let mut st = CollectorState::default();
        let exec = ExecSnapshot { stats: Some(ExecStats { count: 150, p50: 40, p99: 300, max: 900 }), last_block: Some(110) };
        let b = tick(&mut st, 110, 0, &metrics(), exec, None);
        assert_eq!(keys(&b.frame), vec!["EXEC_SLOW"]);
    }

    #[test]
    fn net_degraded_fires_on_abnormal_tc_share_or_back_to_back_tcs() {
        let warn = |net10m: NetWindow| -> Vec<crate::types::Warning> {
            let mut st = CollectorState::default();
            let rounds = summary(net_agg(1500, 10), 0, 0, net10m, None);
            let b = tick(&mut st, 110, 0, &MetricsSnapshot { exec_block: Some(110.0), ..metrics() }, ExecSnapshot::default(), Some(rounds));
            b.frame.warnings.into_iter().filter(|w| w.key == "NET_DEGRADED").collect()
        };
        assert!(warn(NetWindow { rounds: 1500, tc: 10, max_tc_run: 1 }).is_empty(), "normal: ~0.7%, single TCs");
        let pct = warn(NetWindow { rounds: 1500, tc: 60, max_tc_run: 1 });
        assert_eq!(pct[0].short, "NET TC 4.0%");
        assert_eq!(pct[0].level, Level::Warn);
        assert!(pct[0].detail.contains("not our node"));
        assert_eq!(warn(NetWindow { rounds: 1500, tc: 12, max_tc_run: 3 })[0].short, "NET TC ×3 in a row");
        assert!(warn(NetWindow { rounds: 50, tc: 10, max_tc_run: 1 }).is_empty(), "too few rounds for a share");
    }

    #[test]
    fn stuck_needs_the_block_unchanged_for_stuck_sec() {
        let mut st = CollectorState::default();
        let mut stuck_at = |block: i64, now_ms: i64| -> bool {
            let b = tick(
                &mut st,
                block,
                now_ms,
                &MetricsSnapshot { exec_block: Some(block as f64), ..metrics() },
                ExecSnapshot::default(),
                None,
            );
            b.frame.warnings.iter().any(|w| w.key == "STUCK")
        };
        stuck_at(100, 0);
        // A timed-out round: same block for 1.5s → not stuck.
        assert!(!stuck_at(100, 1500));
        assert!(!stuck_at(100, 4000));
        assert!(stuck_at(100, 5000));
        assert!(!stuck_at(101, 6000), "clears once blocks advance");
    }
}
