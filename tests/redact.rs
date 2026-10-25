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

fn write_body(dir: &Path, body: &str) -> std::path::PathBuf {
    let path = dir.join("secrets.log");
    fs::write(&path, body).expect("schrijf log");
    path
}

fn load_app_from(log: &str, redact: bool) -> App {
    let records = symlog::parse::split_records(log)
        .into_iter()
        .map(|block| symlog::parse::parse_record_lossy(block.as_bytes()))
        .collect();
    let mut app = App::new(records);
    app.set_redact(redact);
    app
}

#[test]
fn plain_and_tui_mask_authorization_bearer_json_field() {
    let compact = "compact-bearer-value";
    let spaced = "spaced-bearer-value";
    let ctx = "context-bearer-value";
    let log = format!(
        "[2026-01-15T10:30:52.000000+01:00] app.INFO: headers {{\"Authorization\":\"Bearer {compact}\"}} and {{\"Authorization\": \"Bearer {spaced}\"}} [] []\n\
         {{\"message\":\"auth header\",\"context\":{{\"Authorization\":\"Bearer {ctx}\"}},\"level_name\":\"ERROR\",\"channel\":\"security\",\"datetime\":\"2026-01-15T10:30:53.000000+01:00\",\"extra\":[]}}\n"
    );
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_body(dir.path(), &log);
    let before = fs::read(&path).expect("lees vooraf");

    let stdout = plain_stdout(&path, false);
    for secret in [compact, spaced] {
        assert!(!stdout.contains(secret), "plain lekt {secret:?}:\n{stdout}");
    }
    assert!(stdout.contains("Bearer ***"), "{stdout}");
    assert!(stdout.contains("auth header"), "{stdout}");
    assert_eq!(fs::read(&path).expect("lees achteraf"), before);

    let shown = plain_stdout(&path, true);
    assert!(shown.contains(compact), "{shown}");
    assert!(shown.contains(spaced), "{shown}");

    let mut app = load_app_from(&log, true);
    let _ = screen(&mut app, 220, 40);
    app.go_top();
    search(&mut app, "auth header");
    app.toggle_detail();
    let detail = screen(&mut app, 220, 40);
    assert!(
        !detail.contains(ctx),
        "detail lekt het JSON-veld:\n{detail}"
    );
    assert!(
        !detail.contains(compact),
        "lijst of detail lekt het compacte veld:\n{detail}"
    );
    assert!(detail.contains("Bearer ***"), "{detail}");
    assert!(detail.contains("auth header"), "{detail}");

    let mut open = load_app_from(&log, false);
    let _ = screen(&mut open, 220, 40);
    open.go_top();
    search(&mut open, "auth header");
    open.toggle_detail();
    let raw = screen(&mut open, 220, 40);
    assert!(raw.contains(ctx), "--no-redact detail:\n{raw}");
    assert!(raw.contains(compact), "--no-redact lijst:\n{raw}");
}

#[test]
fn plain_output_masks_basic_auth_password_in_urls() {
    let pass = "s3cret-pass";
    let log = format!(
        "[2026-01-15T10:30:54.000000+01:00] app.INFO: connect https://ada:{pass}@db.internal:5432/app via https://example.com/health [] []\n"
    );
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_body(dir.path(), &log);

    let stdout = plain_stdout(&path, false);
    assert!(
        !stdout.contains(pass),
        "plain lekt het URL-wachtwoord:\n{stdout}"
    );
    assert!(
        stdout.contains("https://ada:***@db.internal:5432/app"),
        "{stdout}"
    );
    assert!(stdout.contains("https://example.com/health"), "{stdout}");
    assert!(stdout.contains("ada:"), "{stdout}");

    let shown = plain_stdout(&path, true);
    assert!(shown.contains(pass), "--no-redact:\n{shown}");
    assert!(!shown.contains("***"), "{shown}");
}

#[test]
fn plain_output_masks_ghp_xoxb_and_jwt() {
    let ghp = "ghp_1234567890abcdefghijklmnopqrstuvwxyz";
    let xoxb = "xoxb-1234567890-123456789012-AbCdEfGhIjKlMnOpQrStUvWx";
    let jwt = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
    let log = format!(
        "[2026-01-15T10:30:55.000000+01:00] app.INFO: gh {ghp} slack {xoxb} jwt {jwt} ip 10.0.0.5 ver 1.2.3 [] []\n"
    );
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_body(dir.path(), &log);

    let stdout = plain_stdout(&path, false);
    for secret in [ghp, xoxb, jwt] {
        assert!(!stdout.contains(secret), "plain lekt {secret:?}:\n{stdout}");
    }
    assert!(stdout.contains("ip 10.0.0.5 ver 1.2.3"), "{stdout}");
    assert!(stdout.contains("***"), "{stdout}");

    let shown = plain_stdout(&path, true);
    for secret in [ghp, xoxb, jwt] {
        assert!(
            shown.contains(secret),
            "--no-redact mist {secret:?}:\n{shown}"
        );
    }
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

const GHP_PAT: &str = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
const GITHUB_PAT: &str =
    "github_pat_11AAAAAAA0abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz012345678_";
const XOXB_TOKEN: &str = "xoxb-1234567890-123456789012-AbCdEfGhIjKlMnOpQrStUvWx";
const SK_LIVE_TOKEN: &str = "sk_live_abcdef123456789";
const JWT_TOKEN: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";

fn monolog_line(message: &str) -> String {
    format!("[2026-01-15T10:30:44.000000+01:00] app.INFO: {message} [] []\n")
}

/// Plain-uitvoer mag `token` niet meer bevatten. `--no-redact` toont hem weer.
/// Het bestand op schijf blijft byte-voor-byte gelijk.
fn assert_plain_masks_token(message: &str, token: &str, kept: &[&str]) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_body(dir.path(), &monolog_line(message));
    let before = fs::read(&path).expect("lees vooraf");

    let stdout = plain_stdout(&path, false);
    assert!(
        !stdout.contains(token),
        "plain lekt {token:?} (uitvoer {} bytes)",
        stdout.len()
    );
    for piece in kept {
        assert!(stdout.contains(piece), "verwacht {piece:?} in:\n{stdout}");
    }
    assert_eq!(fs::read(&path).expect("lees achteraf"), before);

    let shown = plain_stdout(&path, true);
    assert!(
        shown.contains(token),
        "--no-redact mist {token:?}:\n{shown}"
    );
    assert!(
        !shown.contains("***"),
        "geen masker bij --no-redact: {shown}"
    );
}

#[test]
fn plain_output_masks_ghp_username_in_urls() {
    let message = format!(
        "git fetch https://{GHP_PAT}:x@github.com/org/repo.git and https://{GHP_PAT}@github.com/org/repo.git"
    );
    assert_plain_masks_token(
        &message,
        GHP_PAT,
        &[
            "https://***:***@github.com/org/repo.git",
            "https://***@github.com/org/repo.git",
            "git fetch",
        ],
    );
}

#[test]
fn plain_output_masks_xoxb_username_in_urls() {
    let message = format!(
        "post https://{XOXB_TOKEN}:x@slack.example/api and https://{XOXB_TOKEN}@slack.example/api"
    );
    assert_plain_masks_token(
        &message,
        XOXB_TOKEN,
        &[
            "https://***:***@slack.example/api",
            "https://***@slack.example/api",
        ],
    );
}

#[test]
fn plain_output_masks_sk_live_username_in_urls() {
    let message = format!(
        "charge https://{SK_LIVE_TOKEN}:x@api.stripe.com/v1 and https://{SK_LIVE_TOKEN}@api.stripe.com/v1"
    );
    assert_plain_masks_token(
        &message,
        SK_LIVE_TOKEN,
        &[
            "https://***:***@api.stripe.com/v1",
            "https://***@api.stripe.com/v1",
        ],
    );
}

#[test]
fn plain_output_masks_akia_username_in_urls() {
    let message = format!(
        "s3 https://{AKIA}:x@s3.amazonaws.com/bucket and https://{AKIA}@s3.amazonaws.com/bucket"
    );
    assert_plain_masks_token(
        &message,
        AKIA,
        &[
            "https://***:***@s3.amazonaws.com/bucket",
            "https://***@s3.amazonaws.com/bucket",
        ],
    );
}

#[test]
fn plain_output_masks_jwt_username_in_urls() {
    let message = format!(
        "auth https://{JWT_TOKEN}:x@auth.example/v1 and https://{JWT_TOKEN}@auth.example/v1"
    );
    assert_plain_masks_token(
        &message,
        JWT_TOKEN,
        &[
            "https://***:***@auth.example/v1",
            "https://***@auth.example/v1",
        ],
    );
}

#[test]
fn plain_output_masks_git_pat_over_https() {
    // Letterlijke vorm uit de B4-bevinding: PAT als gebruikersnaam, vast woord
    // als wachtwoord, zoals git die in HTTPS-fouten zet.
    let message =
        format!("git fetch https://{GHP_PAT}:x-oauth-basic@github.com/org/repo.git failed");
    assert_plain_masks_token(
        &message,
        GHP_PAT,
        &["git fetch https://***:***@github.com/org/repo.git failed"],
    );
    let as_password =
        format!("git fetch https://x-access-token:{GHP_PAT}@github.com/org/repo.git failed");
    assert_plain_masks_token(
        &as_password,
        GHP_PAT,
        &["https://x-access-token:***@github.com/org/repo.git"],
    );
}

#[test]
fn plain_output_leaves_empty_urls_unchanged() {
    let message = "see https:// and http:// and https://@host and https://:@host and https://user:@host/x end";
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_body(dir.path(), &monolog_line(message));
    let stdout = plain_stdout(&path, false);
    assert!(stdout.contains(message), "{stdout}");
    assert!(!stdout.contains("***"), "{stdout}");
    let shown = plain_stdout(&path, true);
    assert!(shown.contains(message), "{shown}");
    assert!(!shown.contains("***"), "{shown}");
}

#[test]
fn plain_output_masks_oversized_url_token_username() {
    let bulk = "B".repeat(1024 * 1024);
    let message = format!("fetch https://{GHP_PAT}:{bulk}@github.com/org/repo.git tail");
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_body(dir.path(), &monolog_line(&message));
    let before = fs::read(&path).expect("lees vooraf");
    let stdout = plain_stdout(&path, false);
    assert!(
        !stdout.contains(GHP_PAT),
        "te grote URL lekt het token (uitvoer {} bytes)",
        stdout.len()
    );
    assert!(
        !stdout.contains(&bulk),
        "te grote URL lekt het wachtwoord (uitvoer {} bytes)",
        stdout.len()
    );
    assert!(
        stdout.contains("https://***:***@github.com/org/repo.git tail"),
        "uitvoer {} bytes, begin: {}",
        stdout.len(),
        stdout.chars().take(180).collect::<String>()
    );
    assert!(
        stdout.len() < 400,
        "te grote URL lekt door ({} bytes)",
        stdout.len()
    );
    assert_eq!(fs::read(&path).expect("lees achteraf"), before);
}

#[test]
fn plain_output_broken_urls_do_not_leak_tokens() {
    let truncated_akia = "AKIAIOSFODNN7EXAMPL";
    let message = format!(
        "a https://{GHP_PAT}:x-oauth-basic b https://{GHP_PAT}:x-oauth-basic@ c http:/{GHP_PAT}@github.com/org/repo.git d ://{AKIA}:x@ e https://[ f {truncated_akia}"
    );
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_body(dir.path(), &monolog_line(&message));
    let stdout = plain_stdout(&path, false);
    assert!(!stdout.contains(GHP_PAT), "{stdout}");
    assert!(!stdout.contains(AKIA), "{stdout}");
    assert!(stdout.contains("https://***:x-oauth-basic "), "{stdout}");
    assert!(stdout.contains("https://***:x-oauth-basic@"), "{stdout}");
    assert!(
        stdout.contains("http:/***@github.com/org/repo.git"),
        "{stdout}"
    );
    assert!(stdout.contains("://***:x@"), "{stdout}");
    assert!(stdout.contains("https://["), "{stdout}");
    assert!(
        stdout.contains(truncated_akia),
        "afgekapte sleutel is geen AKIA: {stdout}"
    );
    let shown = plain_stdout(&path, true);
    assert!(shown.contains(GHP_PAT), "{shown}");
    assert!(shown.contains(AKIA), "{shown}");
    assert!(!shown.contains("***"), "{shown}");
}

#[test]
fn plain_output_does_not_mask_ordinary_git_or_user_names() {
    let message = "clone git@github.com:org/repo.git mail user@example.com ssh ssh://git@github.com/org/repo.git https https://user@github.com/org/repo.git https://git@github.com/org/repo.git pass https://git:s3cret-pass@github.com/org/repo.git https://user:s3cret-pass@github.com/org/repo.git";
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_body(dir.path(), &monolog_line(message));
    let stdout = plain_stdout(&path, false);
    for kept in [
        "git@github.com:org/repo.git",
        "user@example.com",
        "ssh://git@github.com/org/repo.git",
        "https://user@github.com/org/repo.git",
        "https://git@github.com/org/repo.git",
        "https://git:***@github.com/org/repo.git",
        "https://user:***@github.com/org/repo.git",
    ] {
        assert!(stdout.contains(kept), "mist {kept:?} in:\n{stdout}");
    }
    assert!(!stdout.contains("s3cret-pass"), "{stdout}");
    let shown = plain_stdout(&path, true);
    assert!(shown.contains("s3cret-pass"), "{shown}");
    assert!(shown.contains("git@github.com:org/repo.git"), "{shown}");
    assert!(shown.contains("user@example.com"), "{shown}");
    assert!(!shown.contains("***"), "{shown}");
}

#[test]
fn plain_output_masks_github_pat_username() {
    let message = format!(
        "git fetch https://{GITHUB_PAT}:x-oauth-basic@github.com/org/repo.git failed and https://{GITHUB_PAT}@github.com/org/repo.git note {GITHUB_PAT} end https://x-access-token-{GITHUB_PAT}:x@github.com/org/repo.git https://github_pat_short:s3cret@github.com/org/repo.git"
    );
    assert_plain_masks_token(
        &message,
        GITHUB_PAT,
        &[
            "git fetch https://***:***@github.com/org/repo.git failed",
            "https://***@github.com/org/repo.git",
            "note *** end",
            "https://***:***@github.com/org/repo.git",
            "https://github_pat_short:***@github.com/org/repo.git",
        ],
    );
}

#[test]
fn plain_output_masks_prefixed_token_username() {
    let message = format!(
        "git fetch https://x-access-token-{GHP_PAT}:x-oauth-basic@github.com/org/repo.git failed https://svc-{AKIA}:x@s3.amazonaws.com/bucket https://bearer-{JWT_TOKEN}:x@auth.example/v1 https://pre_{GHP_PAT}:x@github.com/org/repo.git https://x-access-token-{GHP_PAT}@github.com/org/repo.git clone git@github.com:org/repo.git mail user@example.com https://git:s3cret-pass@github.com/org/repo.git https://user:s3cret-pass@github.com/org/repo.git https://x-access-token:x-oauth-basic@github.com/org/repo.git"
    );
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_body(dir.path(), &monolog_line(&message));
    let before = fs::read(&path).expect("lees vooraf");
    let stdout = plain_stdout(&path, false);
    for secret in [GHP_PAT, AKIA, JWT_TOKEN] {
        assert!(
            !stdout.contains(secret),
            "plain lekt {secret:?} (uitvoer {} bytes)",
            stdout.len()
        );
    }
    assert!(!stdout.contains("eyJ"), "{stdout}");
    assert!(!stdout.contains("s3cret-pass"), "{stdout}");
    for kept in [
        "git fetch https://***:***@github.com/org/repo.git failed",
        "https://***:***@s3.amazonaws.com/bucket",
        "https://***:***@auth.example/v1",
        "https://***:***@github.com/org/repo.git",
        "https://x-access-token-***@github.com/org/repo.git",
        "git@github.com:org/repo.git",
        "user@example.com",
        "https://git:***@github.com/org/repo.git",
        "https://user:***@github.com/org/repo.git",
        "https://x-access-token:***@github.com/org/repo.git",
    ] {
        assert!(stdout.contains(kept), "mist {kept:?} in:\n{stdout}");
    }
    assert_eq!(fs::read(&path).expect("lees achteraf"), before);

    let shown = plain_stdout(&path, true);
    for secret in [GHP_PAT, AKIA, JWT_TOKEN, "s3cret-pass"] {
        assert!(shown.contains(secret), "--no-redact mist {secret:?}");
    }
    assert!(shown.contains("git@github.com:org/repo.git"), "{shown}");
    assert!(shown.contains("user@example.com"), "{shown}");
    assert!(!shown.contains("***"), "{shown}");
}

#[test]
fn plain_output_empty_and_broken_urls_do_not_leak_github_pat_or_prefixed_tokens() {
    let empty = "see https:// and http:// and :// and https://@host and https://:@host and https://user:@host/x and https://git:@github.com/org/repo.git and https://[ end";
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_body(dir.path(), &monolog_line(empty));
    let stdout = plain_stdout(&path, false);
    assert!(stdout.contains(empty), "{stdout}");
    assert!(!stdout.contains("***"), "{stdout}");
    let shown = plain_stdout(&path, true);
    assert!(shown.contains(empty), "{shown}");
    assert!(!shown.contains("***"), "{shown}");

    let message = format!(
        "a https://{GITHUB_PAT}:x-oauth-basic b https://{GITHUB_PAT}:x-oauth-basic@ c http:/{GITHUB_PAT}@github.com/org/repo.git d https://x-access-token-{GHP_PAT}:x-oauth-basic@ e https://svc-{AKIA}:x@ f https://bearer-{JWT_TOKEN}:x@ g https://["
    );
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_body(dir.path(), &monolog_line(&message));
    let before = fs::read(&path).expect("lees vooraf");
    let stdout = plain_stdout(&path, false);
    for secret in [GITHUB_PAT, GHP_PAT, AKIA, JWT_TOKEN] {
        assert!(
            !stdout.contains(secret),
            "kapotte URL lekt {secret:?}:\n{stdout}"
        );
    }
    assert!(!stdout.contains("eyJ"), "{stdout}");
    for kept in [
        "https://***:x-oauth-basic ",
        "https://***:x-oauth-basic@",
        "http:/***@github.com/org/repo.git",
        "https://x-access-token-***:x-oauth-basic@",
        "https://svc-***:x@",
        "https://bearer-***:x@",
        "https://[",
    ] {
        assert!(stdout.contains(kept), "mist {kept:?} in:\n{stdout}");
    }
    assert_eq!(fs::read(&path).expect("lees achteraf"), before);

    let shown = plain_stdout(&path, true);
    for secret in [GITHUB_PAT, GHP_PAT, AKIA, JWT_TOKEN] {
        assert!(shown.contains(secret), "--no-redact mist {secret:?}");
    }
    assert!(!shown.contains("***"), "{shown}");
}

#[test]
fn plain_output_masks_bearer_after_cr_lf_ff_vt_following_colon() {
    // symlog-crlffix-0001, sluit bevinding 3 uit symlog-crlfcheck-0001:
    // CR, LF, FF en VT direct na de `:` en vóór `Bearer` lieten
    // `match_bearer` de match opgeven, omdat alleen `skip_quotes` (spatie/tab)
    // ná de `:` werd geprobeerd. LF ontbreekt hier bewust: een losse `\n`
    // binnen de monolog-boodschap laat de regelkop-parser het vervolg van de
    // regel (los van deze fix) niet tonen, dus die zit alleen in de
    // `redact::tests`-variant in `src/redact.rs`, rechtstreeks op
    // `redact_secrets`.
    let token = "opaque-token-crlf-0001";
    for gap in ['\r', '\u{c}', '\u{b}'] {
        let message = format!("Authorization:{gap}Bearer {token}");
        assert_plain_masks_token(&message, token, &["Bearer ***"]);
    }
}

#[test]
fn plain_output_masks_url_password_after_cr_ff_vt_or_tab() {
    // symlog-crlffixrev-0001, bevinding 2: FF tussen `:` en het wachtwoord in
    // URL-userinfo liet `userinfo_at` meteen op de leeg-check stranden
    // (ASCII-witruimte voor `is_url_pass_byte`), net als CR en tab eerder
    // (symlog-crlffix-0001). VT is geen ASCII-witruimte en zat hier al nooit
    // vast. LF ontbreekt hier bewust, zelfde reden als bij
    // `plain_output_masks_bearer_after_cr_lf_ff_vt_following_colon` hierboven:
    // een losse `\n` binnen de monolog-boodschap laat de regelkop-parser het
    // vervolg niet tonen. Die zit alleen in de `redact::tests`-variant in
    // `src/redact.rs`, rechtstreeks op `redact_secrets`.
    let pass = "opaque-pass-crlf-0001";
    for gap in ['\r', '\x0C', '\x0B', '\t'] {
        let message = format!("connect https://user:{gap}{pass}@host.example/a done");
        assert_plain_masks_token(&message, pass, &["https://user:***@host.example/a", "done"]);
    }
}
