//! Opstartcontroles: leeg en ontbrekend bestand (task.md T11; blueprint.md
//! "Foutafhandeling", exitcodes uit de CLI-schets).
//!
//! Net als [`crate::editor`]'s scheiding tussen een pure beslisboom ([`editor::decide`])
//! en de daadwerkelijke uitvoering is deze module puur: [`classify`] en [`outcome_for`]
//! raken geen stderr/stdout aan en roepen nooit `process::exit`, zodat ze zonder een
//! echt subproces getest kunnen worden. Alleen [`wait_until_present`] blokkeert
//! (bedoeld voor `--follow` op een nog ontbrekend bestand); de aanroeper (`main.rs`)
//! bepaalt wat er daadwerkelijk naar stderr gaat en met welke exitcode het proces
//! stopt.

use crate::follow::{Watcher, DEFAULT_POLL_INTERVAL};
use std::fs::{self, File};
use std::io;
use std::path::Path;

/// Exitcode "bestand ontbreekt" (zonder `--follow`), task.md CLI-schets.
pub const EXIT_MISSING: i32 = 1;
/// Exitcode "gebruiksfout" (flags, config, of hier: pad is een map), task.md
/// CLI-schets.
pub const EXIT_USAGE: i32 = 2;
/// Exitcode "I/O-fout tijdens lezen die niet herstelbaar is"; task.md T11 gebruikt
/// deze ook voor "geen leesrechten" bij het openen.
pub const EXIT_IO_ERROR: i32 = 4;

/// Toestand van `path` op dit moment (task.md T11 "Doet"): het resultaat van één
/// `stat` + (als dat geen map is) één poging tot `File::open`.
#[derive(Debug)]
pub enum Classification {
    /// Bestaat niet (of een map ergens in het pad erboven bestaat niet).
    Missing,
    /// Bestaat, maar is een map, geen bestand.
    Directory,
    /// Bestaat als bestand, maar mag niet gelezen worden.
    PermissionDenied,
    /// Bestaat, is leesbaar, 0 bytes.
    Empty,
    /// Bestaat, is leesbaar, heeft inhoud.
    NonEmpty,
    /// Een onverwachte, niet-herstelbare I/O-fout (bijvoorbeeld een symlink-lus).
    Io(io::Error),
}

/// Bepaalt [`Classification`] voor `path`. Opent het bestand alleen om leesrechten en
/// lengte te controleren en sluit het meteen weer; de echte leespijplijn (task.md
/// T13, nog niet aangesloten) opent het zelf opnieuw.
pub fn classify(path: &Path) -> Classification {
    let meta = match fs::metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Classification::Missing,
        Err(err) if err.kind() == io::ErrorKind::PermissionDenied => {
            return Classification::PermissionDenied;
        }
        Err(err) => return Classification::Io(err),
    };
    if meta.is_dir() {
        return Classification::Directory;
    }
    match File::open(path) {
        Ok(file) => {
            let len = file.metadata().map(|m| m.len()).unwrap_or(meta.len());
            if len == 0 {
                Classification::Empty
            } else {
                Classification::NonEmpty
            }
        }
        Err(err) if err.kind() == io::ErrorKind::PermissionDenied => {
            Classification::PermissionDenied
        }
        Err(err) => Classification::Io(err),
    }
}

/// Blokkeert tot `path` niet meer [`Classification::Missing`] is (task.md T11:
/// "Met `--follow`: wachten tot het bestand verschijnt"). Gebruikt [`Watcher`]
/// (notify + de polling-terugval van 500 ms, blueprint.md T10) zodat dit zowel op een
/// echte `inotify`/`FSEvents`-gebeurtenis als zonder (een oudermap die nog niet
/// bestaat) wakker wordt. `on_first_wait` wordt precies één keer aangeroepen, vóór de
/// eerste keer wachten, zodat de aanroeper één keer een "wacht op ..."-boodschap kan
/// tonen in plaats van bij elke poll-tick.
pub fn wait_until_present(path: &Path, on_first_wait: impl FnOnce()) -> Classification {
    let watcher = Watcher::new(path, DEFAULT_POLL_INTERVAL);
    let mut on_first_wait = Some(on_first_wait);
    loop {
        match classify(path) {
            Classification::Missing => {
                if let Some(f) = on_first_wait.take() {
                    f();
                }
                watcher.wait();
            }
            other => return other,
        }
    }
}

/// Wat `main` met een [`Classification`] moet doen (task.md T11 "Doet"). De echte
/// leespijplijn voor [`Outcome::Ready`] is task.md T13, nog niet aangesloten; `main`
/// beslist zelf wat het daarmee doet (voor `--follow`: normaal volgen, task.md T10).
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Meteen afsluiten: `message` naar stderr, exitcode `code`.
    Exit { message: String, code: i32 },
    /// Leeg bestand: TUI toont "log is leeg", plain niets (task.md T11 "Doet").
    /// Exitcode 0, geen boodschap.
    Empty,
    /// Bestand bestaat, is leesbaar en heeft inhoud: klaar voor de echte
    /// leespijplijn.
    Ready,
}

/// Vertaalt [`Classification`] naar [`Outcome`] (task.md T11 "Doet" en de
/// CLI-schets' exitcode-tabel).
pub fn outcome_for(path: &Path, classification: Classification) -> Outcome {
    match classification {
        Classification::Missing => Outcome::Exit {
            message: format!(
                "{p} bestaat niet; draait de app in een container? gebruik --map of geef het juiste pad",
                p = path.display()
            ),
            code: EXIT_MISSING,
        },
        Classification::Directory => Outcome::Exit {
            message: format!("{p} is een map, geen bestand", p = path.display()),
            code: EXIT_USAGE,
        },
        Classification::PermissionDenied => Outcome::Exit {
            message: format!("{p}: geen leesrechten", p = path.display()),
            code: EXIT_IO_ERROR,
        },
        Classification::Io(err) => Outcome::Exit {
            message: format!("{p}: {err}", p = path.display()),
            code: EXIT_IO_ERROR,
        },
        Classification::Empty => Outcome::Empty,
        Classification::NonEmpty => Outcome::Ready,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    fn write_file(path: &Path, content: &[u8]) {
        File::create(path)
            .expect("create")
            .write_all(content)
            .expect("write");
    }

    #[test]
    fn missing_path_is_classified_as_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nooit-aangemaakt.log");
        assert!(matches!(classify(&path), Classification::Missing));
    }

    #[test]
    fn missing_parent_directory_is_also_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("geen-map").join("app.log");
        assert!(matches!(classify(&path), Classification::Missing));
    }

    #[test]
    fn directory_path_is_classified_as_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(matches!(classify(dir.path()), Classification::Directory));
    }

    #[test]
    fn empty_file_is_classified_as_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("empty.log");
        write_file(&path, b"");
        assert!(matches!(classify(&path), Classification::Empty));
    }

    #[test]
    fn non_empty_file_is_classified_as_non_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("app.log");
        write_file(&path, b"[2026-01-01 00:00:00] a.INFO: one\n");
        assert!(matches!(classify(&path), Classification::NonEmpty));
    }

    #[test]
    fn unreadable_file_is_classified_as_permission_denied_unless_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("geheim.log");
        write_file(&path, b"[2026-01-01 00:00:00] a.INFO: geheim\n");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).expect("chmod");

        // Als de test als root draait, negeert File::open leesrechten alsnog (task.md
        // T11 "Klaar als": "overgeslagen als de test als root draait").
        if File::open(&path).is_ok() {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).ok();
            eprintln!("overgeslagen: draait als root, leesrechten hebben geen effect");
            return;
        }

        assert!(matches!(classify(&path), Classification::PermissionDenied));
        // Permissies terugzetten zodat tempdir zichzelf netjes kan opruimen.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).ok();
    }

    #[test]
    fn outcome_for_missing_exits_with_a_hint_and_code_1() {
        let path = Path::new("/tmp/does-not-matter.log");
        let outcome = outcome_for(path, Classification::Missing);
        match outcome {
            Outcome::Exit { message, code } => {
                assert_eq!(code, EXIT_MISSING);
                assert!(message.contains("bestaat niet"));
            }
            other => panic!("verwacht Outcome::Exit, kreeg {other:?}"),
        }
    }

    #[test]
    fn outcome_for_directory_exits_with_code_2() {
        let path = Path::new("/tmp/een-map");
        let outcome = outcome_for(path, Classification::Directory);
        assert_eq!(
            outcome,
            Outcome::Exit {
                message: "/tmp/een-map is een map, geen bestand".to_string(),
                code: EXIT_USAGE,
            }
        );
    }

    #[test]
    fn outcome_for_permission_denied_exits_with_code_4() {
        let path = Path::new("/tmp/geheim.log");
        let outcome = outcome_for(path, Classification::PermissionDenied);
        assert_eq!(
            outcome,
            Outcome::Exit {
                message: "/tmp/geheim.log: geen leesrechten".to_string(),
                code: EXIT_IO_ERROR,
            }
        );
    }

    #[test]
    fn outcome_for_empty_is_empty_with_no_message() {
        let path = Path::new("/tmp/empty.log");
        assert_eq!(outcome_for(path, Classification::Empty), Outcome::Empty);
    }

    #[test]
    fn outcome_for_non_empty_is_ready() {
        let path = Path::new("/tmp/app.log");
        assert_eq!(outcome_for(path, Classification::NonEmpty), Outcome::Ready);
    }

    #[test]
    fn wait_until_present_blocks_until_the_file_appears_and_warns_once() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("komt-nog.log");
        let path_for_writer = path.clone();

        thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            write_file(
                &path_for_writer,
                b"[2026-01-01 00:00:00] a.INFO: hier ben ik\n",
            );
        });

        let calls = Arc::new(AtomicUsize::new(0));
        let calls_in_closure = Arc::clone(&calls);
        let classification = wait_until_present(&path, move || {
            calls_in_closure.fetch_add(1, Ordering::SeqCst);
        });

        // `on_first_wait` moet precies één keer gevuurd zijn, vóór het bestand er was.
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(matches!(classification, Classification::NonEmpty));
    }

    #[test]
    fn wait_until_present_does_not_wait_at_all_if_already_present() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("al-daar.log");
        write_file(&path, b"");

        let calls = Arc::new(AtomicUsize::new(0));
        let calls_in_closure = Arc::clone(&calls);
        let classification = wait_until_present(&path, move || {
            calls_in_closure.fetch_add(1, Ordering::SeqCst);
        });

        // Het bestand bestond al: `on_first_wait` had nooit mogen vuren.
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(matches!(classification, Classification::Empty));
    }
}
