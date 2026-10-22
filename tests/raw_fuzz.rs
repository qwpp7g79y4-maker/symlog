//! Fuzz/property-test op de parser (task.md T6 "Klaar als": "een fuzz/property-test
//! (proptest, 10 000 cases) op de parser vindt geen panic"). Drie generators worden
//! gemengd: volledig willekeurige bytes (ook ongeldige UTF-8), en een geldig
//! lijnformaat- resp. JsonFormatter-record dat op een willekeurige positie wordt
//! afgekapt (precies de "afgekapte regels"/"halve JSON" uit task.md T6, maar dan
//! systematisch over elke afkappositie in plaats van een paar handgekozen fixtures).

use proptest::prelude::*;
use symlog::parse::parse_record_lossy;

const LINE_TEMPLATE: &str = "[2026-01-15T10:30:45.123456+01:00] app.ERROR: boom {\"n\":1,\"tags\":[\"a\",\"b\"]} [\"extra\"]";
const JSON_TEMPLATE: &str = r##"{"message":"boom","level":500,"channel":"app","datetime":"2026-01-15T10:30:45.123456+01:00","context":{"exception":{"file":"/a.php:10","trace":["#0 {main}"]}}}"##;

fn arbitrary_bytes() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(any::<u8>(), 0..400)
}

fn truncated(template: &'static str) -> impl Strategy<Value = Vec<u8>> {
    (0..=template.len()).prop_map(move |cut| template.as_bytes()[..cut].to_vec())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 10_000, .. ProptestConfig::default() })]

    #[test]
    fn parse_record_lossy_never_panics(
        bytes in prop_oneof![
            arbitrary_bytes(),
            truncated(LINE_TEMPLATE),
            truncated(JSON_TEMPLATE),
        ]
    ) {
        let parsed = parse_record_lossy(&bytes);
        // De weergavepaden (task.md T6 "Doet": neutraliseren vóór weergave) moeten
        // ook zonder paniek werken, ongeacht wat er binnenkomt.
        let _ = parsed.to_plain_line();
        let _ = parsed.to_json();
    }
}
