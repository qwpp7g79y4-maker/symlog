//! Zet `SYMLOG_GIT_SHA` voor een debug-build, alleen als `git rev-parse` zelf
//! een objectnaam print. Een release-build krijgt de variabele niet. Geen
//! verzonnen sha, en geen git-aanroep als deze map geen checkout is.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn main() {
    let Some(manifest) = std::env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from) else {
        return;
    };
    watch_git_head(&manifest);
    if !debug_profile() {
        return;
    }
    if let Some(sha) = git_head_sha(&manifest) {
        println!("cargo:rustc-env=SYMLOG_GIT_SHA={sha}");
    }
}

/// `cargo test` en `cargo build` gebruiken het debug-profiel. Release en bench niet.
fn debug_profile() -> bool {
    std::env::var("PROFILE").ok().as_deref() == Some("debug")
}

fn git_head_sha(manifest: &Path) -> Option<String> {
    if !manifest.join(".git").exists() {
        return None;
    }
    let output = git(manifest, &["rev-parse", "--verify", "HEAD"])?;
    let text = std::str::from_utf8(&output.stdout).ok()?.trim();
    if is_git_object_name(text) {
        Some(text.to_string())
    } else {
        None
    }
}

/// 40 (SHA-1) of 64 (SHA-256) kleine hex-tekens, de vorm die `git rev-parse` print.
fn is_git_object_name(text: &str) -> bool {
    matches!(text.len(), 40 | 64)
        && text
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn watch_git_head(manifest: &Path) {
    if !manifest.join(".git").exists() {
        return;
    }
    for spec in ["HEAD", "packed-refs"] {
        watch_git_path(manifest, spec);
    }
    let Some(output) = git(manifest, &["symbolic-ref", "-q", "HEAD"]) else {
        return;
    };
    let Ok(text) = std::str::from_utf8(&output.stdout) else {
        return;
    };
    let spec = text.trim();
    if !spec.is_empty() {
        watch_git_path(manifest, spec);
    }
}

fn watch_git_path(manifest: &Path, spec: &str) {
    let Some(output) = git(manifest, &["rev-parse", "--git-path", spec]) else {
        return;
    };
    let Ok(text) = std::str::from_utf8(&output.stdout) else {
        return;
    };
    let rel = text.trim();
    if rel.is_empty() {
        return;
    }
    let path = manifest.join(rel);
    if path.exists() {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

fn git(manifest: &Path, args: &[&str]) -> Option<Output> {
    let output = Command::new("git")
        .args(args)
        .current_dir(manifest)
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if output.status.success() {
        Some(output)
    } else {
        None
    }
}
