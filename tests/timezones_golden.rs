//! Golden-test voor tijdzone-weergave (task.md T7 "Klaar als"): F9 toont golden tijden
//! in `log`, `local` (met `TZ=Europe/Amsterdam`) en `America/New_York`, over de
//! DST-wissel van 25 okt 2026 (Europa) heen. F9 bevat ook een Monolog 1-record zonder
//! offset: dat moet in elke modus hetzelfde "zonder zone"-resultaat geven (task.md T7
//! "Doet": "niet stil als UTC aangenomen").
//!
//! Zet `TZ` alleen in dit losse testbestand (een eigen proces per `tests/*.rs`-bestand,
//! zie `cargo test`-documentatie): zo kan geen andere test in een ander testbestand of
//! in de lib-unittests hier last van hebben. Binnen dit bestand staat alles in één
//! `#[test]`-functie, zodat er ook geen race is met een parallelle test die `TZ` zelf
//! niet zet.

use std::fs;
use std::path::Path;
use symlog::parse::line::{parse_line, split_records};
use symlog::time::{display, TzMode};

#[test]
fn f9_timezones_match_golden_in_log_local_and_named_modes() {
    let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/timezones.log");
    let content = fs::read_to_string(&fixture_path).expect("F9-fixture");
    let records: Vec<_> = split_records(&content)
        .into_iter()
        .map(|block| parse_line(block).expect("F9 parseert altijd als lineformat"))
        .collect();
    assert_eq!(records.len(), 4);

    let golden_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/timezones.json");
    let golden_text = fs::read_to_string(&golden_path).expect("golden-bestand");
    let golden: serde_json::Value =
        serde_json::from_str(&golden_text).expect("golden-bestand is geldig JSON");

    let render = |mode: &TzMode| -> Vec<String> {
        records
            .iter()
            .map(|r| {
                let ts =
                    r.ts.as_ref()
                        .unwrap_or_else(|| panic!("elk F9-record moet een ts hebben: {r:?}"));
                display(ts, mode).unwrap_or_else(|e| panic!("display faalt: {e}"))
            })
            .collect()
    };

    // "log": de offset blijft precies zoals die in de log stond, ongeacht de
    // omgeving waarin de test draait.
    let log_actual = render(&TzMode::Log);
    let log_expected: Vec<String> = golden["log"]
        .as_array()
        .expect("golden.log is een array")
        .iter()
        .map(|v| {
            v.as_str()
                .expect("golden.log-elementen zijn strings")
                .to_string()
        })
        .collect();
    assert_eq!(
        log_actual, log_expected,
        "tz=log komt niet overeen met golden"
    );

    // "local": TZ=Europe/Amsterdam bepaalt de systeemtijdzone voor deze ene test.
    // SAFETY: deze test is de enige in dit proces die `TZ` zet (zie de moduledoc
    // hierboven); er is geen gelijktijdige lezer/schrijver van de omgeving.
    unsafe {
        std::env::set_var("TZ", "Europe/Amsterdam");
    }
    let local_actual = render(&TzMode::Local);
    unsafe {
        std::env::remove_var("TZ");
    }
    let local_expected: Vec<String> = golden["local"]
        .as_array()
        .expect("golden.local is een array")
        .iter()
        .map(|v| {
            v.as_str()
                .expect("golden.local-elementen zijn strings")
                .to_string()
        })
        .collect();
    assert_eq!(
        local_actual, local_expected,
        "tz=local (Europe/Amsterdam) komt niet overeen met golden"
    );

    // "America/New_York": een expliciete IANA-naam, geen afhankelijkheid van `TZ`.
    let ny_mode = TzMode::Named("America/New_York".to_string());
    let ny_actual = render(&ny_mode);
    let ny_expected: Vec<String> = golden["america_new_york"]
        .as_array()
        .expect("golden.america_new_york is een array")
        .iter()
        .map(|v| {
            v.as_str()
                .expect("golden.america_new_york-elementen zijn strings")
                .to_string()
        })
        .collect();
    assert_eq!(
        ny_actual, ny_expected,
        "tz=America/New_York komt niet overeen met golden"
    );
}
