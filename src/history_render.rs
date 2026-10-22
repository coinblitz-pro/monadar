use crate::colors::{dim, green, red, yellow};
use crate::types::{EventLevel, HistoryEntry};

const EOL: &str = "\x1b[K";

pub fn event_icon(level: EventLevel) -> String {
    match level {
        EventLevel::Crit => red("●"),
        EventLevel::Error => red("✗"),
        EventLevel::Resolved => green("✓"),
        EventLevel::Warn => yellow("▲"),
    }
}

// Event time for display: MM-DD HH:MM:SS.
pub fn event_time(stamp: &str) -> String {
    stamp.get(5..19).unwrap_or(stamp).into()
}

pub fn render_history(entries: &[HistoryEntry]) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let mut out = format!("{EOL}\n{}{EOL}\n", dim("Recent events:"));
    for e in entries {
        out += &format!("{} {} {}{EOL}\n", dim(&event_time(&e.time)), event_icon(e.level), e.text);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_time_shows_month_day_and_time() {
        assert_eq!(event_time("2026-10-22 16:36:01"), "10-22 16:36:01");
        assert_eq!(event_time("16:36:01"), "16:36:01");
    }
}
