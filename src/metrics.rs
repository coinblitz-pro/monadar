use regex::Regex;

fn metric_prefix_matches(line: &str, name: &str) -> bool {
    line.strip_prefix(name).is_some_and(|rest| rest.starts_with('{') || rest.starts_with(' '))
}

// The sample value: first token after the labels (label values may contain spaces).
fn sample_value(line: &str) -> Option<&str> {
    let rest = match line.find('{') {
        Some(_) => &line[line.rfind('}')? + 1..],
        None => line.split_once(char::is_whitespace)?.1,
    };
    rest.split_whitespace().next()
}

pub fn metric_value(text: &str, name: &str) -> Option<f64> {
    let mut result = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || !metric_prefix_matches(line, name) {
            continue;
        }
        if let Some(n) = sample_value(line).and_then(|v| v.parse::<f64>().ok()).filter(|n| n.is_finite()) {
            result = Some(n);
        }
    }
    result
}

pub fn metric_label(text: &str, metric: &str, label: &str) -> String {
    let mut result = "?".to_string();
    let re = Regex::new(&format!(r#"{}="((?:\\.|[^"\\])*)""#, regex::escape(label))).expect("label regex");
    for raw in text.lines() {
        let line = raw.trim();
        if metric_prefix_matches(line, metric)
            && let Some(m) = re.captures(line).and_then(|c| c.get(1))
            && !m.as_str().is_empty()
        {
            result = m.as_str().to_string();
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_labeled_and_unlabeled_samples() {
        let text = [
            "# HELP ignored ignored",
            "monad_peer_disc_num_peers{network=\"test\"} 42 1710000000000",
            "monad_total_uptime_us 123000000",
            "monad_with_space{v=\"a b\"} 7",
        ]
        .join("\n");
        assert_eq!(metric_value(&text, "monad_peer_disc_num_peers"), Some(42.0));
        assert_eq!(metric_value(&text, "monad_total_uptime_us"), Some(123000000.0));
        assert_eq!(metric_value(&text, "monad_with_space"), Some(7.0));
        assert_eq!(metric_value(&text, "monad_peer_disc"), None);
    }

    #[test]
    fn ignores_invalid_numeric_samples() {
        assert_eq!(metric_value("metric_a 12\nmetric_a NaN", "metric_a"), Some(12.0));
    }

    #[test]
    fn extracts_requested_label() {
        let text = r#"monad_node_info{service_version="0.14.4",network="testnet"} 1"#;
        assert_eq!(metric_label(text, "monad_node_info", "network"), "testnet");
        assert_eq!(metric_label(text, "monad_node_info", "secp_key"), "?");
    }
}
