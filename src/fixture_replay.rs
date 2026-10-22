// Replays logs and OTLP exports captured on a mainnet validator and checks the
// results against the TS reference (expected.json, monadar 1b6dfc1). The
// fixtures are ~100 MB, so they live outside the repo:
//   MONADAR_FIXTURES=/path/to/2026-10-22 cargo test fixture -- --nocapture
// Without the variable the test is a no-op.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{Value, json};

use crate::bft_log::{BftEvent, RoundTracker, parse_bft_line, size_ratio};
use crate::exec_log::{parse_exec_block_line, summarize};
use crate::metric_snapshot::read_metrics_snapshot;
use crate::otlp::{Labels, SeriesStore, decode_export_request};

fn fixtures() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("MONADAR_FIXTURES")?);
    assert!(dir.join("expected.json").exists(), "MONADAR_FIXTURES has no expected.json: {}", dir.display());
    Some(dir)
}

// Numbers compare as f64: the TS side writes 362 where Rust has 362.0.
fn assert_same(path: &str, got: &Value, want: &Value) {
    match (got, want) {
        (Value::Number(a), Value::Number(b)) => {
            assert!((a.as_f64().unwrap() - b.as_f64().unwrap()).abs() < 1e-9, "{path}: got {a}, want {b}")
        }
        (Value::Object(a), Value::Object(b)) => {
            for (k, w) in b {
                assert_same(&format!("{path}.{k}"), a.get(k).unwrap_or(&Value::Null), w);
            }
        }
        _ => assert_eq!(got, want, "{path}"),
    }
}

fn kind(e: &BftEvent) -> &'static str {
    match e {
        BftEvent::Leader { .. } => "leader",
        BftEvent::Proposal { .. } => "proposal",
        BftEvent::Tc { .. } => "tc",
        BftEvent::LocalTmo { .. } => "local_tmo",
        BftEvent::Commit { .. } => "commit",
        BftEvent::Stats { .. } => "stats",
        BftEvent::Fwd { .. } => "fwd",
        BftEvent::SelfKey { .. } => "self_key",
    }
}

#[test]
fn fixture_replay_matches_ts_reference() {
    let Some(dir) = fixtures() else { return };
    let expected: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("expected.json")).unwrap()).unwrap();
    let node_key = expected["node_key"].as_str().unwrap();

    // bft.log → round tracker
    let mut tracker = RoundTracker::new();
    tracker.configure_self(Some(node_key));
    let mut kinds: BTreeMap<&str, u64> = BTreeMap::new();
    let mut last_t = 0;
    for line in std::fs::read_to_string(dir.join("bft.log")).unwrap().lines() {
        if let Some(e) = parse_bft_line(line) {
            *kinds.entry(kind(&e)).or_default() += 1;
            last_t = match e {
                BftEvent::Leader { t, .. }
                | BftEvent::Proposal { t, .. }
                | BftEvent::Tc { t, .. }
                | BftEvent::LocalTmo { t, .. }
                | BftEvent::Commit { t, .. }
                | BftEvent::Stats { t, .. }
                | BftEvent::Fwd { t, .. }
                | BftEvent::SelfKey { t, .. } => t,
            };
            tracker.ingest(e);
        }
    }
    assert_same("bft.parsed_events_by_kind", &json!(kinds), &expected["bft"]["parsed_events_by_kind"]);
    let summary = tracker.summary(last_t).unwrap();
    let mut got = serde_json::to_value(&summary).unwrap();
    got["size_ratio_24h"] = json!(size_ratio(&summary.h24));
    assert_same("bft.summary_at_last_event", &got, &expected["bft"]["summary_at_last_event"]);

    // Same log without node_key: our key comes from our own proposal lines,
    // early enough that no round of ours is lost.
    let mut learned = RoundTracker::new();
    for e in std::fs::read_to_string(dir.join("bft.log")).unwrap().lines().filter_map(parse_bft_line) {
        learned.ingest(e);
    }
    assert_eq!(learned.summary(last_t).unwrap(), summary, "self key learned from the journal");

    // exec.log → latency stats over the whole file
    let samples: Vec<_> = std::fs::read_to_string(dir.join("exec.log")).unwrap().lines().filter_map(parse_exec_block_line).collect();
    let lat: Vec<f64> = samples.iter().map(|s| s.latency_ms).collect();
    let got = json!({
        "blocks": samples.len(),
        "first_block": samples.first().unwrap().block,
        "last_block": samples.last().unwrap().block,
        "latency_ms": serde_json::to_value(summarize(&lat).unwrap()).unwrap(),
    });
    assert_same("exec", &got, &expected["exec"]);

    // otlp/*.bin → series store → metrics snapshot
    let index: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(dir.join("otlp/index.json")).unwrap()).unwrap();
    let mut store = SeriesStore::new(Labels::from([("secp_key".to_string(), node_key.to_string())]));
    let mut last_export = 0;
    for msg in &index {
        let buf = std::fs::read(dir.join("otlp").join(msg["file"].as_str().unwrap())).unwrap();
        let points = decode_export_request(&buf).unwrap();
        assert_eq!(points.len() as u64, msg["points"].as_u64().unwrap(), "{msg}");
        last_export = msg["t"].as_i64().unwrap();
        store.ingest(points, last_export);
    }
    for (name, text) in [("otlp", store.text(last_export)), ("metrics.txt", std::fs::read_to_string(dir.join("metrics.txt")).unwrap())] {
        let m = read_metrics_snapshot(&text);
        let got = json!({
            "ver": m.ver, "peers": m.peers, "stakeBps": m.stake_bps, "sessions": m.sessions, "syncing": m.syncing,
            "uptimeUs": m.uptime_us, "commits": m.commits, "votes": m.votes, "timeouts": m.timeouts, "tc": m.tc,
            "lagging": m.lagging, "proposals": m.proposals, "handled": m.handled, "execBlock": m.exec_block,
            "txCommits": m.tx_commits, "txpool": m.txpool, "fwd": m.fwd, "latP99": m.lat_p99,
            "syncProgress": m.sync_progress, "syncTarget": m.sync_target, "nodeKey": m.node_key, "network": m.network,
        });
        assert_same(&format!("metrics_snapshot({name})"), &got, &expected["metrics_snapshot"]);
    }
    eprintln!("fixture replay: bft {kinds:?}, exec {} blocks, otlp {} messages — all match", samples.len(), index.len());
}
