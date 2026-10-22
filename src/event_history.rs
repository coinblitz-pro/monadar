use std::collections::VecDeque;
use std::sync::Mutex;

use crate::events::{load_events, persist_event};
use crate::types::HistoryEntry;

const MAX_HISTORY: usize = 5;

static HISTORY: Mutex<VecDeque<HistoryEntry>> = Mutex::new(VecDeque::new());

pub fn push_history(entry: HistoryEntry) {
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
        h.push_back(HistoryEntry { time: e.at.get(11..19).unwrap_or("").to_string(), level: e.level, text: e.text.clone() });
    }
}

pub fn get_history() -> Vec<HistoryEntry> {
    HISTORY.lock().unwrap().iter().cloned().collect()
}
