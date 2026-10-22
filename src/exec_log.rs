use std::collections::VecDeque;
use std::sync::{Arc, LazyLock, Mutex};

use chrono::NaiveDate;
use regex::Regex;

use crate::format::js_round;
use crate::journal::follow_journal;
use crate::types::ExecStats;

// Per-block execution time from monad-execution's journal. Each executed block
// logs a line like:
//   2026-10-22 09:31:36.204673697 [118970] runloop_monad.cpp:381 LOG_INFO __exec_block,bl=115577020,id=0x..,ts=1792661496200,tx=12,...
// The leading timestamp (UTC, ns) is when execution finished; `ts` (ms) is
// when execution of the block started (propose_block, runloop_monad.cpp). Their
// difference is the block's execution time (the line's `tot` plus logging),
// not the wait before execution.

const WINDOW_MS: f64 = 60_000.0;
static LINE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d{4})-(\d\d)-(\d\d) (\d\d):(\d\d):(\d\d)\.(\d+) .*__exec_block,bl=(\d+),.*?,ts=(\d+)").unwrap());

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExecSample {
    pub block: i64,
    pub latency_ms: f64,
    pub done_ms: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ExecSnapshot {
    pub stats: Option<ExecStats>,
    pub last_block: Option<i64>,
}

pub fn parse_exec_block_line(line: &str) -> Option<ExecSample> {
    let m = LINE_RE.captures(line)?;
    let n = |i: usize| m[i].parse::<u32>().ok();
    let secs =
        NaiveDate::from_ymd_opt(m[1].parse().ok()?, n(2)?, n(3)?)?.and_hms_opt(n(4)?, n(5)?, n(6)?)?.and_utc().timestamp_millis() as f64;
    let frac_ms = format!("0.{}", &m[7]).parse::<f64>().ok()? * 1000.0;
    let done_ms = secs + frac_ms;
    let ts = m[9].parse::<f64>().ok()?;
    Some(ExecSample { block: m[8].parse().ok()?, latency_ms: done_ms - ts, done_ms })
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    sorted[((sorted.len() as f64 * p).floor() as usize).min(sorted.len() - 1)]
}

pub fn summarize(latencies: &[f64]) -> Option<ExecStats> {
    if latencies.is_empty() {
        return None;
    }
    let mut sorted = latencies.to_vec();
    sorted.sort_by(f64::total_cmp);
    Some(ExecStats {
        count: sorted.len() as u64,
        p50: js_round(percentile(&sorted, 0.5)) as i64,
        p99: js_round(percentile(&sorted, 0.99)) as i64,
        max: js_round(sorted[sorted.len() - 1]) as i64,
    })
}

// Rolling window of samples keyed by execution finish time.
pub struct ExecWindow {
    window_ms: f64,
    samples: VecDeque<ExecSample>,
    last_block: Option<i64>,
}

impl ExecWindow {
    pub fn new(window_ms: f64) -> ExecWindow {
        ExecWindow { window_ms, samples: VecDeque::new(), last_block: None }
    }

    pub fn push(&mut self, s: ExecSample) {
        self.samples.push_back(s);
        self.last_block = Some(s.block);
    }

    // The newest executed block, if the journal is live.
    pub fn last_block(&self, now_ms: f64) -> Option<i64> {
        self.samples.back().filter(|s| s.done_ms >= now_ms - self.window_ms).map(|s| s.block)
    }

    pub fn snapshot(&mut self, now_ms: f64) -> ExecSnapshot {
        while self.samples.front().is_some_and(|s| s.done_ms < now_ms - self.window_ms) {
            self.samples.pop_front();
        }
        // No recent lines (execution down, journal unreadable): report nothing so
        // the caller falls back to metrics instead of showing a stale block.
        if self.samples.is_empty() {
            return ExecSnapshot::default();
        }
        let lat: Vec<f64> = self.samples.iter().map(|s| s.latency_ms).collect();
        ExecSnapshot { stats: summarize(&lat), last_block: self.last_block }
    }
}

// Follows the monad-execution journal and feeds parsed lines into a window.
pub fn start_exec_log_tail() -> Arc<Mutex<ExecWindow>> {
    let window = Arc::new(Mutex::new(ExecWindow::new(WINDOW_MS)));
    let w = window.clone();
    follow_journal("monad-execution", None, move |line| {
        if let Some(s) = parse_exec_block_line(line) {
            w.lock().unwrap().push(s);
        }
    });
    window
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: &str = concat!(
        "2026-10-22 09:31:36.204673697 [118970] runloop_monad.cpp:381 LOG_INFO        ",
        "__exec_block,bl=115577020,id=0xb5435aaedcc0e2344f230f720998c4507e015527bb5b19779c1734bc93ec3b0a,",
        "ts=1792661496200,tx=   12,rt=  10,rtp=83.33%,sr=  143us,txe=  2479us,cmt=  1950us"
    );

    #[test]
    fn extracts_block_and_latency_from_exec_block_line() {
        let s = parse_exec_block_line(LINE).unwrap();
        assert_eq!(s.block, 115577020);
        let base = NaiveDate::from_ymd_opt(2026, 10, 22).unwrap().and_hms_opt(9, 31, 36).unwrap().and_utc().timestamp_millis() as f64;
        assert_eq!(s.done_ms, base + 204.673697);
        // Epoch ms (~1.8e12) leaves ~0.2µs of double precision for the fraction.
        assert!((s.latency_ms - 4.673697).abs() < 1e-3);
    }

    #[test]
    fn ignores_other_execution_log_lines() {
        assert_eq!(
            parse_exec_block_line("2026-10-22 09:31:36.204699535 [118970] runloop_monad.cpp:102 LOG_INFO        Run to block= 115577020"),
            None
        );
        assert_eq!(parse_exec_block_line(""), None);
    }

    #[test]
    fn summarize_computes_percentiles_and_max() {
        let lat: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(summarize(&lat), Some(ExecStats { count: 100, p50: 51, p99: 100, max: 100 }));
        assert_eq!(summarize(&[]), None);
    }

    #[test]
    fn window_drops_samples_older_than_the_window() {
        let mut w = ExecWindow::new(1000.0);
        w.push(ExecSample { block: 1, latency_ms: 500.0, done_ms: 0.0 });
        w.push(ExecSample { block: 2, latency_ms: 5.0, done_ms: 900.0 });
        assert_eq!(w.snapshot(1500.0), ExecSnapshot { stats: Some(ExecStats { count: 1, p50: 5, p99: 5, max: 5 }), last_block: Some(2) });
        assert_eq!(w.snapshot(5000.0), ExecSnapshot::default());
    }
}
