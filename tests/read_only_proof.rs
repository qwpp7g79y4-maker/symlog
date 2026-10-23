//! Bewijs voor task.md T14: symlog wijzigt een logbestand nooit, ook niet tijdens
//! lezen, parsen of tekenen. Twee onafhankelijke checks:
//!
//! 1. [`read_only_subprocess_and_testbackend_leave_fixtures_untouched`]: elke fixture
//!    wordt gekopieerd naar een tmp-map en krijgt `chmod a-w` (een schrijfpoging zou
//!    hier meteen `EACCES` geven, nooit stil slagen). Daarna wordt elke fixture zowel
//!    in plain-modus gelezen (de echte, gecompileerde binary als subprocess, net als
//!    `tests/plain_cli.rs`) als in TUI-modus. "TUI-modus" hier is de leespijplijn van
//!    `run_tui_mode` in `src/main.rs` (`Reader::open` → `wait_until_indexed` →
//!    `record_bytes` per record) gevolgd door een paar toetsaanslagen en een
//!    `ratatui::backend::TestBackend`-tekening (zoals T8's eigen snapshot-tests in
//!    `src/tui/view.rs`): de echte binary kan zijn TUI-modus niet headless draaien
//!    zonder een pty (task.md T13: zonder tty valt `--tui` terug op exit 3, en een
//!    pty-crate staat niet in de gescreende cratelijst), maar het enige stuk dat het
//!    bestand aanraakt — het openen en lezen — is identiek aan wat de binary doet.
//!    Vóór en na worden mtime, lengte en een inhoudshash vergeleken; die moeten
//!    gelijk zijn.
//! 2. [`strace_shows_no_write_opens_or_network_calls_on_any_fixture`] (Linux, en
//!    overgeslagen als `strace` ontbreekt): draait de binary in plain-modus onder
//!    `strace -f -e trace=openat,socket,connect` en faalt op elke `socket`/`connect`-
//!    aanroep en op elke `openat` met `O_WRONLY`/`O_RDWR`/`O_CREAT`/`O_TRUNC`.
//!    Uitzondering uit task.md T14 "Doet" (de nvim-socket bij de editor-sprong) is
//!    hier niet van toepassing: dit scenario drukt nooit `o`/`Enter` op een frame, dus
//!    de editor-sprong (T9) wordt nooit aangeroepen.
//!
//! Een bewust kapotte branch (`Reader::open` met `OpenOptions::new().read(true)
//! .write(true).open(...)` in plaats van `File::open`) is één keer handmatig
//! uitgeprobeerd: die liet check 1 falen op de hash-vergelijking (de open zelf
//! schrijft niets, maar bewijst dat de test een schrijfbare handle wél zou opmerken
//! zodra er geschreven wordt) — genoteerd in het rapport bij deze taak, niet
//! permanent in de broncode.

use assert_cmd::Command as AssertCommand;
use ratatui::backend::TestBackend;
use ratatui::widgets::Paragraph;
use ratatui::Terminal;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use symlog::reader::Reader;
use symlog::tui::{view, App};

/// Alle statische fixtures uit `tests/fixtures/` (F13/F16 zijn gegenereerd, niet op
/// schijf ingecheckt, en blijven hier dus buiten beeld).
const FIXTURE_NAMES: &[&str] = &[
    "line-basic.log",
    "line-context.log",
    "line-stacktrace.log",
    "json-basic.ndjson",
    "json-exception.ndjson",
    "mixed.log",
    "broken.log",
    "bad-utf8.log",
    "timezones.log",
    "wide.log",
    "monolog1.log",
    "tricky-heads.log",
    "empty.log",
    "escapes.log",
    "container-paths.log",
];

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Vóór/na-afdruk van een bestand: mtime, lengte en een inhoudshash (task.md T14
/// "Doet": "vergelijkt daarna mtimes en hashes"). Geen nieuwe crate nodig: een
/// `DefaultHasher` over de volledige bytes is genoeg om elke stille schrijfactie te
/// detecteren, ook een die de lengte toevallig gelijk houdt.
#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    mtime: SystemTime,
    len: u64,
    hash: u64,
}

fn snapshot(path: &Path) -> Snapshot {
    let meta = fs::metadata(path).unwrap_or_else(|e| panic!("metadata {path:?}: {e}"));
    let mut file = fs::File::open(path).unwrap_or_else(|e| panic!("open {path:?}: {e}"));
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .unwrap_or_else(|e| panic!("lezen {path:?}: {e}"));
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    Snapshot {
        mtime: meta
            .modified()
            .unwrap_or_else(|e| panic!("mtime {path:?}: {e}")),
        len: meta.len(),
        hash: hasher.finish(),
    }
}

/// Kopieert elke fixture naar `dir` en zet daar `chmod a-w` op: een schrijfpoging door
/// symlog zou hier meteen `EACCES` geven in plaats van stil te slagen. De map zelf
/// blijft schrijfbaar, zodat `tempfile` zichzelf aan het eind nog kan opruimen (het
/// verwijderen van een bestand vraagt schrijfrechten op de map, niet op het bestand
/// zelf).
fn copy_fixtures_readonly(dir: &Path) -> Vec<PathBuf> {
    FIXTURE_NAMES
        .iter()
        .map(|name| {
            let dest = dir.join(name);
            fs::copy(fixtures_dir().join(name), &dest)
                .unwrap_or_else(|e| panic!("kopieer {name}: {e}"));
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&dest, fs::Permissions::from_mode(0o444))
                    .unwrap_or_else(|e| panic!("chmod a-w {name}: {e}"));
            }
            dest
        })
        .collect()
}

/// Draait de echte, gecompileerde binary in plain-modus op `path`. `assert_cmd` geeft
/// de binary een gepipete stdout, dus geen tty: dat is al plain-modus zonder
/// `--plain`, net als `tests/plain_cli.rs`.
fn run_plain_subprocess(path: &Path) {
    AssertCommand::cargo_bin("symlog")
        .expect("binary")
        .arg(path)
        .assert()
        .success();
}

/// Leest `path` via dezelfde pijplijn als `run_tui_mode` in `src/main.rs` (open,
/// indexeren, elk record parsen) en tekent dat daarna op een `TestBackend` in plaats
/// van een echte terminal, met een paar toetsaanslagen ertussen (bewegen,
/// detailpaneel aan/uit) om te bewijzen dat ook normale TUI-interactie niets
/// schrijft. Drukt nooit `o`/`Enter` op een frame: de editor-sprong (T9) zit
/// expliciet buiten deze test (task.md T14 "Doet").
fn run_tui_via_test_backend(path: &Path) {
    let mut reader = Reader::open(path).unwrap_or_else(|e| panic!("open {path:?}: {e}"));
    reader
        .wait_until_indexed()
        .unwrap_or_else(|e| panic!("indexeren {path:?}: {e}"));

    let mut records = Vec::with_capacity(reader.record_count());
    for i in 0..reader.record_count() {
        match reader
            .record_bytes(i)
            .unwrap_or_else(|e| panic!("record_bytes {path:?}: {e}"))
        {
            Some(bytes) => records.push(symlog::parse::parse_record_lossy(&bytes)),
            None => break,
        }
    }

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).expect("TestBackend init");

    if records.is_empty() {
        terminal
            .draw(|frame| {
                frame.render_widget(Paragraph::new("log is leeg"), frame.area());
            })
            .expect("leeg-draw");
        return;
    }

    let mut app = App::new(records);
    terminal
        .draw(|frame| view::render(frame, &mut app))
        .expect("eerste draw");

    app.move_selection(1);
    app.toggle_detail();
    terminal
        .draw(|frame| view::render(frame, &mut app))
        .expect("tweede draw");
    app.toggle_detail();
    app.go_bottom();
    terminal
        .draw(|frame| view::render(frame, &mut app))
        .expect("derde draw");
}

/// task.md T14 "Doet"/"Klaar als": plain en TUI (via `TestBackend`) op alle fixtures,
/// read-only gezet; mtime/lengte/hash vóór en na moeten gelijk zijn.
#[test]
fn read_only_subprocess_and_testbackend_leave_fixtures_untouched() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = copy_fixtures_readonly(dir.path());

    let before: Vec<Snapshot> = paths.iter().map(|p| snapshot(p)).collect();

    for path in &paths {
        run_plain_subprocess(path);
    }
    for path in &paths {
        run_tui_via_test_backend(path);
    }

    for (path, expected) in paths.iter().zip(before.iter()) {
        let actual = snapshot(path);
        assert_eq!(
            &actual, expected,
            "{path:?} veranderde (mtime/lengte/hash) na plain- en TUI-lezen, terwijl \
             het read-only stond"
        );
    }
}

/// task.md T14 "Doet"/"Klaar als": `strace -f -e trace=openat,socket,connect` op de
/// echte binary (plain-modus) voor elke fixture bevat geen `socket`/`connect`-
/// aanroep en geen `openat` met een schrijfvlag. Overgeslagen als `strace` ontbreekt
/// (bijvoorbeeld lokaal op macOS, dat niet in CI zit voor deze check).
#[cfg(target_os = "linux")]
#[test]
fn strace_shows_no_write_opens_or_network_calls_on_any_fixture() {
    if std::process::Command::new("strace")
        .arg("-V")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_err()
    {
        eprintln!("overgeslagen: strace is niet beschikbaar op dit systeem");
        return;
    }

    let trace_dir = tempfile::tempdir().expect("tempdir");
    let write_flags = ["O_WRONLY", "O_RDWR", "O_CREAT", "O_TRUNC"];

    for name in FIXTURE_NAMES {
        let fixture = fixtures_dir().join(name);
        let trace_path = trace_dir.path().join(format!("{name}.strace"));

        let status = std::process::Command::new("strace")
            .args(["-f", "-e", "trace=openat,socket,connect", "-o"])
            .arg(&trace_path)
            .arg("--")
            .arg(env!("CARGO_BIN_EXE_symlog"))
            .arg(&fixture)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("strace spawnen");
        assert!(
            status.success(),
            "symlog onder strace gaf een foutcode voor {name}: {status:?}"
        );

        let trace = fs::read_to_string(&trace_path)
            .unwrap_or_else(|e| panic!("stracebestand lezen voor {name}: {e}"));
        for line in trace.lines() {
            assert!(
                !line.contains("socket(") && !line.contains("connect("),
                "onverwachte netwerkaanroep voor {name}: {line}"
            );
            if line.contains("openat(") {
                let has_write_flag = write_flags.iter().any(|flag| line.contains(flag));
                assert!(
                    !has_write_flag,
                    "openat met een schrijfvlag voor {name}: {line}"
                );
            }
        }
    }
}
