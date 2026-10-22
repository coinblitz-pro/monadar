use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::Command;

use crate::colors::{cyan, green, red, yellow};
use crate::config::settings;
use crate::socket::socket_path;

const SYSTEMD_PATH: &str = "/etc/systemd/system/monadar.service";
const LAUNCHD_LABEL: &str = "com.monadar";
const LAUNCHD_PATH: &str = "/Library/LaunchDaemons/com.monadar.plist";

const IS_MAC: bool = cfg!(target_os = "macos");

// Flags the user passed to `install` are forwarded to the daemon's ExecStart
// (e.g. --config, --socket, --cpu, --log, interval), so the service runs with
// the same configuration it was installed with.
fn daemon_argv() -> Vec<String> {
    let exe = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| "monadar".into());
    let mut argv = vec![exe, "daemon".to_string()];
    argv.extend(settings().args.service_passthrough.iter().cloned());
    argv
}

pub fn quote_systemd_arg(value: &str) -> String {
    let escaped = value.replace('%', "%%").replace('$', "$$");
    if !escaped.is_empty() && escaped.chars().all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c)) {
        return escaped;
    }
    format!("\"{}\"", escaped.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\t', "\\t"))
}

fn xml_escape(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

pub fn build_systemd_unit(argv: &[String]) -> String {
    let exec: Vec<String> = argv.iter().map(|a| quote_systemd_arg(a)).collect();
    [
        "[Unit]",
        "Description=monadar node monitor",
        "After=network.target",
        "",
        "[Service]",
        "Type=simple",
        &format!("ExecStart={}", exec.join(" ")),
        "Restart=always",
        "RestartSec=2",
        "",
        "[Install]",
        "WantedBy=multi-user.target",
        "",
    ]
    .join("\n")
}

pub fn build_launchd_plist(argv: &[String]) -> String {
    let args: Vec<String> = argv.iter().map(|a| format!("    <string>{}</string>", xml_escape(a))).collect();
    [
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>",
        "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">",
        "<plist version=\"1.0\">",
        "<dict>",
        &format!("  <key>Label</key><string>{LAUNCHD_LABEL}</string>"),
        "  <key>ProgramArguments</key>",
        "  <array>",
        &args.join("\n"),
        "  </array>",
        "  <key>RunAtLoad</key><true/>",
        "  <key>KeepAlive</key><true/>",
        "  <key>StandardErrorPath</key><string>/var/log/monadar.err.log</string>",
        "  <key>StandardOutPath</key><string>/var/log/monadar.out.log</string>",
        "</dict>",
        "</plist>",
        "",
    ]
    .join("\n")
}

fn daemon_reload() {
    let _ = Command::new("systemctl").arg("daemon-reload").status();
}

pub fn install(print: bool) {
    let argv = daemon_argv();
    let unit = if IS_MAC { build_launchd_plist(&argv) } else { build_systemd_unit(&argv) };
    let path = if IS_MAC { LAUNCHD_PATH } else { SYSTEMD_PATH };

    if print {
        print!("{unit}");
        return;
    }

    if argv[0].contains("/target/debug/") {
        eprintln!("{} installing a debug build. Install the release binary (cargo build --release) for production.", yellow("Warning:"));
    }

    if let Err(e) = std::fs::write(path, unit) {
        eprintln!("{}{path}: {e}", red("Failed to write "));
        eprintln!("Run with sudo (system-level service).");
        std::process::exit(1);
    }

    println!("Wrote {path}");
    println!("Enable & start:");
    if IS_MAC {
        println!("  {}", cyan(&format!("sudo launchctl load -w {path}")));
    } else {
        daemon_reload();
        println!("  {}", cyan("sudo systemctl enable --now monadar"));
    }
}

pub fn uninstall() {
    let path = if IS_MAC { LAUNCHD_PATH } else { SYSTEMD_PATH };
    println!("Stop & remove:");
    if IS_MAC {
        println!("  {}", cyan(&format!("sudo launchctl unload {path}")));
    } else {
        println!("  {}", cyan("sudo systemctl disable --now monadar"));
    }
    if Path::new(path).exists() {
        if let Err(e) = std::fs::remove_file(path) {
            eprintln!("{}{path}: {e}", red("Failed to remove "));
            std::process::exit(1);
        }
        println!("Removed {path}");
    } else {
        println!("Not installed: {path}");
    }
    if !IS_MAC {
        daemon_reload();
    }
}

// Probe the socket: connect briefly and report whether a daemon is bound.
pub fn status() {
    let path = socket_path();
    match UnixStream::connect(&path) {
        Ok(_) => {
            println!("{} — daemon listening on {path}", green("● running"));
            std::process::exit(0);
        }
        Err(_) => {
            println!("{} — no daemon at {path}", red("● stopped"));
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn quote_systemd_arg_quotes_whitespace_and_escapes_specifiers() {
        assert_eq!(quote_systemd_arg("/tmp/a b/monadar"), "\"/tmp/a b/monadar\"");
        assert_eq!(quote_systemd_arg("/tmp/%h/$HOME"), "\"/tmp/%%h/$$HOME\"");
    }

    #[test]
    fn systemd_unit_quotes_argv_in_exec_start() {
        let unit = build_systemd_unit(&argv(&["/usr/local/bin/monadar", "/tmp/a b/c.json", "daemon"]));
        assert!(unit.lines().any(|l| l == "ExecStart=/usr/local/bin/monadar \"/tmp/a b/c.json\" daemon"));
    }

    #[test]
    fn launchd_plist_xml_escapes_argv_values() {
        let plist = build_launchd_plist(&argv(&["/usr/local/bin/monadar", "/tmp/a & b/<monadar>.json"]));
        assert!(plist.contains("<string>/tmp/a &amp; b/&lt;monadar&gt;.json</string>"));
    }
}
