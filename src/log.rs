use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::sync::Mutex;

use crate::config::{dirname, settings, try_settings};

// In-memory ring buffer for the Ctrl+O log view (filled regardless of --log)
const MAX_RECENT: usize = 1000;

struct LogState {
    buffer: String,
    recent: VecDeque<String>,
}

static STATE: Mutex<LogState> = Mutex::new(LogState { buffer: String::new(), recent: VecDeque::new() });

// Size-based rotation: when the log reaches log_max_bytes, shift backups
// (file.N-1 → file.N, …, file.1 → file.2) and move the current file to file.1,
// so the live file starts fresh. Keeps at most log_keep numbered backups.
// The events file is rotated the same way.
pub fn rotate_if_needed(file: &str, max_bytes: i64, keep: i64) {
    if max_bytes <= 0 {
        return;
    }
    let Ok(meta) = fs::metadata(file) else { return };
    if (meta.len() as i64) < max_bytes {
        return;
    }
    if keep < 1 {
        let _ = fs::remove_file(file);
        return;
    }
    for i in (1..keep).rev() {
        let _ = fs::rename(format!("{file}.{i}"), format!("{file}.{}", i + 1));
    }
    let _ = fs::rename(file, format!("{file}.1"));
}

pub fn flush_log() {
    let Some(s) = try_settings() else { return };
    let Some(file) = &s.log_file else { return };
    let chunk = std::mem::take(&mut STATE.lock().unwrap().buffer);
    if chunk.is_empty() {
        return;
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(file) {
        let _ = f.write_all(chunk.as_bytes());
    }
    rotate_if_needed(file, s.log_max_bytes, s.log_keep);
}

pub fn log_line(line: &str) {
    let mut st = STATE.lock().unwrap();
    st.recent.push_back(line.to_string());
    if st.recent.len() > MAX_RECENT {
        st.recent.pop_front();
    }
    if try_settings().is_some_and(|s| s.log_file.is_some()) {
        st.buffer.push_str(line);
        st.buffer.push('\n');
    }
}

pub fn get_recent(n: usize) -> Vec<String> {
    let st = STATE.lock().unwrap();
    st.recent.iter().skip(st.recent.len().saturating_sub(n)).cloned().collect()
}

pub fn recent_count() -> usize {
    STATE.lock().unwrap().recent.len()
}

pub fn init_log_timer() {
    let s = settings();
    let Some(file) = &s.log_file else { return };
    let _ = fs::create_dir_all(dirname(file));
    let every = s.log_flush;
    tokio::spawn(async move {
        let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + every, every);
        loop {
            tick.tick().await;
            flush_log();
        }
    });
}
