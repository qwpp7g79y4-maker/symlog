//! W-24: elke foutmelding zegt wat er misging en wat je daarna doet. Per foutpad
//! controleert een test de exitcode en dat de tekst een vervolgstap noemt.
//! Exitcodes: 1 ontbreekt, 2 gebruik, 3 `--tui` zonder tty, 4 I/O.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::Stdio;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn symlog() -> Command {
    Command::cargo_bin("symlog").expect("binary")
}

#[test]
fn missing_file_names_the_path_and_next_step() {
    symlog()
        .arg("/nonexistent/symlog-fout/dev.log")
        .assert()
        .code(1)
        .stdout(predicate::str::is_empty())
        .stderr(
            predicate::str::contains("/nonexistent/symlog-fout/dev.log bestaat niet")
                .and(predicate::str::contains("geef het juiste pad op")),
        );
}

#[test]
fn directory_names_the_problem_and_an_example() {
    symlog().arg("/").assert().code(2).stderr(
        predicate::str::contains("is een map, geen bestand").and(predicate::str::contains(
            "geef het pad van een logbestand op",
        )),
    );
}

#[test]
fn bad_level_names_valid_forms() {
    symlog()
        .args(["x.log", "--level", "bogus"])
        .assert()
        .code(2)
        .stderr(
            predicate::str::contains("ongeldig --level")
                .and(predicate::str::contains("gebruik een lijst")),
        );
}

#[test]
fn tui_without_tty_says_what_to_drop() {
    symlog()
        .arg(fixture("line-basic.log"))
        .arg("--tui")
        .assert()
        .code(3)
        .stdout(predicate::str::is_empty())
        .stderr(
            predicate::str::contains("--tui vereist een terminal")
                .and(predicate::str::contains("laat --tui weg")),
        );
}

#[test]
fn stdin_with_follow_says_what_to_drop() {
    symlog()
        .args(["-", "--follow"])
        .write_stdin("")
        .assert()
        .code(2)
        .stderr(
            predicate::str::contains("--follow en --tui zijn niet beschikbaar")
                .and(predicate::str::contains("laat --follow en --tui weg")),
        );
}

fn full() -> Stdio {
    Stdio::from(
        OpenOptions::new()
            .write(true)
            .open("/dev/full")
            .expect("/dev/full"),
    )
}

#[test]
fn stdout_full_in_file_mode_names_the_output() {
    if !Path::new("/dev/full").exists() {
        return;
    }
    let o = std::process::Command::new(env!("CARGO_BIN_EXE_symlog"))
        .arg(fixture("line-basic.log"))
        .stdin(Stdio::null())
        .stdout(full())
        .stderr(Stdio::piped())
        .output()
        .expect("run");
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(4), "{err}");
    assert!(err.contains("lezen of schrijven mislukt"), "{err}");
    assert!(
        err.contains("uitvoer (schijf of pijp) nog open staat"),
        "{err}"
    );
    assert!(!err.contains("panicked"), "{err}");
}

#[test]
fn stdout_full_in_stdin_mode_names_the_output() {
    use std::io::Write;
    if !Path::new("/dev/full").exists() {
        return;
    }
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_symlog"))
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(full())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let input = std::fs::read(fixture("line-basic.log")).expect("fixture");
    let mut pipe = child.stdin.take().expect("stdin");
    let _ = pipe.write_all(&input);
    drop(pipe);
    let o = child.wait_with_output().expect("wait");
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(4), "{err}");
    assert!(
        err.starts_with("stdin: lezen of schrijven mislukt"),
        "{err}"
    );
    assert!(
        err.contains("controleer of de invoerpijp en de uitvoer"),
        "{err}"
    );
}
