//! Monolog LineFormatter-parser (task.md T3/T4; blueprint.md `parse::line`).
//!
//! Standaardformaat: `[%datetime%] %channel%.%level_name%: %message% %context%
//! %extra%`. `context` en `extra` zijn JSON (`{...}` of `[]`) aan het eind van het
//! record; de grens met `message` wordt van achteren gevonden via een gebalanceerde
//! JSON-scan ([`rfind_json_blob`]), zodat een `{` of `]` in de message zelf de
//! grensbepaling niet verstoort: de scan stopt zodra de buitenste haak van het
//! *laatste* blok sluit en gaat niet verder naar links.
//!
//! Een record kan meerdere fysieke regels beslaan (task.md T4): `%message%` is een
//! kale stringvervanging in Monolog (niet JSON-ge-escaped), dus een bericht dat zelf
//! `\n` bevat (bijvoorbeeld PHP's eigen `Stack trace:`-tekst bij een ongevangen
//! exception) komt ongewijzigd in de log terecht. [`parse_line`] accepteert daarom een
//! heel recordblok (header-regel plus eventuele vervolgregels); de grensbepaling van
//! `%context%`/`%extra%` werkt ongewijzigd, want [`rfind_json_blob`] scant simpelweg
//! vanaf het einde van de tekst en behandelt `\n` als een gewoon teken. [`split_records`]
//! splitst de inhoud van een heel bestand in zulke blokken; de echte reader
//! ([`crate::reader::index`]) doet hetzelfde maar dan streaming op bytes.
//!
//! Een regel die niet aan dit standaardformaat voldoet (bijvoorbeeld een aangepaste
//! LineFormatter-string) levert `None` op. Task.md T3 "Blijft": zulke regels vallen
//! terug op "onbekend formaat"; die terugval zelf (een `Raw`-record) komt in T6.

use crate::model::{Level, Record};
use crate::parse::trace;

/// Parseert één logrecord (header-regel plus eventuele vervolgregels, zie de
/// moduledoc) in het Monolog LineFormatter-standaardformaat (Monolog 1–3). `None` als
/// de header niet aan dat formaat voldoet.
pub fn parse_line(block: &str) -> Option<Record> {
    let block = strip_line_ending(block);

    let after_open = block.strip_prefix('[')?;
    let close = after_open.find(']')?;
    let datetime = &after_open[..close];
    if !looks_like_datetime(datetime.as_bytes()) {
        return None;
    }

    let after_close = after_open[close + 1..].strip_prefix(' ')?;

    let colon = after_close.find(':')?;
    let header = &after_close[..colon];
    let (channel, level_name) = header.split_once('.')?;
    if channel.is_empty() || !channel.bytes().all(is_channel_byte) {
        return None;
    }
    let level = Level::parse(level_name)?;

    let remainder = after_close[colon + 1..].strip_prefix(' ')?;

    let (before_extra, extra) = take_trailing_json(remainder)?;
    let before_extra = before_extra.strip_suffix(' ')?;
    let (before_context, context) = take_trailing_json(before_extra)?;
    let message = before_context.strip_suffix(' ')?;

    let frames = trace::frames_from_message(message);

    Some(Record {
        datetime: datetime.to_string(),
        channel: channel.to_string(),
        level,
        message: message.to_string(),
        context,
        extra,
        frames,
    })
}

/// Splitst de volledige inhoud van een LineFormatter-logbestand in afzonderlijke
/// recordblokken (task.md T4): elke regel die met een geldige recordkop begint, opent
/// een nieuw blok; andere regels worden aan het lopende blok toegevoegd (een
/// stacktrace-vervolgregel, bijvoorbeeld). De eerste regel van `content` opent altijd
/// het eerste blok, ongeacht of die een geldige kop heeft (vergelijk de
/// achtergrondindexer, [`crate::reader::index`], die offset 0 ook altijd als
/// recordstart publiceert).
pub fn split_records(content: &str) -> Vec<&str> {
    let mut starts = Vec::new();
    let mut offset = 0usize;
    for (i, raw_line) in content.split_inclusive('\n').enumerate() {
        let line = strip_line_ending(raw_line);
        if i == 0 || is_record_header(line.as_bytes()) {
            starts.push(offset);
        }
        offset += raw_line.len();
    }

    let mut blocks: Vec<&str> = starts.windows(2).map(|w| &content[w[0]..w[1]]).collect();
    if let Some(&last) = starts.last() {
        blocks.push(&content[last..]);
    }
    blocks
}

/// Of `line` (zonder regeleinde) een geldige LineFormatter-recordkop is: `[` gevolgd
/// door een herkende datetime en een sluitende `]`. Alleen het datetime-patroon wordt
/// strikt gecontroleerd, niet channel/level/colon (blueprint.md: "Om valse starts te
/// vermijden checkt de indexer de kop strikt (datetime-patroon), niet alleen `[`").
/// Gedeeld door [`split_records`] en de achtergrondindexer
/// ([`crate::reader::index::build_index`]), zodat beide dezelfde regel toepassen voor
/// "dit is het begin van een nieuw record". Werkt op bytes (niet `&str`): de
/// achtergrondindexer kan dit ook aanroepen met tekst die (nog) niet geldige UTF-8 is.
pub(crate) fn is_record_header(line: &[u8]) -> bool {
    let Some(rest) = line.strip_prefix(b"[") else {
        return false;
    };
    let Some(close) = memchr::memchr(b']', rest) else {
        return false;
    };
    looks_like_datetime(&rest[..close])
}

fn strip_line_ending(line: &str) -> &str {
    let line = line.strip_suffix('\n').unwrap_or(line);
    line.strip_suffix('\r').unwrap_or(line)
}

fn is_channel_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// Herkent de drie datetime-varianten uit task.md T3:
/// - Monolog 2/3 default `Y-m-d\TH:i:s.uP` (microseconden optioneel, offset als
///   `+HH:MM`/`-HH:MM` of `Z`);
/// - dezelfde vorm zonder microseconden;
/// - Monolog 1 `Y-m-d H:i:s` (spatie in plaats van `T`, geen microseconden, geen
///   offset: "tijd zonder zone", blueprint.md T7).
fn looks_like_datetime(b: &[u8]) -> bool {
    if b.len() < 19 {
        return false;
    }
    let digits = |range: std::ops::Range<usize>| {
        range
            .into_iter()
            .all(|i| matches!(b.get(i), Some(c) if c.is_ascii_digit()))
    };
    if !digits(0..4) || b[4] != b'-' {
        return false;
    }
    if !digits(5..7) || b[7] != b'-' {
        return false;
    }
    if !digits(8..10) {
        return false;
    }
    let sep = b[10];
    if sep != b' ' && sep != b'T' {
        return false;
    }
    if !digits(11..13) || b[13] != b':' {
        return false;
    }
    if !digits(14..16) || b[16] != b':' {
        return false;
    }
    if !digits(17..19) {
        return false;
    }

    let rest = &b[19..];
    if sep == b' ' {
        // Monolog 1: na de seconden mag niets meer volgen.
        return rest.is_empty();
    }

    let mut idx = 0usize;
    if rest.first() == Some(&b'.') {
        idx += 1;
        let start = idx;
        while matches!(rest.get(idx), Some(c) if c.is_ascii_digit()) {
            idx += 1;
        }
        if idx == start {
            return false; // '.' zonder minstens één cijfer microseconden
        }
    }
    match rest.get(idx) {
        Some(b'Z') => idx + 1 == rest.len(),
        Some(b'+') | Some(b'-') => {
            let off = &rest[idx + 1..];
            off.len() == 5
                && off[0].is_ascii_digit()
                && off[1].is_ascii_digit()
                && off[2] == b':'
                && off[3].is_ascii_digit()
                && off[4].is_ascii_digit()
        }
        _ => false,
    }
}

/// Vindt het laatste JSON-blok (`{...}` of `[...]`) aan het eind van `s` en parset
/// het. Geeft `(prefix, value)` terug, waarbij `prefix` alles vóór het blok is
/// (inclusief een eventuele scheidingsspatie, die de aanroeper zelf strip).
fn take_trailing_json(s: &str) -> Option<(&str, serde_json::Value)> {
    let start = rfind_json_blob(s)?;
    let value = serde_json::from_str(&s[start..]).ok()?;
    Some((&s[..start], value))
}

/// Scant `s` van rechts naar links en zoekt de startpositie van het *laatste*
/// gebalanceerde `{...}`/`[...]`-blok, met inachtneming van JSON-stringliterals
/// (zodat `{`/`[`/`}`/`]` binnen een stringwaarde niet meetellen) en escaped quotes
/// (`\"`, met de gebruikelijke pariteitsregel voor een reeks backslashes ervoor).
///
/// Stopt zodra de buitenste haak van dát blok balanceert (diepte terug op 0) en kijkt
/// dus nooit verder naar links dan nodig: een `{` of `]` die toevallig vóór het blok
/// in de message staat, wordt nooit meegenomen (task.md T3).
fn rfind_json_blob(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let len = b.len();
    if len == 0 || !matches!(b[len - 1], b'}' | b']') {
        return None;
    }

    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut idx = len;
    loop {
        if idx == 0 {
            return None; // onbalans: nooit op diepte 0 teruggekomen
        }
        idx -= 1;
        let c = b[idx];
        if in_string {
            if c == b'"' && preceding_backslashes(b, idx) % 2 == 0 {
                in_string = false;
            }
            continue;
        }
        match c {
            b'"' => in_string = true,
            b'}' | b']' => depth += 1,
            b'{' | b'[' => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx);
                }
            }
            _ => {}
        }
    }
}

/// Aantal opeenvolgende `\`-bytes direct vóór `idx` in `b`.
fn preceding_backslashes(b: &[u8], idx: usize) -> usize {
    let mut count = 0;
    let mut j = idx;
    while j > 0 && b[j - 1] == b'\\' {
        count += 1;
        j -= 1;
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_monolog3_default_format() {
        let record =
            parse_line("[2026-01-15T10:30:45.123456+01:00] app.DEBUG: User cache warmed [] []")
                .expect("parses");
        assert_eq!(record.datetime, "2026-01-15T10:30:45.123456+01:00");
        assert_eq!(record.channel, "app");
        assert_eq!(record.level, Level::Debug);
        assert_eq!(record.message, "User cache warmed");
        assert_eq!(record.context, json!([]));
        assert_eq!(record.extra, json!([]));
    }

    #[test]
    fn parses_without_microseconds() {
        let record =
            parse_line("[2026-01-15T10:30:45+01:00] console.ERROR: no micros here {\"n\":1} []")
                .expect("parses");
        assert_eq!(record.datetime, "2026-01-15T10:30:45+01:00");
        assert_eq!(record.context, json!({"n": 1}));
    }

    #[test]
    fn parses_zulu_offset() {
        let record =
            parse_line("[2026-01-15T10:30:45.000000Z] app.INFO: utc line [] []").expect("parses");
        assert_eq!(record.datetime, "2026-01-15T10:30:45.000000Z");
    }

    #[test]
    fn parses_monolog1_format_without_timezone() {
        let record = parse_line("[2015-05-01 12:34:56] app.INFO: legacy entry [] []")
            .expect("parses monolog 1");
        assert_eq!(record.datetime, "2015-05-01 12:34:56");
        assert_eq!(record.channel, "app");
        assert_eq!(record.level, Level::Info);
    }

    #[test]
    fn brace_inside_message_does_not_confuse_the_boundary() {
        let record = parse_line(
            "[2026-01-15T11:00:00.000000+01:00] app.INFO: payload was {not json} end {\"ok\":true} []",
        )
        .expect("parses");
        assert_eq!(record.message, "payload was {not json} end");
        assert_eq!(record.context, json!({"ok": true}));
        assert_eq!(record.extra, json!([]));
    }

    #[test]
    fn closing_bracket_inside_message_does_not_confuse_the_boundary() {
        let record = parse_line(
            "[2026-01-15T11:00:01.000000+01:00] app.INFO: array bounds arr[5] exceeded [] []",
        )
        .expect("parses");
        assert_eq!(record.message, "array bounds arr[5] exceeded");
        assert_eq!(record.context, json!([]));
        assert_eq!(record.extra, json!([]));
    }

    #[test]
    fn empty_context_and_extra() {
        let record =
            parse_line("[2026-01-15T11:00:02.000000+01:00] app.WARNING: nothing to say [] []")
                .expect("parses");
        assert_eq!(record.context, json!([]));
        assert_eq!(record.extra, json!([]));
    }

    #[test]
    fn nested_context_json() {
        let record = parse_line(
            "[2026-01-15T11:00:03.000000+01:00] app.WARNING: nested example {\"user\":{\"id\":42,\"roles\":[\"admin\",\"editor\"]},\"tags\":[1,2,3]} {\"request_id\":\"abc-123\"}",
        )
        .expect("parses");
        assert_eq!(
            record.context,
            json!({"user": {"id": 42, "roles": ["admin", "editor"]}, "tags": [1, 2, 3]})
        );
        assert_eq!(record.extra, json!({"request_id": "abc-123"}));
    }

    #[test]
    fn escaped_quote_and_brace_inside_json_string_value() {
        // De contextwaarde zelf bevat een geëscapete quote gevolgd door een `}`-teken;
        // de backward-scan mag dat niet als het einde van het blok lezen.
        let record = parse_line(
            r#"[2026-01-15T11:00:04.000000+01:00] app.INFO: quoted {"note":"say \"hi}\" please"} []"#,
        )
        .expect("parses");
        assert_eq!(record.context, json!({"note": "say \"hi}\" please"}));
    }

    #[test]
    fn all_eight_levels_are_recognised_case_insensitively() {
        let names = [
            ("debug", Level::Debug),
            ("INFO", Level::Info),
            ("Notice", Level::Notice),
            ("WARNING", Level::Warning),
            ("error", Level::Error),
            ("CRITICAL", Level::Critical),
            ("Alert", Level::Alert),
            ("EMERGENCY", Level::Emergency),
        ];
        for (name, expected) in names {
            let line = format!("[2026-01-15T10:30:45.000000+01:00] app.{name}: msg [] []");
            let record = parse_line(&line).unwrap_or_else(|| panic!("moet parsen: {line}"));
            assert_eq!(record.level, expected, "niveau {name}");
        }
    }

    #[test]
    fn trailing_newline_and_crlf_are_stripped() {
        let lf = parse_line("[2026-01-15T10:30:45.000000+01:00] app.INFO: msg [] []\n")
            .expect("parses lf");
        let crlf = parse_line("[2026-01-15T10:30:45.000000+01:00] app.INFO: msg [] []\r\n")
            .expect("parses crlf");
        assert_eq!(lf.message, "msg");
        assert_eq!(crlf.message, "msg");
    }

    #[test]
    fn rejects_unknown_level_name() {
        assert!(parse_line("[2026-01-15T10:30:45.000000+01:00] app.TRACE: msg [] []").is_none());
    }

    #[test]
    fn rejects_missing_opening_bracket() {
        assert!(parse_line("2026-01-15T10:30:45.000000+01:00] app.INFO: msg [] []").is_none());
    }

    #[test]
    fn rejects_garbage_datetime() {
        assert!(parse_line("[not-a-date] app.INFO: msg [] []").is_none());
    }

    #[test]
    fn rejects_monolog1_datetime_with_trailing_offset() {
        // Monolog 1 heeft nooit een offset; zo'n regel is geen van de drie varianten.
        assert!(parse_line("[2015-05-01 12:34:56+01:00] app.INFO: msg [] []").is_none());
    }

    #[test]
    fn rejects_invalid_trailing_json() {
        // Balanceert qua haakjes, maar is geen geldige JSON (ontbrekende waarde).
        assert!(
            parse_line("[2026-01-15T10:30:45.000000+01:00] app.INFO: msg {\"n\":} []").is_none()
        );
    }

    #[test]
    fn rejects_line_without_context_and_extra() {
        assert!(parse_line("[2026-01-15T10:30:45.000000+01:00] app.INFO: msg").is_none());
    }

    #[test]
    fn rejects_empty_channel() {
        assert!(parse_line("[2026-01-15T10:30:45.000000+01:00] .INFO: msg [] []").is_none());
    }
}
