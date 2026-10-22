use crate::types::ServerMsg;

// One JSON message per line; the viewer reads the socket line by line.
pub fn encode_msg(msg: &ServerMsg) -> String {
    let mut s = serde_json::to_string(msg).unwrap_or_default();
    s.push('\n');
    s
}

pub fn decode_msg(line: &str) -> Option<ServerMsg> {
    if line.trim().is_empty() {
        return None;
    }
    serde_json::from_str(line).ok()
}
