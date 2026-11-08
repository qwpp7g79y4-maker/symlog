//! `--output PATH` schrijft de plain-uitvoer naar een bestand. Een bestaand bestand
//! wordt vervangen. Een ontbrekende bovenliggende map is exit 4, dezelfde code als
//! een andere mislukte schrijfactie, en die map wordt niet aangemaakt.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

fn run(args: &[&str]) -> (i32, Vec<u8>, Vec<u8>) {
    let output = Command::new(env!("CARGO_BIN_EXE_symlog"))
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("symlog draait");
    (
        output.status.code().expect("exitcode"),
        output.stdout,
        output.stderr,
    )
}

fn scratch() -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("klok")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "symlog-output-{}-{}-{}",
        std::process::id(),
        nanos,
        N.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&dir).expect("scratchmap");
    dir
}

const LOG: &str = "tests/fixtures/line-basic.log";

#[test]
fn output_flag_writes_a_new_file() {
    let help = run(&["--help"]);
    assert_eq!(help.0, 0);
    let help_text = String::from_utf8(help.1).expect("utf8");
    assert!(help_text.contains("--output"), "{help_text}");

    let root = scratch();
    let report = root.join("report.txt");
    assert!(!report.exists());
    let report_path = report.to_str().expect("utf8-pad");
    let (code, stdout, stderr) = run(&["--plain", "--no-color", "--output", report_path, LOG]);
    let err = String::from_utf8_lossy(&stderr);
    assert_eq!(code, 0, "{err}");
    assert!(stdout.is_empty(), "stdout moet leeg blijven");
    assert!(stderr.is_empty(), "{err}");
    let (plain_code, expected, plain_err) = run(&["--plain", "--no-color", LOG]);
    let plain_err = String::from_utf8_lossy(&plain_err);
    assert_eq!(plain_code, 0, "{plain_err}");
    assert!(!expected.is_empty());
    assert_eq!(fs::read(&report).expect("rapport lezen"), expected);
    fs::remove_dir_all(&root).ok();
}

#[test]
fn output_flag_overwrites_an_existing_file() {
    let root = scratch();
    let report = root.join("report.txt");
    fs::write(&report, b"stale report that must be replaced\n").unwrap();
    let report_path = report.to_str().expect("utf8-pad");
    let (code, stdout, stderr) = run(&["--plain", "--no-color", "--output", report_path, LOG]);
    let err = String::from_utf8_lossy(&stderr);
    assert_eq!(code, 0, "{err}");
    assert!(stdout.is_empty(), "{stdout:?}");
    assert!(stderr.is_empty(), "{err}");
    let (plain_code, expected, plain_err) = run(&["--plain", "--no-color", LOG]);
    let plain_err = String::from_utf8_lossy(&plain_err);
    assert_eq!(plain_code, 0, "{plain_err}");
    let body = fs::read(&report).expect("rapport lezen");
    assert_eq!(body, expected);
    assert!(!String::from_utf8_lossy(&body).contains("stale"));
    fs::remove_dir_all(&root).ok();
}

#[test]
fn output_flag_missing_directory_exits_four() {
    let root = scratch();
    let missing_dir = root.join("no-such-dir");
    let report = missing_dir.join("report.txt");
    let report_path = report.to_str().expect("utf8-pad");
    let (code, stdout, stderr) = run(&["--plain", "--no-color", "--output", report_path, LOG]);
    let err = String::from_utf8_lossy(&stderr);
    assert_eq!(code, 4, "{err}");
    assert!(stdout.is_empty(), "{stdout:?}");
    assert!(err.contains("schrijven mislukt"), "{err}");
    assert!(err.contains("maakt die map niet aan"), "{err}");
    assert!(!err.contains("panicked"), "{err}");
    assert!(!report.exists());
    assert!(
        !missing_dir.exists(),
        "een ontbrekende bovenliggende map mag niet aangemaakt worden"
    );
    fs::remove_dir_all(&root).ok();
}
