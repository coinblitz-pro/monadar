use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, LazyLock, Mutex};

use regex::Regex;
use serde_json::Value;

use crate::journal::follow_journal;
use crate::ledger_log::LedgerEvent;
use crate::types::{Latency, MissedRound, NetWindow, RoundAgg, RoundSummary};

// Leader-round accounting from monad-bft's DEBUG journal. Every round logs:
//   "leader for round"  {round, round_leader: "Some(<secp key>)"}  — who should propose
//   "proposal message"  {author, proposal: "... proposal_round: R ... seq_num: N ..."}
//   "advancing round"   {certificate: "Qc(QC { info: Vote { .. round: R }" | "Tc(TimeoutCertificate { epoch: E, round: R"}
//   "committed block"   {block_num, num_tx}
//   "local timeout"     {round, leader}  — our pacemaker gave up on the round
//   "proposal stats"    {txs, total_gas} — follows the "proposal message" it describes
//   "txpool executor received forwarded txs" {sender, num_txs}
//   "emitting create proposal command to txpool" {node_id} — only our own node logs it,
//     so it tells us our key without node_key in the config
// The round right after a TC has no "leader for round" line; its proposal's
// author is the leader. A round is classified once the chain is FINAL_LAG
// rounds past it, so its TC (logged when entering R+1) has surely arrived:
//   proposal, no TC → ok        no proposal, TC → missed
//   proposal + TC   → failed    neither         → not_received (we didn't see it)
// A local timeout on a round that got no TC means the network finished the
// round normally and only we were late — that's our problem (network/CPU).
// A local timeout on a TC round is just the leader's miss.
//
// monad-ledger-tail (ledger_log.rs) carries the same round data at INFO level:
// proposals with their author, timeouts with the round's leader, finalized
// blocks. While it is live it is the source of proposals, TCs and commits, and
// the bft journal adds only what ledger-tail lacks (leader schedule for unseen
// rounds, our local timeouts, forwarded txs, our key), all of it DEBUG-only.

const FINAL_LAG: i64 = 10;
const MINUTE: i64 = 60_000;
const DAY_MINUTES: i64 = 24 * 60;
const MISSED_ALERT_MINUTES: i64 = 10;
const MAX_OUR_SEQS: usize = 1000;
const SIZE_NEIGHBOURS: usize = 50;
// A source counts as live while its last line is this recent.
const LIVE_MS: i64 = 30_000;
const DEBUG_LIVE_MS: i64 = 60_000;
const LATENCY_WINDOW_MS: i64 = MISSED_ALERT_MINUTES * MINUTE;

#[derive(Clone, Debug, PartialEq)]
pub enum BftEvent {
    Leader { round: i64, leader: Option<String>, t: i64 },
    Proposal { round: i64, author: String, seq: Option<i64>, t: i64 },
    Tc { round: i64, t: i64 },
    LocalTmo { round: i64, t: i64 },
    Commit { block: i64, txs: u64, t: i64 },
    Stats { txs: u64, t: i64 },
    Fwd { txs: u64, t: i64 },
    SelfKey { key: String, t: i64 },
}

impl BftEvent {
    fn t(&self) -> i64 {
        match self {
            BftEvent::Leader { t, .. }
            | BftEvent::Proposal { t, .. }
            | BftEvent::Tc { t, .. }
            | BftEvent::LocalTmo { t, .. }
            | BftEvent::Commit { t, .. }
            | BftEvent::Stats { t, .. }
            | BftEvent::Fwd { t, .. }
            | BftEvent::SelfKey { t, .. } => *t,
        }
    }
}

const MARKERS: [&str; 8] = [
    "\"leader for round\"",
    "\"proposal message\"",
    "\"advancing round\"",
    "\"committed block\"",
    "\"local timeout\"",
    "\"proposal stats\"",
    "\"txpool executor received forwarded txs\"",
    "\"emitting create proposal command to txpool\"",
];

static LEADER_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^Some\((\w+)\)$").unwrap());
static ROUND_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"proposal_round: (\d+)").unwrap());
static SEQ_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"seq_num: (\d+)").unwrap());
static TC_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^Tc\(TimeoutCertificate \{ epoch: \d+, round: (\d+)").unwrap());

// Field as text, the way JS String() would print it (numbers without quotes).
fn text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

// Numeric field: monad-bft logs some as numbers, some as strings.
fn int(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn capture_int(re: &Regex, s: &str) -> Option<i64> {
    re.captures(s)?.get(1)?.as_str().parse().ok()
}

pub fn parse_bft_line(line: &str) -> Option<BftEvent> {
    // Cheap substring check first: the journal carries ~100 lines/s, most irrelevant.
    if !MARKERS.iter().any(|m| line.contains(m)) {
        return None;
    }
    let j: Value = serde_json::from_str(line).ok()?;
    let t = chrono::DateTime::parse_from_rfc3339(j.get("timestamp")?.as_str()?).ok()?.timestamp_millis();
    let f = j.get("fields")?;

    match f.get("message")?.as_str()? {
        "leader for round" => {
            let leader = LEADER_RE.captures(&text(f.get("round_leader"))).map(|m| m[1].to_string());
            Some(BftEvent::Leader { round: int(f.get("round"))?, leader, t })
        }
        "proposal message" => {
            let p = text(f.get("proposal"));
            let round = capture_int(&ROUND_RE, &p)?;
            Some(BftEvent::Proposal { round, author: text(f.get("author")), seq: capture_int(&SEQ_RE, &p), t })
        }
        "advancing round" => Some(BftEvent::Tc { round: capture_int(&TC_RE, &text(f.get("certificate")))?, t }),
        "committed block" => Some(BftEvent::Commit { block: int(f.get("block_num"))?, txs: count(f.get("num_tx")), t }),
        "local timeout" => Some(BftEvent::LocalTmo { round: int(f.get("round"))?, t }),
        "proposal stats" => Some(BftEvent::Stats { txs: count(f.get("txs")), t }),
        "txpool executor received forwarded txs" => Some(BftEvent::Fwd { txs: count(f.get("num_txs")), t }),
        "emitting create proposal command to txpool" => Some(BftEvent::SelfKey { key: text(f.get("node_id")), t }),
        _ => None,
    }
}

fn count(v: Option<&Value>) -> u64 {
    int(v).filter(|n| *n > 0).unwrap_or(0) as u64
}

#[derive(Clone, Debug)]
struct RoundInfo {
    leader: Option<String>,
    author: Option<String>,
    tc: bool,
    local_tmo: bool,
    t: i64,
}

#[derive(Clone, Copy, Debug)]
struct TcRun {
    start: i64,
    len: u64,
    t: i64,
}

fn median(xs: &VecDeque<f64>) -> f64 {
    let mut s: Vec<f64> = xs.iter().copied().collect();
    s.sort_by(f64::total_cmp);
    let m = s.len() / 2;
    if s.len() % 2 == 1 { s[m] } else { (s[m - 1] + s[m]) / 2.0 }
}

fn percentile(sorted: &[i64], p: f64) -> i64 {
    sorted[((sorted.len() as f64 * p) as usize).min(sorted.len() - 1)]
}

// Delays observed in the last LATENCY_WINDOW_MS.
#[derive(Default)]
struct DelayWindow(VecDeque<(i64, i64)>);

impl DelayWindow {
    fn push(&mut self, t: i64, ms: i64) {
        self.0.push_back((t, ms));
        while self.0.front().is_some_and(|(at, _)| *at < t - LATENCY_WINDOW_MS) {
            self.0.pop_front();
        }
    }

    fn latency(&self, now_ms: i64) -> Option<Latency> {
        let mut v: Vec<i64> = self.0.iter().filter(|(t, _)| *t >= now_ms - LATENCY_WINDOW_MS).map(|(_, ms)| *ms).collect();
        if v.is_empty() {
            return None;
        }
        v.sort_unstable();
        Some(Latency { p50: percentile(&v, 0.5), p99: percentile(&v, 0.99) })
    }
}

// Our block size relative to the network: 1 = same as neighbouring blocks.
pub fn size_ratio(a: &RoundAgg) -> Option<f64> {
    (a.size_samples > 0 && a.size_peer_txs > 0.0).then(|| a.size_our_txs / a.size_peer_txs)
}

#[derive(Default)]
pub struct RoundTracker {
    self_key: Option<String>,
    // node_key from the config: fixed, the journal only cross-checks it.
    self_configured: bool,
    // Learned from our own "emitting create proposal" line: beats the metric label.
    self_from_journal: bool,
    // Wall-clock time of the last journal line of any kind (not just parsed ones).
    last_line_ms: Option<i64>,
    // Wall-clock time of the last ledger-tail line.
    last_ledger_line_ms: Option<i64>,
    pending: BTreeMap<i64, RoundInfo>,
    buckets: BTreeMap<i64, RoundAgg>, // minute index → counters
    our_seqs: BTreeMap<i64, u64>,     // our proposed block → its tx count
    max_round: i64,
    last_missed: Option<MissedRound>,
    // Event time of the last round-level event from either source.
    last_round_t: Option<i64>,
    last_ledger_t: Option<i64>,
    last_debug_t: Option<i64>,
    last_proposal_author: Option<String>,
    // Sizes of the last SIZE_NEIGHBOURS network blocks (not ours), in gas when
    // size_in_gas, else in txs.
    peer_sizes: VecDeque<f64>,
    size_in_gas: bool,
    proposal_delays: DelayWindow,
    finality: DelayWindow,
    // Runs of consecutive TC rounds (≥2): several leaders in a row produced no
    // block, so the whole network stood still for seconds.
    last_final: i64,
    tc_run: u64,
    tc_runs: VecDeque<TcRun>,
}

impl RoundTracker {
    pub fn new() -> RoundTracker {
        RoundTracker::default()
    }

    fn valid_key(key: Option<&str>) -> Option<String> {
        key.filter(|k| !k.is_empty() && k.bytes().all(|b| b.is_ascii_hexdigit())).map(|k| k.to_ascii_lowercase())
    }

    // node_key from the config.
    pub fn configure_self(&mut self, key: Option<&str>) {
        self.self_key = Self::valid_key(key);
        self.self_configured = self.self_key.is_some();
    }

    // The secp_key metric label: used only while nothing better is known.
    pub fn suggest_self(&mut self, key: Option<&str>) {
        if !self.self_configured && !self.self_from_journal {
            self.self_key = Self::valid_key(key);
        }
    }

    fn learn_self(&mut self, key: &str) {
        let Some(key) = Self::valid_key(Some(key)) else { return };
        if self.self_key.as_deref() == Some(key.as_str()) {
            self.self_from_journal = true;
        } else if self.self_configured {
            if !self.self_from_journal {
                eprintln!(
                    "bft: node_key in the config ({}) differs from the key this node proposes with ({key})",
                    self.self_key.as_deref().unwrap_or("")
                );
                self.self_from_journal = true;
            }
        } else {
            eprintln!("bft: our node key is {key} (from our own proposals)");
            self.self_key = Some(key);
            self.self_from_journal = true;
        }
    }

    pub fn note_line(&mut self, now_ms: i64) {
        self.last_line_ms = Some(now_ms);
    }

    // How long the bft journal has been quiet; None if it never produced a line.
    pub fn silent_ms(&self, now_ms: i64) -> Option<i64> {
        self.last_line_ms.map(|t| now_ms - t)
    }

    pub fn note_ledger_line(&mut self, now_ms: i64) {
        self.last_ledger_line_ms = Some(now_ms);
    }

    pub fn ledger_silent_ms(&self, now_ms: i64) -> Option<i64> {
        self.last_ledger_line_ms.map(|t| now_ms - t)
    }

    fn ledger_live(&self, t: i64) -> bool {
        self.last_ledger_t.is_some_and(|l| t - l < LIVE_MS)
    }

    fn is_self(&self, key: Option<&str>) -> bool {
        self.self_key.is_some() && self.self_key.as_deref() == key
    }

    fn bucket(&mut self, t: i64) -> &mut RoundAgg {
        let m = t.div_euclid(MINUTE);
        if let Entry::Vacant(e) = self.buckets.entry(m) {
            e.insert(RoundAgg::default());
            self.buckets = self.buckets.split_off(&(m - DAY_MINUTES));
        }
        self.buckets.get_mut(&m).unwrap()
    }

    fn info(&mut self, round: i64, t: i64) -> &mut RoundInfo {
        self.pending.entry(round).or_insert(RoundInfo { leader: None, author: None, tc: false, local_tmo: false, t })
    }

    fn finalize(&mut self, round: i64, r: RoundInfo) {
        let leader = r.leader.clone().or(r.author.clone());
        let proposed = r.author.is_some() && r.author == leader;
        let b = self.bucket(r.t);
        b.net_rounds += 1;
        if r.tc {
            b.net_missed += 1;
        }
        if r.local_tmo && !r.tc {
            b.late_tmo += 1;
        }
        self.track_tc_run(round, &r);
        if !self.is_self(leader.as_deref()) {
            return;
        }

        let b = self.bucket(r.t);
        b.led += 1;
        match (proposed, r.tc) {
            (true, false) => b.ok += 1,
            (true, true) => b.failed += 1,
            (false, true) => b.missed += 1,
            (false, false) => b.not_received += 1,
        }
        if r.tc {
            self.last_missed = Some(MissedRound { round, t: r.t });
        }
    }

    fn track_tc_run(&mut self, round: i64, r: &RoundInfo) {
        self.tc_run = if r.tc { (if round == self.last_final + 1 { self.tc_run } else { 0 }) + 1 } else { 0 };
        self.last_final = round;
        if self.tc_run < 2 {
            return;
        }
        let start = round - self.tc_run as i64 + 1;
        match self.tc_runs.back_mut() {
            Some(last) if last.start == start => last.len = self.tc_run,
            _ => self.tc_runs.push_back(TcRun { start, len: self.tc_run, t: r.t }),
        }
        while self.tc_runs.front().is_some_and(|x| x.t < r.t - DAY_MINUTES * MINUTE) {
            self.tc_runs.pop_front();
        }
    }

    fn block_size(&mut self, author: &str, size: f64, in_gas: bool, t: i64) {
        if in_gas != self.size_in_gas {
            self.size_in_gas = in_gas;
            self.peer_sizes.clear();
        }
        if self.is_self(Some(author)) {
            if self.peer_sizes.len() < SIZE_NEIGHBOURS / 2 {
                return; // not enough context yet
            }
            let peer = median(&self.peer_sizes);
            let b = self.bucket(t);
            b.size_our_txs += size;
            b.size_peer_txs += peer;
            b.size_samples += 1;
        } else {
            self.peer_sizes.push_back(size);
            if self.peer_sizes.len() > SIZE_NEIGHBOURS {
                self.peer_sizes.pop_front();
            }
        }
    }

    fn proposal(&mut self, round: i64, author: String, seq: Option<i64>, txs: u64, t: i64) {
        // Remember our block numbers now: the commit arrives a few rounds later,
        // well before the round is finalized.
        if let Some(seq) = seq.filter(|_| self.is_self(Some(&author))) {
            self.our_seqs.insert(seq, txs);
            if self.our_seqs.len() > MAX_OUR_SEQS {
                self.our_seqs.pop_first();
            }
        }
        self.info(round, t).author = Some(author);
    }

    fn commit(&mut self, block: i64, txs: Option<u64>, t: i64) {
        if let Some(proposed_txs) = self.our_seqs.remove(&block) {
            let b = self.bucket(t);
            b.committed += 1;
            b.txs += txs.unwrap_or(proposed_txs);
        }
    }

    pub fn ingest(&mut self, e: BftEvent) {
        let t = e.t();
        // Proposals, TCs, commits and block sizes come from ledger-tail while it is live.
        let ledger = self.ledger_live(t);
        let round = match e {
            BftEvent::Stats { txs, t } => {
                if let Some(author) = self.last_proposal_author.take().filter(|_| !ledger) {
                    self.block_size(&author, txs as f64, false, t);
                }
                return;
            }
            BftEvent::Fwd { txs, t } => {
                self.bucket(t).fwd_txs += txs;
                return;
            }
            BftEvent::SelfKey { key, .. } => return self.learn_self(&key),
            BftEvent::Commit { block, txs, t } => {
                if !ledger {
                    self.commit(block, Some(txs), t);
                }
                return;
            }
            BftEvent::Leader { round, leader, t } => {
                self.last_debug_t = Some(t);
                self.info(round, t).leader = leader;
                round
            }
            BftEvent::Proposal { round, author, seq, t } => {
                self.last_debug_t = Some(t);
                if ledger {
                    return;
                }
                self.last_proposal_author = Some(author.clone());
                self.proposal(round, author, seq, 0, t);
                round
            }
            BftEvent::LocalTmo { round, t } => {
                self.last_debug_t = Some(t);
                self.info(round, t).local_tmo = true;
                round
            }
            BftEvent::Tc { round, t } => {
                self.last_debug_t = Some(t);
                if ledger {
                    return;
                }
                self.info(round, t).tc = true;
                round
            }
        };
        self.advance(round, t);
    }

    pub fn ingest_ledger(&mut self, e: LedgerEvent) {
        let t = e.t();
        self.last_ledger_t = Some(t);
        let round = match e {
            LedgerEvent::Proposed { round, author, seq, txs, gas, block_ts, .. } => {
                self.proposal_delays.push(t, t - block_ts);
                self.block_size(&author, gas as f64, true, t);
                self.proposal(round, author, Some(seq), txs, t);
                round
            }
            LedgerEvent::Finalized { seq, block_ts, .. } => {
                self.finality.push(t, t - block_ts);
                return self.commit(seq, None, t);
            }
            // Logged for a round that ended by TC, with that round's leader.
            LedgerEvent::Timeout { round, leader, .. } => {
                let info = self.info(round, t);
                info.leader = Some(leader);
                info.tc = true;
                round
            }
        };
        self.advance(round, t);
    }

    fn advance(&mut self, round: i64, t: i64) {
        self.last_round_t = Some(t);
        if round > self.max_round {
            self.max_round = round;
            // Finalize in round order so consecutive-TC runs are detected correctly.
            while let Some(entry) = self.pending.first_entry() {
                if *entry.key() > self.max_round - FINAL_LAG {
                    break;
                }
                let (r, info) = entry.remove_entry();
                self.finalize(r, info);
            }
        }
    }

    fn sum(&self, from_minute: i64, to_minute: i64) -> RoundAgg {
        let mut a = RoundAgg::default();
        for b in self.buckets.range(from_minute..=to_minute).map(|(_, b)| b) {
            a += b;
        }
        a
    }

    // None until the first round-level event arrives: no ledger-tail and bft not
    // at DEBUG, or no journal access.
    pub fn summary(&self, now_ms: i64) -> Option<RoundSummary> {
        self.last_round_t?;
        let now = now_ms.div_euclid(MINUTE);
        let last10 = self.sum(now - MISSED_ALERT_MINUTES + 1, now);
        let prev_minute = self.sum(now - 1, now - 1);
        let max_tc_run = self.tc_runs.iter().filter(|x| x.t >= now_ms - MISSED_ALERT_MINUTES * MINUTE).map(|x| x.len).max().unwrap_or(0);
        Some(RoundSummary {
            h1: self.sum(now - 59, now),
            h24: self.sum(now - DAY_MINUTES + 1, now),
            missed10m: last10.missed + last10.failed,
            late_tmo10m: last10.late_tmo,
            net10m: NetWindow { rounds: last10.net_rounds, tc: last10.net_missed, max_tc_run },
            last_missed: self.last_missed,
            rounds_per_sec: prev_minute.net_rounds as f64 / 60.0,
            bft_debug: self.last_debug_t.is_some_and(|t| now_ms - t < DEBUG_LIVE_MS),
            proposal_delay: self.proposal_delays.latency(now_ms),
            finality: self.finality.latency(now_ms),
        })
    }
}

pub fn start_bft_log_tail() -> Arc<Mutex<RoundTracker>> {
    let tracker = Arc::new(Mutex::new(RoundTracker::new()));
    let tr = tracker.clone();
    follow_journal("monad-bft", None, move |line| {
        let mut tr = tr.lock().unwrap();
        tr.note_line(chrono::Utc::now().timestamp_millis());
        if let Some(e) = parse_bft_line(line) {
            tr.ingest(e);
        }
    });
    tracker
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger_log::LedgerEvent;
    use serde_json::json;

    const ME: &str = "035c51a98dbf03fe411cb723aa89d2e5ec764ece056cf08bc76de2bea2107e6d4d";
    const OTHER: &str = "02fbe48c47aa";

    fn ms(ts: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(ts).unwrap().timestamp_millis()
    }

    fn t0() -> i64 {
        ms("2026-10-22T11:36:41.385386Z")
    }

    // Line shapes as monad-bft 0.16.5 logs them (proposal/certificate bodies trimmed).
    fn leader_line(round: i64, key: Option<&str>) -> String {
        json!({
            "timestamp": "2026-10-22T11:36:41.385386Z", "level": "DEBUG",
            "fields": { "message": "leader for round", "round": round.to_string(), "round_leader": key.map(|k| format!("Some({k})")).unwrap_or("None".into()) },
            "target": "monad_consensus_state",
        })
        .to_string()
    }

    fn proposal_line(round: i64, author: &str, seq: i64) -> String {
        json!({
            "timestamp": "2026-10-22T11:36:41.385256Z", "level": "DEBUG",
            "fields": {
                "message": "proposal message", "author": author,
                "proposal": format!("ProposalMessage {{ proposal_round: {round}, proposal_epoch: 2312, tip: ConsensusTip {{ block_header: ConsensusBlockHeader {{ author: {author}, epoch: 2312, block_round: {round}, seq_num: {seq}, timestamp_ns: 1792667576967387110 }} }}, last_round_tc: None }}"),
            },
            "target": "monad_consensus_state",
        })
        .to_string()
    }

    fn advance_line(cert: &str) -> String {
        json!({
            "timestamp": "2026-10-22T11:13:10.169671Z", "level": "DEBUG",
            "fields": { "message": "advancing round", "certificate": cert },
            "target": "monad_consensus_state",
        })
        .to_string()
    }

    fn commit_line(block: i64, txs: u64) -> String {
        json!({
            "timestamp": "2026-10-22T11:36:41.387355Z", "level": "INFO",
            "fields": { "message": "committed block", "num_tx": txs, "block_num": block },
            "target": "monad_ledger",
        })
        .to_string()
    }

    fn leader(round: i64, key: &str, t: i64) -> BftEvent {
        BftEvent::Leader { round, leader: Some(key.to_string()), t }
    }

    fn proposal(round: i64, author: &str, seq: i64, t: i64) -> BftEvent {
        BftEvent::Proposal { round, author: author.to_string(), seq: Some(seq), t }
    }

    fn agg(f: impl FnOnce(&mut RoundAgg)) -> RoundAgg {
        let mut a = RoundAgg::default();
        f(&mut a);
        a
    }

    #[test]
    fn parses_leader_proposal_tc_and_commit_lines() {
        assert_eq!(parse_bft_line(&leader_line(115733163, Some(ME))), Some(leader(115733163, ME, t0())));
        assert_eq!(parse_bft_line(&leader_line(5, None)), Some(BftEvent::Leader { round: 5, leader: None, t: t0() }));
        assert_eq!(
            parse_bft_line(&proposal_line(115728526, OTHER, 115596946)),
            Some(proposal(115728526, OTHER, 115596946, ms("2026-10-22T11:36:41.385256Z")))
        );
        assert_eq!(
            parse_bft_line(&advance_line("Tc(TimeoutCertificate { epoch: 2312, round: 115728525, tip_rounds: LimitedVec([]) })")),
            Some(BftEvent::Tc { round: 115728525, t: ms("2026-10-22T11:13:10.169671Z") })
        );
        assert_eq!(
            parse_bft_line(&commit_line(115601547, 10)),
            Some(BftEvent::Commit { block: 115601547, txs: 10, t: ms("2026-10-22T11:36:41.387355Z") })
        );
        let local_tmo = json!({
            "timestamp": "2026-10-22T11:14:37.142280Z", "level": "DEBUG",
            "fields": { "message": "local timeout", "round": "115728811", "leader": OTHER, "next_leader": ME },
            "target": "monad_consensus_state",
        })
        .to_string();
        assert_eq!(parse_bft_line(&local_tmo), Some(BftEvent::LocalTmo { round: 115728811, t: ms("2026-10-22T11:14:37.142280Z") }));
    }

    #[test]
    fn ignores_qc_advances_other_messages_and_non_json() {
        assert_eq!(parse_bft_line(&advance_line("Qc(QC { info: Vote { id: 8b7d..b058, epoch: 2312, round: 115728483 } })")), None);
        assert_eq!(parse_bft_line(&json!({ "timestamp": "2026-10-22T11:36:41Z", "fields": { "message": "try vote" } }).to_string()), None);
        assert_eq!(parse_bft_line("\"leader for round\" but not json"), None);
    }

    #[test]
    fn classifies_our_rounds_and_counts_our_committed_blocks() {
        let mut tr = RoundTracker::new();
        tr.configure_self(Some(&ME.to_uppercase()));
        let t = t0();
        let ev = vec![
            // 100: we lead and propose block 1000 → ok, later committed with 7 txs
            leader(100, ME, t),
            proposal(100, ME, 1000, t),
            BftEvent::Commit { block: 1000, txs: 7, t },
            // 101: other leader never proposes → TC (network miss, not ours)
            leader(101, OTHER, t),
            BftEvent::Tc { round: 101, t },
            // 102: after a TC there is no leader line; the author is the leader
            proposal(102, OTHER, 1001, t),
            // 103: we lead, no proposal, TC → missed
            leader(103, ME, t),
            BftEvent::Tc { round: 103, t },
            // 104: we lead, our proposal is seen but the round still times out → failed
            leader(104, ME, t),
            proposal(104, ME, 1002, t),
            BftEvent::Tc { round: 104, t },
            // 105: we lead, nothing seen, no TC → the network went on without us seeing it
            leader(105, ME, t),
            // push the chain far enough to finalize rounds 100-105
            leader(115, OTHER, t),
        ];
        for e in ev {
            tr.ingest(e);
        }

        let s = tr.summary(t).unwrap();
        let want = agg(|a| {
            a.led = 4;
            a.ok = 1;
            a.missed = 1;
            a.failed = 1;
            a.not_received = 1;
            a.committed = 1;
            a.txs = 7;
            a.net_rounds = 6;
            a.net_missed = 3;
        });
        assert_eq!(s.h24, want);
        assert_eq!(s.h1, s.h24);
        assert_eq!(s.missed10m, 2);
        assert_eq!(s.last_missed, Some(MissedRound { round: 104, t }));
    }

    #[test]
    fn local_timeouts_count_as_late_only_without_a_network_tc() {
        let mut tr = RoundTracker::new();
        tr.configure_self(Some(ME));
        let t = t0();
        // 200: other leader missed → network TC; our local timeout is just the miss.
        tr.ingest(leader(200, OTHER, t));
        tr.ingest(BftEvent::LocalTmo { round: 200, t });
        tr.ingest(BftEvent::Tc { round: 200, t });
        // 201: proposal arrived and the round got a QC, but we had already timed out → late.
        tr.ingest(proposal(201, OTHER, 5, t));
        tr.ingest(BftEvent::LocalTmo { round: 201, t });
        tr.ingest(leader(220, OTHER, t));

        let s = tr.summary(t).unwrap();
        assert_eq!(s.h24.late_tmo, 1);
        assert_eq!(s.late_tmo10m, 1);
        assert_eq!(s.h24.net_missed, 1);
        assert_eq!(s.missed10m, 0);
    }

    #[test]
    fn windows_drop_old_misses() {
        let mut tr = RoundTracker::new();
        tr.configure_self(Some(ME));
        let t = t0();
        tr.ingest(leader(1, ME, t));
        tr.ingest(BftEvent::Tc { round: 1, t });
        tr.ingest(leader(20, OTHER, t));

        assert_eq!(tr.summary(t).unwrap().missed10m, 1);
        assert_eq!(tr.summary(t + 11 * 60_000).unwrap().missed10m, 0);
        assert_eq!(tr.summary(t + 11 * 60_000).unwrap().h24.missed, 1);
        assert_eq!(tr.summary(t + 2 * 3_600_000).unwrap().h1.missed, 0);
        assert_eq!(tr.summary(t + 25 * 3_600_000).unwrap().h24.missed, 0);
    }

    #[test]
    fn none_before_any_line_and_ignores_rounds_while_self_unknown() {
        let mut tr = RoundTracker::new();
        let t = t0();
        assert_eq!(tr.summary(t), None);
        tr.ingest(leader(1, ME, t));
        tr.ingest(leader(20, OTHER, t));
        assert_eq!(tr.summary(t).unwrap().h24.led, 0);
        assert_eq!(tr.summary(t).unwrap().h24.net_rounds, 1);
    }

    #[test]
    fn net10m_counts_tc_rounds_and_longest_run() {
        let mut tr = RoundTracker::new();
        let t = t0();
        // Rounds 1-10: TCs at 3, 6, 7, 8 → one single TC and a run of 3.
        for round in 1..=10 {
            tr.ingest(leader(round, OTHER, t));
            if [3, 6, 7, 8].contains(&round) {
                tr.ingest(BftEvent::Tc { round, t });
            }
        }
        // Events can arrive out of round order; finalization must still be ordered.
        tr.ingest(BftEvent::Tc { round: 12, t });
        tr.ingest(leader(11, OTHER, t));
        tr.ingest(leader(30, OTHER, t));

        let s = tr.summary(t).unwrap();
        assert_eq!(s.net10m, NetWindow { rounds: 12, tc: 5, max_tc_run: 3 });
        assert_eq!(tr.summary(t + 11 * 60_000).unwrap().net10m.max_tc_run, 0, "old runs drop out of the 10 min window");
    }

    #[test]
    fn block_size_is_compared_with_median_of_preceding_network_blocks() {
        let mut tr = RoundTracker::new();
        tr.configure_self(Some(ME));
        let t = t0();
        let mut round = 1000;
        let mut block = |tr: &mut RoundTracker, author: &str, txs: u64| {
            tr.ingest(proposal(round, author, round + 1, t));
            round += 1;
            tr.ingest(BftEvent::Stats { txs, t });
        };
        // Too little context: our first block is skipped.
        block(&mut tr, ME, 5);
        // 50 network blocks of 20 txs with two outliers that the median ignores.
        for _ in 0..48 {
            block(&mut tr, OTHER, 20);
        }
        block(&mut tr, OTHER, 500);
        block(&mut tr, OTHER, 0);
        block(&mut tr, ME, 15);
        tr.ingest(BftEvent::Fwd { txs: 3, t });
        tr.ingest(BftEvent::Fwd { txs: 4, t });

        let a = tr.summary(t).unwrap().h24;
        assert_eq!(a.size_samples, 1);
        assert_eq!(a.size_our_txs, 15.0);
        assert_eq!(a.size_peer_txs, 20.0);
        assert_eq!(size_ratio(&a), Some(0.75));
        assert_eq!(a.fwd_txs, 7);
    }

    #[test]
    fn learns_our_key_from_our_own_proposal_command() {
        let line = json!({
            "timestamp": "2026-10-22T11:52:36.840296Z", "level": "DEBUG",
            "fields": { "message": "emitting create proposal command to txpool", "node_id": ME, "round": "115736284" },
            "target": "monad_consensus_state",
        })
        .to_string();
        let e = parse_bft_line(&line).unwrap();
        assert_eq!(e, BftEvent::SelfKey { key: ME.into(), t: ms("2026-10-22T11:52:36.840296Z") });

        let t = t0();
        let mut tr = RoundTracker::new();
        tr.suggest_self(Some("?")); // OTLP without secp_key: label is "?"
        tr.ingest(e.clone());
        tr.suggest_self(Some(OTHER)); // the metric label no longer overrides it
        tr.ingest(leader(1, ME, t));
        tr.ingest(leader(20, OTHER, t));
        assert_eq!(tr.summary(t).unwrap().h24.led, 1);

        // A configured key wins even if the journal disagrees.
        let mut tr = RoundTracker::new();
        tr.configure_self(Some(OTHER));
        tr.ingest(e);
        tr.ingest(leader(1, OTHER, t));
        tr.ingest(leader(20, ME, t));
        assert_eq!(tr.summary(t).unwrap().h24.led, 1);
    }

    fn proposed(round: i64, author: &str, seq: i64, t: i64) -> LedgerEvent {
        LedgerEvent::Proposed { round, author: author.into(), seq, txs: 10, gas: 1000, block_ts: t - 30, t }
    }

    #[test]
    fn ledger_tail_alone_classifies_our_rounds() {
        let t = t0();
        let mut tr = RoundTracker::new();
        tr.configure_self(Some(ME));
        for r in 1..=40 {
            tr.ingest_ledger(proposed(r, OTHER, 100 + r, t));
        }
        tr.ingest_ledger(proposed(41, ME, 141, t)); // ok
        tr.ingest_ledger(LedgerEvent::Finalized { seq: 141, block_ts: t - 600, t });
        tr.ingest_ledger(proposed(42, ME, 142, t)); // proposed, then the round timed out: failed
        tr.ingest_ledger(LedgerEvent::Timeout { round: 42, leader: ME.into(), t });
        tr.ingest_ledger(LedgerEvent::Timeout { round: 43, leader: ME.into(), t }); // missed
        for r in 44..=60 {
            tr.ingest_ledger(proposed(r, OTHER, 100 + r, t));
        }
        let s = tr.summary(t).unwrap();
        assert_eq!((s.h24.led, s.h24.ok, s.h24.failed, s.h24.missed), (3, 1, 1, 1));
        assert_eq!((s.h24.committed, s.h24.txs), (1, 10));
        assert_eq!(s.h24.net_missed, 2);
        assert_eq!(s.proposal_delay, Some(Latency { p50: 30, p99: 30 }));
        assert_eq!(s.finality, Some(Latency { p50: 600, p99: 600 }));
        assert!(!s.bft_debug, "no DEBUG lines: late and unseen are unknown");
    }

    #[test]
    fn bft_round_events_step_aside_while_ledger_tail_is_live() {
        let t = t0();
        let mut tr = RoundTracker::new();
        tr.configure_self(Some(ME));
        tr.ingest_ledger(proposed(1, ME, 101, t));
        // The same proposal and commit from the bft journal must not count twice.
        tr.ingest(proposal(1, ME, 101, t));
        tr.ingest(BftEvent::Commit { block: 101, txs: 10, t });
        tr.ingest(BftEvent::LocalTmo { round: 1, t });
        tr.ingest_ledger(LedgerEvent::Finalized { seq: 101, block_ts: t, t });
        for r in 2..=20 {
            tr.ingest_ledger(proposed(r, OTHER, 100 + r, t));
        }
        let s = tr.summary(t).unwrap();
        assert_eq!((s.h24.led, s.h24.ok, s.h24.committed, s.h24.txs), (1, 1, 1, 10));
        assert!(s.bft_debug);
    }
}
