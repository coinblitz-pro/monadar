use std::collections::HashSet;

const VALUE_FLAGS: [&str; 12] = [
    "--config",
    "--socket",
    "--peers-min",
    "--p99-max",
    "--lag-max",
    "--rpc-max",
    "--tc-max",
    "--exec-ms-max",
    "--stuck-sec",
    "--net-tc-pct-max",
    "--net-tc-run-max",
    "--cpu",
];

const OPTIONAL_VALUE_FLAGS: [&str; 1] = ["--log"];
const COMMANDS: [&str; 4] = ["install", "uninstall", "status", "daemon"];
const SERVICE_DROP: [&str; 7] = ["install", "uninstall", "status", "daemon", "--foreground", "--init", "--print"];

const THRESHOLD_FLAGS: [(&str, &str); 9] = [
    ("peers_min", "--peers-min"),
    ("p99_max", "--p99-max"),
    ("lag_max", "--lag-max"),
    ("rpc_max", "--rpc-max"),
    ("tc_max", "--tc-max"),
    ("exec_ms_max", "--exec-ms-max"),
    ("stuck_sec", "--stuck-sec"),
    ("net_tc_pct_max", "--net-tc-pct-max"),
    ("net_tc_run_max", "--net-tc-run-max"),
];

#[derive(Clone, Debug, Default)]
pub struct CliArgs {
    pub install: bool,
    pub uninstall: bool,
    pub status: bool,
    pub daemon: bool,
    pub foreground: bool,
    pub verbose: bool,
    pub bell: bool,
    pub should_init: bool,
    pub print: bool,
    pub version: bool,
    pub config_path: Option<String>,
    pub socket: Option<String>,
    pub cpu: Option<String>,
    pub log_file: Option<String>,
    pub interval_sec: u64,
    // (Thresholds key, value) pairs given on the command line.
    pub threshold_overrides: Vec<(&'static str, f64)>,
    pub service_passthrough: Vec<String>,
}

fn is_value_token(token: Option<&String>) -> bool {
    token.is_some_and(|t| !t.starts_with('-') && !COMMANDS.contains(&t.as_str()))
}

const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

fn parse_int_arg(value: Option<&str>) -> Option<u64> {
    let v = value?;
    if v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    v.parse::<u64>().ok().filter(|n| *n <= MAX_SAFE_INTEGER)
}

fn find_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let i = args.iter().position(|a| a == flag)?;
    is_value_token(args.get(i + 1)).then(|| args[i + 1].as_str())
}

pub fn parse_args(raw: &[String]) -> CliArgs {
    let mut consumed = HashSet::new();

    for (i, arg) in raw.iter().enumerate() {
        if VALUE_FLAGS.contains(&arg.as_str()) || OPTIONAL_VALUE_FLAGS.contains(&arg.as_str()) {
            consumed.insert(i);
            if is_value_token(raw.get(i + 1)) {
                consumed.insert(i + 1);
            }
        } else if arg.starts_with('-') || COMMANDS.contains(&arg.as_str()) {
            consumed.insert(i);
        }
    }

    let interval_sec = raw
        .iter()
        .enumerate()
        .find(|(i, a)| !consumed.contains(i) && !a.is_empty() && a.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|(_, a)| parse_int_arg(Some(a)))
        .unwrap_or(1);

    let log_file = raw
        .iter()
        .position(|a| a == "--log")
        .map(|i| if is_value_token(raw.get(i + 1)) { raw[i + 1].clone() } else { "monadar.log".to_string() });

    let threshold_overrides =
        THRESHOLD_FLAGS.iter().filter_map(|(key, flag)| parse_int_arg(find_value(raw, flag)).map(|n| (*key, n as f64))).collect();

    let has = |s: &str| raw.iter().any(|a| a == s);
    CliArgs {
        install: has("install"),
        uninstall: has("uninstall"),
        status: has("status"),
        daemon: has("daemon"),
        foreground: has("--foreground"),
        verbose: has("--verbose") || has("-v"),
        bell: has("--bell"),
        should_init: has("--init"),
        print: has("--print"),
        version: has("--version") || has("-V"),
        config_path: find_value(raw, "--config").map(String::from),
        socket: find_value(raw, "--socket").map(String::from),
        cpu: find_value(raw, "--cpu").map(String::from),
        log_file,
        interval_sec,
        threshold_overrides,
        service_passthrough: raw.iter().filter(|a| !SERVICE_DROP.contains(&a.as_str())).cloned().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn does_not_treat_another_flag_as_log_value() {
        let a = parse_args(&args(&["--log", "--bell", "5"]));
        assert_eq!(a.log_file.as_deref(), Some("monadar.log"));
        assert!(a.bell);
        assert_eq!(a.interval_sec, 5);
    }

    #[test]
    fn keeps_install_passthrough_args_for_daemon_service() {
        let a = parse_args(&args(&["install", "--print", "--config", "/tmp/my config.json", "--socket", "/tmp/m.sock", "2"]));
        assert!(a.install);
        assert!(a.print);
        assert_eq!(a.service_passthrough, args(&["--config", "/tmp/my config.json", "--socket", "/tmp/m.sock", "2"]));
    }

    #[test]
    fn parses_threshold_overrides_without_consuming_positional_interval() {
        let a = parse_args(&args(&["--peers-min", "0", "--p99-max", "bad", "9"]));
        assert_eq!(a.threshold_overrides, vec![("peers_min", 0.0)]);
        assert_eq!(a.interval_sec, 9);
    }
}
