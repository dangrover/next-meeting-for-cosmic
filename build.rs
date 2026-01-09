use std::process::Command;

fn main() {
    // Get the short git commit hash
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok();

    let git_hash = output
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map_or_else(|| "unknown".to_string(), |s| s.trim().to_string());

    println!("cargo:rustc-env=GIT_HASH={git_hash}");

    // Rerun if git HEAD changes
    println!("cargo:rerun-if-changed=.git/HEAD");
}
