use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::json;

use crate::alerts::fmt_duration;
use crate::bft_log::size_ratio;
use crate::config::BUILD_VERSION;
use crate::format::{js_round, num};
use crate::http;
use crate::notify::{NoticeKind, Pending};
use crate::rich::{Block, Msg, TableKind, Text, b, code, i, p};
use crate::types::{EventLevel, Frame, FrameError, RoundAgg, RoundSummary, Snapshot, Status};

// Delivered later than this after the event: say so in the message.
const LATE_MS: i64 = 60_000;

pub fn html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

// Shown in every message footer, so alerts from several nodes stay apart.
static HOST: LazyLock<String> = LazyLock::new(|| {
    let mut buf = [0u8; 256];
    let ok = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0;
    let name = if ok { String::from_utf8_lossy(&buf[..buf.iter().position(|c| *c == 0).unwrap_or(0)]).into_owned() } else { String::new() };
    if name.is_empty() { "monadar".into() } else { name }
});

fn clock(ms: i64, now_ms: i64) -> String {
    let Some(t) = chrono::DateTime::from_timestamp_millis(ms) else { return "?".into() };
    let t = t.with_timezone(&chrono::Local);
    let now = chrono::DateTime::from_timestamp_millis(now_ms).map(|n| n.with_timezone(&chrono::Local));
    if now.is_some_and(|n| n.date_naive() == t.date_naive()) { t.format("%H:%M:%S") } else { t.format("%m-%d %H:%M:%S") }.to_string()
}

fn kv(rows: Vec<(String, String)>) -> Block {
    Block::Table(TableKind::KeyValue, rows.into_iter().map(|(k, v)| vec![vec![p(k)], vec![p(v)]]).collect())
}

fn sync_text(syncing: Option<f64>, sync_pct: &Option<String>) -> String {
    match (syncing, sync_pct.as_deref().filter(|p| !p.is_empty())) {
        (Some(1.0), Some(pct)) => format!("syncing {pct}%"),
        (Some(1.0), None) => "syncing".into(),
        _ => "synced".into(),
    }
}

fn block_text(block: i64, delta: Option<i64>, bps: &str) -> String {
    let mut s = block.to_string();
    if let Some(d) = delta {
        s += &format!(" · {d:+}");
    }
    if bps != "?" {
        s += &format!(" · {bps}/s");
    }
    s
}

// Node state at the moment of an alert: label/value rows, unknown fields left out.
pub fn snapshot_rows(s: &Snapshot) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    let mut row = |k: &str, v: String| rows.push((k.to_string(), v));
    if let Some(bl) = s.block {
        row("Block", block_text(bl, s.block_delta, &s.bps));
    }
    if let Some(l) = s.lag {
        row("Exec lag", format!("{l} block{}", if l == 1 { "" } else { "s" }));
    }
    if let Some(e) = s.exec_p99 {
        row("Exec p99", format!("{e} ms"));
    }
    if let Some(led) = s.led24h {
        row("Rounds 24h", format!("{led} led · {} missed", s.missed24h.unwrap_or(0)));
    }
    if let Some(v) = s.peers {
        row("Peers", num(v));
    }
    if let Some(v) = s.p99 {
        row("Broadcast p99", format!("{} ms", num(v)));
    }
    if let Some(v) = s.rpc_ms {
        row("RPC", format!("{v} ms"));
    }
    if s.syncing.is_some() {
        row("Sync", sync_text(s.syncing, &s.sync_pct));
    }
    if let Some(u) = &s.uptime {
        row("Uptime", u.clone());
    }
    if let Some(v) = &s.ver {
        row("Version", v.clone());
    }
    rows
}

pub fn notice_message(pending: &Pending, now_ms: i64) -> Msg {
    let n = &pending.notice;
    let mut m = Msg::default();
    let span = |from: i64, to: i64| {
        vec![p("Lasted "), b(fmt_duration(to - from)), p(format!(" · {} → {}", clock(from, now_ms), clock(to, now_ms)))]
    };
    match (n.kind, pending.resolved_at) {
        (NoticeKind::Fired, None) => {
            m.push(Block::Heading(vec![p(format!("🚨 {}", n.title))], 2)).push(Block::Para(vec![p(&n.detail)]));
        }
        (NoticeKind::Fired, Some(r)) => {
            m.push(Block::Heading(vec![p(format!("⚠️ {} · resolved", n.title))], 2))
                .push(Block::Para(vec![p(&n.detail)]))
                .push(Block::Para(span(n.at, r)));
        }
        (NoticeKind::Resolved, _) => {
            m.push(Block::Heading(vec![p(format!("✅ {} resolved", n.title))], 2));
            if let Some(s) = n.since {
                m.push(Block::Para(span(s, n.at)));
            }
        }
        (NoticeKind::Info, _) => {
            m.push(Block::Heading(vec![p(format!("🔄 {}", n.title))], 2)).push(Block::Para(vec![p(&n.detail)]));
        }
    }
    if !n.snap.is_empty() {
        m.push(kv(n.snap.clone()));
    }
    // Footer: host, when, and anything unusual about the delivery.
    let last_event = pending.resolved_at.unwrap_or(n.at);
    let mut foot = HOST.clone();
    if pending.resolved_at.is_none() && n.since.is_none() {
        foot += &format!(" · {}", clock(n.at, now_ms));
    }
    if now_ms - last_event > LATE_MS {
        foot += &format!(" · delivered {} late", fmt_duration(now_ms - last_event));
    }
    if n.suppressed > 0 {
        foot += &format!(" · {} repeat(s) within cooldown not sent", n.suppressed);
    }
    m.push(Block::Footer(vec![p(foot)]));
    m
}

fn status_title(s: Status) -> &'static str {
    match s {
        Status::Crit => "🔴 Critical",
        Status::Warn => "🟡 Warning",
        Status::Ok => "🟢 All good",
    }
}

fn pct(n: u64, of: u64) -> String {
    if of == 0 { "—".into() } else { format!("{:.2}%", n as f64 / of as f64 * 100.0) }
}

fn rounds_grid(r: &RoundSummary) -> Block {
    let size = |a: &RoundAgg| size_ratio(a).map(|x| format!("{}%", js_round(x * 100.0))).unwrap_or("—".into());
    let rows: Vec<(&str, String, String)> = vec![
        ("Led", r.h1.led.to_string(), r.h24.led.to_string()),
        ("Proposed OK", r.h1.ok.to_string(), r.h24.ok.to_string()),
        ("Missed", (r.h1.missed + r.h1.failed).to_string(), (r.h24.missed + r.h24.failed).to_string()),
        ("Unseen", r.h1.not_received.to_string(), r.h24.not_received.to_string()),
        ("Late timeouts", r.h1.late_tmo.to_string(), r.h24.late_tmo.to_string()),
        ("Blocks", r.h1.committed.to_string(), r.h24.committed.to_string()),
        ("Txs", r.h1.txs.to_string(), r.h24.txs.to_string()),
        ("Size vs network", size(&r.h1), size(&r.h24)),
        ("Txs forwarded to us", r.h1.fwd_txs.to_string(), r.h24.fwd_txs.to_string()),
    ];
    let mut grid = vec![vec![vec![], vec![p("1h")], vec![p("24h")]]];
    grid.extend(rows.into_iter().map(|(k, a, d)| vec![vec![p(k)], vec![p(a)], vec![p(d)]]));
    Block::Table(TableKind::Grid, grid)
}

fn footer(ts: &str) -> Block {
    Block::Footer(vec![p(format!("{} · {ts} · monadar {BUILD_VERSION}", *HOST))])
}

// Reply to /status.
pub fn status_message(f: &Frame) -> Msg {
    let mut m = Msg::default();
    let title = match f.error {
        Some(FrameError::Rpc) => "🔴 RPC down".into(),
        Some(FrameError::Metrics) => format!("{} · metrics unavailable", status_title(f.status)),
        None => status_title(f.status).into(),
    };
    m.push(Block::Heading(vec![p(title)], 2));
    if !f.warnings.is_empty() {
        m.push(Block::List(f.warnings.iter().map(|w| vec![b(&w.short), p(format!(" — {}", w.detail))]).collect()));
    }
    if f.error == Some(FrameError::Rpc) {
        m.push(footer(&f.ts));
        return m;
    }

    m.push(Block::Heading(vec![p("Node")], 3));
    let mut rows = vec![("Block".to_string(), block_text(f.block, f.block_delta, &f.bps))];
    if f.error.is_none() {
        let exec = match f.exec {
            Some(e) => format!("lag {} · p50 {} · p99 {} · max {} ms", f.exec_lag, e.p50, e.p99, e.max),
            None => format!("lag {} block{}", f.exec_lag, if f.exec_lag == 1 { "" } else { "s" }),
        };
        rows.extend([
            ("Execution".to_string(), exec),
            ("Peers".into(), f.peers.map(num).unwrap_or("?".into())),
            ("Broadcast p99".into(), f.p99.map(|v| format!("{} ms", num(v))).unwrap_or("?".into())),
            ("RPC".into(), format!("{} ms", f.rpc_ms)),
            ("Sync".into(), sync_text(f.syncing, &f.sync_pct)),
            ("Voted".into(), f.voted_pct.map(|v| format!("{v:.1}%")).unwrap_or("?".into())),
            ("Stake".into(), f.stake_pct.clone()),
            ("Uptime".into(), f.uptime.clone()),
            ("Version".into(), format!("{} · {}", f.ver, f.network)),
            ("Key".into(), f.key.clone()),
        ]);
    }
    m.push(kv(rows));

    match &f.rounds {
        Some(r) => {
            m.push(Block::Heading(vec![p("Our leader rounds")], 3)).push(rounds_grid(r));
            m.push(Block::Heading(vec![p("Network")], 3));
            let mut net = vec![
                ("Rounds/s".to_string(), format!("{:.2}", r.rounds_per_sec)),
                (
                    "Ended by TC, 24h".into(),
                    format!("{} of {} · {}", r.h24.net_missed, r.h24.net_rounds, pct(r.h24.net_missed, r.h24.net_rounds)),
                ),
                ("Ended by TC, 10 min".into(), format!("{} of {} · {}", r.net10m.tc, r.net10m.rounds, pct(r.net10m.tc, r.net10m.rounds))),
                ("Longest TC run, 10 min".into(), r.net10m.max_tc_run.to_string()),
            ];
            if let Some(lm) = r.last_missed {
                net.push((
                    "Our last missed round".into(),
                    format!("{} · {}", lm.round, clock(lm.t, chrono::Utc::now().timestamp_millis())),
                ));
            }
            m.push(kv(net));
        }
        None => {
            m.push(Block::Para(vec![i("Round data unavailable: the monad-bft journal is not at DEBUG or not readable.")]));
        }
    }
    m.push(footer(&f.ts));
    m
}

// Reply to /events.
pub fn events_message(f: &Frame) -> Msg {
    let mut m = Msg::default();
    m.push(Block::Heading(vec![p("Recent events")], 3));
    if f.history.is_empty() {
        m.push(Block::Para(vec![i("No recent events.")]));
    } else {
        let rows = f
            .history
            .iter()
            .map(|e| {
                let icon = match e.level {
                    EventLevel::Resolved => "✅",
                    EventLevel::Crit | EventLevel::Error => "🔴",
                    EventLevel::Warn => "🟡",
                };
                vec![vec![p(&e.time)], vec![p(icon)], vec![p(&e.text)]]
            })
            .collect();
        m.push(Block::Table(TableKind::Rows, rows));
    }
    m.push(footer(&f.ts));
    m
}

pub fn help_message() -> Msg {
    let cmd = |c: &str, what: &str| -> Text { vec![code(c), p(format!(" — {what}"))] };
    Msg(vec![
        Block::Heading(vec![p("monadar bot")], 3),
        Block::List(vec![
            cmd("/status", "node status, our rounds, network"),
            cmd("/events", "recent events"),
            cmd("/help", "this message"),
        ]),
    ])
}

pub fn no_data_message() -> Msg {
    Msg(vec![Block::Para(vec![i("No data yet — the daemon is still starting.")])])
}

// Rich messages until Telegram rejects one (an older Bot API server behind
// api_url, or a block it does not accept); then HTML for the rest of the run.
static RICH: AtomicBool = AtomicBool::new(true);

pub fn set_rich(enabled: bool) {
    RICH.store(enabled, Ordering::Relaxed);
}

pub async fn send(base: &str, chat_id: &str, msg: &Msg, timeout: Duration) -> Result<http::Response, String> {
    if RICH.load(Ordering::Relaxed) {
        let body = json!({ "chat_id": chat_id, "rich_message": msg.rich() });
        let resp = http::post_json(&format!("{base}/sendRichMessage"), &body, timeout).await?;
        let rejected = (400..500).contains(&resp.status) && resp.status != 429;
        if !rejected {
            return Ok(resp);
        }
        let why = resp.json().and_then(|j| j["description"].as_str().map(String::from)).unwrap_or_default();
        eprintln!("telegram: rich message rejected (HTTP {} {why}), switching to HTML messages", resp.status);
        RICH.store(false, Ordering::Relaxed);
    }
    let body = json!({ "chat_id": chat_id, "text": msg.html(), "parse_mode": "HTML", "link_preview_options": { "is_disabled": true } });
    http::post_json(&format!("{base}/sendMessage"), &body, timeout).await
}
