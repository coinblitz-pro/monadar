use std::collections::VecDeque;
use std::sync::Mutex;

use crate::events::{load_events, persist_event};
use crate::types::HistoryEntry;

const MAX_HISTORY: usize = 5;

static HISTORY: Mutex<VecDeque<HistoryEntry>> = Mutex::new(VecDeque::new());

pub fn push_history(mut entry: HistoryEntry) {
    entry.time = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    persist_event(&entry);
    let mut h = HISTORY.lock().unwrap();
    h.push_back(entry);
    if h.len() > MAX_HISTORY {
        h.pop_front();
    }
}

pub fn seed_history() {
    let events = load_events(MAX_HISTORY);
    let mut h = HISTORY.lock().unwrap();
    h.clear();
    for e in &events {
        h.push_back(HistoryEntry { time: e.at.clone(), level: e.level, text: e.text.clone() });
    }
}

pub fn get_history() -> Vec<HistoryEntry> {
    HISTORY.lock().unwrap().iter().cloned().collect()
}
