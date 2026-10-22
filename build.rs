use std::process::Command;

// Bakes the git short hash into the binary, so the viewer can tell when it
// talks to a daemon built from a different revision.
// MONADAR_GIT_HASH overrides it for builds without git (the musl container).
fn main() {
    let hash = std::env::var("MONADAR_GIT_HASH").ok().filter(|s| !s.is_empty()).unwrap_or_else(git_hash);
    println!("cargo:rustc-env=MONADAR_BUILD_HASH={hash}");
    println!("cargo:rerun-if-env-changed=MONADAR_GIT_HASH");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/heads");
}

fn git_hash() -> String {
    Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "dev".to_string())
}
