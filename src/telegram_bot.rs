use std::time::Duration;

use serde_json::{Value, json};

use crate::config::TelegramConfig;
use crate::daemon::LatestRx;
use crate::http;
use crate::telegram::{frame_events_message, frame_status_message};
use crate::types::Frame;

const HELP: &str = "monadar bot commands:\n/status — current node status\n/events — recent events\n/help — this message";
const NO_DATA: &str = "No data yet — daemon is still starting.";
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

fn reply_for(text: &str, frame: Option<&Frame>) -> Option<String> {
    // Strip a trailing @botname and any arguments; match the bare command.
    let cmd = text.split_whitespace().next().unwrap_or("").split('@').next().unwrap_or("").to_lowercase();
    match cmd.as_str() {
        "/status" => Some(frame.map(frame_status_message).unwrap_or(NO_DATA.into())),
        "/events" => Some(frame.map(frame_events_message).unwrap_or(NO_DATA.into())),
        "/help" | "/start" => Some(HELP.into()),
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
                    let base = base.clone();
                    let body = json!({ "chat_id": chat_id, "text": reply, "parse_mode": "HTML" });
                    tokio::spawn(async move {
                        let _ = tg_call(&base, "sendMessage", body, Duration::from_secs(5)).await;
                    });
                }
            }
        }
    });
}
