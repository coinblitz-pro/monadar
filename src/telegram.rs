use crate::bft_log::size_ratio;
use crate::format::opt_num;
use crate::types::{EventLevel, Frame, RoundAgg, RoundSummary, Snapshot, Status};

pub fn html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn sync_line(syncing: Option<f64>, sync_pct: &Option<String>) -> String {
    if syncing == Some(1.0) {
        match sync_pct.as_deref().filter(|p| !p.is_empty()) {
            Some(p) => format!("yes {p}%"),
            None => "yes".into(),
        }
    } else {
        "no".into()
    }
}

fn opt<T: ToString>(v: Option<T>) -> String {
    v.map(|v| v.to_string()).unwrap_or_else(|| "?".into())
}

// Node state at the moment of an alert, as plain text (the outbox escapes it).
pub fn fmt_snap(s: &Snapshot) -> String {
    let mut lines = Vec::new();
    if let Some(b) = s.block {
        lines.push(format!("block  {b} Δ{} ({}/s)", opt(s.block_delta), s.bps));
    }
    if let Some(l) = s.lag {
        lines.push(format!("lag    {l}"));
    }
    if let Some(p) = s.exec_p99 {
        lines.push(format!("exec   p99 {p}ms"));
    }
    if let Some(led) = s.led24h {
        lines.push(format!("rounds {led} led, {} missed (24h)", opt(s.missed24h)));
    }
    if let Some(p) = s.peers {
        lines.push(format!("peers  {}", opt_num(Some(p))));
    }
    if let Some(p) = s.p99 {
        lines.push(format!("p99    {}ms", opt_num(Some(p))));
    }
    if let Some(r) = s.rpc_ms {
        lines.push(format!("rpc    {r}ms"));
    }
    if s.syncing.is_some() {
        lines.push(format!("sync   {}", sync_line(s.syncing, &s.sync_pct)));
    }
    if let Some(u) = &s.uptime {
        lines.push(format!("up     {u}"));
    }
    if let Some(v) = &s.ver {
        lines.push(format!("ver    {v}"));
    }
    lines.join("\n")
}

fn round_lines(r: &RoundSummary) -> Vec<String> {
    let line = |label: &str, a: &RoundAgg| {
        format!(
            "{label} led {}, ok {}, missed {}, unseen {}, late {}, blocks {}, txs {}",
            a.led,
            a.ok,
            a.missed + a.failed,
            a.not_received,
            a.late_tmo,
            a.committed,
            a.txs
        )
    };
    let net = if r.h24.net_rounds > 0 { format!("{:.2}", r.h24.net_missed as f64 / r.h24.net_rounds as f64 * 100.0) } else { "?".into() };
    let size = |a: &RoundAgg| match size_ratio(a) {
        Some(ratio) => format!("{}% ({} blocks)", crate::format::js_round(ratio * 100.0), a.size_samples),
        None => "?".into(),
    };
    let mut lines = vec![
        line("1h   ", &r.h1),
        line("24h  ", &r.h24),
        format!("size   our blocks vs network: 1h {}, 24h {}", size(&r.h1), size(&r.h24)),
        format!("fwd    txs forwarded to us: 1h {}, 24h {}", r.h1.fwd_txs, r.h24.fwd_txs),
        format!("net    {:.2} rounds/s, TC {}/{} ({net}%) 24h", r.rounds_per_sec, r.h24.net_missed, r.h24.net_rounds),
        format!("net10m TC {}/{}, longest TC run {}", r.net10m.tc, r.net10m.rounds, r.net10m.max_tc_run),
    ];
    if let Some(m) = r.last_missed {
        let at = chrono::DateTime::from_timestamp_millis(m.t).map(|d| d.format("%Y-%m-%dT%H:%M:%S").to_string()).unwrap_or_default();
        lines.push(format!("last   missed round {} at {at}Z", m.round));
    }
    lines
}

fn status_icon(s: Status) -> &'static str {
    match s {
        Status::Crit => "🔴",
        Status::Warn => "🟡",
        Status::Ok => "🟢",
    }
}

// On-demand status reply for the Telegram bot's /status command.
pub fn frame_status_message(f: &Frame) -> String {
    if let Some(e) = f.error {
        let name = if e == crate::types::FrameError::Rpc { "RPC" } else { "METRICS" };
        return format!("⚠️ <b>monadar</b> — {name} error");
    }
    let mut lines = vec![
        format!("block  {} Δ{} ({}/s)", f.block, opt(f.block_delta), f.bps),
        format!("lag    {}", f.exec_lag),
        format!("exec   {}", f.exec.map(|e| format!("p50 {}ms p99 {}ms max {}ms", e.p50, e.p99, e.max)).unwrap_or_else(|| "?".into())),
    ];
    match &f.rounds {
        Some(r) => lines.extend(round_lines(r)),
        None => lines.push("rounds ?".into()),
    }
    lines.extend([
        format!("peers  {}", opt_num(f.peers)),
        format!("p99    {}ms", opt_num(f.p99)),
        format!("rpc    {}ms", f.rpc_ms),
        format!("sync   {}", sync_line(f.syncing, &f.sync_pct)),
        format!("up     {}", f.uptime),
        format!("ver    {}", f.ver),
    ]);
    let head = format!("{} <b>monadar</b> — {}  <i>{}</i>", status_icon(f.status), f.status.as_str().to_uppercase(), html(&f.ts));
    let warns: String = f.warnings.iter().map(|w| format!("\n• {}", html(&w.short))).collect();
    format!("{head}\n<pre>{}</pre>{warns}", html(&lines.join("\n")))
}

// Recent-events reply for the Telegram bot's /events command.
pub fn frame_events_message(f: &Frame) -> String {
    if f.history.is_empty() {
        return "No recent events.".into();
    }
    let rows: Vec<String> = f
        .history
        .iter()
        .map(|e| {
            let icon = match e.level {
                EventLevel::Resolved => "✓",
                EventLevel::Crit | EventLevel::Error => "✗",
                EventLevel::Warn => "▲",
            };
            format!("{} {icon} {}", e.time, e.text)
        })
        .collect();
    format!("<b>Recent events</b>\n<pre>{}</pre>", html(&rows.join("\n")))
}
