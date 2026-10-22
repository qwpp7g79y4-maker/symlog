//! Terugval voor recordblokken die geen van de bekende formaten halen (task.md T6;
//! blueprint.md `parse::raw`).
//!
//! [`super::parse_record_lossy`] converteert eerst de ruwe bytes van een blok lossy
//! naar UTF-8 (`String::from_utf8_lossy`, U+FFFD voor ongeldige byte-sequenties,
//! task.md T6 "Doet") en parset die tekst dan net als [`super::parse_record`]. Lukt
//! dat niet, dan is het resultaat hier een [`RawRecord`] in plaats van een
//! weggevallen `None`: task.md T6 "Doet" vereist dat zulke regels zichtbaar blijven
//! ("Raw-records zijn zichtbaar, doorzoekbaar"). [`RawReason`] onderscheidt de twee
//! gevallen uit de taaktitel ("Kapotte regels, ongeldige JSON"): een blok dat met `{`
//! begint maar geen geldig JSON-object is, krijgt [`RawReason::InvalidJson`]; alles
//! anders dat geen enkele kop haalt (ook een lijnformaat-kop met een verder kapotte
//! regel, bijvoorbeeld afgekapte trailing-JSON) krijgt [`RawReason::UnknownFormat`].
//! Task.md T7 voegt deze onderverdeling later samen met het gewone `Record` tot één
//! `Record { ..., raw }`-veld; tot die consolidatie blijft [`ParsedRecord`] de losse
//! draagvorm.
//!
//! [`sanitize_for_display`] is de andere helft van task.md T6 "Doet": NUL-bytes en
//! ANSI-escapes (en alle andere C0-besturingstekens behalve de regeleinde `\n`, zoals
//! een losse CR die een terminalcursor kan terugzetten) worden vóór weergave vervangen
//! door hun zichtbare Unicode "control picture"-tegenhanger, zodat geen enkel
//! besturingsteken uit de logtekst zelf als terminal-commando uitgevoerd kan worden
//! (geen terminal-injectie). Dit geldt voor **elke** weergegeven tekst, niet alleen
//! voor Raw-records: F15 (task.md T6) is een gewoon parseerbaar record waarvan het
//! `message`-veld ANSI-escapes en een NUL-byte bevat.

use super::{detect, Format};
use crate::model::Record;

/// Waarom een blok niet als LineFormatter of JsonFormatter parseerde (task.md T6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawReason {
    /// Geen van de twee recordkoppen herkend, of een herkende kop met een verder
    /// kapotte regel (bijvoorbeeld een lijnformaat-record met afgekapte trailing-JSON).
    UnknownFormat,
    /// Begint met `{` (de JsonFormatter-kop) maar is geen geldig JSON-object: afgekapt,
    /// enkele in plaats van dubbele quotes, of een ander veld-/typefout.
    InvalidJson,
}

impl RawReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            RawReason::UnknownFormat => "unknown_format",
            RawReason::InvalidJson => "invalid_json",
        }
    }
}

/// Een recordblok dat niet als LineFormatter of JsonFormatter parseerde (task.md T6).
/// `text` is de lossy-UTF-8-tekst van de originele bytes (`String::from_utf8_lossy`);
/// de bytes op schijf zelf blijven altijd ongewijzigd (task.md T6 "Blijft").
#[derive(Debug, Clone, PartialEq)]
pub struct RawRecord {
    pub text: String,
    pub reason: RawReason,
}

impl RawRecord {
    /// Bouwt een `RawRecord` uit lossy-UTF-8-tekst die al door geen van beide parsers
    /// herkend is; de reden volgt uit [`detect`] op dezelfde tekst (zie moduledoc).
    fn from_lossy_text(text: String) -> Self {
        let reason = match detect(&text) {
            Some(Format::Json) => RawReason::InvalidJson,
            _ => RawReason::UnknownFormat,
        };
        Self { text, reason }
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "raw": self.text,
            "reason": self.reason.as_str(),
        })
    }

    /// Eén-regelweergave voor plain-uitvoer (task.md T6 "Klaar als": "golden
    /// plain-uitvoer"), met de ruwe tekst gesaneerd via [`sanitize_for_display`]. De
    /// volledige plain-modus met filters en kolomopmaak komt in T13; dit is alleen het
    /// deel dat nodig is om te laten zien dat een Raw-record zichtbaar is, met een
    /// eigen markering, en zonder terminal-injectie.
    pub fn to_plain_line(&self) -> String {
        format!(
            "RAW[{}] {}",
            self.reason.as_str(),
            sanitize_for_display(&self.text)
        )
    }
}

/// Resultaat van [`super::parse_record_lossy`]: ofwel een herkend record, ofwel de
/// Raw-terugval. Geeft, in tegenstelling tot [`super::parse_record`], nooit `None`
/// terug (task.md T6 "Doet").
#[derive(Debug, Clone, PartialEq)]
pub enum ParsedRecord {
    Record(Record),
    Raw(RawRecord),
}

impl ParsedRecord {
    pub(super) fn from_parse(text: String, parsed: Option<Record>) -> Self {
        match parsed {
            Some(record) => ParsedRecord::Record(record),
            None => ParsedRecord::Raw(RawRecord::from_lossy_text(text)),
        }
    }

    pub fn as_raw(&self) -> Option<&RawRecord> {
        match self {
            ParsedRecord::Raw(raw) => Some(raw),
            ParsedRecord::Record(_) => None,
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        match self {
            ParsedRecord::Record(record) => record.to_json(),
            ParsedRecord::Raw(raw) => raw.to_json(),
        }
    }

    /// Zie [`RawRecord::to_plain_line`]; voor een gewoon geparst record is dit de
    /// minimale lijnformaat-achtige weergave die T13 verder uitbreidt, ook met
    /// [`sanitize_for_display`] over het bericht (F15, task.md T6).
    pub fn to_plain_line(&self) -> String {
        match self {
            ParsedRecord::Record(record) => sanitize_for_display(&format!(
                "[{}] {}.{}: {}",
                record.datetime,
                record.channel,
                record.level.as_str(),
                record.message
            )),
            ParsedRecord::Raw(raw) => raw.to_plain_line(),
        }
    }
}

/// Vervangt elk C0-besturingsteken (`U+0000..=U+001F`, `U+007F`) behalve `\n` door
/// zijn zichtbare Unicode "control picture"-tegenhanger (`U+2400..` resp. `U+2421`
/// voor DEL), zodat NUL-bytes en ANSI-escapes (die altijd met het ESC-byte `U+001B`
/// beginnen) nooit als terminal-besturing bij de gebruiker terechtkomen (task.md T6
/// "Doet": "geen terminal-injectie"). `\n` blijft een echte regelovergang: berichten
/// mogen meerdere regels beslaan (stacktraces, task.md T4).
pub fn sanitize_for_display(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\n' => '\n',
            '\u{7f}' => '\u{2421}',
            c if (c as u32) < 0x20 => {
                char::from_u32(0x2400 + c as u32).expect("0x2400..=0x241F zijn geldige scalars")
            }
            c => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_replaces_nul_and_esc_but_keeps_newline() {
        let input = "a\u{0}b\u{1b}[31mc\nd";
        let sanitized = sanitize_for_display(input);
        assert_eq!(sanitized, "a\u{2400}b\u{241b}[31mc\nd");
    }

    #[test]
    fn sanitize_replaces_del_and_lone_cr() {
        let input = "x\u{7f}y\rz";
        assert_eq!(sanitize_for_display(input), "x\u{2421}y\u{240d}z");
    }

    #[test]
    fn sanitize_leaves_printable_and_replacement_chars_untouched() {
        let input = "café 🎉 \u{fffd}";
        assert_eq!(sanitize_for_display(input), input);
    }

    #[test]
    fn from_lossy_text_marks_brace_prefixed_blocks_as_invalid_json() {
        let raw = RawRecord::from_lossy_text("{not valid json".to_string());
        assert_eq!(raw.reason, RawReason::InvalidJson);
    }

    #[test]
    fn from_lossy_text_marks_everything_else_as_unknown_format() {
        let raw = RawRecord::from_lossy_text("zomaar wat tekst".to_string());
        assert_eq!(raw.reason, RawReason::UnknownFormat);

        let raw = RawRecord::from_lossy_text("[not-a-date] app.INFO: msg [] []".to_string());
        assert_eq!(raw.reason, RawReason::UnknownFormat);
    }

    #[test]
    fn parsed_record_round_trips_through_json_and_plain() {
        let parsed = ParsedRecord::from_parse("{half".to_string(), None);
        assert!(parsed.as_raw().is_some());
        assert_eq!(parsed.to_json()["reason"], "invalid_json");
        assert!(parsed.to_plain_line().starts_with("RAW[invalid_json]"));
    }
}
