use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;

use tokio::io::AsyncWriteExt;
use tokio::net::UnixListener;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::watch;
use tokio::time::MissedTickBehavior;

use crate::collect::Collector;
use crate::config::settings;
use crate::event_history::seed_history;
use crate::log::{flush_log, get_recent, init_log_timer, log_line};
use crate::protocol::encode_msg;
use crate::socket::socket_path;
use crate::telegram_bot::start_telegram_bot;
use crate::types::{Frame, ServerMsg};

const BACKLOG_LINES: usize = 200;

// The latest frame and its encoded snapshot message. Viewers get it through a
// watch channel: a slow or suspended viewer skips frames instead of queueing
// them in the daemon's memory.
pub struct Latest {
    pub frame: Frame,
    msg: String,
}

pub type LatestRx = watch::Receiver<Option<Arc<Latest>>>;

fn shutdown(path: &str) -> ! {
    let _ = std::fs::remove_file(path);
    flush_log();
    std::process::exit(0)
}

// Headless collector: polls on the interval, broadcasts each Frame to connected
// viewers over a unix socket, writes the log file, and sends Telegram alerts.
pub async fn run_daemon() {
    seed_history();
    let path = socket_path();
    if Path::new(&path).exists() {
        // Stale socket from a previous run, or another daemon. Try to remove it;
        // if a live daemon is already bound, bind() below will fail loudly.
        let _ = std::fs::remove_file(&path);
    }

    let (latest_tx, latest_rx) = watch::channel::<Option<Arc<Latest>>>(None);

    // Telegram bot: answers /status, /events on demand via long polling.
    if let Some(t) = settings().config.telegram.as_ref().filter(|t| !t.token.is_empty() && !t.chat_id.is_empty()) {
        start_telegram_bot(t, latest_rx.clone());
    }

    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Socket error on {path}: {e}");
            std::process::exit(1);
        }
    };
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666));
    eprintln!("monadar daemon listening on {path}");

    init_log_timer();

    for kind in [SignalKind::interrupt(), SignalKind::terminate()] {
        let mut sig = signal(kind).expect("signal handler");
        let path = path.clone();
        tokio::spawn(async move {
            sig.recv().await;
            shutdown(&path);
        });
    }

    tokio::spawn(async move {
        loop {
            let Ok((sock, _)) = listener.accept().await else { continue };
            tokio::spawn(serve_viewer(sock, latest_rx.clone()));
        }
    });

    let mut ticks = tokio::time::interval(settings().interval);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut collector = Collector::new().await;
    loop {
        ticks.tick().await;
        let frame = collector.tick().await;
        if !frame.log_line.is_empty() {
            log_line(&frame.log_line);
        }
        let msg = encode_msg(&ServerMsg::Snapshot { frame: Box::new(frame.clone()) });
        latest_tx.send_replace(Some(Arc::new(Latest { frame, msg })));
    }
}

async fn serve_viewer(mut sock: tokio::net::UnixStream, mut latest: LatestRx) {
    // Greet with recent log backlog (for the Ctrl+O view) + the latest frame.
    let frame = latest.borrow_and_update().as_ref().map(|l| Box::new(l.frame.clone()));
    let hello = encode_msg(&ServerMsg::Hello { backlog: get_recent(BACKLOG_LINES), frame });
    if sock.write_all(hello.as_bytes()).await.is_err() {
        return;
    }
    while latest.changed().await.is_ok() {
        let Some(msg) = latest.borrow_and_update().as_ref().map(|l| l.msg.clone()) else { continue };
        if sock.write_all(msg.as_bytes()).await.is_err() {
            return;
        }
    }
}
