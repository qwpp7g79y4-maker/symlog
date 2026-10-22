//! Parsers voor Monolog-logregels (task.md T3/T4/T5; blueprint.md `parse/`).
//!
//! T3 voegde [`line`] toe (LineFormatter, Monolog 1–3). T4 voegt [`trace`] toe
//! (frames uit stacktrace-tekst en JSON-exceptiedata) en breidt [`line`] uit met
//! multiline-recordondersteuning. T5 voegt [`json`] toe (JsonFormatter/NDJSON) en de
//! dispatch tussen formaten uit blueprint.md (`detect`/[`parse_record`]): het formaat
//! wordt per record bepaald, niet per bestand (task.md T5 "Doet"), zodat een gemengd
//! bestand (lijn- en JSON-records door elkaar) werkt. [`is_record_header`] combineert
//! de twee formaat-specifieke kopcontroles tot één die de achtergrondindexer
//! ([`crate::reader::index`]) gebruikt, zodat recordgrenzen ook in een gemengd
//! bestand correct liggen.
//!
//! Een blok dat aan geen van beide formaten voldoet, levert `None` op bij
//! [`parse_record`] ("Blijft" in task.md T3: een onbekend formaat valt terug op een
//! ruwe weergave, niet op een fout). T6 voegt [`raw`] toe: [`parse_record_lossy`] is
//! de bytes-bewuste tegenhanger van [`parse_record`] die dat `None` opvangt met de
//! Raw-terugval, en nooit paniekt op ongeldige UTF-8-invoer.

pub mod json;
pub mod line;
pub mod raw;
pub mod trace;

use crate::model::Record;
use raw::ParsedRecord;

/// Welk formaat een recordblok heeft (task.md T5; blueprint.md `parse::mod`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// LineFormatter: `[%datetime%] %channel%.%level_name%: %message% %context%
    /// %extra%` (task.md T3/T4).
    Line,
    /// JsonFormatter: één JSON-object per regel (task.md T5).
    Json,
}

/// Bepaalt het formaat van `block` aan het eerste teken, zonder het blok al te
/// parsen: `[` is LineFormatter, `{` is JsonFormatter, iets anders is geen van
/// beide bekende recordformaten. Dit is een snelle route-beslissing; de echte
/// validatie (is de kop ook werkelijk geldig?) gebeurt in [`parse_record`] door de
/// aangewezen parser zelf, die op een niet-geldige kop `None` teruggeeft.
pub fn detect(block: &str) -> Option<Format> {
    match block.as_bytes().first() {
        Some(b'[') => Some(Format::Line),
        Some(b'{') => Some(Format::Json),
        _ => None,
    }
}

/// Parset één recordblok, met het formaat bepaald per record via [`detect`] (task.md
/// T5 "Doet"). `None` als geen van de twee parsers het blok herkent; dat is "onbekend
/// formaat", geen paniek (de `Raw`-terugval zelf komt in T6).
pub fn parse_record(block: &str) -> Option<Record> {
    match detect(block)? {
        Format::Line => line::parse_line(block),
        Format::Json => json::parse_json(block),
    }
}

/// Parset `block` met een vast, afgedwongen formaat (task.md T5: `--format` dwingt
/// één parser af). `None` als `block` niet aan dát formaat voldoet, ook als het wel
/// aan het andere formaat zou voldoen — forceren van het verkeerde formaat levert dus
/// bewust "onbekend formaat" op, niet een paniek en niet een stille misinterpretatie.
pub fn parse_record_as(block: &str, format: Format) -> Option<Record> {
    match format {
        Format::Line => line::parse_line(block),
        Format::Json => json::parse_json(block),
    }
}

/// Parset `bytes` zonder ooit `None` terug te geven (task.md T6 "Doet"). De bytes
/// worden eerst lossy naar UTF-8 geconverteerd (`String::from_utf8_lossy`, U+FFFD
/// voor ongeldige byte-sequenties) en die tekst dan net als [`parse_record`] geparst;
/// lukt dat niet, dan is het resultaat de Raw-terugval uit [`raw`] in plaats van een
/// weggevallen record. Dit is de eerste functie in deze module die rauwe bytes
/// aanvaardt in plaats van al-gevalideerde tekst (vergelijk [`is_record_header`], die
/// ook al op bytes werkt): juist ongeldige UTF-8-invoer (task.md T6, F8) moet hier
/// zonder paniek doorheen kunnen.
pub fn parse_record_lossy(bytes: &[u8]) -> ParsedRecord {
    let text = String::from_utf8_lossy(bytes).into_owned();
    let parsed = parse_record(&text);
    ParsedRecord::from_parse(text, parsed)
}

/// Format-agnostische recordkop-check: of `line` (bytes van één fysieke regel, zonder
/// regeleinde) een geldige LineFormatter- of JsonFormatter-kop is (blueprint.md:
/// "Recordstart = regel die begint met een geldige kop (`[YYYY-` of `{"` met een
/// `"datetime"`-veld)"). Gebruikt door de achtergrondindexer
/// ([`crate::reader::index::build_index`]) zodat recordgrenzen ook in een gemengd
/// bestand (task.md T5, F6) correct liggen: elke regel die geen van beide koppen is,
/// is een vervolgregel van het lopende record, ongeacht welk formaat dat record zelf
/// heeft.
pub(crate) fn is_record_header(line: &[u8]) -> bool {
    self::line::is_record_header(line) || json::is_record_header(line)
}

/// Splitst de volledige inhoud van een (mogelijk gemengd) logbestand in
/// recordblokken, net als [`line::split_records`] maar met de format-agnostische
/// [`is_record_header`] in plaats van de LineFormatter-only kopcontrole. Gebruikt
/// door de T5-golden-tests voor F6 (lijn- en JSON-records door elkaar); de echte
/// reader ([`crate::reader::index`]) doet hetzelfde maar dan streaming op bytes.
pub fn split_records(content: &str) -> Vec<&str> {
    let mut starts = Vec::new();
    let mut offset = 0usize;
    for (i, raw_line) in content.split_inclusive('\n').enumerate() {
        let without_lf = raw_line.strip_suffix('\n').unwrap_or(raw_line);
        let trimmed = without_lf.strip_suffix('\r').unwrap_or(without_lf);
        if i == 0 || is_record_header(trimmed.as_bytes()) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Level;

    const LINE_BLOCK: &str = "[2026-01-15T10:00:00.000000+01:00] app.INFO: hi [] []";
    const JSON_BLOCK: &str = r#"{"message":"hi","level":200,"channel":"app","datetime":"2026-01-15T10:00:00.000000+01:00"}"#;

    #[test]
    fn detect_routes_on_first_byte() {
        assert_eq!(detect(LINE_BLOCK), Some(Format::Line));
        assert_eq!(detect(JSON_BLOCK), Some(Format::Json));
        assert_eq!(detect("geen van beide"), None);
        assert_eq!(detect(""), None);
    }

    #[test]
    fn parse_record_dispatches_per_block() {
        let line = parse_record(LINE_BLOCK).expect("lineformat moet parsen");
        assert_eq!(line.level, Level::Info);
        let json = parse_record(JSON_BLOCK).expect("json moet parsen");
        assert_eq!(json.level, Level::Info);
    }

    #[test]
    fn parse_record_returns_none_for_unknown_format_without_panicking() {
        assert!(parse_record("gewoon tekst, geen enkele kop").is_none());
        assert!(parse_record("").is_none());
    }

    #[test]
    fn parse_record_as_forces_a_single_parser() {
        // Task.md T5 "Klaar als": `--format line` op een JSON-regel geeft onbekend
        // formaat, geen paniek en geen stille misinterpretatie als lineformat.
        assert!(parse_record_as(JSON_BLOCK, Format::Line).is_none());
        assert!(parse_record_as(LINE_BLOCK, Format::Json).is_none());
        assert!(parse_record_as(LINE_BLOCK, Format::Line).is_some());
        assert!(parse_record_as(JSON_BLOCK, Format::Json).is_some());
    }

    #[test]
    fn split_records_separates_mixed_line_and_json_records() {
        let content = format!("{LINE_BLOCK}\n{JSON_BLOCK}\n{LINE_BLOCK}\n");
        let blocks = split_records(&content);
        assert_eq!(blocks.len(), 3);
        assert_eq!(parse_record(blocks[0]).unwrap().level, Level::Info);
        assert_eq!(parse_record(blocks[1]).unwrap().channel, "app");
        assert_eq!(parse_record(blocks[2]).unwrap().level, Level::Info);
    }

    #[test]
    fn split_records_keeps_line_format_stacktrace_continuation_intact() {
        let content = format!(
            "{LINE_BLOCK}\n[2026-01-15T10:00:01.000000+01:00] app.ERROR: boom\nStack trace:\n#0 {{main}} [] []\n{JSON_BLOCK}\n"
        );
        let blocks = split_records(&content);
        assert_eq!(blocks.len(), 3);
        assert!(blocks[1].contains("Stack trace:"));
        assert_eq!(parse_record(blocks[1]).unwrap().frames.len(), 1);
    }

    #[test]
    fn parse_record_lossy_wraps_recognised_blocks() {
        let parsed = parse_record_lossy(LINE_BLOCK.as_bytes());
        assert!(parsed.as_raw().is_none());
        assert_eq!(parsed.to_json()["level"], "INFO");
    }

    #[test]
    fn parse_record_lossy_falls_back_to_raw_without_panicking() {
        // task.md T6 "Doet": een blok dat geen enkel formaat haalt, wordt Raw in
        // plaats van weggevallen.
        let parsed = parse_record_lossy(b"gewoon tekst, geen enkele kop");
        assert!(parsed.as_raw().is_some());
    }

    #[test]
    fn parse_record_lossy_never_panics_on_invalid_utf8() {
        // task.md T6, F8: ongeldige UTF-8-bytes mogen nooit een paniek geven; de
        // lossy-conversie vangt ze op als U+FFFD.
        let parsed = parse_record_lossy(b"caf\xE9 zonder kop, \xFF\xFE ongeldige bytes");
        let raw = parsed.as_raw().expect("geen van beide parsers herkent dit");
        assert!(raw.text.contains('\u{fffd}'));
    }
}
