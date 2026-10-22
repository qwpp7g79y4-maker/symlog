//! Gedeelde datatypes voor geparste records (task.md T3; blueprint.md `model.rs`).
//!
//! T3 legt alleen vast wat de LineFormatter-parser al kan leveren: datetime als
//! ruwe string (de echte tijdzone-semantiek met `Zoned`/`OffsetDateTime` komt in T7),
//! kanaal, niveau, message en context/extra als JSON. Latere taken breiden
//! [`Record`] uit met `offset`, `frames` (T4) en `raw` (T6), en vervangen `channel`
//! door een geïnterneerd type.
//!
//! Geen `serde::Serialize`-afleiding hier: dat zou de `derive`-feature van `serde`
//! vereisen (en daarmee `proc-macro2`/`quote`/`syn`/`unicode-ident`, met een
//! `Unicode-3.0`-licentieclausule die nog niet in `deny.toml` staat). De golden-tests
//! in [`crate::parse::line`] bouwen hun eigen `serde_json::Value` op uit
//! [`Level::as_str`] en de andere velden.

/// Monolog-niveaus, hoofdletterongevoelig bij het parsen (task.md T3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Debug,
    Info,
    Notice,
    Warning,
    Error,
    Critical,
    Alert,
    Emergency,
}

impl Level {
    /// Herkent een niveaunaam hoofdletterongevoelig, of `None` als `name` geen
    /// bekend Monolog-niveau is.
    pub fn parse(name: &str) -> Option<Level> {
        match_level(name)
    }

    /// Niveaunaam zoals Monolog die zelf schrijft (hoofdletters), voor weergave en
    /// voor de golden-tests.
    pub fn as_str(&self) -> &'static str {
        match self {
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Notice => "NOTICE",
            Level::Warning => "WARNING",
            Level::Error => "ERROR",
            Level::Critical => "CRITICAL",
            Level::Alert => "ALERT",
            Level::Emergency => "EMERGENCY",
        }
    }
}

fn match_level(name: &str) -> Option<Level> {
    if name.eq_ignore_ascii_case("DEBUG") {
        Some(Level::Debug)
    } else if name.eq_ignore_ascii_case("INFO") {
        Some(Level::Info)
    } else if name.eq_ignore_ascii_case("NOTICE") {
        Some(Level::Notice)
    } else if name.eq_ignore_ascii_case("WARNING") {
        Some(Level::Warning)
    } else if name.eq_ignore_ascii_case("ERROR") {
        Some(Level::Error)
    } else if name.eq_ignore_ascii_case("CRITICAL") {
        Some(Level::Critical)
    } else if name.eq_ignore_ascii_case("ALERT") {
        Some(Level::Alert)
    } else if name.eq_ignore_ascii_case("EMERGENCY") {
        Some(Level::Emergency)
    } else {
        None
    }
}

/// Een geparst logrecord (vooralsnog alleen het resultaat van de LineFormatter-parser,
/// zie [`crate::parse::line`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    /// Ruwe datetime-tekst tussen `[` en `]`, ongewijzigd uit de log. T7 rekent dit
    /// om naar een echte tijd met behoud van de offset uit de log.
    pub datetime: String,
    pub channel: String,
    pub level: Level,
    pub message: String,
    pub context: serde_json::Value,
    pub extra: serde_json::Value,
}

impl Record {
    /// Bouwt een `serde_json::Value` op uit dit record, voor golden-tests en latere
    /// plain-uitvoer (T13). Geen `Serialize`-afleiding, zie de moduledoc hierboven.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "datetime": self.datetime,
            "channel": self.channel,
            "level": self.level.as_str(),
            "message": self.message,
            "context": self.context,
            "extra": self.extra,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_parse_is_case_insensitive() {
        assert_eq!(Level::parse("warning"), Some(Level::Warning));
        assert_eq!(Level::parse("WARNING"), Some(Level::Warning));
        assert_eq!(Level::parse("Warning"), Some(Level::Warning));
    }

    #[test]
    fn level_parse_rejects_unknown_names() {
        assert_eq!(Level::parse("TRACE"), None);
        assert_eq!(Level::parse(""), None);
    }

    #[test]
    fn as_str_roundtrips_through_parse() {
        let levels = [
            Level::Debug,
            Level::Info,
            Level::Notice,
            Level::Warning,
            Level::Error,
            Level::Critical,
            Level::Alert,
            Level::Emergency,
        ];
        for level in levels {
            assert_eq!(Level::parse(level.as_str()), Some(level));
        }
    }

    #[test]
    fn record_to_json_has_expected_shape() {
        let record = Record {
            datetime: "2026-01-15T10:30:45.000000+01:00".to_string(),
            channel: "app".to_string(),
            level: Level::Info,
            message: "hallo".to_string(),
            context: serde_json::json!([]),
            extra: serde_json::json!({"k": "v"}),
        };
        assert_eq!(
            record.to_json(),
            serde_json::json!({
                "datetime": "2026-01-15T10:30:45.000000+01:00",
                "channel": "app",
                "level": "INFO",
                "message": "hallo",
                "context": [],
                "extra": {"k": "v"},
            })
        );
    }
}
