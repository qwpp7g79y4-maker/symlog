//! `--version` en `-V` tonen de crate-versie en stoppen met exit 0.
//! Een debug-build toont de git-sha alleen als `git rev-parse` die zelf geeft.
//! Een release-build toont die sha niet.

use assert_cmd::Command;
use std::path::Path;
use std::process::Command as Process;

fn is_git_object_name(text: &str) -> bool {
    matches!(text.len(), 40 | 64)
        && text
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

/// Zelfde bron als `build.rs`: `git rev-parse --verify HEAD` in deze checkout.
fn git_head() -> Option<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    if !root.join(".git").exists() {
        return None;
    }
    let output = Process::new("git")
        .current_dir(root)
        .args(["rev-parse", "--verify", "HEAD"])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let sha = text.trim();
    if is_git_object_name(sha) {
        Some(sha.to_string())
    } else {
        None
    }
}

#[test]
fn version_flags_print_crate_version_and_exit_zero() {
    assert_eq!(env!("CARGO_PKG_VERSION"), "1.0.0");
    let from_git = git_head();
    if cfg!(debug_assertions) {
        assert_eq!(option_env!("SYMLOG_GIT_SHA"), from_git.as_deref());
    } else {
        assert!(option_env!("SYMLOG_GIT_SHA").is_none());
    }
    let expected = if cfg!(debug_assertions) {
        match from_git.as_deref() {
            Some(sha) => format!("symlog 1.0.0 {sha}\n"),
            None => "symlog 1.0.0\n".to_string(),
        }
    } else {
        "symlog 1.0.0\n".to_string()
    };
    for flag in ["--version", "-V"] {
        let output = Command::cargo_bin("symlog")
            .expect("binary vinden")
            .arg(flag)
            .output()
            .expect("symlog draait");
        assert_eq!(output.status.code(), Some(0), "{flag}");
        assert!(output.stderr.is_empty(), "{flag}");
        let stdout = String::from_utf8(output.stdout).expect("utf8");
        assert_eq!(stdout, expected, "{flag}");
    }
}
