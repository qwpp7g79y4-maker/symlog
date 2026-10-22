//! Golden-tests voor multiline-records en stacktraces (task.md T4 "Klaar als"): F3
//! (stacktrace in lijnformaat met 40 frames en een previous exception), F4 (exception
//! in JSON) en F12 (vervolgregel die toevallig met `[` begint, geen valse
//! recordstart).

use std::fs;
use std::path::{Path, PathBuf};
use symlog::model::Frame;

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

/// Splitst een LineFormatter-fixture in recordblokken (task.md T4) en parset elk blok,
/// zoals de echte reader dat (streaming op bytes) ook zou doen.
fn parse_line_fixture(name: &str) -> serde_json::Value {
    let content = fs::read_to_string(fixture_path(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    let records: Vec<serde_json::Value> = symlog::parse::line::split_records(&content)
        .into_iter()
        .map(|block| {
            symlog::parse::line::parse_line(block)
                .unwrap_or_else(|| panic!("blok parseert niet als standaard lineformat: {block:?}"))
                .to_json()
        })
        .collect();
    serde_json::Value::Array(records)
}

fn assert_matches_golden(actual: &serde_json::Value, golden: &str) {
    let golden_path = golden_path(golden);
    let expected_text =
        fs::read_to_string(&golden_path).unwrap_or_else(|e| panic!("{golden_path:?}: {e}"));
    let expected: serde_json::Value =
        serde_json::from_str(&expected_text).unwrap_or_else(|e| panic!("{golden_path:?}: {e}"));
    assert_eq!(
        actual,
        &expected,
        "komt niet overeen met {golden}:\n{}",
        serde_json::to_string_pretty(actual).unwrap()
    );
}

#[test]
fn f3_line_stacktrace_matches_golden() {
    let actual = parse_line_fixture("line-stacktrace.log");
    assert_matches_golden(&actual, "line_stacktrace.json");

    let records = actual.as_array().expect("array");
    assert_eq!(records.len(), 1, "F3 is één record (één exception-melding)");
    let frames = records[0]["frames"].as_array().expect("frames array");
    assert_eq!(
        frames.len(),
        40,
        "task.md T4 'Klaar als': 40 frames, inclusief previous exception"
    );
}

#[test]
fn f12_tricky_heads_matches_golden() {
    let actual = parse_line_fixture("tricky-heads.log");
    assert_matches_golden(&actual, "tricky_heads.json");

    let records = actual.as_array().expect("array");
    assert_eq!(
        records.len(),
        2,
        "de '[previous exception]'-regel mag geen vals nieuw record openen"
    );
}

#[test]
fn f4_json_exception_frames_match_golden() {
    let content = fs::read_to_string(fixture_path("json-exception.ndjson")).expect("fixture");
    let value: serde_json::Value = serde_json::from_str(content.trim_end()).expect("geldige JSON");
    let exception = &value["context"]["exception"];

    let frames = symlog::parse::trace::frames_from_exception_json(exception);
    let actual: Vec<serde_json::Value> = frames.iter().map(Frame::to_json).collect();

    assert_matches_golden(
        &serde_json::Value::Array(actual),
        "json_exception_frames.json",
    );
}
