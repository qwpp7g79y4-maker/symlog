//! Golden-tests voor de Raw-terugval (task.md T6 "Klaar als"): F7 (kapotte regels en
//! halve JSON), F8 (ongeldige UTF-8-bytes, Latin-1-regel) en F15 (ANSI-escape en NUL
//! in message) tonen zonder paniek, met golden plain-uitvoer.

use std::fs;
use std::path::{Path, PathBuf};
use symlog::parse::parse_record_lossy;
use symlog::parse::raw::RawReason;

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

fn assert_matches_golden_plain(actual: &str, golden: &str) {
    let golden_path = golden_path(golden);
    let expected_text =
        fs::read_to_string(&golden_path).unwrap_or_else(|e| panic!("{golden_path:?}: {e}"));
    assert_eq!(
        actual,
        expected_text.trim_end_matches('\n'),
        "komt niet overeen met {golden}:\n{actual}"
    );
}

#[test]
fn f7_broken_lines_become_raw_without_panicking() {
    let content = fs::read_to_string(fixture_path("broken.log")).expect("fixture");
    let parsed: Vec<_> = content
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| parse_record_lossy(line.as_bytes()))
        .collect();
    assert_eq!(parsed.len(), 5, "F7 bevat 5 kapotte-regel-situaties");
    assert!(
        parsed.iter().all(|p| p.as_raw().is_some()),
        "elke regel in F7 moet Raw worden, geen enkele mag per ongeluk parsen"
    );

    let reasons: Vec<RawReason> = parsed.iter().map(|p| p.as_raw().unwrap().reason).collect();
    assert_eq!(
        reasons,
        vec![
            RawReason::UnknownFormat, // regel zonder enige kop
            RawReason::UnknownFormat, // lijnformaat-kop, maar afgekapte trailing-JSON
            RawReason::InvalidJson,   // halve JSON (geen sluit-haak)
            RawReason::InvalidJson,   // enkele quotes in plaats van dubbele
            RawReason::UnknownFormat, // kop met een ongeldige datetime
        ]
    );

    let plain: String = parsed
        .iter()
        .map(|p| p.to_plain_line())
        .collect::<Vec<_>>()
        .join("\n");
    assert_matches_golden_plain(&plain, "raw_broken.txt");
}

#[test]
fn f8_bad_utf8_shows_replacement_char_without_panicking() {
    let bytes = fs::read(fixture_path("bad-utf8.log")).expect("fixture");
    let lines: Vec<&[u8]> = bytes
        .split(|&b| b == b'\n')
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(lines.len(), 3, "F8 bevat 3 regels");
    let parsed: Vec<_> = lines.iter().map(|line| parse_record_lossy(line)).collect();

    // De eerste twee regels hebben een geldige lijnformaat-kop en parsen normaal,
    // ondanks de ongeldige bytes in het bericht (die worden U+FFFD, geen paniek).
    assert!(
        parsed[0].as_raw().is_none(),
        "regel met één ongeldige byte moet nog steeds als lijnformaat parsen"
    );
    assert!(
        parsed[1].as_raw().is_none(),
        "regel met twee ongeldige bytes moet nog steeds als lijnformaat parsen"
    );
    // De derde regel (de Latin-1-regel zonder kop) heeft geen geldig recordformaat.
    assert!(parsed[2].as_raw().is_some());

    let plain: String = parsed
        .iter()
        .map(|p| p.to_plain_line())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        plain.contains('\u{fffd}'),
        "de ongeldige bytes moeten als U+FFFD zichtbaar zijn"
    );
    assert_matches_golden_plain(&plain, "raw_bad_utf8.txt");
}

#[test]
fn f15_escapes_and_nul_are_neutralised_without_panicking() {
    let content = fs::read_to_string(fixture_path("escapes.log")).expect("fixture");
    let parsed: Vec<_> = content
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| parse_record_lossy(line.as_bytes()))
        .collect();
    assert_eq!(parsed.len(), 3, "F15 bevat 3 records");
    assert!(
        parsed.iter().all(|p| p.as_raw().is_none()),
        "alle drie parsen normaal, ondanks de besturingstekens in het bericht"
    );

    let plain: String = parsed
        .iter()
        .map(|p| p.to_plain_line())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !plain.contains('\u{1b}'),
        "geen kaal ESC-byte in de weergave (geen terminal-injectie): {plain:?}"
    );
    assert!(
        !plain.contains('\u{0}'),
        "geen kaal NUL-byte in de weergave: {plain:?}"
    );
    assert!(
        !plain.contains('\r'),
        "geen kale losse CR in de weergave (kan de terminalcursor terugzetten): {plain:?}"
    );
    assert_matches_golden_plain(&plain, "raw_escapes.txt");
}
