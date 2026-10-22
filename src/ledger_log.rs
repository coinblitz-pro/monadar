use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::bft_log::RoundTracker;
use crate::journal::follow_journal;

// Block events from monad-ledger-tail's journal, logged at INFO:
//   "proposed_block"  {round, parent_round, seq_num, num_tx, gas_used, author, block_ts_ms, now_ts_ms}
//   "finalized_block" {round, seq_num, author, block_ts_ms, now_ts_ms}
//   "timeout"         {round, author, now_ts_ms} — a round that ended by TC, author = its leader
// Numbers are strings. block_ts_ms is the proposer's timestamp and now_ts_ms
// ours, so their difference is how late a proposal reached us (or, for a
// finalized block, how long finalization took).

#[derive(Clone, Debug, PartialEq)]
pub enum LedgerEvent {
    Proposed { round: i64, author: String, seq: i64, txs: u64, gas: u64, block_ts: i64, t: i64 },
    Finalized { seq: i64, block_ts: i64, t: i64 },
    Timeout { round: i64, leader: String, t: i64 },
}

impl LedgerEvent {
    pub fn t(&self) -> i64 {
        match self {
            LedgerEvent::Proposed { t, .. } | LedgerEvent::Finalized { t, .. } | LedgerEvent::Timeout { t, .. } => *t,
        }
    }
}

fn int(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

pub fn parse_ledger_line(line: &str) -> Option<LedgerEvent> {
    let j: Value = serde_json::from_str(line).ok()?;
    let f = j.get("fields")?;
    let author = || f.get("author")?.as_str().map(str::to_ascii_lowercase);
    let t = int(f.get("now_ts_ms"))?;
    match f.get("message")?.as_str()? {
        "proposed_block" => Some(LedgerEvent::Proposed {
            round: int(f.get("round"))?,
            author: author()?,
            seq: int(f.get("seq_num"))?,
            txs: int(f.get("num_tx")).unwrap_or(0).max(0) as u64,
            gas: int(f.get("gas_used")).unwrap_or(0).max(0) as u64,
            block_ts: int(f.get("block_ts_ms"))?,
            t,
        }),
        "finalized_block" => Some(LedgerEvent::Finalized { seq: int(f.get("seq_num"))?, block_ts: int(f.get("block_ts_ms"))?, t }),
        "timeout" => Some(LedgerEvent::Timeout { round: int(f.get("round"))?, leader: author()?, t }),
        _ => None,
    }
}

// Follows monad-ledger-tail and feeds the round tracker shared with the bft
// tail. Starts with the last 24h of the journal, so the 24h round counters are
// complete right after a monadar restart (about 600k lines, a few seconds).
pub fn start_ledger_tail(tracker: Arc<Mutex<RoundTracker>>) {
    follow_journal("monad-ledger-tail", Some("-24h"), move |line| {
        let mut tr = tracker.lock().unwrap();
        tr.note_ledger_line(chrono::Utc::now().timestamp_millis());
        if let Some(e) = parse_ledger_line(line) {
            tr.ingest_ledger(e);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_proposed_finalized_and_timeout_lines() {
        let proposed = r#"{"timestamp":"2026-10-22T18:31:09.209853Z","level":"INFO","fields":{"message":"proposed_block","round":"115815373","parent_round":"115815372","epoch":"2314","seq_num":"115683613","num_tx":"20","gas_used":"12474726","author":"02C67B","block_ts_ms":"1792693869187","now_ts_ms":"1792693869209","author_address":"64.130.43.22:8000"},"target":"ledger_tail"}"#;
        assert_eq!(
            parse_ledger_line(proposed),
            Some(LedgerEvent::Proposed {
                round: 115815373,
                author: "02c67b".into(),
                seq: 115683613,
                txs: 20,
                gas: 12474726,
                block_ts: 1792693869187,
                t: 1792693869209
            })
        );
        let finalized = r#"{"fields":{"message":"finalized_block","round":"115815371","seq_num":"115683611","author":"0316e0","block_ts_ms":"1792693868583","now_ts_ms":"1792693869209"}}"#;
        assert_eq!(
            parse_ledger_line(finalized),
            Some(LedgerEvent::Finalized { seq: 115683611, block_ts: 1792693868583, t: 1792693869209 })
        );
        let timeout = r#"{"fields":{"message":"timeout","round":"115808579","author":"03bdde","now_ts_ms":"1792691817255"}}"#;
        assert_eq!(parse_ledger_line(timeout), Some(LedgerEvent::Timeout { round: 115808579, leader: "03bdde".into(), t: 1792691817255 }));
        assert_eq!(parse_ledger_line(r#"{"fields":{"message":"other","now_ts_ms":"1"}}"#), None);
        assert_eq!(parse_ledger_line("not json"), None);
    }
}
