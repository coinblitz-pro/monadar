use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};

use crate::config::{dirname, try_settings};
use crate::log::rotate_if_needed;
use crate::types::{EventLevel, HistoryEntry};

// Stored events carry a local `YYYY-MM-DD HH:MM:SS` timestamp so display is
// consistent with the dashboard (which uses local time), and `at[5..10]` /
// `at[11..19]` cleanly yield the MM-DD and HH:MM:SS parts.
pub struct StoredEvent {
    pub at: String,
    pub level: EventLevel,
    pub text: String,
}

fn events_file() -> Option<&'static str> {
    try_settings()?.events_file.as_deref()
}

// Append one event as `timestamp<TAB>level<TAB>text`. Called only on warning
// state changes / restarts (low-frequency), so a sync append is fine and keeps
// the file consistent without a flush timer.
pub fn persist_event(e: &HistoryEntry) {
    let Some(s) = try_settings() else { return };
    let Some(file) = &s.events_file else { return };
    let _ = fs::create_dir_all(dirname(file));
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(file) {
        let _ = writeln!(f, "{}\t{}\t{}", e.time, e.level.as_str(), e.text);
    }
    rotate_if_needed(file, s.log_max_bytes, s.log_keep);
}

fn parse_event(line: &str) -> Option<StoredEvent> {
    let mut parts = line.splitn(3, '\t');
    let at = parts.next()?;
    let level = parts.next()?;
    let text = parts.next()?;
    Some(StoredEvent { at: at.to_string(), level: EventLevel::parse(level), text: text.to_string() })
}

const TAIL_BLOCK: u64 = 64 * 1024;

// The last `n` lines of a file, read backwards block by block: the events file
// can hold millions of lines and only its end is ever shown.
fn tail_lines(path: &str, n: usize, block: u64) -> Vec<String> {
    let Ok(mut f) = File::open(path) else { return Vec::new() };
    let Ok(mut pos) = f.seek(SeekFrom::End(0)) else { return Vec::new() };
    let mut buf: Vec<u8> = Vec::new();
    while pos > 0 && buf.iter().filter(|&&b| b == b'\n').count() <= n {
        let step = pos.min(block);
        pos -= step;
        let mut chunk = vec![0; step as usize];
        if f.seek(SeekFrom::Start(pos)).is_err() || f.read_exact(&mut chunk).is_err() {
            return Vec::new();
        }
        chunk.extend_from_slice(&buf);
        buf = chunk;
    }
    let text = String::from_utf8_lossy(&buf);
    // Stopped mid-file: the first line may be cut.
    let lines: Vec<&str> = text.lines().skip(usize::from(pos > 0)).collect();
    lines[lines.len().saturating_sub(n)..].iter().map(|l| l.to_string()).collect()
}

// The last `n` persisted events, oldest first; right after a rotation the
// rest comes from the previous file.
pub fn load_events(n: usize) -> Vec<StoredEvent> {
    let Some(file) = events_file() else { return Vec::new() };
    let mut lines = tail_lines(file, n, TAIL_BLOCK);
    if lines.len() < n {
        let mut older = tail_lines(&format!("{file}.1"), n - lines.len(), TAIL_BLOCK);
        older.append(&mut lines);
        lines = older;
    }
    lines.iter().filter_map(|l| parse_event(l)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_reads_only_the_last_lines_across_block_boundaries() {
        let dir = std::env::temp_dir().join(format!("monadar-tail-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("events.log");
        let body: String = (1..=50).map(|i| format!("line {i}\n")).collect();
        fs::write(&path, &body).unwrap();
        let p = path.to_str().unwrap();
        for block in [3, 7, 64, 4096] {
            assert_eq!(tail_lines(p, 3, block), vec!["line 48", "line 49", "line 50"], "block {block}");
        }
        assert_eq!(tail_lines(p, 100, 7).len(), 50);
        fs::write(&path, "no newline at end").unwrap();
        assert_eq!(tail_lines(p, 5, 4), vec!["no newline at end"]);
        assert!(tail_lines(dir.join("missing").to_str().unwrap(), 5, 4).is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn parses_tab_separated_events() {
        let e = parse_event("2026-10-22 16:36:01\tcrit\tNOT VOTING\tx").unwrap();
        assert_eq!(e.at, "2026-10-22 16:36:01");
        assert_eq!(e.text, "NOT VOTING\tx");
        assert!(parse_event("garbage").is_none());
    }
}
