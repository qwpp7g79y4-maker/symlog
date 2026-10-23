//! `assert_cmd`-tests voor task.md T11 "Klaar als": F14 (leeg), ontbrekend,
//! ontbrekend + `--follow` (bestand verschijnt na 1 s), geen rechten (overgeslagen
//! als root) en map als pad. Elke test controleert de exitcode en de stderr-tekst
//! van de echte, gecompileerde `symlog`-binary.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::thread;
use std::time::Duration;

const EXIT_OK: i32 = 0;
const EXIT_MISSING: i32 = 1;
const EXIT_USAGE: i32 = 2;
const EXIT_IO_ERROR: i32 = 4;

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn write_file(path: &Path, content: &[u8]) {
    fs::File::create(path)
        .expect("create")
        .write_all(content)
        .expect("write");
}

/// F14: leeg bestand (0 bytes) geeft geen TUI/plain-uitvoer (de echte weergave komt
/// pas in T13) en sluit af met exitcode 0, zonder stderr-tekst.
#[test]
fn f14_empty_file_exits_zero_silently() {
    Command::cargo_bin("symlog")
        .expect("binary")
        .arg(fixture_path("empty.log"))
        .assert()
        .success()
        .code(EXIT_OK)
        .stderr(predicate::str::is_empty())
        .stdout(predicate::str::is_empty());
}

/// Ontbrekend bestand zonder `--follow`: melding met het pad en een hint, exit 1.
#[test]
fn missing_file_without_follow_exits_one_with_a_hint() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("var-log-dev.log");

    Command::cargo_bin("symlog")
        .expect("binary")
        .arg(&path)
        .assert()
        .code(EXIT_MISSING)
        .stderr(
            predicate::str::contains(path.to_string_lossy().as_ref())
                .and(predicate::str::contains("bestaat niet")),
        );
}

/// Pad is een map, geen bestand: exit 2.
#[test]
fn directory_as_path_exits_two() {
    let dir = tempfile::tempdir().expect("tempdir");

    Command::cargo_bin("symlog")
        .expect("binary")
        .arg(dir.path())
        .assert()
        .code(EXIT_USAGE)
        .stderr(predicate::str::contains("map"));
}

/// Geen leesrechten: duidelijke melding, exit 4. Overgeslagen als de test als root
/// draait (task.md T11 "Klaar als"): `chmod 000` heeft dan geen effect.
#[test]
fn no_read_permission_exits_four() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("geheim.log");
    write_file(&path, b"[2026-01-01 00:00:00] a.INFO: geheim\n");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).expect("chmod");
    }

    if fs::File::open(&path).is_ok() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).ok();
        }
        eprintln!("overgeslagen: draait als root, leesrechten hebben geen effect");
        return;
    }

    Command::cargo_bin("symlog")
        .expect("binary")
        .arg(&path)
        .assert()
        .code(EXIT_IO_ERROR)
        .stderr(predicate::str::contains("leesrechten"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).ok();
    }
}

/// Ontbrekend bestand + `--follow`: wacht ("wacht op ...") tot het bestand verschijnt,
/// en sluit daarna niet met de "ontbrekend"-fout af; het proces blijft normaal volgen
/// (task.md T11 "Doet": "daarna normaal volgen"),
/// precies zoals `tail -f`. De test beheert het kindproces zelf (geen
/// `assert_cmd::Command::assert`, dat op volledige afsluiting wacht) en beëindigt het
/// aan het eind.
#[test]
fn missing_file_with_follow_waits_then_keeps_following() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("komt-nog.log");

    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_symlog"))
        .arg(&path)
        .arg("--follow")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");

    let mut stderr = BufReader::new(child.stderr.take().expect("stderr piped"));
    let mut wait_line = String::new();
    stderr
        .read_line(&mut wait_line)
        .expect("lees 'wacht op'-regel");
    assert!(
        wait_line.contains("wacht op") && wait_line.contains(&path.to_string_lossy().to_string()),
        "onverwachte eerste stderr-regel: {wait_line:?}"
    );

    // Nog geen exit: het proces zit te wachten, niet afgesloten met de
    // "ontbrekend"-foutcode.
    assert!(
        child.try_wait().expect("try_wait").is_none(),
        "symlog --follow stopte al vóór het bestand bestond"
    );

    write_file(&path, b"[2026-01-01 00:00:00] a.INFO: hier ben ik\n");
    // Ruim boven de polling-terugval van 500 ms (blueprint.md T10) plus marge voor
    // een trage CI-machine.
    thread::sleep(Duration::from_millis(1500));

    // Het bestand bestaat nu; symlog volgt gewoon door (geen foutafsluiting).
    if let Some(status) = child.try_wait().expect("try_wait") {
        let mut rest = String::new();
        use std::io::Read;
        stderr.read_to_string(&mut rest).ok();
        panic!(
            "symlog --follow stopte na het verschijnen van het bestand in plaats van te blijven \
             volgen: status={status:?} resterende stderr={rest:?}"
        );
    }

    child.kill().expect("kill");
    child.wait().expect("wait na kill");
}
