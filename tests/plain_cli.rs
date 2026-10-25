//! `assert_cmd`-tests voor task.md T13 "Klaar als": `symlog F1 | cat` geeft golden
//! plain-uitvoer; `--level error --channel request` plain is golden; `--tui` zonder
//! tty geeft exit 3; plain + `--follow` met een groeiend bestand geeft de nieuwe
//! regels. `assert_cmd` geeft de binary zelf al een niet-tty stdout (een pipe), dus
//! geen van deze tests heeft `--plain` nodig om de plain-modus te krijgen.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::thread;
use std::time::Duration;

const EXIT_TUI_REQUIRES_TTY: i32 = 3;

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn golden_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name)
}

fn assert_matches_golden_plain(actual: &str, golden: &str) {
    let path = golden_path(golden);
    let expected = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    assert_eq!(
        actual,
        expected.trim_end_matches('\n'),
        "komt niet overeen met {golden}:\n{actual}"
    );
}

fn write_file(path: &Path, content: &[u8]) {
    fs::File::create(path)
        .expect("create")
        .write_all(content)
        .expect("write");
}

/// F1 zonder enige vlag geeft, gepiped (dus zonder tty), golden plain-uitvoer: één
/// regel per record, zonder kleurcodes (geen tty) en zonder frames (F1 heeft geen
/// stacktraces).
#[test]
fn f1_plain_output_matches_golden() {
    let assert = Command::cargo_bin("symlog")
        .expect("binary")
        .arg(fixture_path("line-basic.log"))
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8");
    assert!(
        !stdout.contains('\u{1b}'),
        "gepiped stdout is geen tty, dus geen ANSI-kleurcodes: {stdout:?}"
    );
    assert_matches_golden_plain(stdout.trim_end_matches('\n'), "plain_line_basic.txt");
}

/// `--level error --channel request` filtert op de combinatie van beide (task.md T13
/// "Klaar als"); op `mixed.log` (lijn- én JSON-records door elkaar, task.md T5) komen
/// daar de twee `request.ERROR`-records uit, één van elk bronformaat.
#[test]
fn level_and_channel_filtered_plain_output_matches_golden() {
    let assert = Command::cargo_bin("symlog")
        .expect("binary")
        .arg(fixture_path("mixed.log"))
        .arg("--level")
        .arg("error")
        .arg("--channel")
        .arg("request")
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8");
    assert_matches_golden_plain(
        stdout.trim_end_matches('\n'),
        "plain_mixed_error_request.txt",
    );
}

/// Een eenmalige plain-run (geen `--follow`) laat de staartregel zonder afsluitende
/// `\n` niet stil verdwijnen (breaktest-symlog-0004 bevinding 1 kritiek): die regel
/// moet, net als de TUI-weg via `IndexScanner::finish`, alsnog verschijnen — en
/// geheimen erin blijven gemaskeerd.
#[test]
fn plain_mode_without_follow_shows_unterminated_tail_line_redacted() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("app.log");
    write_file(
        &path,
        b"[2026-01-01 00:00:00] a.INFO: one [] []\n\
          [2026-01-01 00:00:01] a.INFO: tail token=abc123secretvalue [] []",
    );

    let assert = Command::cargo_bin("symlog")
        .expect("binary")
        .arg(&path)
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8");
    assert_eq!(
        stdout,
        "[2026-01-01 00:00:00] a.INFO: one\n\
         [2026-01-01 00:00:01] a.INFO: tail token=***\n"
    );
}

/// `--tui` zonder een tty op stdout geeft exit 3 met een uitleg, in plaats van stil
/// terug te vallen op plain of te crashen op het aanzetten van raw mode.
#[test]
fn tui_flag_without_a_tty_exits_three() {
    Command::cargo_bin("symlog")
        .expect("binary")
        .arg(fixture_path("line-basic.log"))
        .arg("--tui")
        .assert()
        .code(EXIT_TUI_REQUIRES_TTY)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("--tui").and(predicate::str::contains("tty")));
}

/// Plain + `--follow` streamt nieuwe regels zodra ze geschreven worden (task.md T13
/// "Doet": "`--follow` werkt ook in plain (streamt)"), precies zoals `tail -f`. De
/// test beheert het kindproces zelf (geen `assert_cmd::Command::assert`, dat op
/// volledige afsluiting wacht) en beëindigt het aan het eind, net als
/// `tests/startup_cli.rs`'s `missing_file_with_follow_waits_then_keeps_following`.
#[test]
fn plain_follow_streams_new_lines_as_the_file_grows() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("app.log");
    write_file(&path, b"[2026-01-01 00:00:00] a.INFO: one [] []\n");

    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_symlog"))
        .arg(&path)
        .arg("--follow")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");

    let mut stdout = BufReader::new(child.stdout.take().expect("stdout piped"));
    let mut first_line = String::new();
    stdout
        .read_line(&mut first_line)
        .expect("lees de eerste regel");
    assert_eq!(first_line, "[2026-01-01 00:00:00] a.INFO: one\n");

    use std::fs::OpenOptions;
    OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("open voor append")
        .write_all(b"[2026-01-01 00:00:01] a.INFO: two [] []\n")
        .expect("append");
    // Ruim boven de polling-terugval van 500 ms (blueprint.md T10) plus marge voor een
    // trage CI-machine.
    thread::sleep(Duration::from_millis(1500));

    let mut second_line = String::new();
    stdout
        .read_line(&mut second_line)
        .expect("lees de tweede regel");
    assert_eq!(second_line, "[2026-01-01 00:00:01] a.INFO: two\n");

    child.kill().expect("kill");
    child.wait().expect("wait na kill");
}
