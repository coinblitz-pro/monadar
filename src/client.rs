use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::UnixStream;
use tokio::signal::unix::{SignalKind, signal};

use crate::colors::{cyan, dim, yellow};
use crate::config::settings;
use crate::log::log_line;
use crate::protocol::decode_msg;
use crate::render::{frame_body, paint};
use crate::socket::socket_path;
use crate::types::ServerMsg;
use crate::ui::{Key, Ui, quit, setup_input, write_out};

fn hint(path: &str) -> String {
    let start = if cfg!(target_os = "macos") {
        "sudo launchctl load /Library/LaunchDaemons/com.monadar.plist"
    } else {
        "sudo systemctl start monadar"
    };
    [
        format!("{} (no socket at {path})", yellow("daemon not running")),
        String::new(),
        "Start it with:".into(),
        format!("  {}", cyan(start)),
        String::new(),
        format!("Install the service first with {},", cyan("monadar install")),
        format!("or run standalone without a daemon: {}", cyan("monadar --foreground")),
    ]
    .join("\n")
        + "\n"
}

// Attach viewer: connects to the daemon socket and renders incoming frames.
// Overlays and keypresses are handled locally. Reconnects if the daemon
// restarts; exits with a hint if no daemon is running at startup.
pub async fn attach() {
    let path = socket_path();
    let bell = settings().args.bell;
    let mut ui = Ui::new(settings().args.verbose);
    let mut body = String::new();
    let mut keys = setup_input();
    let mut sigint = signal(SignalKind::interrupt()).expect("signal handler");
    let mut sigterm = signal(SignalKind::terminate()).expect("signal handler");
    let mut ever_connected = false;
    let mut backoff = Duration::from_millis(500);

    write_out("\x1b[2J\x1b[H\x1b[?25l");

    loop {
        match UnixStream::connect(&path).await {
            Ok(stream) => {
                ever_connected = true;
                backoff = Duration::from_millis(500);
                let mut lines = BufReader::new(stream).lines();
                loop {
                    tokio::select! {
                        line = lines.next_line() => {
                            let Ok(Some(line)) = line else { break };
                            match decode_msg(&line) {
                                Some(ServerMsg::Hello { backlog, frame }) => {
                                    for l in &backlog {
                                        log_line(l);
                                    }
                                    if let Some(f) = frame {
                                        body = frame_body(&f);
                                    }
                                    paint(&ui, &body);
                                }
                                Some(ServerMsg::Snapshot { frame }) => {
                                    body = frame_body(&frame);
                                    if !frame.log_line.is_empty() {
                                        log_line(&frame.log_line);
                                    }
                                    if bell && frame.new_crit {
                                        write_out("\x07");
                                    }
                                    paint(&ui, &body);
                                }
                                None => {}
                            }
                        }
                        Some(k) = keys.recv() => match ui.handle_key(k) {
                            Some(Key::Quit) => quit(0, "\x1b[?25h\n"),
                            Some(Key::Repaint) => paint(&ui, &body),
                            None => {}
                        },
                        _ = sigint.recv() => quit(0, "\x1b[?25h\n"),
                        _ = sigterm.recv() => quit(0, "\x1b[?25h\n"),
                    }
                }
            }
            Err(_) if !ever_connected => {
                // Never reached the daemon — it isn't running. Restore terminal + hint.
                quit(1, &format!("\x1b[?25h\x1b[2J\x1b[H{}", hint(&path)));
            }
            Err(_) => {}
        }

        write_out(&format!("\x1b[H{}\x1b[K\x1b[J", dim(&format!("reconnecting to {path}…"))));
        let wait = tokio::time::sleep(backoff);
        tokio::pin!(wait);
        loop {
            tokio::select! {
                _ = &mut wait => break,
                Some(k) = keys.recv() => if let Some(Key::Quit) = ui.handle_key(k) { quit(0, "\x1b[?25h\n") },
                _ = sigint.recv() => quit(0, "\x1b[?25h\n"),
                _ = sigterm.recv() => quit(0, "\x1b[?25h\n"),
            }
        }
        backoff = (backoff * 2).min(Duration::from_secs(5));
    }
}
