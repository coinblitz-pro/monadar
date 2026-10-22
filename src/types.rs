use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thresholds {
    pub peers_min: f64,
    pub p99_max: f64,
    pub lag_max: f64,
    pub rpc_max: f64,
    pub tc_max: f64,
    pub exec_ms_max: f64,
    pub stuck_sec: f64,
    pub net_tc_pct_max: f64,
    pub net_tc_run_max: f64,
}

pub const THRESHOLD_KEYS: [&str; 9] =
    ["peers_min", "p99_max", "lag_max", "rpc_max", "tc_max", "exec_ms_max", "stuck_sec", "net_tc_pct_max", "net_tc_run_max"];

impl Thresholds {
    pub fn set(&mut self, key: &str, v: f64) {
        match key {
            "peers_min" => self.peers_min = v,
            "p99_max" => self.p99_max = v,
            "lag_max" => self.lag_max = v,
            "rpc_max" => self.rpc_max = v,
            "tc_max" => self.tc_max = v,
            "exec_ms_max" => self.exec_ms_max = v,
            "stuck_sec" => self.stuck_sec = v,
            "net_tc_pct_max" => self.net_tc_pct_max = v,
            "net_tc_run_max" => self.net_tc_run_max = v,
            _ => {}
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExecStats {
    pub count: u64,
    pub p50: i64,
    pub p99: i64,
    pub max: i64,
}

// p50/p99 of a delay over the last 10 minutes, ms.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Latency {
    pub p50: i64,
    pub p99: i64,
}

// Leader-round counters (bft_log.rs). led/ok/missed/failed/not_received/committed/txs
// are for our node; net_rounds/net_missed are network-wide (rounds seen / rounds ended by TC).
// late_tmo: rounds our node timed out on while the network completed them with a QC.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoundAgg {
    pub led: u64,
    pub ok: u64,
    pub missed: u64,
    pub failed: u64,
    pub not_received: u64,
    pub committed: u64,
    pub txs: u64,
    pub net_rounds: u64,
    pub net_missed: u64,
    pub late_tmo: u64,
    // Block size vs network: the size of each of our proposals, and the median
    // size of the network blocks right before it (load changes over time, so we
    // compare with neighbours; the median ignores outliers). Size is gas used
    // (ledger-tail) or the tx count (bft journal). size_samples = our blocks counted.
    pub size_our_txs: f64,
    pub size_peer_txs: f64,
    pub size_samples: u64,
    // Transactions other nodes forwarded to our txpool (our mempool intake).
    pub fwd_txs: u64,
}

impl std::ops::AddAssign<&RoundAgg> for RoundAgg {
    fn add_assign(&mut self, a: &RoundAgg) {
        self.led += a.led;
        self.ok += a.ok;
        self.missed += a.missed;
        self.failed += a.failed;
        self.not_received += a.not_received;
        self.committed += a.committed;
        self.txs += a.txs;
        self.net_rounds += a.net_rounds;
        self.net_missed += a.net_missed;
        self.late_tmo += a.late_tmo;
        self.size_our_txs += a.size_our_txs;
        self.size_peer_txs += a.size_peer_txs;
        self.size_samples += a.size_samples;
        self.fwd_txs += a.fwd_txs;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetWindow {
    pub rounds: u64,
    pub tc: u64,
    pub max_tc_run: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MissedRound {
    pub round: i64,
    pub t: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoundSummary {
    pub h1: RoundAgg,
    pub h24: RoundAgg,
    pub missed10m: u64,
    pub late_tmo10m: u64,
    // Network health over the last 10 min: rounds seen, rounds ended by TC,
    // and the longest run of consecutive TC rounds.
    pub net10m: NetWindow,
    pub last_missed: Option<MissedRound>,
    pub rounds_per_sec: f64,
    // monad-bft logs at DEBUG: late timeouts, unseen rounds and forwarded txs
    // come only from there.
    #[serde(default)]
    pub bft_debug: bool,
    // From ledger-tail: how late proposals reach us (now - block timestamp), and
    // how long blocks take to finalize.
    #[serde(default)]
    pub proposal_delay: Option<Latency>,
    #[serde(default)]
    pub finality: Option<Latency>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Warn,
    Crit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventLevel {
    Warn,
    Crit,
    Error,
    Resolved,
}

impl EventLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            EventLevel::Warn => "warn",
            EventLevel::Crit => "crit",
            EventLevel::Error => "error",
            EventLevel::Resolved => "resolved",
        }
    }

    pub fn parse(s: &str) -> EventLevel {
        match s {
            "crit" => EventLevel::Crit,
            "error" => EventLevel::Error,
            "resolved" => EventLevel::Resolved,
            _ => EventLevel::Warn,
        }
    }
}

impl From<Level> for EventLevel {
    fn from(l: Level) -> Self {
        match l {
            Level::Warn => EventLevel::Warn,
            Level::Crit => EventLevel::Crit,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Warning {
    pub level: Level,
    pub key: String,
    pub short: String,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    // Local `YYYY-MM-DD HH:MM:SS` once recorded; shown via event_time().
    pub time: String,
    pub level: EventLevel,
    pub text: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CounterCell {
    pub value: Option<f64>,
    pub delta: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FrameError {
    Rpc,
    Metrics,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Crit,
    Warn,
    Ok,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Frame {
    pub ts: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<FrameError>,
    pub block: i64,
    pub block_delta: Option<i64>,
    pub bps: String,
    pub ver: String,
    pub uptime: String,
    pub syncing: Option<f64>,
    pub sync_pct: Option<String>,
    pub peers: Option<f64>,
    pub sessions: Option<f64>,
    pub votes: CounterCell,
    pub timeouts: CounterCell,
    pub tc: CounterCell,
    // Growth of rare counters over a window: their per-tick deltas are 0 on
    // almost every tick. None until the counter is seen.
    #[serde(default)]
    pub timeouts_10m: Option<f64>,
    #[serde(default)]
    pub tc_10m: Option<f64>,
    #[serde(default)]
    pub fwd_10m: Option<f64>,
    #[serde(default)]
    pub fwd_1h: Option<f64>,
    #[serde(default)]
    pub fwd_24h: Option<f64>,
    pub exec_lag: i64,
    pub exec_block: Option<i64>,
    // exec_block came from monad_execution_ledger_block_num, which is the
    // finalized height (2 behind the proposed head), not the executed one.
    #[serde(default)]
    pub exec_finalized: bool,
    // Block execution time (start of execution → done) over the last 60s, from
    // monad-execution's journal. None when no recent log lines.
    pub exec: Option<ExecStats>,
    // Our leader rounds and network round stats (ledger-tail, bft journal).
    pub rounds: Option<RoundSummary>,
    pub tx: CounterCell,
    // Transactions and our votes per second over the last 30s.
    #[serde(default)]
    pub tps: Option<f64>,
    #[serde(default)]
    pub votes_per_sec: Option<f64>,
    pub txpool: Option<f64>,
    // Largest txpool size seen in the last 10 minutes: the pool drains within a
    // second (any leader's block removes its txs), so the current value is
    // almost always 0.
    #[serde(default)]
    pub txpool_max: Option<f64>,
    pub fwd: CounterCell,
    pub p99: Option<f64>,
    pub rpc_ms: i64,
    pub key: String,
    pub network: String,
    pub stake_pct: String,
    pub proposals: CounterCell,
    pub leader_pct: String,
    // Votes we created per proposal handled, over the last 10 minutes.
    pub voted_pct: Option<f64>,
    pub build_version: String,
    pub status: Status,
    pub new_crit: bool,
    pub warnings: Vec<Warning>,
    pub history: Vec<HistoryEntry>,
    pub log_line: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ServerMsg {
    Hello { backlog: Vec<String>, frame: Option<Box<Frame>> },
    Snapshot { frame: Box<Frame> },
}

// Node state attached to an alert. None/"?" fields are unknown (node down)
// and left out of the message.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub block: Option<i64>,
    pub block_delta: Option<i64>,
    pub bps: String,
    pub lag: Option<i64>,
    pub exec_p99: Option<i64>,
    pub led24h: Option<u64>,
    pub missed24h: Option<u64>,
    pub peers: Option<f64>,
    pub p99: Option<f64>,
    pub rpc_ms: Option<i64>,
    pub syncing: Option<f64>,
    pub sync_pct: Option<String>,
    pub uptime: Option<String>,
    pub ver: Option<String>,
}
