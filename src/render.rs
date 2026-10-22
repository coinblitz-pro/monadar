use std::sync::Mutex;

use crate::bft_log::size_ratio;
use crate::colors::{cyan, dim, green, red, yellow};
use crate::config::{BUILD_VERSION, settings};
use crate::events::load_events;
use crate::format::{js_round, num, opt_num, vis_len};
use crate::history_render::{event_icon, render_history};
use crate::log::{get_recent, recent_count};
use crate::types::{CounterCell, Frame, FrameError, Latency, RoundAgg, Status};
use crate::ui::{Overlay, Ui, terminal_size, write_out};

const EOL: &str = "\x1b[K";

// Our block size relative to neighbouring network blocks (100% = same), gas
// with ledger-tail. Yellow below 80% once there are enough samples.
fn size_vs_net(a: &RoundAgg) -> String {
    let Some(ratio) = size_ratio(a) else { return "?".into() };
    let pct = format!("{}% net", js_round(ratio * 100.0));
    if ratio < 0.8 && a.size_samples >= 10 { yellow(&pct) } else { pct }
}

fn latency(l: Option<Latency>) -> String {
    l.map(|l| format!("{}/{}ms", l.p50, l.p99)).unwrap_or("?".into())
}

// A count over a window, e.g. "3 10m"; "?" until the counter is seen.
fn windowed(n: Option<f64>, window: &str) -> String {
    format!("{} {}", n.map(num).unwrap_or("?".into()), dim(window))
}

fn fmt_cell(cell: &CounterCell) -> String {
    format!("{} Δ{}", opt_num(cell.value), cell.delta.map(num).unwrap_or("-".into()))
}

type Rows = Vec<(&'static str, String)>;

// Space between a label and its value.
const GAP: usize = 2;

// A group of values in a rounded, dim box with its title in the top edge.
struct Panel {
    title: &'static str,
    rows: Rows,
}

// Widest content each panel has had, so a value like 99.9% → 100.0% does not
// shift every panel to its right on each frame. Reset when the terminal width
// changes.
static WIDTHS: Mutex<(usize, Vec<(&'static str, usize)>)> = Mutex::new((0, Vec::new()));

impl Panel {
    fn label_width(&self) -> usize {
        self.rows.iter().map(|(l, _)| l.chars().count()).max().unwrap_or(0)
    }

    fn inner_width(&self) -> usize {
        let value_w = self.rows.iter().map(|(_, v)| vis_len(v)).max().unwrap_or(0);
        (self.label_width() + GAP + value_w).max(self.title.chars().count() + 2)
    }

    // `height` rows, padding with blank ones so panels side by side line up.
    fn lines(&self, height: usize, inner: usize) -> Vec<String> {
        let label_w = self.label_width();
        let title_w = self.title.chars().count();
        let mut out = vec![format!("{}{}{}", dim("╭─ "), cyan(self.title), dim(&format!(" {}╮", "─".repeat(inner - title_w - 1))))];
        for i in 0..height {
            let content = match self.rows.get(i) {
                Some((l, v)) => {
                    format!("{}{}{v}{}", dim(&format!("{l:<label_w$}")), " ".repeat(GAP), " ".repeat(inner - label_w - GAP - vis_len(v)))
                }
                None => " ".repeat(inner),
            };
            out.push(format!("{}{content}{}", dim("│ "), dim(" │")));
        }
        out.push(dim(&format!("╰{}╯", "─".repeat(inner + 2))));
        out
    }
}

// Panels side by side, wrapping onto another band when the terminal is narrow.
fn layout(panels: &[Panel], cols: usize) -> Vec<String> {
    let inner: Vec<usize> = {
        let mut seen = WIDTHS.lock().unwrap();
        if seen.0 != cols {
            *seen = (cols, Vec::new());
        }
        panels
            .iter()
            .map(|p| {
                let w = p.inner_width();
                match seen.1.iter_mut().find(|(t, _)| *t == p.title) {
                    Some((_, max)) => {
                        *max = (*max).max(w);
                        *max
                    }
                    None => {
                        seen.1.push((p.title, w));
                        w
                    }
                }
            })
            .collect()
    };
    let width = |i: usize| inner[i] + 4;
    let mut out = Vec::new();
    let mut start = 0;
    while start < panels.len() {
        let mut end = start + 1;
        let mut w = width(start);
        while end < panels.len() && w + 1 + width(end) <= cols {
            w += 1 + width(end);
            end += 1;
        }
        let height = panels[start..end].iter().map(|p| p.rows.len()).max().unwrap_or(0);
        let boxes: Vec<Vec<String>> = (start..end).map(|i| panels[i].lines(height, inner[i])).collect();
        for i in 0..height + 2 {
            out.push(boxes.iter().map(|b| b[i].as_str()).collect::<Vec<_>>().join(" "));
        }
        start = end;
    }
    out
}

// Build the colored dashboard body from a Frame (or an error line), using this
// viewer's own thresholds for coloring. One row per question: is the node
// keeping up (chain), voting (votes), leading well (leader), how is the network
// (network), are transactions reaching us (mempool). Values that rarely change
// go into the header.
pub fn frame_body(f: &Frame) -> String {
    let th = &settings().thresholds;
    if f.error == Some(FrameError::Rpc) {
        return format!("{}{EOL}\n", red("ERROR: RPC failed")) + &render_history(&f.history);
    }
    let block_delta_str = f.block_delta.map(|d| d.to_string()).unwrap_or("-".into());
    if f.error == Some(FrameError::Metrics) {
        return format!("block: {} Δ{block_delta_str}  |  {}{EOL}\n", f.block, yellow("metrics unavailable")) + &render_history(&f.history);
    }
    let firing = |key: &str| f.warnings.iter().any(|w| w.key == key);

    // chain: is the node keeping up
    let block_str = format!("{} Δ{block_delta_str}", f.block);
    let block_val = if firing("STUCK") { red(&block_str) } else { block_str };
    let exec_val = match f.exec_block {
        Some(eb) => {
            let diff = eb - f.block;
            let diff_str = if diff >= 0 { format!("+{diff}") } else { diff.to_string() };
            let note = if f.exec_finalized { format!("({diff_str} finalized)") } else { format!("({diff_str})") };
            if f.exec_lag as f64 > th.lag_max { yellow(&format!("{eb} {note}")) } else { format!("{eb} {}", dim(&note)) }
        }
        None => "?".into(),
    };
    let exec_time = match f.exec {
        Some(e) => {
            let ms = format!("{}/{}ms", e.p50, e.p99);
            if e.p99 as f64 > th.exec_ms_max { yellow(&ms) } else { ms }
        }
        None => "?".into(),
    };
    let mut chain = vec![
        ("block", block_val),
        ("rate", format!("{} {}", f.bps, dim("blocks/s"))),
        ("exec", exec_val),
        ("exec_ms", exec_time),
        ("tps", f.tps.map(|t| format!("{t:.0}")).unwrap_or("?".into())),
    ];
    if f.syncing == Some(1.0) {
        chain.push(("sync", red(&format!("YES {}%", f.sync_pct.as_deref().unwrap_or("?")))));
    }
    if f.rpc_ms as f64 > th.rpc_max {
        chain.push(("rpc", yellow(&format!("{}ms", f.rpc_ms))));
    }

    // votes: are we participating
    let voted_val = match f.voted_pct {
        Some(v) => {
            let s = format!("{v:.1}%");
            if firing("NOT_VOTING") || v < 95.0 {
                red(&s)
            } else if v < 99.0 {
                yellow(&s)
            } else {
                green(&s)
            }
        }
        None => "?".into(),
    };
    // The lifetime counter says nothing; the per-tick growth does.
    let votes_val = format!("Δ{}", f.votes.delta.map(num).unwrap_or("-".into()));
    let votes_val = if firing("NOT_VOTING") { red(&votes_val) } else { votes_val };
    let tmo_val = windowed(f.timeouts_10m, "10m");
    let tmo_val = if firing("LATE") || firing("TMO") { yellow(&tmo_val) } else { tmo_val };
    let votes = vec![("voted", format!("{voted_val} {}", dim("10m"))), ("votes", votes_val), ("tmo", tmo_val)];

    // leader: our leader rounds
    let leader = match &f.rounds {
        Some(r) => {
            let d = &r.h24;
            let lost = d.missed + d.failed;
            let missed_val = if r.missed10m > 0 {
                red(&lost.to_string())
            } else if lost > 0 {
                yellow(&lost.to_string())
            } else {
                lost.to_string()
            };
            let mut rows = vec![("led", d.led.to_string()), ("led 1h", r.h1.led.to_string()), ("missed", missed_val)];
            // Unseen rounds and late timeouts are known only while bft logs at DEBUG.
            if r.bft_debug {
                let unseen = d.not_received.to_string();
                rows.push(("unseen", if d.not_received > 0 { yellow(&unseen) } else { unseen }));
                let late = d.late_tmo.to_string();
                rows.push(("late", if r.late_tmo10m > 0 { yellow(&late) } else { late }));
            }
            rows.extend([("txs", d.txs.to_string()), ("size", size_vs_net(d))]);
            rows
        }
        None => vec![("prop", fmt_cell(&f.proposals)), ("share", f.leader_pct.clone())],
    };

    // network: the network around us, not our node
    let peers_val = match f.peers {
        Some(p) if p < th.peers_min => yellow(&num(p)),
        p => opt_num(p),
    };
    let tc_val = match &f.rounds {
        Some(r) => {
            let s = windowed(Some(r.net10m.tc as f64), "10m");
            if firing("NET_DEGRADED") { yellow(&s) } else { s }
        }
        None => windowed(f.tc_10m, "10m"),
    };
    let recv_val = match f.p99 {
        Some(p) if p > th.p99_max => yellow(&format!("{}ms", num(p))),
        p => format!("{}ms", opt_num(p)),
    };
    let mut network = vec![("peers", peers_val), ("sessions", opt_num(f.sessions)), ("tc", tc_val)];
    if let Some(r) = &f.rounds {
        network.push(("delay", latency(r.proposal_delay)));
        network.push(("final", latency(r.finality)));
    }
    network.push(("recv_p99", recv_val));

    // mempool: are transactions reaching us
    let mempool = vec![("txpool", opt_num(f.txpool)), ("max", windowed(f.txpool_max, "10m")), ("fwd", windowed(f.fwd_10m, "10m"))];

    let panels = [
        Panel { title: "chain", rows: chain },
        Panel { title: "votes", rows: votes },
        Panel { title: if f.rounds.is_some() { "leader · 24h" } else { "leader" }, rows: leader },
        Panel { title: "network", rows: network },
        Panel { title: "mempool", rows: mempool },
    ];
    let cols = terminal_size().map(|(_, c)| c).unwrap_or(200);

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
    let node = dim(&format!("{} · {} · {} · up {} · stake {}", f.key, f.network, f.ver, f.uptime, f.stake_pct));
    let mut body = format!("{}  {}  {node}{version_hint}{EOL}\n{EOL}\n", cyan("monadar"), dim(&f.ts));
    for line in layout(&panels, cols) {
        body += &format!("{line}{EOL}\n");
    }
    body += &format!("{EOL}\n{status}{EOL}\n");
    body += &render_history(&f.history);
    body
}

fn render_help() -> String {
    let mut out = format!("{EOL}\n");
    let h = |l: &str, d: &str| format!("{}{EOL}\n", dim(&format!("{l:<10} {d}")));
    out += &h("header", "our key · network · node version · uptime · stake");
    out += &format!("{EOL}\n");
    out += &h("chain", "is the node keeping up");
    out += &h("  block", "proposed head (RPC), Δ per tick");
    out += &h("  rate", "blocks per second, last 30 samples");
    out += &h("  exec", "executed block (execution journal), diff to block; 'finalized' = metric fallback");
    out += &h("  exec_ms", "block execution time p50/p99, last 60s");
    out += &h("  tps", "transactions per second, last 30s");
    out += &h("votes", "are we voting");
    out += &h("  voted", "votes created per proposal handled, last 10 min");
    out += &h("  votes", "votes created this tick");
    out += &h("  tmo", "our local round timeouts, last 10 min (most follow other leaders' misses)");
    out += &h("leader", "our leader rounds over 24h (ledger-tail, or bft at DEBUG)");
    out += &h("  led", "rounds we led; led 1h = over the last hour");
    out += &h("  missed", "our rounds ended by TC (missed + failed)");
    out += &h("  unseen", "our rounds we never saw (bft DEBUG only)");
    out += &h("  late", "rounds we timed out on that the network completed (bft DEBUG only)");
    out += &h("  txs", "transactions in our committed blocks");
    out += &h("  size", "our block size vs the 50 network blocks before it (gas)");
    out += &h("network", "the network, not our node");
    out += &h("  peers", "discovered peers; sessions = encrypted sessions");
    out += &h("  tc", "network rounds ended by TC, last 10 min");
    out += &h("  delay", "how late proposals reach us, p50/p99, 10 min");
    out += &h("  final", "block timestamp to finalization, p50/p99, 10 min");
    out += &h("  recv_p99", "receive latency of broadcast messages, p99 over the node's 30s window");
    out += &h("mempool", "are transactions reaching us");
    out += &h("  txpool", "transactions in our pool now (it drains within a second)");
    out += &h("  max", "largest txpool size over 10 min: whether txs reach us at all");
    out += &h("  fwd", "transactions other nodes forwarded to us, last 10 min");
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
// Keeps the lines that fit in `rows` screen rows, counting a line wider than
// the terminal as the rows it wraps onto.
fn fit(text: &str, rows: usize, cols: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for line in text.split_inclusive('\n') {
        let h = vis_len(line.trim_end_matches('\n')).div_ceil(cols.max(1)).max(1);
        if used + h > rows {
            break;
        }
        used += h;
        out += line;
    }
    out
}

pub fn paint(ui: &Ui, body: &str) {
    let content = match ui.overlay {
        Overlay::Log => render_log(),
        Overlay::Events => render_events(),
        Overlay::Help => format!("{body}{}", render_help()),
        Overlay::None => body.to_string(),
    };
    let footer = render_footer(ui);
    // More lines than the terminal has would scroll it, and every repaint would
    // jump. The last row stays free: the footer's final newline moves there.
    let (rows, cols) = terminal_size().unwrap_or((usize::MAX / 2, usize::MAX / 2));
    let footer_rows = footer.matches('\n').count();
    let mut out = String::from("\x1b[H");
    out += &fit(&content, rows.saturating_sub(footer_rows + 1), cols);
    out += &footer;
    out += "\x1b[J";
    write_out(&out);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_stops_before_the_screen_would_scroll() {
        let text = "a\nbbbbbbbbbb\nc\nd\n";
        assert_eq!(fit(text, 10, 80), text);
        // "bbbbbbbbbb" wraps onto two rows of a 5-column terminal.
        assert_eq!(fit(text, 3, 5), "a\nbbbbbbbbbb\n");
        assert_eq!(fit(text, 2, 5), "a\n");
        assert_eq!(fit("\x1b[2mab\x1b[0m\nc\n", 1, 2), "\x1b[2mab\x1b[0m\n", "colour codes take no space");
    }

    #[test]
    fn wrap_line_breaks_on_spaces_and_indents_continuations() {
        assert_eq!(wrap_line("aaaa bbbb cccc dddd", 10), vec!["aaaa bbbb", "    cccc dddd"]);
        assert_eq!(wrap_line("short", 10), vec!["short"]);
        assert_eq!(wrap_line("abcdefghijklmnop", 10), vec!["abcdefghij", "    klmnop"]);
    }
}
