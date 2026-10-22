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

pub fn render_history(entries: &[HistoryEntry]) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let mut out = format!("{EOL}\n{}{EOL}\n", dim("Recent events:"));
    for e in entries {
        out += &format!("{} {} {}{EOL}\n", dim(&e.time), event_icon(e.level), e.text);
    }
    out
}
