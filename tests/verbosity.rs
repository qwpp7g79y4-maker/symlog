//! `--quiet` en `--verbose`. Een pipe print geen hint en geen contextregel.
//! `--verbose` zet die context erbij. Exitcodes blijven 0, 1, 2, 3 en 4.
//! Een ontbrekende bovenliggende map voor `--output` blijft exit 4.

use assert_cmd::Command;
use std::fs;
use std::path::PathBuf;
use std::process::Command as StdCommand;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

fn symlog() -> Command {
    Command::cargo_bin("symlog").expect("binary")
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn scratch() -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "symlog-verbosity-{}-{}-{}",
        std::process::id(),
        nanos,
        N.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&dir).expect("scratch");
    dir
}

#[test]
fn help_lists_quiet_and_verbose() {
    let output = symlog().arg("--help").output().expect("help");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert!(stdout.contains("--quiet"), "{stdout}");
    assert!(stdout.contains("--verbose"), "{stdout}");
    assert!(!stdout.contains("--threshold"), "{stdout}");
}

#[test]
fn pipe_and_quiet_omit_the_hint_verbose_prints_it() {
    let missing = "/nonexistent/symlog-verbosity/dev.log";
    let plain = symlog().arg(missing).output().expect("run");
    assert_eq!(plain.status.code(), Some(1));
    let stderr = String::from_utf8(plain.stderr).expect("utf8");
    assert!(plain.stdout.is_empty());
    assert_eq!(stderr.lines().count(), 1, "{stderr:?}");
    assert!(stderr.contains("bestaat niet"), "{stderr}");
    assert!(!stderr.contains("hint:"), "{stderr}");

    let quiet = symlog().args(["--quiet", missing]).output().expect("run");
    assert_eq!(quiet.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(quiet.stderr).expect("utf8"),
        stderr,
        "quiet matches a pipe"
    );

    let verbose = symlog().args(["--verbose", missing]).output().expect("run");
    assert_eq!(verbose.status.code(), Some(1));
    assert!(verbose.stdout.is_empty());
    let verr = String::from_utf8(verbose.stderr).expect("utf8");
    assert_eq!(verr.lines().count(), 2, "{verr:?}");
    assert_eq!(verr.lines().next(), stderr.lines().next(), "{verr}");
    assert!(
        verr.lines()
            .nth(1)
            .unwrap()
            .starts_with("hint: draait de app in een container?"),
        "{verr}"
    );
    assert_ne!(verbose.status.code(), Some(0));
}

#[test]
fn quiet_with_verbose_stays_exit_2() {
    let log = fixture("line-basic.log");
    let output = symlog()
        .args(["--quiet", "--verbose", "--plain", log.to_str().unwrap()])
        .output()
        .expect("run");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("utf8");
    assert!(stderr.contains("error:"), "{stderr}");
}

#[test]
fn verbose_context_stays_off_stdout() {
    let log = fixture("line-basic.log");
    let path = log.to_str().unwrap();
    let plain = symlog()
        .args(["--plain", "--no-color", path])
        .output()
        .expect("run");
    assert_eq!(plain.status.code(), Some(0));
    assert!(plain.stderr.is_empty());
    let verbose = symlog()
        .args(["--verbose", "--plain", "--no-color", path])
        .output()
        .expect("run");
    assert_eq!(verbose.status.code(), Some(0));
    assert_eq!(
        verbose.stdout, plain.stdout,
        "verbose changes no plain line"
    );
    let verr = String::from_utf8(verbose.stderr).expect("utf8");
    assert_eq!(verr, "context: records 8\n", "{verr:?}");
    assert!(!verr.contains("chain"), "{verr}");
    assert!(!verr.contains("host"), "{verr}");
    let quiet = symlog()
        .args(["--quiet", "--plain", "--no-color", path])
        .output()
        .expect("run");
    assert_eq!(quiet.status.code(), Some(0));
    assert_eq!(quiet.stdout, plain.stdout);
    assert!(quiet.stderr.is_empty());
}

#[test]
fn missing_output_directory_stays_exit_4() {
    let root = scratch();
    let report = root.join("no-such-dir").join("out.txt");
    let report_path = report.to_str().expect("utf8");
    let log = fixture("line-basic.log");
    let path = log.to_str().unwrap();
    let run = |args: &[&str]| {
        StdCommand::new(env!("CARGO_BIN_EXE_symlog"))
            .args(args)
            .output()
            .expect("run")
    };
    let plain = run(&["--plain", "--no-color", "--output", report_path, path]);
    let stderr = String::from_utf8(plain.stderr).expect("utf8");
    assert_eq!(plain.status.code(), Some(4), "{stderr}");
    assert!(plain.stdout.is_empty());
    assert!(stderr.contains("schrijven mislukt"), "{stderr}");
    assert!(stderr.contains("maakt die map niet aan"), "{stderr}");
    assert!(!stderr.contains("hint:"), "{stderr}");
    assert!(!report.exists());

    let verbose = run(&[
        "--verbose",
        "--plain",
        "--no-color",
        "--output",
        report_path,
        path,
    ]);
    let verr = String::from_utf8(verbose.stderr).expect("utf8");
    assert_eq!(verbose.status.code(), Some(4), "{verr}");
    assert!(verbose.stdout.is_empty());
    assert_eq!(verr.lines().next(), stderr.lines().next(), "{verr}");
    assert!(verr.lines().nth(1).unwrap().starts_with("hint: "), "{verr}");
    assert!(!report.exists());
    assert!(!root.join("no-such-dir").exists());
    fs::remove_dir_all(&root).ok();
}
