use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use serde_json::Value;

use crate::args::{CliArgs, parse_args};
use crate::types::{THRESHOLD_KEYS, Thresholds};

pub const RPC_URL: &str = "http://localhost:8080";
pub const METRICS_URL: &str = "http://localhost:8889/metrics";
pub const BUILD_VERSION: &str = env!("MONADAR_BUILD_HASH");

pub const DEFAULT_THRESHOLDS: Thresholds = Thresholds {
    peers_min: 50.0,
    p99_max: 500.0,
    lag_max: 5.0,
    rpc_max: 200.0,
    tc_max: 0.0,
    exec_ms_max: 250.0,
    stuck_sec: 5.0,
    net_tc_pct_max: 3.0,
    net_tc_run_max: 1.0,
};

#[derive(Clone, Debug, Default)]
pub struct TelegramConfig {
    // Bot API base, https://api.telegram.org unless a local Bot API server or proxy is used.
    pub api_url: String,
    pub token: String,
    pub chat_id: String,
    pub delay_ticks: Option<u64>,
    pub cooldown_min: Option<f64>,
}

#[derive(Clone, Debug, Default)]
pub struct LogConfig {
    pub file: Option<String>,
    pub flush_sec: Option<f64>,
    pub max_size_mb: Option<f64>,
    pub keep: Option<i64>,
    pub events: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub telegram: Option<TelegramConfig>,
    pub thresholds: Thresholds,
    pub log: Option<LogConfig>,
    pub cpu: Option<String>,
    pub socket: Option<String>,
    // Built-in OTLP/gRPC receiver address ("" disables it → scrape METRICS_URL).
    pub otlp: Option<String>,
    // secp_key label to attach to OTLP metrics (otelcol used to add it via config).
    pub node_key: Option<String>,
    // Dead man's switch: GET this URL every heartbeat_sec while the daemon runs
    // (healthchecks.io, Uptime Kuma push monitor, …), so an outside service
    // notices when the host, its network, or monadar itself goes away.
    pub heartbeat_url: Option<String>,
    pub heartbeat_sec: Option<f64>,
}

// JSON scalar → string the way JS String() does it (numbers without ".0").
fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        other => Some(other.to_string()),
    }
}

// Like `json.x || undefined`: empty strings count as unset.
fn non_empty(v: Option<&Value>) -> Option<String> {
    v.and_then(scalar).filter(|s| !s.is_empty())
}

fn number(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64)
}

pub fn default_config_path() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_default();
    exe.parent().unwrap_or(Path::new(".")).join("monadar.config.json")
}

pub fn load_config(config_path: Option<&str>) -> Config {
    let path = config_path.map(PathBuf::from).unwrap_or_else(default_config_path);
    let json: Value = std::fs::read_to_string(&path).ok().and_then(|raw| serde_json::from_str(&raw).ok()).unwrap_or(Value::Null);

    let mut thresholds = DEFAULT_THRESHOLDS;
    if let Some(t) = json.get("thresholds") {
        for key in THRESHOLD_KEYS {
            if let Some(v) = number(t.get(key)) {
                thresholds.set(key, v);
            }
        }
    }

    let telegram = json.get("telegram").filter(|t| t.is_object()).map(|t| TelegramConfig {
        api_url: non_empty(t.get("api_url")).unwrap_or_else(|| "https://api.telegram.org".into()).trim_end_matches('/').to_string(),
        token: t.get("token").and_then(scalar).unwrap_or_default(),
        chat_id: t.get("chat_id").and_then(scalar).unwrap_or_default(),
        delay_ticks: t.get("delay_ticks").and_then(Value::as_u64),
        cooldown_min: number(t.get("cooldown_min")),
    });

    let log = json.get("log").filter(|l| l.is_object()).map(|l| LogConfig {
        file: non_empty(l.get("file")),
        flush_sec: number(l.get("flush_sec")),
        max_size_mb: number(l.get("max_size_mb")),
        keep: l.get("keep").and_then(Value::as_i64),
        events: non_empty(l.get("events")),
    });

    Config {
        telegram,
        thresholds,
        log,
        cpu: json.get("cpu").and_then(scalar),
        socket: non_empty(json.get("socket")),
        otlp: json.get("otlp").and_then(scalar),
        node_key: non_empty(json.get("node_key")),
        heartbeat_url: non_empty(json.get("heartbeat_url")),
        heartbeat_sec: number(json.get("heartbeat_sec")),
    }
}

// Everything derived from the command line and the config file, resolved once at startup.
pub struct Settings {
    pub args: CliArgs,
    pub config: Config,
    pub interval: Duration,
    pub socket: Option<String>,
    pub cpu: Option<String>,
    // Where monad-bft/monad-rpc push OTLP (their --otel-endpoint). If the port is
    // taken (an otelcol is running) the collector falls back to METRICS_URL.
    pub otlp_listen: String,
    pub node_key: Option<String>,
    pub thresholds: Thresholds,
    pub log_file: Option<String>,
    // Persistent event log (warning state changes, restarts) — survives daemon
    // restarts. Defaults next to the main log file; disabled if neither is set.
    pub events_file: Option<String>,
    pub log_flush: Duration,
    // Rotate the log when it reaches this size, keeping `log_keep` numbered backups
    // (file.1 … file.N). max_size_mb <= 0 disables rotation (unbounded growth).
    pub log_max_bytes: i64,
    pub log_keep: i64,
    // Undelivered Telegram messages, kept next to the event log.
    pub outbox_file: Option<PathBuf>,
}

pub fn dirname(p: &str) -> String {
    match Path::new(p).parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_string_lossy().into_owned(),
        _ => ".".into(),
    }
}

impl Settings {
    pub fn from_args(args: CliArgs) -> Settings {
        let config = load_config(args.config_path.as_deref());
        let mut thresholds = config.thresholds;
        for (key, v) in &args.threshold_overrides {
            thresholds.set(key, *v);
        }
        let log = config.log.clone().unwrap_or_default();
        let log_file = args.log_file.clone().filter(|s| !s.is_empty()).or(log.file.clone());
        let events_file = log.events.clone().or_else(|| log_file.as_ref().map(|f| format!("{}/events.log", dirname(f))));
        let outbox_file = events_file.as_ref().map(|f| Path::new(&dirname(f)).join("telegram-outbox.json"));
        Settings {
            interval: Duration::from_secs(args.interval_sec),
            socket: args.socket.clone().or(config.socket.clone()),
            cpu: args.cpu.clone().or(config.cpu.clone()).filter(|s| !s.is_empty()),
            otlp_listen: config.otlp.clone().unwrap_or_else(|| "127.0.0.1:4317".into()),
            node_key: config.node_key.clone(),
            thresholds,
            log_file,
            events_file,
            log_flush: Duration::from_secs_f64(log.flush_sec.unwrap_or(60.0).max(0.001)),
            log_max_bytes: (log.max_size_mb.unwrap_or(50.0) * 1024.0 * 1024.0).round() as i64,
            log_keep: log.keep.unwrap_or(5),
            outbox_file,
            config,
            args,
        }
    }
}

static SETTINGS: OnceLock<Settings> = OnceLock::new();

pub fn init() -> &'static Settings {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    SETTINGS.get_or_init(|| Settings::from_args(parse_args(&raw)))
}

pub fn settings() -> &'static Settings {
    SETTINGS.get().expect("settings are initialized in main")
}

// For modules that also run in unit tests, where main never initialized settings.
pub fn try_settings() -> Option<&'static Settings> {
    SETTINGS.get()
}
