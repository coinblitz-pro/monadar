use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

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

// Metrics come with the node's OTLP export once a second, so the full tick
// stays at 1s. In between, while someone is watching, the block and the
// execution height follow the chain (~0.3s per block): local RPC is polled
// this often and viewers get a frame whenever either moves. RPC shows a new
// block ~0.2s before journald delivers the bft line about it.
const LIVE_EVERY: Duration = Duration::from_millis(100);
// A tick's own RPC read may land before the last live update; within this
// window the tick frame keeps the newer block so the number never goes back.
const LIVE_KEEP: Duration = Duration::from_secs(2);

// The latest frame and its encoded snapshot message. Viewers get it through a
// watch channel: a slow or suspended viewer skips frames instead of queueing
// them in the daemon's memory.
pub struct Latest {
    pub frame: Frame,
    msg: String,
}

pub type LatestRx = watch::Receiver<Option<Arc<Latest>>>;

fn publish(tx: &watch::Sender<Option<Arc<Latest>>>, frame: Frame) {
    let msg = encode_msg(&ServerMsg::Snapshot { frame: Box::new(frame.clone()) });
    tx.send_replace(Some(Arc::new(Latest { frame, msg })));
}

// Moves the frame to a newer block / execution height; false if neither is newer.
fn advance(f: &mut Frame, (block, exec): (i64, i64)) -> bool {
    let block = block.max(f.block);
    let exec = f.exec_block.map_or(exec, |e| e.max(exec));
    if block == f.block && Some(exec) == f.exec_block {
        return false;
    }
    f.block = block;
    f.exec_block = Some(exec);
    f.exec_lag = (block - exec).max(0);
    true
}

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

    let viewers = Arc::new(AtomicUsize::new(0));
    let v = viewers.clone();
    tokio::spawn(async move {
        loop {
            let Ok((sock, _)) = listener.accept().await else { continue };
            let v = v.clone();
            let rx = latest_rx.clone();
            tokio::spawn(async move {
                v.fetch_add(1, Ordering::Relaxed);
                serve_viewer(sock, rx).await;
                v.fetch_sub(1, Ordering::Relaxed);
            });
        }
    });

    let mut ticks = tokio::time::interval(settings().interval);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut live = tokio::time::interval(LIVE_EVERY);
    live.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut shown: Option<((i64, i64), Instant)> = None;
    let mut collector = Collector::new().await;
    loop {
        tokio::select! {
            _ = ticks.tick() => {
                let mut frame = collector.tick().await;
                if !frame.log_line.is_empty() {
                    log_line(&frame.log_line);
                }
                if let Some((b, at)) = shown
                    && frame.error.is_none()
                    && at.elapsed() < LIVE_KEEP
                {
                    advance(&mut frame, b);
                }
                publish(&latest_tx, frame);
            }
            _ = live.tick(), if viewers.load(Ordering::Relaxed) > 0 => {
                let Some(b) = collector.live_blocks().await else { continue };
                let Some(mut frame) = latest_tx.borrow().as_ref().filter(|l| l.frame.error.is_none()).map(|l| l.frame.clone()) else {
                    continue;
                };
                if !advance(&mut frame, b) {
                    continue;
                }
                // Already logged and alerted with the tick.
                frame.log_line.clear();
                frame.new_crit = false;
                shown = Some((b, Instant::now()));
                publish(&latest_tx, frame);
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advance_only_moves_forward() {
        let mut f = crate::frame::error_frame("12:00:00".into(), Vec::new(), crate::types::FrameError::Rpc, None);
        f.error = None;
        f.block = 100;
        f.exec_block = Some(98);
        assert!(!advance(&mut f, (100, 98)));
        assert!(!advance(&mut f, (99, 97)), "an older RPC read changes nothing");
        assert!(advance(&mut f, (101, 98)));
        assert_eq!((f.block, f.exec_block, f.exec_lag), (101, Some(98), 3));
        assert!(advance(&mut f, (101, 100)));
        assert_eq!(f.exec_lag, 1);
    }
}
