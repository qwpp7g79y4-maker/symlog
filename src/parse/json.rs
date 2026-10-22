//! Monolog JsonFormatter-parser (NDJSON) (task.md T5; blueprint.md `parse::json`).
//!
//! Elke regel is een los JSON-object: `message`, `channel`, `datetime` (ruwe tekst,
//! net als [`crate::parse::line`]; T7 rekent dit om naar een echte tijd) en een
//! niveau als `level` (Monolog/PSR-3-integer, zie [`crate::model::Level::from_monolog_int`])
//! en/of `level_name` (zelfde tekstvorm en hoofdletterongevoeligheid als de
//! LineFormatter-parser). Is `level_name` aanwezig en herkend, dan is dat het niveau;
//! anders valt [`parse_json`] terug op `level`. `context`/`extra` ontbreken ze, dan is
//! dat gelijk aan een lege PHP-array (`[]`), dezelfde conventie als een lege
//! `%context%`/`%extra%` in het lijnformaat (task.md T3). Frames komen uit
//! `context.exception` via [`crate::parse::trace::frames_from_exception_json`],
//! dezelfde functie als de JSON-exceptiekant van T4.
//!
//! Geen `serde::Deserialize`-afleiding: net als [`crate::model`] al toelicht zou dat
//! de `derive`-feature van `serde` vereisen (`proc-macro2`/`quote`/`syn`/
//! `unicode-ident`, met een `Unicode-3.0`-licentieclausule die niet in `deny.toml`
//! staat, en die blijft staan zoals in T1 — zie `docs/crates.md`). [`parse_json`]
//! haalt velden daarom met `serde_json::Value::get`/`as_str`/`as_i64` uit een eenmalig
//! geparste `Value`; dat is de "borrowed strings waar mogelijk" uit task.md T5: elk
//! veld wordt pas naar een eigen `String` gekopieerd zodra het record geldig blijkt,
//! niet eerder.
//!
//! Een regel die geen geldig JSON-object is, of een vereist veld mist (`message`,
//! `channel`, `datetime`, en minstens één bruikbaar niveau), levert `None` op: zoals
//! bij [`crate::parse::line::parse_line`] is dat "onbekend formaat", geen fout; de
//! `Raw`-terugval zelf komt in T6.
//!
//! JSON-records zijn altijd één fysieke regel (task.md T5 "Blijft": pretty-printed,
//! meerregelige JSON wordt in v1 niet samengevoegd). [`is_record_header`] parset
//! daarom een regel in zijn geheel; een pretty-printed record waarvan alleen de
//! openende `{` op de eerste regel staat, heeft geen `"datetime"`-veld op die regel
//! en wordt dus nooit als recordstart herkend (zie `tests/json_golden.rs`).

use crate::model::{Level, Record};
use crate::parse::trace;
use serde_json::Value;

/// Parseert één NDJSON-regel in het Monolog JsonFormatter-formaat (task.md T5).
/// `None` als `line` geen geldig JSON-object is, of een verplicht veld mist of van
/// het verkeerde type is.
pub fn parse_json(line: &str) -> Option<Record> {
    let line = strip_line_ending(line);
    let value: Value = serde_json::from_str(line).ok()?;
    let obj = value.as_object()?;

    let message = obj.get("message")?.as_str()?.to_string();
    let channel = obj.get("channel")?.as_str()?.to_string();
    let datetime = obj.get("datetime")?.as_str()?.to_string();
    let level = level_from_fields(obj)?;

    let context = obj
        .get("context")
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]));
    let extra = obj
        .get("extra")
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]));

    let frames = context
        .get("exception")
        .map(trace::frames_from_exception_json)
        .unwrap_or_default();

    Some(Record {
        datetime,
        channel,
        level,
        message,
        context,
        extra,
        frames,
    })
}

/// Niveau uit `level_name` (tekst, hoofdletterongevoelig) als die aanwezig en herkend
/// is; anders uit `level` (Monolog-integer). `None` als geen van beide een bruikbaar
/// niveau oplevert.
fn level_from_fields(obj: &serde_json::Map<String, Value>) -> Option<Level> {
    if let Some(name) = obj.get("level_name").and_then(|v| v.as_str()) {
        if let Some(level) = Level::parse(name) {
            return Some(level);
        }
    }
    let n = obj.get("level")?.as_i64()?;
    Level::from_monolog_int(n)
}

/// Format-agnostische recordkop-check voor de achtergrondindexer
/// ([`crate::reader::index`], via [`crate::parse::is_record_header`]): is `line`
/// (bytes van één fysieke regel, zonder regeleinde) het begin van een JSON-record?
/// Task.md T4/T5: "`{` met een `"datetime"`-veld voor JSON". Parset de regel in zijn
/// geheel als JSON in plaats van met een tekstpatroon te matchen, want objectvelden
/// kunnen in elke volgorde staan; werkt op bytes, niet `&str`, omdat de indexer ook
/// aangeroepen kan worden met tekst die (nog) niet geldige UTF-8 is (dan faalt de
/// JSON-parse gewoon en levert dit `false` op, geen paniek).
pub(crate) fn is_record_header(line: &[u8]) -> bool {
    if !line.starts_with(b"{") {
        return false;
    }
    match serde_json::from_slice::<Value>(line) {
        Ok(Value::Object(map)) => map.contains_key("datetime"),
        _ => false,
    }
}

fn strip_line_ending(line: &str) -> &str {
    let line = line.strip_suffix('\n').unwrap_or(line);
    line.strip_suffix('\r').unwrap_or(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_level_name_and_int_together() {
        let record = parse_json(
            r#"{"message":"boot","context":[],"level":200,"level_name":"INFO","channel":"app","datetime":"2026-01-15T10:00:00.000000+01:00","extra":[]}"#,
        )
        .expect("parses");
        assert_eq!(record.level, Level::Info);
        assert_eq!(record.channel, "app");
        assert_eq!(record.message, "boot");
    }

    #[test]
    fn parses_level_int_without_level_name() {
        let record = parse_json(
            r#"{"message":"boot","level":500,"channel":"app","datetime":"2026-01-15T10:00:01.000000+01:00"}"#,
        )
        .expect("parses");
        assert_eq!(record.level, Level::Critical);
    }

    #[test]
    fn parses_level_name_without_level_int() {
        let record = parse_json(
            r#"{"message":"boot","level_name":"warning","channel":"app","datetime":"2026-01-15T10:00:02.000000+01:00"}"#,
        )
        .expect("parses");
        assert_eq!(record.level, Level::Warning);
    }

    #[test]
    fn missing_context_and_extra_default_to_empty_array() {
        let record = parse_json(
            r#"{"message":"boot","level":100,"channel":"app","datetime":"2026-01-15T10:00:03.000000+01:00"}"#,
        )
        .expect("parses");
        assert_eq!(record.context, json!([]));
        assert_eq!(record.extra, json!([]));
    }

    #[test]
    fn extracts_frames_from_context_exception() {
        let record = parse_json(
            r##"{"message":"boom","level":500,"channel":"app","datetime":"2026-01-15T10:00:04.000000+01:00","context":{"exception":{"file":"/a.php:10","trace":["#0 {main}"]}}}"##,
        )
        .expect("parses");
        assert_eq!(record.frames.len(), 2);
        assert_eq!(record.frames[0].line, Some(10));
        assert_eq!(record.frames[1].func, "{main}");
    }

    #[test]
    fn rejects_unknown_level_int() {
        assert!(parse_json(
            r#"{"message":"boot","level":999,"channel":"app","datetime":"2026-01-15T10:00:05.000000+01:00"}"#
        )
        .is_none());
    }

    #[test]
    fn rejects_unrecognised_level_name_without_int_fallback() {
        assert!(parse_json(
            r#"{"message":"boot","level_name":"TRACE","channel":"app","datetime":"2026-01-15T10:00:06.000000+01:00"}"#
        )
        .is_none());
    }

    #[test]
    fn falls_back_to_level_int_when_level_name_is_unrecognised() {
        // level_name wint alleen als het zelf herkend wordt; anders mag een geldige
        // level-int er nog steeds voor zorgen dat het record parseert.
        let record = parse_json(
            r#"{"message":"boot","level":400,"level_name":"nonsense","channel":"app","datetime":"2026-01-15T10:00:07.000000+01:00"}"#,
        )
        .expect("parses via level-int fallback");
        assert_eq!(record.level, Level::Error);
    }

    #[test]
    fn rejects_missing_message() {
        assert!(parse_json(
            r#"{"level":100,"channel":"app","datetime":"2026-01-15T10:00:08.000000+01:00"}"#
        )
        .is_none());
    }

    #[test]
    fn rejects_missing_channel() {
        assert!(parse_json(
            r#"{"message":"boot","level":100,"datetime":"2026-01-15T10:00:09.000000+01:00"}"#
        )
        .is_none());
    }

    #[test]
    fn rejects_missing_datetime() {
        assert!(parse_json(r#"{"message":"boot","level":100,"channel":"app"}"#).is_none());
    }

    #[test]
    fn rejects_non_object_json() {
        assert!(parse_json("[1, 2, 3]").is_none());
        assert!(parse_json(r#""just a string""#).is_none());
    }

    #[test]
    fn rejects_invalid_json_without_panicking() {
        assert!(parse_json("{not json at all").is_none());
        assert!(parse_json("").is_none());
    }

    #[test]
    fn strips_trailing_newline_and_crlf() {
        let lf = parse_json(
            "{\"message\":\"m\",\"level\":100,\"channel\":\"app\",\"datetime\":\"2026-01-15T10:00:10.000000+01:00\"}\n",
        )
        .expect("parses lf");
        let crlf = parse_json(
            "{\"message\":\"m\",\"level\":100,\"channel\":\"app\",\"datetime\":\"2026-01-15T10:00:11.000000+01:00\"}\r\n",
        )
        .expect("parses crlf");
        assert_eq!(lf.message, "m");
        assert_eq!(crlf.message, "m");
    }

    #[test]
    fn is_record_header_requires_object_with_datetime_field() {
        assert!(is_record_header(
            br#"{"message":"m","datetime":"2026-01-15T10:00:00.000000+01:00"}"#
        ));
        // Alleen een openende `{` (het begin van een pretty-printed record): geen
        // geldige JSON op deze regel alleen, dus geen recordstart (task.md T5
        // "Blijft").
        assert!(!is_record_header(b"{"));
        // Geldig JSON-object, maar zonder `"datetime"`-veld: geen recordkop.
        assert!(!is_record_header(br#"{"message":"m"}"#));
        // Een array is geen record, ook al balanceert hij qua haakjes.
        assert!(!is_record_header(b"[1,2,3]"));
        // Ongeldige UTF-8/JSON mag nooit paniek geven.
        assert!(!is_record_header(b"{\xff\xfe"));
    }
}
