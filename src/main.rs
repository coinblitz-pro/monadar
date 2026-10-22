mod alerts;
mod args;
mod bft_log;
mod client;
mod collect;
mod colors;
mod config;
mod daemon;
mod event_history;
mod events;
mod exec_log;
#[cfg(test)]
mod fixture_replay;
mod format;
mod frame;
mod history_render;
mod http;
mod install;
mod journal;
mod ledger_log;
mod log;
mod metric_snapshot;
mod metrics;
mod notify;
mod otlp;
mod poll;
mod protocol;
mod render;
mod rich;
mod socket;
mod telegram;
mod telegram_bot;
mod types;
mod ui;

use std::process::Command;

use serde_json::json;
use tokio::signal::unix::{SignalKind, signal};

use crate::collect::Collector;
use crate::config::{Settings, default_config_path};
use crate::render::{frame_body, paint};
use crate::ui::{Key, Ui, quit, setup_input, write_out};

fn write_config_template() {
    let p = default_config_path();
    if p.exists() {
        println!("Config already exists: {}", p.display());
        return;
    }
    let template = json!({
        "telegram": { "token": "", "chat_id": "", "delay_ticks": 3, "cooldown_min": 5, "rich": true, "api_url": "" },
        "thresholds": { "peers_min": 50, "p99_max": 500, "lag_max": 5, "rpc_max": 200, "tc_max": 0, "exec_ms_max": 250, "stuck_sec": 5, "net_tc_pct_max": 3, "net_tc_run_max": 1 },
        "log": { "file": "", "flush_sec": 60, "max_size_mb": 50, "keep": 5, "events": "" },
        "socket": "",
        "cpu": "",
        "otlp": "127.0.0.1:4317",
        "node_key": "",
        "heartbeat_url": "",
        "heartbeat_sec": 60,
    });
    if let Err(e) = std::fs::write(&p, serde_json::to_string_pretty(&template).unwrap() + "\n") {
        eprintln!("Failed to write {}: {e}", p.display());
        std::process::exit(1);
    }
    println!("Created {}", p.display());
}

fn apply_affinity(s: &Settings) {
    let Some(cpu) = &s.cpu else { return };
    let ok = Command::new("taskset")
        .args(["-cp", cpu, &std::process::id().to_string()])
        .stdout(std::process::Stdio::null())
        .status()
        .is_ok_and(|st| st.success());
    if !ok {
        eprintln!("Failed to set CPU affinity to {cpu}");
        std::process::exit(1);
    }
}

// Standalone mode: poll + render in one process, no daemon/socket.
async fn run_foreground(s: &Settings) {
    event_history::seed_history();
    log::init_log_timer();

    let on_quit = || -> ! {
        log::flush_log();
        quit(0, "\x1b[?25h\n")
    };
    let mut ui = Ui::new(s.args.verbose);
    let mut body = String::new();
    let mut keys = setup_input();
    let mut sigint = signal(SignalKind::interrupt()).expect("signal handler");
    let mut sigterm = signal(SignalKind::terminate()).expect("signal handler");
    write_out("\x1b[2J\x1b[H\x1b[?25l");

    // Ticks run in their own task so keypresses repaint instantly mid-tick.
    let (tx, mut frames) = tokio::sync::mpsc::channel(1);
    let interval = s.interval;
    tokio::spawn(async move {
        let mut ticks = tokio::time::interval(interval);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut collector = Collector::new().await;
        loop {
            ticks.tick().await;
            if tx.send(collector.tick().await).await.is_err() {
                return;
            }
        }
    });

    loop {
        tokio::select! {
            Some(frame) = frames.recv() => {
                if !frame.log_line.is_empty() {
                    log::log_line(&frame.log_line);
                }
                if s.args.bell && frame.new_crit {
                    write_out("\x07");
                }
                body = frame_body(&frame);
                paint(&ui, &body);
            }
            Some(k) = keys.recv() => match ui.handle_key(k) {
                Some(Key::Quit) => on_quit(),
                Some(Key::Repaint) => paint(&ui, &body),
                None => {}
            },
            _ = sigint.recv() => on_quit(),
            _ = sigterm.recv() => on_quit(),
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let s = config::init();

    if s.args.help {
        print!("{}", args::USAGE);
        return;
    }
    if !s.args.errors.is_empty() {
        for e in &s.args.errors {
            eprintln!("monadar: {e}");
        }
        eprintln!("run 'monadar --help' for usage");
        std::process::exit(2);
    }

    if let Some(t) = &s.config.telegram {
        telegram::set_rich(t.rich);
    }

    if s.args.version {
        println!("monadar {}", config::BUILD_VERSION);
    } else if s.args.should_init {
        write_config_template();
    } else if s.args.install {
        install::install(s.args.print);
    } else if s.args.uninstall {
        install::uninstall();
    } else if s.args.status {
        install::status();
    } else if s.args.telegram_test {
        if let Err(e) = telegram_bot::send_samples().await {
            eprintln!("monadar: {e}");
            std::process::exit(1);
        }
    } else {
        apply_affinity(s);
        if s.args.daemon {
            daemon::run_daemon().await;
        } else if s.args.foreground {
            run_foreground(s).await;
        } else {
            client::attach().await;
        }
    }
}
