use std::env;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs");

    let pkg_version = env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string());
    let git_desc = git_describe().unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=SSH_AGENT_MUX_GIT_DESCRIBE={}", git_desc);
    println!(
        "cargo:rustc-env=SSH_AGENT_MUX_BUILD_VERSION={}",
        format!("{} ({})", pkg_version, git_desc)
    );
}

fn git_describe() -> Result<String, std::io::Error> {
    let output = Command::new("git")
        .args(["describe", "--always", "--dirty", "--tags"])
        .output()?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "git describe failed",
        ))
    }
}
