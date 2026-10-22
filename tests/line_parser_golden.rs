//! Golden-tests voor de LineFormatter-parser (task.md T3 "Klaar als"): F1, F2 en F11
//! parsen naar `tests/golden/line_*.json`.

use std::fs;
use std::path::Path;

fn parse_fixture(name: &str) -> serde_json::Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let content = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));

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

fn assert_matches_golden(fixture: &str, golden: &str) {
    let actual = parse_fixture(fixture);
    let golden_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(golden);
    let expected_text =
        fs::read_to_string(&golden_path).unwrap_or_else(|e| panic!("{golden_path:?}: {e}"));
    let expected: serde_json::Value =
        serde_json::from_str(&expected_text).unwrap_or_else(|e| panic!("{golden_path:?}: {e}"));

    assert_eq!(
        actual,
        expected,
        "geparste records voor {fixture} komen niet overeen met {golden}:\n{}",
        serde_json::to_string_pretty(&actual).unwrap()
    );
}

#[test]
fn f1_line_basic_matches_golden() {
    assert_matches_golden("line-basic.log", "line_basic.json");
}

#[test]
fn f2_line_context_matches_golden() {
    assert_matches_golden("line-context.log", "line_context.json");
}

#[test]
fn f11_monolog1_matches_golden() {
    assert_matches_golden("monolog1.log", "line_monolog1.json");
}
