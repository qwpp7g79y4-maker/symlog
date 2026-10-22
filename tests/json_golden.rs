//! Golden-tests voor de JsonFormatter-parser en de format-dispatch (task.md T5
//! "Klaar als"): F5 (zuiver NDJSON) en F6 (gemengd, lijn- en JSON-records door elkaar)
//! parsen naar `tests/golden/json_*.json`; `--format line` op F5 toont alles als
//! onbekend formaat zonder paniek.

use std::fs;
use std::path::{Path, PathBuf};

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

/// Elke regel van een zuiver NDJSON-bestand is zijn eigen record (task.md T5
/// "Blijft": JSON-records lopen nooit over meerdere regels in v1).
fn parse_ndjson_fixture(name: &str) -> Vec<String> {
    fs::read_to_string(fixture_path(name))
        .unwrap_or_else(|e| panic!("{name}: {e}"))
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

#[test]
fn f5_json_basic_matches_golden() {
    let lines = parse_ndjson_fixture("json-basic.ndjson");
    assert_eq!(lines.len(), 7, "F5 bevat 7 records");

    let records: Vec<serde_json::Value> = lines
        .iter()
        .map(|line| {
            symlog::parse::json::parse_json(line)
                .unwrap_or_else(|| panic!("regel parseert niet als JsonFormatter: {line:?}"))
                .to_json()
        })
        .collect();
    assert_matches_golden(&serde_json::Value::Array(records), "json_basic.json");
}

#[test]
fn f5_json_basic_via_dispatch_matches_direct_parse() {
    // task.md T5 "Doet": het formaat wordt per record bepaald via
    // `symlog::parse::parse_record`, niet via een apart per-bestand pad; dat moet
    // hetzelfde resultaat geven als rechtstreeks `json::parse_json` aanroepen.
    let lines = parse_ndjson_fixture("json-basic.ndjson");
    for line in &lines {
        let direct = symlog::parse::json::parse_json(line).expect("direct parseert");
        let dispatched = symlog::parse::parse_record(line).expect("dispatch parseert");
        assert_eq!(direct.to_json(), dispatched.to_json());
    }
}

#[test]
fn f5_json_basic_as_forced_line_format_is_unknown_without_panicking() {
    // task.md T5 "Klaar als": `--format line` op F5 toont alles als onbekend formaat
    // zonder crash. Op de parse-laag (de CLI-flag zelf komt in een latere taak) is dat
    // `parse_record_as(.., Format::Line)`.
    let lines = parse_ndjson_fixture("json-basic.ndjson");
    for line in &lines {
        assert!(
            symlog::parse::parse_record_as(line, symlog::parse::Format::Line).is_none(),
            "JSON-regel mag niet als lineformat parsen: {line:?}"
        );
    }
}

#[test]
fn f6_mixed_matches_golden() {
    let content = fs::read_to_string(fixture_path("mixed.log")).expect("fixture");
    let blocks = symlog::parse::split_records(&content);
    assert_eq!(blocks.len(), 8, "F6 bevat 8 records, 50/50 lijn en JSON");

    let records: Vec<serde_json::Value> = blocks
        .iter()
        .map(|block| {
            symlog::parse::parse_record(block)
                .unwrap_or_else(|| panic!("blok parseert in geen van beide formaten: {block:?}"))
                .to_json()
        })
        .collect();
    assert_matches_golden(&serde_json::Value::Array(records), "mixed.json");
}

#[test]
fn f6_mixed_records_alternate_format_per_record_not_per_file() {
    // task.md T5 "Doet": het formaat wordt per record bepaald, niet per bestand.
    let content = fs::read_to_string(fixture_path("mixed.log")).expect("fixture");
    let blocks = symlog::parse::split_records(&content);
    let formats: Vec<symlog::parse::Format> = blocks
        .iter()
        .map(|block| symlog::parse::detect(block).expect("elk blok heeft een herkenbare kop"))
        .collect();
    assert_eq!(
        formats,
        vec![
            symlog::parse::Format::Line,
            symlog::parse::Format::Json,
            symlog::parse::Format::Line,
            symlog::parse::Format::Json,
            symlog::parse::Format::Line,
            symlog::parse::Format::Json,
            symlog::parse::Format::Line,
            symlog::parse::Format::Json,
        ],
        "F6 wisselt per record van formaat (50/50 door elkaar)"
    );
}
