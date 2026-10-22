// f64 → text the way JS prints numbers: integers without a fraction.
pub fn num(v: f64) -> String {
    format!("{v}")
}

pub fn opt_num(v: Option<f64>) -> String {
    v.map(num).unwrap_or_else(|| "?".into())
}

// Math.round: halves round up (towards +∞), unlike f64::round.
pub fn js_round(v: f64) -> f64 {
    (v + 0.5).floor()
}

pub fn format_uptime(us: f64) -> String {
    let s = (us / 1_000_000.0).floor() as i64;
    let h = s / 3600;
    let m = (s % 3600) / 60;
    if h >= 24 {
        return format!("{}d{}h", h / 24, h % 24);
    }
    format!("{h}h{m}m")
}

pub fn delta(cur: Option<f64>, prev: Option<f64>) -> Option<f64> {
    Some(cur? - prev?)
}

pub fn vis_len(s: &str) -> usize {
    let mut n = 0;
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if !(c.is_ascii_digit() || c == ';') {
                    break;
                }
            }
            continue;
        }
        n += 1;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vis_len_skips_ansi_codes() {
        assert_eq!(vis_len("\x1b[31mabc\x1b[0m Δ"), 5);
    }

    #[test]
    fn numbers_print_like_js() {
        assert_eq!(num(213.0), "213");
        assert_eq!(num(0.5), "0.5");
        assert_eq!(js_round(2.5), 3.0);
        assert_eq!(format_uptime(90_000_000_000.0), "1d1h");
        assert_eq!(format_uptime(3_660_000_000.0), "1h1m");
    }
}
