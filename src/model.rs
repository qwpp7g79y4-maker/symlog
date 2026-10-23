//! Gedeelde datatypes voor geparste records (task.md T3/T4/T7; blueprint.md
//! `model.rs`).
//!
//! T3 legde vast wat de LineFormatter-parser kan leveren: datetime als ruwe string,
//! kanaal, niveau, message en context/extra als JSON. T4 voegt [`Frame`] toe en breidt
//! [`Record`] uit met `frames`: een record kan meerdere fysieke regels beslaan
//! (stacktraces), en elk herkend stacktrace-frame wordt een `Frame`. T6 voegt de
//! Raw-terugval toe als los type ([`crate::parse::raw::RawRecord`]/
//! [`crate::parse::raw::ParsedRecord`]), voor blokken die geen van beide formaten
//! halen. T7 voegt [`Ts`] toe: de echte tijdzone-semantiek bovenop de ruwe
//! `datetime`-string, zodat [`crate::time`] die kan omrekenen naar `local` of een
//! IANA-naam. `channel` blijft een `String` (geen interning in T7: dat raakt het
//! geheugenbudget uit T12, niet "Model, tijdzones en filters", en is hier niet nodig).
//!
//! Geen `serde::Serialize`-afleiding hier: dat zou de `derive`-feature van `serde`
//! vereisen (en daarmee `proc-macro2`/`quote`/`syn`/`unicode-ident`, met een
//! `Unicode-3.0`-licentieclausule die nog niet in `deny.toml` staat). De golden-tests
//! in [`crate::parse::line`] bouwen hun eigen `serde_json::Value` op uit
//! [`Level::as_str`] en de andere velden.

/// Monolog-niveaus, hoofdletterongevoelig bij het parsen (task.md T3).
///
/// De volgorde van de varianten is bewust gelijk aan de PSR-3/Monolog-ernstrangorde
/// (`Debug` minst ernstig, `Emergency` meest), dezelfde volgorde als
/// [`Level::from_monolog_int`]. De afgeleide `PartialOrd`/`Ord` steunt daarop voor
/// `--level >=warning` (task.md T7, [`crate::filter::LevelFilter::AtLeast`]): verander
/// deze volgorde niet zonder ook de filtersemantiek te controleren.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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

    /// Herkent een Monolog/PSR-3-niveauwaarde (het integer `level`-veld van de
    /// JsonFormatter, task.md T5), `None` als `value` geen van de acht vaste waarden
    /// is. Monolog kent deze waarden vast toe (RFC 5424-stijl, niet configureerbaar),
    /// dus een onbekend getal is altijd een teken van een niet-Monolog-bron en wordt
    /// niet geraden.
    pub fn from_monolog_int(value: i64) -> Option<Level> {
        match value {
            100 => Some(Level::Debug),
            200 => Some(Level::Info),
            250 => Some(Level::Notice),
            300 => Some(Level::Warning),
            400 => Some(Level::Error),
            500 => Some(Level::Critical),
            550 => Some(Level::Alert),
            600 => Some(Level::Emergency),
            _ => None,
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

/// Een geparste tijd uit een logrecord (task.md T7; blueprint.md `model.rs`).
///
/// Bewaart de tijd zoals die in de log staat: [`Ts::Offset`] behoudt de exacte offset
/// uit de log (als vaste, niet-IANA tijdzone, zie [`Ts::parse`]); een datetime zonder
/// offset (Monolog 1, `Y-m-d H:i:s`) wordt nooit stil als UTC aangenomen, maar bewaard
/// als [`Ts::NoZone`] ("zonder zone", task.md T7 "Doet"). `--tz local`/`--tz <IANA>`
/// ([`crate::time::display`]) rekent een [`Ts::Offset`] om naar een andere tijdzone;
/// een [`Ts::NoZone`] is alleen een kloktijd zonder vastgelegd moment en kan dus in
/// geen enkele modus omgerekend worden.
#[derive(Debug, Clone)]
pub enum Ts {
    /// Tijd met de exacte offset uit de log, als vaste tijdzone (geen IANA-naam).
    Offset(jiff::Zoned),
    /// Tijd zonder offset (Monolog 1): alleen een kloktijd, geen moment in de tijd.
    NoZone(jiff::civil::DateTime),
}

impl Ts {
    /// Parseert de ruwe `%datetime%`-tekst van een record (task.md T3) naar een
    /// [`Ts`]. `None` als de tekst geen van de drie bekende varianten is (Monolog 2/3
    /// met/zonder microseconden, of Monolog 1 zonder offset). Een LineFormatter-record
    /// heeft dit al laten slagen via
    /// [`crate::parse::line::looks_like_datetime`] vóórdat dit aangeroepen wordt; een
    /// JsonFormatter-record ([`crate::parse::json`]) valideert het `datetime`-veld
    /// niet op formaat, dus daar kan dit wél `None` opleveren bij een vreemde waarde —
    /// de ruwe tekst in [`Record::datetime`] blijft in dat geval gewoon zichtbaar.
    pub fn parse(text: &str) -> Option<Ts> {
        parse_ts(text)
    }
}

impl PartialEq for Ts {
    /// `jiff::Zoned`'s eigen `PartialEq` vergelijkt alleen het tijdstip (instant), niet
    /// de offset waarmee het weergegeven wordt; voor dit model telt de offset uit de
    /// log ook mee (twee records met hetzelfde instant maar een andere offset-tekst in
    /// de log zijn geen gelijke `Ts`'en).
    fn eq(&self, other: &Ts) -> bool {
        match (self, other) {
            (Ts::Offset(a), Ts::Offset(b)) => {
                a.timestamp() == b.timestamp() && a.offset() == b.offset()
            }
            (Ts::NoZone(a), Ts::NoZone(b)) => a == b,
            _ => false,
        }
    }
}

/// Zet een vaste-breedte cijferreeks om naar een geheel getal, `None` bij een niet-cijfer
/// (geen `str::parse`: dat zou voorloop-`+`/witruimte toestaan die hier niet geldig is).
fn parse_fixed_int(bytes: &[u8]) -> Option<i32> {
    let mut value: i32 = 0;
    for &b in bytes {
        if !b.is_ascii_digit() {
            return None;
        }
        value = value * 10 + i32::from(b - b'0');
    }
    Some(value)
}

/// Zet 1 t/m 9 cijfers na een `.` om naar nanoseconden (rechts opgevuld met nullen,
/// zoals `123456` microseconden → `123456000` nanoseconden).
fn fraction_to_nanos(digits: &[u8]) -> Option<i32> {
    if digits.is_empty() || digits.len() > 9 {
        return None;
    }
    let mut value: i64 = 0;
    for &d in digits {
        if !d.is_ascii_digit() {
            return None;
        }
        value = value * 10 + i64::from(d - b'0');
    }
    let scale = 10i64.pow((9 - digits.len()) as u32);
    i32::try_from(value * scale).ok()
}

/// Parseert `%datetime%`-tekst in een van de drie varianten uit task.md T3: Monolog
/// 2/3 (`Y-m-d\TH:i:s.uP`, microseconden optioneel, offset als `+HH:MM`/`-HH:MM`/`Z`)
/// of Monolog 1 (`Y-m-d H:i:s`, geen offset). Puur op bytes, dus robuust tegen tekst
/// die niet vooraf op formaat gecontroleerd is (zie [`Ts::parse`]); dit is bewust een
/// eigen, kleine parser in plaats van jiff's eigen stringparsers, omdat die een
/// IANA-zone-annotatie tussen `[...]` eisen om een offset-only string in een `Zoned`
/// te krijgen (zie de jiff-documentatie bij `DateTimeParser::parse_zoned`) — hier is
/// de offset zelf altijd al bekend uit de tekst, dus die omweg is niet nodig.
fn parse_ts(text: &str) -> Option<Ts> {
    let b = text.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let year = parse_fixed_int(&b[0..4])?;
    if b[4] != b'-' {
        return None;
    }
    let month = parse_fixed_int(&b[5..7])?;
    if b[7] != b'-' {
        return None;
    }
    let day = parse_fixed_int(&b[8..10])?;
    let sep = b[10];
    if sep != b' ' && sep != b'T' {
        return None;
    }
    let hour = parse_fixed_int(&b[11..13])?;
    if b[13] != b':' {
        return None;
    }
    let minute = parse_fixed_int(&b[14..16])?;
    if b[16] != b':' {
        return None;
    }
    let second = parse_fixed_int(&b[17..19])?;

    let rest = &b[19..];
    if sep == b' ' {
        // Monolog 1: na de seconden mag niets meer volgen, en er is nooit een offset
        // (task.md T7 "Doet": "niet stil als UTC aangenomen").
        if !rest.is_empty() {
            return None;
        }
        let dt = jiff::civil::DateTime::new(
            year as i16,
            month as i8,
            day as i8,
            hour as i8,
            minute as i8,
            second as i8,
            0,
        )
        .ok()?;
        return Some(Ts::NoZone(dt));
    }

    let mut idx = 0usize;
    let mut subsec_nanos: i32 = 0;
    if rest.first() == Some(&b'.') {
        idx += 1;
        let start = idx;
        while matches!(rest.get(idx), Some(c) if c.is_ascii_digit()) {
            idx += 1;
        }
        if idx == start {
            return None; // '.' zonder minstens één cijfer microseconden
        }
        subsec_nanos = fraction_to_nanos(&rest[start..idx])?;
    }

    let offset_seconds = match rest.get(idx).copied() {
        Some(b'Z') if idx + 1 == rest.len() => 0,
        Some(sign @ (b'+' | b'-')) => {
            let off = rest.get(idx + 1..)?;
            if off.len() != 5 || off[2] != b':' {
                return None;
            }
            let hours = parse_fixed_int(&off[0..2])?;
            let minutes = parse_fixed_int(&off[3..5])?;
            let magnitude = hours * 3600 + minutes * 60;
            if sign == b'-' {
                -magnitude
            } else {
                magnitude
            }
        }
        _ => return None,
    };

    let dt = jiff::civil::DateTime::new(
        year as i16,
        month as i8,
        day as i8,
        hour as i8,
        minute as i8,
        second as i8,
        subsec_nanos,
    )
    .ok()?;
    let offset = jiff::tz::Offset::from_seconds(offset_seconds).ok()?;
    let zoned = dt.to_zoned(jiff::tz::TimeZone::fixed(offset)).ok()?;
    Some(Ts::Offset(zoned))
}

/// Een geparst logrecord (vooralsnog alleen het resultaat van de LineFormatter-parser,
/// zie [`crate::parse::line`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    /// Ruwe datetime-tekst tussen `[` en `]`, ongewijzigd uit de log.
    pub datetime: String,
    /// Geparste vorm van `datetime` (task.md T7), voor tijdzone-omrekening
    /// ([`crate::time`]). `None` als `datetime` geen van de drie bekende varianten is
    /// (zie [`Ts::parse`]); `datetime` zelf blijft dan gewoon de ruwe tekst, ongewijzigd
    /// zichtbaar.
    pub ts: Option<Ts>,
    pub channel: String,
    pub level: Level,
    /// Het volledige bericht, inclusief eventuele stacktrace-tekst op de regels erna
    /// (task.md T4): een record kan meerdere fysieke regels beslaan, dus `message` kan
    /// zelf `\n`-tekens bevatten.
    pub message: String,
    pub context: serde_json::Value,
    pub extra: serde_json::Value,
    /// Stacktrace-frames herkend in `message` (LineFormatter) of meegegeven door de
    /// JsonFormatter-kant (task.md T4). Leeg als er geen stacktrace is.
    pub frames: Vec<Frame>,
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
            "frames": self.frames.iter().map(Frame::to_json).collect::<Vec<_>>(),
        })
    }
}

/// Eén frame uit een stacktrace (task.md T4; blueprint.md `parse::trace`).
///
/// `path`/`line` zijn `None` voor frames zonder springbare locatie: PHP's eigen
/// `Stack trace:`-tekst schrijft zulke frames als `#N [internal function]: ...` of als
/// `#N {main}` (task.md T4 "Blijft": zulke frames worden getoond maar zijn niet
/// "springbaar").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub path: Option<String>,
    pub line: Option<u32>,
    pub func: String,
}

impl Frame {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "path": self.path,
            "line": self.line,
            "func": self.func,
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
    fn from_monolog_int_recognises_all_eight_values() {
        let pairs = [
            (100, Level::Debug),
            (200, Level::Info),
            (250, Level::Notice),
            (300, Level::Warning),
            (400, Level::Error),
            (500, Level::Critical),
            (550, Level::Alert),
            (600, Level::Emergency),
        ];
        for (value, expected) in pairs {
            assert_eq!(
                Level::from_monolog_int(value),
                Some(expected),
                "waarde {value}"
            );
        }
    }

    #[test]
    fn from_monolog_int_rejects_unknown_values() {
        assert_eq!(Level::from_monolog_int(0), None);
        assert_eq!(Level::from_monolog_int(350), None);
        assert_eq!(Level::from_monolog_int(-1), None);
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
            ts: None,
            channel: "app".to_string(),
            level: Level::Info,
            message: "hallo".to_string(),
            context: serde_json::json!([]),
            extra: serde_json::json!({"k": "v"}),
            frames: vec![
                Frame {
                    path: Some("/app/src/Foo.php".to_string()),
                    line: Some(42),
                    func: "App\\Foo->bar()".to_string(),
                },
                Frame {
                    path: None,
                    line: None,
                    func: "{main}".to_string(),
                },
            ],
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
                "frames": [
                    {"path": "/app/src/Foo.php", "line": 42, "func": "App\\Foo->bar()"},
                    {"path": null, "line": null, "func": "{main}"},
                ],
            })
        );
    }

    #[test]
    fn level_ordering_matches_monolog_severity() {
        // task.md T7: `--level >=warning` steunt op deze volgorde (zie de doc-comment
        // bij `Level`).
        assert!(Level::Debug < Level::Info);
        assert!(Level::Warning < Level::Error);
        assert!(Level::Alert < Level::Emergency);
        assert!(Level::Warning >= Level::Warning);
        assert!(!(Level::Notice >= Level::Warning));
    }

    #[test]
    fn ts_parse_keeps_microsecond_offset() {
        let ts = Ts::parse("2026-01-15T10:30:45.123456+01:00").expect("parses");
        let Ts::Offset(zoned) = ts else {
            panic!("verwacht Ts::Offset")
        };
        assert_eq!(zoned.offset(), jiff::tz::offset(1));
        assert_eq!(
            zoned.datetime(),
            jiff::civil::date(2026, 1, 15).at(10, 30, 45, 123_456_000)
        );
    }

    #[test]
    fn ts_parse_without_microseconds_keeps_offset() {
        let ts = Ts::parse("2026-01-15T10:30:45+01:00").expect("parses");
        let Ts::Offset(zoned) = ts else {
            panic!("verwacht Ts::Offset")
        };
        assert_eq!(zoned.offset(), jiff::tz::offset(1));
    }

    #[test]
    fn ts_parse_zulu_offset_is_zero() {
        let ts = Ts::parse("2026-01-15T10:30:45.000000Z").expect("parses");
        let Ts::Offset(zoned) = ts else {
            panic!("verwacht Ts::Offset")
        };
        assert_eq!(zoned.offset(), jiff::tz::offset(0));
    }

    #[test]
    fn ts_parse_negative_offset_with_minutes() {
        let ts = Ts::parse("2026-01-15T10:30:45.000000-04:30").expect("parses");
        let Ts::Offset(zoned) = ts else {
            panic!("verwacht Ts::Offset")
        };
        let expected = jiff::tz::Offset::from_seconds(-(4 * 3600 + 30 * 60)).unwrap();
        assert_eq!(zoned.offset(), expected);
    }

    #[test]
    fn ts_parse_monolog1_has_no_zone() {
        let ts = Ts::parse("2015-05-01 12:34:56").expect("parses");
        assert_eq!(
            ts,
            Ts::NoZone(jiff::civil::date(2015, 5, 1).at(12, 34, 56, 0))
        );
    }

    #[test]
    fn ts_parse_rejects_monolog1_with_trailing_offset() {
        // Monolog 1 heeft nooit een offset; dat is geen van de drie bekende varianten.
        assert_eq!(Ts::parse("2015-05-01 12:34:56+01:00"), None);
    }

    #[test]
    fn ts_parse_rejects_garbage_and_empty_text() {
        assert_eq!(Ts::parse("not-a-date"), None);
        assert_eq!(Ts::parse(""), None);
        assert_eq!(Ts::parse("2026-01-15T10:30:45"), None); // T-separator vereist offset
        assert_eq!(Ts::parse("2026-13-15T10:30:45+01:00"), None); // maand 13 bestaat niet
        assert_eq!(Ts::parse("2026-01-15T10:30:45.+01:00"), None); // '.' zonder cijfers
        assert_eq!(Ts::parse("2026-01-15T10:30:45+0100"), None); // offset zonder ':'
    }

    #[test]
    fn ts_offset_equality_requires_same_offset_not_just_same_instant() {
        // jiff::Zoned's eigen PartialEq kijkt alleen naar het instant; twee records met
        // hetzelfde instant maar een andere offset-tekst in de log zijn voor ons geen
        // gelijke `Ts`'en (zie de doc-comment bij `impl PartialEq for Ts`).
        let same_instant_utc = Ts::parse("2026-01-15T09:30:45.000000Z").expect("parses");
        let same_instant_plus_one = Ts::parse("2026-01-15T10:30:45.000000+01:00").expect("parses");
        assert_ne!(same_instant_utc, same_instant_plus_one);
    }
}
