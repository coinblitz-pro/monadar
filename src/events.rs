use std::fs::{self, OpenOptions};
use std::io::Write;

use crate::config::{dirname, try_settings};
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
    let Some(file) = events_file() else { return };
    let _ = fs::create_dir_all(dirname(file));
    let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(file) {
        let _ = writeln!(f, "{stamp}\t{}\t{}", e.level.as_str(), e.text);
    }
}

pub fn parse_events(raw: &str) -> Vec<StoredEvent> {
    raw.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let at = parts.next()?;
            let level = parts.next()?;
            let text = parts.next()?;
            Some(StoredEvent { at: at.to_string(), level: EventLevel::parse(level), text: text.to_string() })
        })
        .collect()
}

// All persisted events, oldest first. Read on demand by the events overlay.
pub fn load_events() -> Vec<StoredEvent> {
    events_file().and_then(|f| fs::read_to_string(f).ok()).map(|raw| parse_events(&raw)).unwrap_or_default()
}
