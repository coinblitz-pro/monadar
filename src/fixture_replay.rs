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
    eprintln!("fixture replay: bft {kinds:?}, exec {} blocks, otlp {} messages: all match", samples.len(), index.len());
}

// The same two hours of monad-bft (round lines only) and monad-ledger-tail
// journals from a mainnet validator, replayed through three trackers: bft only,
// ledger-tail only, and both (ledger-tail leading). Our rounds and the network
// TC counts must agree, so the bft journal can drop to INFO.
//   MONADAR_LEDGER_FIXTURES=<dir with bft.jsonl.gz, ledger.jsonl.gz, node_key> cargo test ledger_tail
#[test]
fn ledger_tail_rounds_match_the_bft_journal() {
    use std::io::Read;

    use crate::ledger_log::{LedgerEvent, parse_ledger_line};

    let Some(dir) = std::env::var_os("MONADAR_LEDGER_FIXTURES").map(PathBuf::from) else { return };
    let gz = |name: &str| {
        let mut s = String::new();
        flate2::read::GzDecoder::new(std::fs::File::open(dir.join(name)).unwrap()).read_to_string(&mut s).unwrap();
        s
    };
    let key = std::fs::read_to_string(dir.join("node_key")).unwrap().trim().to_string();
    enum Ev {
        Bft(BftEvent),
        Ledger(LedgerEvent),
    }
    let bft_t = |e: &BftEvent| match e {
        BftEvent::Leader { t, .. }
        | BftEvent::Proposal { t, .. }
        | BftEvent::Tc { t, .. }
        | BftEvent::LocalTmo { t, .. }
        | BftEvent::Commit { t, .. }
        | BftEvent::Stats { t, .. }
        | BftEvent::Fwd { t, .. }
        | BftEvent::SelfKey { t, .. } => *t,
    };
    let bft: Vec<BftEvent> = gz("bft.jsonl.gz").lines().filter_map(parse_bft_line).collect();
    let ledger: Vec<LedgerEvent> = gz("ledger.jsonl.gz").lines().filter_map(parse_ledger_line).collect();
    // Compare whole rounds both sources cover.
    let from = bft_t(&bft[0]).max(ledger[0].t()) + 60_000;
    let to = bft_t(bft.last().unwrap()).min(ledger.last().unwrap().t());
    let mut both: Vec<(i64, Ev)> = bft.iter().cloned().map(|e| (bft_t(&e), Ev::Bft(e))).collect();
    both.extend(ledger.iter().cloned().map(|e| (e.t(), Ev::Ledger(e))));
    both.sort_by_key(|(t, _)| *t);

    let run = |use_bft: bool, use_ledger: bool| -> crate::types::RoundSummary {
        let mut tr = RoundTracker::new();
        tr.configure_self(Some(&key));
        for (t, e) in &both {
            if *t < from || *t > to {
                continue;
            }
            match e {
                Ev::Bft(e) if use_bft => tr.ingest(e.clone()),
                Ev::Ledger(e) if use_ledger => tr.ingest_ledger(e.clone()),
                _ => {}
            }
        }
        tr.summary(to).unwrap()
    };
    let (b, l, x) = (run(true, false), run(false, true), run(true, true));
    let row = |s: &crate::types::RoundSummary| {
        let a = &s.h24;
        (a.led, a.ok, a.missed, a.failed, a.committed, a.txs, a.net_rounds, a.net_missed)
    };
    eprintln!("(led, ok, missed, failed, committed, txs, net_rounds, net_tc)");
    eprintln!("bft     {:?} late {} unseen {}", row(&b), b.h24.late_tmo, b.h24.not_received);
    eprintln!("ledger  {:?} delay {:?} finality {:?}", row(&l), l.proposal_delay, l.finality);
    eprintln!("both    {:?} late {} unseen {}", row(&x), x.h24.late_tmo, x.h24.not_received);
    assert!(b.h24.led > 0, "the window holds some of our rounds");
    let (rb, rl, rx) = (row(&b), row(&l), row(&x));
    // Our rounds: identical. Network rounds: the sources may disagree on a
    // round or two at the window edges.
    assert_eq!((rb.0, rb.1, rb.2, rb.3, rb.4, rb.5), (rl.0, rl.1, rl.2, rl.3, rl.4, rl.5), "our rounds, bft vs ledger-tail");
    assert_eq!((rb.0, rb.1, rb.2, rb.3, rb.4, rb.5), (rx.0, rx.1, rx.2, rx.3, rx.4, rx.5), "our rounds, bft vs both");
    for (name, v) in [("ledger", rl), ("both", rx)] {
        assert!(v.6.abs_diff(rb.6) <= 3, "{name}: network rounds {} vs {}", v.6, rb.6);
        assert_eq!(v.7, rb.7, "{name}: TC rounds");
    }
    assert!(x.bft_debug && !l.bft_debug);
}
