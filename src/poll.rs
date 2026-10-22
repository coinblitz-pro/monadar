use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::http;

const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

fn block_number(json: &Value) -> Option<&str> {
    let r = json.get("result")?.as_str()?;
    let hex = r.strip_prefix("0x")?;
    (!hex.is_empty() && hex.bytes().all(|b| b.is_ascii_hexdigit())).then_some(hex)
}

pub async fn fetch_block_number(rpc_url: &str, timeout: Duration) -> Result<(i64, i64), String> {
    let t0 = Instant::now();
    let body = json!({ "jsonrpc": "2.0", "method": "eth_blockNumber", "params": [], "id": 1 });
    let resp = http::post_json(rpc_url, &body, timeout).await?;
    if !resp.ok() {
        return Err(format!("RPC HTTP {}", resp.status));
    }
    let json = resp.json().ok_or("RPC returned invalid JSON")?;
    let hex = block_number(&json).ok_or("RPC returned an invalid eth_blockNumber response")?;
    let block = u64::from_str_radix(hex, 16)
        .ok()
        .filter(|b| *b <= MAX_SAFE_INTEGER)
        .ok_or_else(|| format!("RPC block number is out of range: 0x{hex}"))?;
    Ok((block as i64, t0.elapsed().as_millis() as i64))
}

pub async fn fetch_metrics_text(metrics_url: &str, timeout: Duration) -> Result<String, String> {
    let resp = http::get(metrics_url, timeout).await?;
    if !resp.ok() {
        return Err(format!("Metrics HTTP {}", resp.status));
    }
    Ok(String::from_utf8_lossy(&resp.body).into_owned())
}
