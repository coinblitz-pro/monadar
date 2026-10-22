use std::time::Duration;

use serde_json::{Value, json};

use crate::config::TelegramConfig;
use crate::daemon::LatestRx;
use crate::http;
use crate::notify::{Notice, NoticeKind, Pending};
use crate::rich::{Block, Msg, b, p};
use crate::telegram::{events_message, help_message, no_data_message, notice_message, send, snapshot_rows, status_message};
use crate::types::{Frame, Snapshot};

const BACKOFF: Duration = Duration::from_secs(2);

async fn tg_call(base: &str, method: &str, body: Value, timeout: Duration) -> Result<Value, String> {
    let resp = http::post_json(&format!("{base}/{method}"), &body, timeout).await?;
    resp.json().ok_or_else(|| format!("HTTP {}: not JSON", resp.status))
}

fn chat_id_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn reply_for(text: &str, frame: Option<&Frame>) -> Option<Msg> {
    // Strip a trailing @botname and any arguments; match the bare command.
    let cmd = text.split_whitespace().next().unwrap_or("").split('@').next().unwrap_or("").to_lowercase();
    match cmd.as_str() {
        "/status" => Some(frame.map(status_message).unwrap_or_else(no_data_message)),
        "/events" => Some(frame.map(events_message).unwrap_or_else(no_data_message)),
        "/help" | "/start" => Some(help_message()),
        _ => None,
    }
}

// Telegram bot via long polling (getUpdates) — no public endpoint needed, so it
// works behind a node's firewall. Only replies to the configured chat_id. Runs
// as an independent task alongside the daemon's tick loop.
pub fn start_telegram_bot(t: &TelegramConfig, latest: LatestRx) {
    let base = format!("{}/bot{}", t.api_url, t.token);
    let chat_id = t.chat_id.clone();
    tokio::spawn(async move {
        let mut offset: i64 = 0;
        // Drain any updates queued before startup so we don't reply to stale commands.
        if let Ok(init) = tg_call(&base, "getUpdates", json!({ "timeout": 0 }), Duration::from_secs(10)).await
            && let Some(last) = init["result"].as_array().and_then(|r| r.last())
        {
            offset = last["update_id"].as_i64().unwrap_or(-1) + 1;
        }

        loop {
            let body = json!({ "offset": offset, "timeout": 50, "allowed_updates": ["message"] });
            let res = match tg_call(&base, "getUpdates", body, Duration::from_secs(55)).await {
                Ok(r) => r,
                Err(_) => {
                    // Network error or long-poll timeout — brief backoff, then retry.
                    tokio::time::sleep(BACKOFF).await;
                    continue;
                }
            };
            let Some(updates) = res["result"].as_array().filter(|_| res["ok"] == true) else {
                tokio::time::sleep(BACKOFF).await;
                continue;
            };
            for u in updates {
                offset = u["update_id"].as_i64().unwrap_or(offset - 1) + 1;
                let msg = &u["message"];
                let Some(text) = msg["text"].as_str() else { continue };
                if chat_id_str(&msg["chat"]["id"]) != chat_id {
                    continue;
                }
                let frame = latest.borrow().as_ref().map(|l| l.frame.clone());
                if let Some(reply) = reply_for(text, frame.as_ref()) {
                    let (base, chat_id) = (base.clone(), chat_id.clone());
                    tokio::spawn(async move {
                        if let Err(e) = send(&base, &chat_id, &reply, Duration::from_secs(10)).await {
                            eprintln!("telegram: reply failed: {e}");
                        }
                    });
                }
            }
        }
    });
}

// `monadar telegram-test`: shows every message kind in the configured chat,
// built from the running daemon's latest frame when there is one.
pub async fn send_samples() -> Result<(), String> {
    let t = crate::config::settings()
        .config
        .telegram
        .as_ref()
        .filter(|t| !t.token.is_empty() && !t.chat_id.is_empty())
        .ok_or("telegram.token and telegram.chat_id are not set in the config")?;
    let base = format!("{}/bot{}", t.api_url, t.token);
    let frame = latest_frame().await;
    let now = chrono::Utc::now().timestamp_millis();
    let snap = frame.as_ref().map(snapshot_of).unwrap_or_default();
    let notice = |key: &str, kind: NoticeKind, title: &str, detail: &str, at: i64, since: Option<i64>| Notice {
        key: key.into(),
        kind,
        at,
        title: title.into(),
        detail: detail.into(),
        snap: snapshot_rows(&snap),
        since,
        suppressed: 0,
    };
    let pending = |n: Notice, resolved_at: Option<i64>| Pending { notice: n, resolved_at };

    let mut msgs: Vec<(&str, Msg)> = vec![(
        "intro",
        Msg(vec![Block::Para(vec![b("monadar telegram-test"), p(": sample messages follow, these are not real alerts.")])]),
    )];
    let not_voting = "No votes for 7s while 18 blocks advanced: consensus participation lost";
    msgs.push((
        "alert",
        notice_message(&pending(notice("NOT_VOTING", NoticeKind::Fired, "NOT VOTING", not_voting, now - 4_000, None), None), now),
    ));
    let stuck = "Block number has not advanced for 5s: node may be stuck or disconnected";
    msgs.push((
        "alert resolved while queued",
        notice_message(&pending(notice("STUCK", NoticeKind::Fired, "STUCK", stuck, now - 140_000, None), Some(now - 6_000)), now),
    ));
    msgs.push((
        "resolved",
        notice_message(&pending(notice("RPC_DOWN", NoticeKind::Resolved, "RPC DOWN", "", now, Some(now - 95_000)), None), now),
    ));
    let mut late = notice(
        "MISSED_ROUND",
        NoticeKind::Fired,
        "MISSED ROUND ×1",
        "Our node was leader and its block did not make it in 1 round(s) during the last 10 min (round ended by timeout certificate)",
        now - 420_000,
        None,
    );
    late.suppressed = 2;
    msgs.push(("late alert", notice_message(&pending(late, None), now)));
    msgs.push((
        "restart",
        notice_message(&pending(notice("RESTART", NoticeKind::Info, "NODE RESTART", "uptime 0h0m", now, None), None), now),
    ));
    match &frame {
        Some(f) => {
            msgs.push(("/status", status_message(f)));
            msgs.push(("/events", events_message(f)));
        }
        None => println!("no running daemon: /status and /events samples skipped"),
    }
    msgs.push(("/help", help_message()));

    for (name, msg) in msgs {
        match send(&base, &t.chat_id, &msg, Duration::from_secs(10)).await {
            Ok(r) if r.ok() => println!("{name}: sent"),
            Ok(r) => println!("{name}: HTTP {} {}", r.status, String::from_utf8_lossy(&r.body)),
            Err(e) => println!("{name}: {e}"),
        }
    }
    Ok(())
}

// The daemon's latest frame, from its hello message.
async fn latest_frame() -> Option<Frame> {
    use tokio::io::AsyncBufReadExt;
    let stream = tokio::net::UnixStream::connect(crate::socket::socket_path()).await.ok()?;
    let mut lines = tokio::io::BufReader::new(stream).lines();
    let line = tokio::time::timeout(Duration::from_secs(3), lines.next_line()).await.ok()?.ok()??;
    match crate::protocol::decode_msg(&line)? {
        crate::types::ServerMsg::Hello { frame, .. } => frame.map(|f| *f),
        crate::types::ServerMsg::Snapshot { frame } => Some(*frame),
    }
}

fn snapshot_of(f: &Frame) -> Snapshot {
    Snapshot {
        block: Some(f.block),
        block_delta: f.block_delta,
        bps: f.bps.clone(),
        lag: Some(f.exec_lag),
        exec_p99: f.exec.map(|e| e.p99),
        led24h: f.rounds.as_ref().map(|r| r.h24.led),
        missed24h: f.rounds.as_ref().map(|r| r.h24.missed + r.h24.failed),
        peers: f.peers,
        p99: f.p99,
        rpc_ms: Some(f.rpc_ms),
        syncing: f.syncing,
        sync_pct: f.sync_pct.clone(),
        uptime: Some(f.uptime.clone()),
        ver: Some(f.ver.clone()),
    }
}
