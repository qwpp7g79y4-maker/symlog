//! CLI-ingang (task.md T11: leeg en ontbrekend bestand). De volledige CLI uit
//! task.md's CLI-schets (`--format`, `--channel`, `--level`, `--grep`, `--tz`,
//! `--map`, config, modus kiezen tussen TUI/plain) wordt in latere taken aangesloten
//! ([`symlog::tui`]'s moduledoc: "die main-loop hoort bij de CLI-laag ... task.md
//! T13, nog niet aangesloten"); dit bestand bevat voor nu alleen het pad-argument,
//! `--follow` en de opstartcontroles uit T11.

use clap::Parser;
use std::io::Write;
use std::path::{Path, PathBuf};
use symlog::follow::{Follower, Watcher, DEFAULT_POLL_INTERVAL};
use symlog::startup::{self, Outcome};

#[derive(Parser)]
#[command(name = "symlog", version)]
struct Cli {
    /// Pad naar het logbestand.
    path: PathBuf,
    /// Blijf het bestand volgen (`tail -f`), ook als het nog niet bestaat.
    #[arg(short = 'f', long)]
    follow: bool,
}

fn main() {
    let cli = Cli::parse();
    let code = run(&cli.path, cli.follow, &mut std::io::stderr());
    std::process::exit(code);
}

/// De eigenlijke opstartlogica, los van `process::exit` en de echte stderr zodat dit
/// zonder een subproces getest kan worden (zie `tests/startup_cli.rs` voor de
/// subproces-tests die ook de echte exitcode en stderr-tekst controleren).
fn run(path: &Path, follow: bool, err_out: &mut impl Write) -> i32 {
    let classification = startup::classify(path);
    let classification = if follow {
        match classification {
            startup::Classification::Missing => startup::wait_until_present(path, || {
                let _ = writeln!(err_out, "wacht op {}...", path.display());
            }),
            other => other,
        }
    } else {
        classification
    };

    match startup::outcome_for(path, classification) {
        Outcome::Exit { message, code } => {
            let _ = writeln!(err_out, "{message}");
            code
        }
        Outcome::Empty => 0,
        Outcome::Ready => {
            if follow {
                follow_forever(path);
            }
            0
        }
    }
}

/// Blijft `path` volgen (task.md T11 "Doet": "daarna normaal volgen"; task.md T10
/// voor groei/rotatie/afkappen). Tekent nog niets (task.md T13, nog niet
/// aangesloten): dit bewijst alleen dat `--follow` normaal door blijft draaien in
/// plaats van af te sluiten zodra het bestand verschenen is. Stopt alleen door het
/// proces te beëindigen (bijvoorbeeld Ctrl-C), net als `tail -f`.
fn follow_forever(path: &Path) -> ! {
    let watcher = Watcher::new(path, DEFAULT_POLL_INTERVAL);
    let mut follower = Follower::open(path);
    loop {
        let _ = follower.tick();
        watcher.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_file(path: &Path, content: &[u8]) {
        std::fs::File::create(path)
            .expect("create")
            .write_all(content)
            .expect("write");
    }

    #[test]
    fn empty_file_exits_zero_without_a_message() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("empty.log");
        write_file(&path, b"");

        let mut out = Vec::new();
        let code = run(&path, false, &mut out);
        assert_eq!(code, 0);
        assert!(out.is_empty());
    }

    #[test]
    fn missing_file_without_follow_exits_one_with_a_hint() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("weg.log");

        let mut out = Vec::new();
        let code = run(&path, false, &mut out);
        assert_eq!(code, startup::EXIT_MISSING);
        let message = String::from_utf8(out).expect("utf8");
        assert!(message.contains(&path.display().to_string()));
        assert!(message.contains("bestaat niet"));
    }

    #[test]
    fn directory_as_path_exits_two() {
        let dir = tempfile::tempdir().expect("tempdir");

        let mut out = Vec::new();
        let code = run(dir.path(), false, &mut out);
        assert_eq!(code, startup::EXIT_USAGE);
        assert!(String::from_utf8(out).expect("utf8").contains("map"));
    }

    #[test]
    fn non_empty_file_without_follow_exits_zero() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("app.log");
        write_file(&path, b"[2026-01-01 00:00:00] a.INFO: one\n");

        let mut out = Vec::new();
        let code = run(&path, false, &mut out);
        assert_eq!(code, 0);
        assert!(out.is_empty());
    }
}
