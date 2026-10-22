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
const BOOL_FLAGS: [&str; 10] = ["--foreground", "--verbose", "-v", "--bell", "--init", "--print", "--version", "-V", "--help", "-h"];
const COMMANDS: [&str; 5] = ["install", "uninstall", "status", "daemon", "telegram-test"];
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
    pub telegram_test: bool,
    pub foreground: bool,
    pub verbose: bool,
    pub bell: bool,
    pub should_init: bool,
    pub print: bool,
    pub version: bool,
    pub help: bool,
    // Unknown flags, stray arguments, missing values. Nothing runs when this is
    // not empty: a typo must not end up in a systemd unit or be silently ignored.
    pub errors: Vec<String>,
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

pub const USAGE: &str = "\
usage: monadar [command] [flags] [interval-sec]

commands:
  (none)       attach a viewer to the running daemon
  daemon       run the collector + socket server (what the service runs)
  install      write the systemd unit / launchd plist (--print: show it instead)
  uninstall    remove the service unit
  status       report whether the daemon is running
  telegram-test  send sample alerts and /status to the configured chat

flags:
  --foreground           poll + render in one process, no daemon
  --config <path>        config file (default: monadar.config.json next to the binary)
  --socket <path>        unix socket path
  --log [file]           write the tick log (default file: monadar.log)
  --cpu <list>           pin to CPUs via taskset
  --bell                 terminal bell on a new critical alert
  -v, --verbose          open the parameter help panel on start
  --init                 write a config template next to the binary
  -V, --version          print the build version
  -h, --help             this help
  --peers-min <n>  --p99-max <ms>  --lag-max <n>  --rpc-max <ms>  --tc-max <n>
  --exec-ms-max <ms>  --stuck-sec <s>  --net-tc-pct-max <n>  --net-tc-run-max <n>
                         alert thresholds (override the config)
";

pub fn parse_args(raw: &[String]) -> CliArgs {
    let mut consumed = HashSet::new();
    let mut errors = Vec::new();
    let mut commands = Vec::new();
    let mut intervals = 0;

    for (i, arg) in raw.iter().enumerate() {
        if consumed.contains(&i) {
            continue;
        }
        let a = arg.as_str();
        if VALUE_FLAGS.contains(&a) || OPTIONAL_VALUE_FLAGS.contains(&a) {
            consumed.insert(i);
            if is_value_token(raw.get(i + 1)) {
                consumed.insert(i + 1);
                let is_threshold = THRESHOLD_FLAGS.iter().any(|(_, f)| *f == a);
                if is_threshold && parse_int_arg(Some(&raw[i + 1])).is_none() {
                    errors.push(format!("{a} expects a whole number, got '{}'", raw[i + 1]));
                }
            } else if VALUE_FLAGS.contains(&a) {
                errors.push(format!("{a} needs a value"));
            }
        } else if BOOL_FLAGS.contains(&a) {
            consumed.insert(i);
        } else if COMMANDS.contains(&a) {
            consumed.insert(i);
            commands.push(a);
        } else if a.starts_with('-') {
            errors.push(format!("unknown flag {a}"));
        } else if !a.is_empty() && a.bytes().all(|b| b.is_ascii_digit()) {
            intervals += 1;
            if intervals > 1 {
                errors.push(format!("unexpected argument {a}: the interval is already set"));
            }
        } else {
            errors.push(format!("unexpected argument {a}"));
        }
    }
    if raw.iter().any(|a| a == "--print") && !commands.contains(&"install") {
        errors.push("--print only works with install".into());
    }
    if commands.len() > 1 {
        errors.push(format!("only one command at a time, got {}", commands.join(", ")));
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
        telegram_test: has("telegram-test"),
        foreground: has("--foreground"),
        verbose: has("--verbose") || has("-v"),
        bell: has("--bell"),
        should_init: has("--init"),
        print: has("--print"),
        version: has("--version") || has("-V"),
        help: has("--help") || has("-h"),
        errors,
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
        let a = parse_args(&args(&["--peers-min", "0", "--p99-max", "250", "9"]));
        assert_eq!(a.threshold_overrides, vec![("peers_min", 0.0), ("p99_max", 250.0)]);
        assert_eq!(a.interval_sec, 9);
        assert!(a.errors.is_empty());
    }

    #[test]
    fn rejects_unknown_flags_and_stray_arguments() {
        // `install --help` once wrote "daemon --help" into ExecStart.
        let a = parse_args(&args(&["install", "--hlep"]));
        assert_eq!(a.errors, vec!["unknown flag --hlep"]);
        assert_eq!(parse_args(&args(&["--config"])).errors, vec!["--config needs a value"]);
        assert_eq!(parse_args(&args(&["--p99-max", "bad"])).errors, vec!["--p99-max expects a whole number, got 'bad'"]);
        assert_eq!(parse_args(&args(&["staus"])).errors, vec!["unexpected argument staus"]);
        assert_eq!(parse_args(&args(&["install", "daemon"])).errors, vec!["only one command at a time, got install, daemon"]);
        assert_eq!(parse_args(&args(&["2", "3"])).errors, vec!["unexpected argument 3: the interval is already set"]);
        assert_eq!(parse_args(&args(&["--print"])).errors, vec!["--print only works with install"]);
    }

    #[test]
    fn help_is_a_flag_not_an_error() {
        let a = parse_args(&args(&["install", "--help"]));
        assert!(a.help);
        assert!(a.errors.is_empty());
    }
}
