use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

const RESTART_DELAY: Duration = Duration::from_secs(5);

// Follows `journalctl -f` for a unit and calls on_line for every new message
// (without the trailing newline). Restarts journalctl if it exits. With
// `backfill` (a journalctl --since value) the first run starts with the
// history since then; restarts continue from new lines only.
// Needs root or the systemd-journal group. journalctl's own complaints (no
// permission, unit unknown) and spawn failures go to our stderr, each distinct
// message once, so a silent tail is never a mystery.
pub fn follow_journal(unit: &'static str, backfill: Option<&'static str>, mut on_line: impl FnMut(&str) + Send + 'static) {
    let reported = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()));
    let report = move |msg: String| {
        if reported.lock().unwrap().insert(msg.clone()) {
            eprintln!("journal {unit}: {msg}");
        }
    };
    tokio::spawn(async move {
        let mut since = backfill;
        loop {
            let mut cmd = Command::new("journalctl");
            match since.take() {
                Some(s) => cmd.args(["-f", "--since", s]),
                None => cmd.args(["-f", "-n", "0"]),
            };
            cmd.args(["-o", "cat", "-u", unit]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
            // journalctl must not outlive us when the daemon exits without unwinding.
            #[cfg(target_os = "linux")]
            unsafe {
                cmd.pre_exec(|| {
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                    Ok(())
                });
            }
            match cmd.spawn() {
                Err(e) => report(format!("cannot run journalctl: {e}")),
                Ok(mut child) => {
                    if let Some(stderr) = child.stderr.take() {
                        let report = report.clone();
                        tokio::spawn(async move {
                            let mut lines = BufReader::new(stderr).lines();
                            while let Ok(Some(l)) = lines.next_line().await {
                                if !l.trim().is_empty() {
                                    report(l);
                                }
                            }
                        });
                    }
                    if let Some(stdout) = child.stdout.take() {
                        let mut reader = BufReader::new(stdout);
                        let mut buf = Vec::new();
                        loop {
                            buf.clear();
                            match reader.read_until(b'\n', &mut buf).await {
                                Ok(0) | Err(_) => break,
                                Ok(_) => {
                                    if buf.last() == Some(&b'\n') {
                                        buf.pop();
                                    } else {
                                        break; // partial last line: journalctl exited mid-write
                                    }
                                    on_line(&String::from_utf8_lossy(&buf));
                                }
                            }
                        }
                    }
                    if let Ok(st) = child.wait().await
                        && !st.success()
                    {
                        report(format!("journalctl exited with {st}"));
                    }
                }
            }
            tokio::time::sleep(RESTART_DELAY).await;
        }
    });
}
