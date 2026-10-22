use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::config::TelegramConfig;
use crate::http;
use crate::telegram::{notice_message, send};

// Telegram delivery. The alert processor decides *what* to say; this outbox
// makes sure it is said:
// - a notice leaves the queue only when Telegram answered ok:true; network
//   errors and 5xx retry with backoff, 429 waits retry_after, other 4xx (bad
//   token, unknown chat) are dropped with a log line instead of looping;
// - every message carries the time of the event, and says so when it arrives
//   late (the host lost its uplink while the alert was queued);
// - an alert still queued when its resolve comes in is sent as one message:
//   "X at 12:03:10, resolved after 2m14s", not as a burst of two;
// - the queue is saved to disk, so a daemon restart mid-outage loses nothing.

const MAX_QUEUE: usize = 200;
const MAX_BACKOFF: Duration = Duration::from_secs(60);
const SEND_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NoticeKind {
    Fired,
    Resolved,
    Info,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Notice {
    pub key: String,
    pub kind: NoticeKind,
    // Event time, epoch ms: when the condition started (Fired), ended (Resolved), happened (Info).
    pub at: i64,
    pub title: String,
    pub detail: String,
    // Node state at the event: label/value rows.
    pub snap: Vec<(String, String)>,
    // Resolved: when the condition started.
    pub since: Option<i64>,
    // Repeats of this key skipped by the cooldown since the previous message.
    pub suppressed: u32,
}

pub trait Notifier: Send + Sync {
    fn push(&self, notice: Notice);
}

pub struct NullNotifier;

impl Notifier for NullNotifier {
    fn push(&self, _: Notice) {}
}

struct OutboxHandle(mpsc::UnboundedSender<Notice>);

impl Notifier for OutboxHandle {
    fn push(&self, notice: Notice) {
        let _ = self.0.send(notice);
    }
}

// A queued message: a notice, possibly with its resolve folded in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pending {
    pub notice: Notice,
    pub resolved_at: Option<i64>,
}

// Fold Resolved notices into their still-queued Fired notice.
pub fn coalesce(queue: &mut VecDeque<Pending>) {
    let mut i = 0;
    while i < queue.len() {
        let p = &queue[i];
        let fold = p.notice.kind == NoticeKind::Resolved && p.resolved_at.is_none();
        let target = fold
            .then(|| {
                queue
                    .iter()
                    .take(i)
                    .rposition(|q| q.notice.kind == NoticeKind::Fired && q.notice.key == p.notice.key && q.resolved_at.is_none())
            })
            .flatten();
        match target {
            Some(t) => {
                let r = queue.remove(i).unwrap();
                queue[t].resolved_at = Some(r.notice.at);
            }
            None => i += 1,
        }
    }
}

pub enum Outcome {
    Delivered,
    Retry(Option<Duration>),
    Drop(String),
}

pub fn classify(resp: Result<http::Response, String>) -> Outcome {
    let resp = match resp {
        Ok(r) => r,
        Err(e) => return Outcome::Retry(None).log(&e),
    };
    let json = resp.json().unwrap_or(Value::Null);
    if resp.ok() && json["ok"] == true {
        return Outcome::Delivered;
    }
    let desc = json["description"].as_str().unwrap_or("").to_string();
    match resp.status {
        429 => Outcome::Retry(json["parameters"]["retry_after"].as_u64().map(Duration::from_secs)),
        400..=499 => Outcome::Drop(format!("HTTP {} {desc}", resp.status)),
        s => Outcome::Retry(None).log(&format!("HTTP {s} {desc}")),
    }
}

impl Outcome {
    fn log(self, why: &str) -> Outcome {
        eprintln!("telegram: send failed, will retry: {why}");
        self
    }
}

fn load(path: &Option<PathBuf>) -> VecDeque<Pending> {
    path.as_ref().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save(path: &Option<PathBuf>, queue: &VecDeque<Pending>) {
    let Some(p) = path else { return };
    if queue.is_empty() {
        let _ = std::fs::remove_file(p);
        return;
    }
    let tmp = p.with_extension("tmp");
    if std::fs::write(&tmp, serde_json::to_vec(queue).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(&tmp, p);
    }
}

fn enqueue(queue: &mut VecDeque<Pending>, notice: Notice) {
    queue.push_back(Pending { notice, resolved_at: None });
    coalesce(queue);
    while queue.len() > MAX_QUEUE {
        queue.pop_front();
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn start_telegram_outbox(t: &TelegramConfig, state_file: Option<PathBuf>) -> Box<dyn Notifier> {
    let base = format!("{}/bot{}", t.api_url, t.token);
    let chat_id = t.chat_id.clone();
    let (tx, mut rx) = mpsc::unbounded_channel::<Notice>();
    tokio::spawn(async move {
        let mut queue = load(&state_file);
        if !queue.is_empty() {
            eprintln!("telegram: {} undelivered message(s) from the previous run", queue.len());
        }
        let mut backoff = Duration::from_secs(1);
        loop {
            if queue.is_empty() {
                let Some(n) = rx.recv().await else { return };
                enqueue(&mut queue, n);
            }
            while let Ok(n) = rx.try_recv() {
                enqueue(&mut queue, n);
            }
            save(&state_file, &queue);

            let msg = notice_message(&queue[0], now_ms());
            let wait = match classify(send(&base, &chat_id, &msg, SEND_TIMEOUT).await) {
                Outcome::Delivered => {
                    queue.pop_front();
                    backoff = Duration::from_secs(1);
                    save(&state_file, &queue);
                    continue;
                }
                Outcome::Drop(why) => {
                    eprintln!("telegram: message dropped, Telegram refused it: {why}");
                    queue.pop_front();
                    save(&state_file, &queue);
                    continue;
                }
                Outcome::Retry(after) => {
                    let wait = after.unwrap_or(backoff);
                    backoff = (backoff * 2).min(MAX_BACKOFF);
                    wait
                }
            };
            // Keep taking new notices while waiting: a resolve that arrives now
            // folds into the alert still in the queue.
            let sleep = tokio::time::sleep(wait);
            tokio::pin!(sleep);
            loop {
                tokio::select! {
                    _ = &mut sleep => break,
                    n = rx.recv() => match n {
                        Some(n) => enqueue(&mut queue, n),
                        None => return,
                    },
                }
            }
        }
    });
    Box::new(OutboxHandle(tx))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice(key: &str, kind: NoticeKind, at: i64) -> Notice {
        Notice {
            key: key.into(),
            kind,
            at,
            title: key.replace('_', " "),
            detail: "detail <x>".into(),
            snap: vec![("Block".into(), "1".into())],
            since: (kind == NoticeKind::Resolved).then_some(at - 134_000),
            suppressed: 0,
        }
    }

    fn queue(ns: Vec<Notice>) -> VecDeque<Pending> {
        let mut q = VecDeque::new();
        for n in ns {
            enqueue(&mut q, n);
        }
        q
    }

    #[test]
    fn a_resolve_folds_into_its_queued_alert() {
        let q = queue(vec![
            notice("STUCK", NoticeKind::Fired, 1_000),
            notice("RPC_DOWN", NoticeKind::Fired, 2_000),
            notice("STUCK", NoticeKind::Resolved, 135_000),
        ]);
        assert_eq!(q.len(), 2);
        assert_eq!(q[0].notice.key, "STUCK");
        assert_eq!(q[0].resolved_at, Some(135_000));
        assert_eq!(q[1].resolved_at, None);
    }

    #[test]
    fn a_resolve_without_a_queued_alert_stays() {
        let q = queue(vec![notice("STUCK", NoticeKind::Resolved, 5_000), notice("STUCK", NoticeKind::Fired, 9_000)]);
        assert_eq!(q.iter().map(|p| p.notice.kind).collect::<Vec<_>>(), vec![NoticeKind::Resolved, NoticeKind::Fired]);
    }

    #[test]
    fn classifies_telegram_answers() {
        let resp = |status: u16, body: &str| Ok(http::Response { status, body: bytes::Bytes::from(body.to_string()) });
        assert!(matches!(classify(resp(200, r#"{"ok":true,"result":{}}"#)), Outcome::Delivered));
        assert!(matches!(
            classify(resp(429, r#"{"ok":false,"parameters":{"retry_after":7}}"#)),
            Outcome::Retry(Some(d)) if d == Duration::from_secs(7)
        ));
        assert!(matches!(classify(resp(400, r#"{"ok":false,"description":"chat not found"}"#)), Outcome::Drop(_)));
        assert!(matches!(classify(resp(502, "bad gateway")), Outcome::Retry(None)));
        assert!(matches!(classify(Err("timeout".into())), Outcome::Retry(None)));
    }

    #[test]
    fn the_queue_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("monadar-outbox-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = Some(dir.join("outbox.json"));
        let q = queue(vec![notice("STUCK", NoticeKind::Fired, 1)]);
        save(&path, &q);
        assert_eq!(load(&path), q);
        save(&path, &VecDeque::new());
        assert!(!dir.join("outbox.json").exists());
        let _ = std::fs::remove_dir(&dir);
    }
}
