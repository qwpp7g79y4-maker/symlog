//! Canary-tests voor het maskeren van geheimen. Ze falen als de uitvoer de
//! oorspronkelijke waarde nog toont. Het logbestand op schijf blijft gelijk.

use assert_cmd::Command;
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use std::fs;
use std::path::Path;
use symlog::parse::raw::ParsedRecord;
use symlog::tui::{view, App};

const CANARY: &str = "password=hunter2 leaked token=sk_live_abcdef123456";
const BEARER: &str = "eyJhbGciOiJIUzI1NiJ9.payload.sig";
const AKIA: &str = "AKIAIOSFODNN7EXAMPLE";
const STRIPE_TEST: &str = "sk_test_zzzz9999";

fn secret_log() -> String {
    format!(
        "{CANARY}\n\
         [2026-01-15T10:30:44.000000+01:00] app.INFO: quiet day [] []\n\
         [2026-01-15T10:30:45.000000+01:00] app.ERROR: {CANARY} [] []\n\
         [2026-01-15T10:30:46.000000+01:00] app.INFO: Authorization: Bearer {BEARER} [] []\n\
         [2026-01-15T10:30:47.000000+01:00] app.INFO: aws {AKIA} done [] []\n\
         [2026-01-15T10:30:48.000000+01:00] app.INFO: PASSWORD=Hunter2 API_KEY=UpperKey PASSWD=pwtoken APIKEY=apk SECRET: secval [] []\n\
         [2026-01-15T10:30:49.000000+01:00] app.INFO: AUTHORIZATION: BEARER S3cretToken [] []\n\
         [2026-01-15T10:30:50.000000+01:00] app.INFO: payload {{\"password\":\"jsonpass99\",\"api_key\":\"jsonkey99\"}} [] []\n\
         {{\"message\":\"login failed\",\"context\":{{\"password\":\"ctxpass99\",\"api_key\":\"ctxkey99\"}},\"level_name\":\"ERROR\",\"channel\":\"security\",\"datetime\":\"2026-01-15T10:30:51.000000+01:00\",\"extra\":{{\"secret\":\"s3cr3t\",\"token\":\"{STRIPE_TEST}\"}}}}\n"
    )
}

const PLAIN_SECRETS: [&str; 12] = [
    "hunter2",
    "sk_live_abcdef123456",
    BEARER,
    AKIA,
    "Hunter2",
    "UpperKey",
    "pwtoken",
    "apk",
    "secval",
    "S3cretToken",
    "jsonpass99",
    "jsonkey99",
];

fn write_log(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("secrets.log");
    fs::write(&path, secret_log()).expect("schrijf log");
    path
}

fn plain_stdout(path: &Path, no_redact: bool) -> String {
    let mut cmd = Command::cargo_bin("symlog").expect("binary");
    cmd.arg(path);
    if no_redact {
        cmd.arg("--no-redact");
    }
    let assert = cmd.assert().success();
    String::from_utf8(assert.get_output().stdout.clone()).expect("utf8")
}

fn assert_secrets_absent(text: &str) {
    for secret in PLAIN_SECRETS {
        assert!(
            !text.contains(secret),
            "geheim {secret:?} lekt in de uitvoer:\n{text}"
        );
    }
}

#[test]
fn plain_output_masks_the_canary_line_and_leaves_the_file_unchanged() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_log(dir.path());
    let before = fs::read(&path).expect("lees vooraf");

    let stdout = plain_stdout(&path, false);

    assert_secrets_absent(&stdout);
    assert!(stdout.contains("***"), "verwacht ***: {stdout}");
    assert!(
        stdout.contains("leaked"),
        "de omringende tekst blijft staan: {stdout}"
    );
    assert_eq!(fs::read(&path).expect("lees achteraf"), before);
    assert!(
        String::from_utf8(before).expect("utf8").contains("hunter2"),
        "het bestand zelf houdt de oorspronkelijke waarde"
    );
}

#[test]
fn plain_output_masks_json_bearer_akia_and_uppercase_keys() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_log(dir.path());
    let stdout = plain_stdout(&path, false);
    assert_secrets_absent(&stdout);
    assert!(stdout.contains("Authorization: Bearer ***"), "{stdout}");
    assert!(stdout.contains("AUTHORIZATION: BEARER ***"), "{stdout}");
    assert!(stdout.contains("PASSWORD=***"), "{stdout}");
    assert!(stdout.contains(r#"payload {"password":"***""#), "{stdout}");
    assert!(!stdout.contains(AKIA), "{stdout}");
}

#[test]
fn no_redact_shows_the_original_values() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_log(dir.path());
    let stdout = plain_stdout(&path, true);
    for secret in PLAIN_SECRETS {
        assert!(
            stdout.contains(secret),
            "--no-redact moet {secret:?} tonen:\n{stdout}"
        );
    }
    assert!(
        !stdout.contains("***"),
        "geen masker bij --no-redact: {stdout}"
    );
}

fn load_app(redact: bool) -> App {
    let records = symlog::parse::split_records(&secret_log())
        .into_iter()
        .map(|block| symlog::parse::parse_record_lossy(block.as_bytes()))
        .collect();
    let mut app = App::new(records);
    app.set_redact(redact);
    app
}

fn search(app: &mut App, term: &str) {
    app.enter_search_mode();
    for c in term.chars() {
        app.push_search_char(c);
    }
    app.commit_search();
}

fn screen(app: &mut App, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("TestBackend");
    terminal
        .draw(|frame| view::render(frame, app))
        .expect("draw");
    let buffer = terminal.backend().buffer();
    let area = buffer.area;
    let mut lines = Vec::new();
    for y in 0..area.height {
        let mut row = String::new();
        for x in 0..area.width {
            let cell = buffer
                .cell((area.x + x, area.y + y))
                .expect("cel binnen de buffer");
            row.push_str(cell.symbol());
        }
        lines.push(row.trim_end().to_string());
    }
    lines.join("\n")
}

#[test]
fn tui_list_search_result_and_detail_mask_secrets() {
    let mut app = load_app(true);
    let _ = screen(&mut app, 200, 30);
    app.go_top();

    search(&mut app, "leaked");
    assert!(app.selected_index() > 0, "de zoekactie landt op de canary");
    let list = screen(&mut app, 200, 30);
    assert!(
        list.contains("RAW["),
        "de ruwe canary-regel staat in de lijst:\n{list}"
    );
    assert_secrets_absent(&list);
    assert!(list.contains("***"), "{list}");
    assert!(list.contains("leaked"), "{list}");

    search(&mut app, "login");
    match app.selected_record() {
        Some(ParsedRecord::Record(record)) => {
            let stored = record.context.to_string();
            assert!(
                stored.contains("ctxpass99"),
                "het record zelf blijft ongemaskeerd: {stored}"
            );
        }
        other => panic!("JSON-record verwacht, kreeg {other:?}"),
    }
    app.toggle_detail();
    let detail = screen(&mut app, 200, 40);
    assert_secrets_absent(&detail);
    for secret in ["ctxpass99", "ctxkey99", "s3cr3t", STRIPE_TEST] {
        assert!(
            !detail.contains(secret),
            "detail lekt {secret:?}:\n{detail}"
        );
    }
    assert!(detail.contains("***"), "{detail}");
    assert!(detail.contains("login failed"), "{detail}");
}

#[test]
fn tui_no_redact_shows_the_original_values() {
    let mut app = load_app(false);
    // Eén keer tekenen zet de viewporthoogte. Zo blijft de lijst bij een latere
    // zoekactie bovenaan staan en is elke rij (niet alleen de geselecteerde) zichtbaar.
    let _ = screen(&mut app, 200, 40);
    app.go_top();
    search(&mut app, "login");
    app.toggle_detail();
    let text = screen(&mut app, 200, 40);
    assert!(text.contains("hunter2"), "{text}");
    assert!(text.contains("sk_live_abcdef123456"), "{text}");
    assert!(text.contains("ctxpass99"), "{text}");
    assert!(text.contains(STRIPE_TEST), "{text}");
    assert!(text.contains(AKIA), "{text}");
    assert!(text.contains(BEARER), "{text}");
}
