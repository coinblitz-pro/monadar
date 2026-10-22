use crate::bft_log::size_ratio;
use crate::colors::{cyan, dim, green, red, yellow};
use crate::config::{BUILD_VERSION, settings};
use crate::events::load_events;
use crate::format::{js_round, num, opt_num, render_table};
use crate::history_render::{event_icon, render_history};
use crate::log::{get_recent, recent_count};
use crate::types::{Cell, CounterCell, Frame, FrameError, Latency, RoundAgg, Status};
use crate::ui::{Overlay, Ui, terminal_size, write_out};

const EOL: &str = "\x1b[K";

// Txs in our committed blocks plus our block size relative to neighbouring
// network blocks (100% = same). Yellow below 80% once there are enough samples.
fn txs_vs_net(a: &RoundAgg) -> String {
    let Some(ratio) = size_ratio(a) else { return a.txs.to_string() };
    let pct = format!("{}% net", js_round(ratio * 100.0));
    let pct = if ratio < 0.8 && a.size_samples >= 10 { yellow(&pct) } else { dim(&pct) };
    format!("{} {pct}", a.txs)
}

fn fmt_cell(cell: &CounterCell) -> String {
    format!("{} Δ{}", opt_num(cell.value), cell.delta.map(num).unwrap_or("-".into()))
}

fn cell(label: &str, value: String) -> Cell {
    Cell { label: label.into(), value }
}

// Build the colored dashboard body from a Frame (or an error line), using this
// viewer's own thresholds for coloring.
pub fn frame_body(f: &Frame) -> String {
    let th = &settings().thresholds;
    if f.error == Some(FrameError::Rpc) {
        return format!("{}{EOL}\n", red("ERROR: RPC failed")) + &render_history(&f.history);
    }
    let block_delta_str = f.block_delta.map(|d| d.to_string()).unwrap_or("-".into());
    if f.error == Some(FrameError::Metrics) {
        return format!("block: {} Δ{block_delta_str}  |  {}{EOL}\n", f.block, yellow("metrics unavailable")) + &render_history(&f.history);
    }

    let sync_val = if f.syncing == Some(1.0) {
        match &f.sync_pct {
            Some(p) if !p.is_empty() => red(&format!("YES {p}%")),
            _ => red("YES!"),
        }
    } else {
        green("no")
    };

    let block_str = format!("{} Δ{block_delta_str}", f.block);
    let block_val = if f.block_delta == Some(0) { red(&block_str) } else { block_str };

    let votes_val =
        if f.votes.delta == Some(0.0) && f.block_delta.is_some_and(|d| d > 0) { red(&fmt_cell(&f.votes)) } else { fmt_cell(&f.votes) };

    // With round data, raw timeout/TC deltas are mostly other leaders' misses:
    // highlight tmo only when we were late on a round the network completed.
    let tmo_bad = match &f.rounds {
        Some(r) => r.late_tmo10m > 0,
        None => f.timeouts.delta.is_some_and(|d| d > 0.0),
    };
    let tmo_val = if tmo_bad { yellow(&fmt_cell(&f.timeouts)) } else { fmt_cell(&f.timeouts) };

    let tc_bad = f.rounds.is_none() && f.tc.delta.is_some_and(|d| d > th.tc_max);
    let tc_val = if tc_bad { yellow(&fmt_cell(&f.tc)) } else { fmt_cell(&f.tc) };

    let lag_evt_val = match f.lag_evt_delta {
        Some(d) if d > 0.0 => yellow(&format!("Δ{}", num(d))),
        d => format!("Δ{}", d.map(num).unwrap_or("-".into())),
    };

    // Prefer journal-based execution latency (p50/p99 ms); fall back to the
    // metric-based block lag when it's unavailable.
    let exec_lag_val = match f.exec {
        Some(e) => {
            let ms = format!("{}/{}ms", e.p50, e.p99);
            if e.p99 as f64 > th.exec_ms_max { yellow(&ms) } else { ms }
        }
        None if f.exec_lag as f64 > th.lag_max => yellow(&f.exec_lag.to_string()),
        None => f.exec_lag.to_string(),
    };

    let peers_val = match f.peers {
        Some(p) if p < th.peers_min => yellow(&num(p)),
        p => opt_num(p),
    };

    let p99_val = match f.p99 {
        Some(p) if p > th.p99_max => yellow(&format!("{}ms", num(p))),
        p => format!("{}ms", opt_num(p)),
    };

    let rpc_str = format!("{}ms", f.rpc_ms);
    let rpc_val = if f.rpc_ms as f64 > th.rpc_max { yellow(&rpc_str) } else { rpc_str };

    let exec_val = match f.exec_block {
        Some(eb) => {
            let diff = eb - f.block;
            let lag = (f.block - eb).max(0);
            let diff_str = if diff >= 0 { format!("+{diff}") } else { diff.to_string() };
            if lag as f64 > th.lag_max { yellow(&format!("{eb} ({diff_str})")) } else { format!("{eb} {}", dim(&format!("({diff_str})"))) }
        }
        None => "?".into(),
    };

    let prop_val = if f.proposals.delta.is_some_and(|d| d > 0.0) { cyan(&fmt_cell(&f.proposals)) } else { fmt_cell(&f.proposals) };

    let voted_val = match f.voted_pct {
        Some(v) => {
            let s = format!("{v:.1}%");
            if v >= 99.9 {
                green(&s)
            } else if v < 95.0 {
                red(&s)
            } else {
                yellow(&s)
            }
        }
        None => "?".into(),
    };

    let mut rows = vec![
        vec![
            cell("block:", format!("{block_val} {}", dim(&format!("{}/s", f.bps)))),
            cell("ver:", f.ver.clone()),
            cell("up:", f.uptime.clone()),
            cell("sync:", sync_val),
            cell("peers:", peers_val),
            cell("sessions:", opt_num(f.sessions)),
        ],
        vec![
            cell("commits:", fmt_cell(&f.commits)),
            cell("votes:", votes_val),
            cell("tmo:", tmo_val),
            cell("tc:", tc_val),
            cell("lag:", exec_lag_val),
            cell("lag_evt:", lag_evt_val),
        ],
        vec![
            cell("exec:", exec_val),
            cell("tx:", fmt_cell(&f.tx)),
            cell("txpool:", opt_num(f.txpool)),
            cell("fwd:", fmt_cell(&f.fwd)),
            cell("p99:", p99_val),
            cell("rpc:", rpc_val),
        ],
        vec![
            cell("key:", cyan(&f.key)),
            cell("net:", f.network.clone()),
            cell("stake:", f.stake_pct.clone()),
            cell("prop:", prop_val),
            cell("leader:", f.leader_pct.clone()),
            cell("voted:", voted_val),
        ],
    ];

    if let Some(r) = &f.rounds {
        let d = &r.h24;
        let lost = (d.missed + d.failed).to_string();
        let missed_val = if r.missed10m > 0 {
            red(&lost)
        } else if lost != "0" {
            yellow(&lost)
        } else {
            lost
        };
        // Unseen rounds and late timeouts are known only while bft logs at DEBUG.
        let unseen_val = if r.bft_debug {
            let late_str = format!("late {}", d.late_tmo);
            format!(
                "{} {}",
                if d.not_received > 0 { yellow(&d.not_received.to_string()) } else { "0".into() },
                if r.late_tmo10m > 0 { yellow(&late_str) } else { dim(&late_str) }
            )
        } else {
            format!("? {}", dim("late ?"))
        };
        let net_pct_str = if d.net_rounds > 0 { format!("{:.2}%", d.net_missed as f64 / d.net_rounds as f64 * 100.0) } else { "?".into() };
        let net_pct = if f.warnings.iter().any(|w| w.key == "NET_DEGRADED") { yellow(&net_pct_str) } else { net_pct_str };
        rows.push(vec![
            cell("led24h:", format!("{} {}", d.led, dim(&format!("1h {}", r.h1.led)))),
            cell("missed:", missed_val),
            cell("unseen:", unseen_val),
            cell("blocks:", d.committed.to_string()),
            cell("txs:", txs_vs_net(d)),
            cell("net_tc:", format!("{net_pct} {}", dim(&format!("{:.2} r/s", r.rounds_per_sec)))),
        ]);
        // From ledger-tail: how late proposals reach us, and time to finality (p50/p99, 10 min).
        if r.proposal_delay.is_some() || r.finality.is_some() {
            let lat = |l: Option<Latency>| l.map(|l| format!("{}/{}ms", l.p50, l.p99)).unwrap_or("?".into());
            rows.push(vec![cell("delay:", lat(r.proposal_delay)), cell("final:", lat(r.finality))]);
        }
    }

    let status = match f.status {
        Status::Crit => red("● CRITICAL"),
        Status::Warn => yellow("▲ WARNING "),
        Status::Ok => green("● OK      "),
    };

    let version_hint = if f.build_version != BUILD_VERSION {
        format!("  {} {}", yellow("restart needed"), dim(&format!("(daemon {} ≠ viewer {BUILD_VERSION})", f.build_version)))
    } else {
        String::new()
    };
    let mut body = format!("{}  {}{version_hint}{EOL}\n{EOL}\n", cyan("monadar"), dim(&f.ts));
    for line in render_table(&rows) {
        body += &format!("{line}{EOL}\n");
    }
    body += &format!("{EOL}\n{status}{EOL}\n");
    body += &render_history(&f.history);
    body
}

fn render_help() -> String {
    let mut out = format!("{EOL}\n");
    let h = |l1: &str, d1: &str, l2: &str, d2: &str| format!("{}{EOL}\n", dim(&format!("{l1:<10} {d1:<35}{l2:<10} {d2}")));
    out += &h("block", "block number + rate (blocks/sec)", "commits", "consensus blocks committed");
    out += &h("ver", "node software version", "votes", "votes created by this node");
    out += &h("up", "node uptime", "tmo", "local timeouts (growth = problem)");
    out += &h("sync", "state sync (no = fully synced)", "tc", "timeout certificates");
    out += &h("peers", "discovered peers", "lag", "exec latency p50/p99 ms, last 60s");
    out += &h("sessions", "wireauth encrypted sessions", "lag_evt", "execution lagging events delta");
    out += &h("exec", "execution ledger block (+diff)", "key", "validator secp256k1 public key");
    out += &h("tx", "transactions committed", "net", "network name");
    out += &h("txpool", "transactions in pool", "stake", "node stake in basis points (%)");
    out += &h("fwd", "forwarded transactions received", "prop", "proposals created (was leader)");
    out += &h("p99", "UDP broadcast latency p99", "leader", "leader rate (prop/handled)");
    out += &h("rpc", "RPC response time", "voted", "vote participation (votes/handled)");
    out += &h("led24h", "rounds we led, 24h (1h)", "missed", "our rounds ended by TC, 24h");
    out += &h("unseen", "our rounds never seen; late = we timed out, net did not", "blocks", "our blocks committed, 24h");
    out += &h("txs", "txs in our blocks; % = size vs neighbours", "net_tc", "network rounds ended by TC, 24h");
    out
}

// Word-wrap a long log line to the available width, breaking on spaces so
// key=val pairs stay intact; continuation lines are indented for readability.
pub fn wrap_line(s: &str, width: usize) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    if width < 10 || chars.len() <= width {
        return vec![s.to_string()];
    }
    let mut out = Vec::new();
    let mut rest = chars.as_slice();
    let mut indent = "";
    while rest.len() > width {
        let limit = width - indent.len();
        let mut cut = rest[..=limit.min(rest.len() - 1)].iter().rposition(|c| *c == ' ').unwrap_or(0);
        if cut == 0 {
            cut = limit;
        }
        out.push(format!("{indent}{}", rest[..cut].iter().collect::<String>()));
        rest = &rest[cut..];
        while rest.first() == Some(&' ') {
            rest = &rest[1..];
        }
        indent = "    ";
    }
    if !rest.is_empty() {
        out.push(format!("{indent}{}", rest.iter().collect::<String>()));
    }
    out
}

fn available_rows() -> usize {
    let rows = terminal_size().map(|(r, _)| r).unwrap_or(40);
    rows.saturating_sub(5).max(5)
}

fn render_log() -> String {
    let cols = terminal_size().map(|(_, c)| c).unwrap_or(120);
    let avail = available_rows();
    // Wrap each entry, then keep the last `avail` visual lines (newest at bottom).
    let wrapped: Vec<String> = get_recent(avail).iter().flat_map(|l| wrap_line(l, cols.saturating_sub(2))).collect();
    let shown = &wrapped[wrapped.len().saturating_sub(avail)..];
    let mut out =
        format!("{EOL}\n{}  {}{EOL}\n{EOL}\n", cyan("▌ Full log"), dim(&format!("({} lines · newest at bottom)", recent_count())));
    if shown.is_empty() {
        out += &format!("{}{EOL}\n", dim("(no log lines yet)"));
    }
    for l in shown {
        out += &format!(" {}{EOL}\n", dim(l));
    }
    out
}

// Persisted event log (warnings, resolves, restarts), newest at bottom.
// Reads the end of the events file each paint so it reflects the daemon's writes.
fn render_events() -> String {
    let all = load_events(available_rows());
    let mut out = format!("{EOL}\n{}  {}{EOL}\n{EOL}\n", cyan("▌ Events"), dim("(newest at bottom)"));
    if all.is_empty() {
        out += &format!("{}{EOL}\n", dim("(no events recorded: set log.file or log.events to persist)"));
    }
    for e in &all {
        let when = format!("{} {}", e.at.get(5..10).unwrap_or(""), e.at.get(11..19).unwrap_or(""));
        out += &format!(" {} {} {}{EOL}\n", dim(&when), event_icon(e.level), e.text);
    }
    out
}

fn render_footer(ui: &Ui) -> String {
    let keys = if ui.overlay == Overlay::None {
        dim("ctrl+o log  ctrl+e events  ctrl+h help")
    } else {
        dim("esc close  ctrl+o log  ctrl+e events  ctrl+h help")
    };
    format!("{EOL}\n{keys}{EOL}\n")
}

// Compose and write the current screen: cursor home → body/overlay → footer →
// clear-to-end. Called after each frame and on every keypress for instant redraw.
pub fn paint(ui: &Ui, body: &str) {
    let mut out = String::from("\x1b[H");
    match ui.overlay {
        Overlay::Log => out += &render_log(),
        Overlay::Events => out += &render_events(),
        Overlay::Help => {
            out += body;
            out += &render_help();
        }
        Overlay::None => out += body,
    }
    out += &render_footer(ui);
    out += "\x1b[J";
    write_out(&out);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_line_breaks_on_spaces_and_indents_continuations() {
        assert_eq!(wrap_line("aaaa bbbb cccc dddd", 10), vec!["aaaa bbbb", "    cccc dddd"]);
        assert_eq!(wrap_line("short", 10), vec!["short"]);
        assert_eq!(wrap_line("abcdefghijklmnop", 10), vec!["abcdefghij", "    klmnop"]);
    }
}
