use std::path::Path;

use crate::config::settings;

const DEFAULT_SOCKET: &str = "/run/monadar.sock";

pub fn resolve_socket_path(override_path: Option<&str>, xdg_runtime_dir: Option<&str>, exists: impl Fn(&str) -> bool) -> String {
    if let Some(p) = override_path.filter(|p| !p.is_empty()) {
        return p.to_string();
    }
    if exists("/run") {
        return DEFAULT_SOCKET.to_string();
    }
    let base = xdg_runtime_dir.filter(|d| !d.is_empty()).unwrap_or("/tmp");
    format!("{base}/monadar.sock")
}

pub fn socket_path() -> String {
    let xdg = std::env::var("XDG_RUNTIME_DIR").ok();
    resolve_socket_path(settings().socket.as_deref(), xdg.as_deref(), |p| Path::new(p).exists())
}
