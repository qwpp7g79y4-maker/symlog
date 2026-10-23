//! Tijdzone-weergave voor een [`crate::model::Ts`] (task.md T7; blueprint.md
//! `time.rs`).
//!
//! `--tz log` toont de tijd zoals die in de log staat (de offset die al in
//! [`crate::model::Ts::Offset`] vastligt, ongewijzigd); `--tz local` rekent om naar de
//! systeemtijdzone (`jiff::tz::TimeZone::system`, bijvoorbeeld via `$TZ`); `--tz
//! <IANA>` (bijvoorbeeld `Europe/Amsterdam`) rekent om naar die naam. Een
//! [`crate::model::Ts::NoZone`] (Monolog 1, geen offset in de log) kan in geen van de
//! drie modi omgerekend worden — er is geen moment-in-de-tijd om vanuit te rekenen,
//! alleen een kloktijd — en wordt daarom in elke modus hetzelfde getoond, met een
//! zichtbare "zonder zone"-markering (task.md T7 "Doet": "niet stil als UTC
//! aangenomen"). De CLI-laag die `--tz` zelf parset komt later met `clap` (main.rs,
//! nog geen dependency, zie `docs/crates.md`); dit is alleen de weergavelogica.

use crate::model::Ts;

/// Welke tijdzone een [`Ts::Offset`](crate::model::Ts::Offset) voor weergave omrekent
/// (CLI-schets in task.md: `--tz log|local|<IANA>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TzMode {
    /// De offset zoals die al in de log stond, ongewijzigd.
    Log,
    /// De systeemtijdzone (`jiff::tz::TimeZone::system`).
    Local,
    /// Een IANA-tijdzonenaam, bijvoorbeeld `Europe/Amsterdam` of `America/New_York`.
    Named(String),
}

impl TzMode {
    /// Herkent de drie vormen uit de CLI-schets. `log` en `local` zijn de twee
    /// gereserveerde namen; elke andere tekst wordt als IANA-naam behandeld en pas bij
    /// weergave ([`display`]) gevalideerd tegen de tijdzonedatabase, niet hier al.
    pub fn parse(spec: &str) -> TzMode {
        match spec {
            "log" => TzMode::Log,
            "local" => TzMode::Local,
            other => TzMode::Named(other.to_string()),
        }
    }
}

/// Formatteert `ts` volgens `mode` als `Y-m-dTH:i:s[.ffffff]±HH:MM` (met zichtbare
/// offset, task.md T7 "Doet"), of als een "zonder zone"-tekst voor
/// [`Ts::NoZone`](crate::model::Ts::NoZone). `Err` alleen voor [`TzMode::Named`] met
/// een tijdzonenaam die de IANA-tijdzonedatabase niet kent.
pub fn display(ts: &Ts, mode: &TzMode) -> Result<String, jiff::Error> {
    match ts {
        Ts::NoZone(dt) => Ok(format!(
            "{} zonder zone",
            dt.strftime("%Y-%m-%dT%H:%M:%S%.f")
        )),
        Ts::Offset(zoned) => {
            let converted = match mode {
                TzMode::Log => zoned.clone(),
                TzMode::Local => zoned.with_time_zone(jiff::tz::TimeZone::system()),
                TzMode::Named(name) => zoned.in_tz(name)?,
            };
            Ok(converted.strftime("%Y-%m-%dT%H:%M:%S%.f%:z").to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Ts;

    #[test]
    fn tz_mode_parse_recognises_reserved_names() {
        assert_eq!(TzMode::parse("log"), TzMode::Log);
        assert_eq!(TzMode::parse("local"), TzMode::Local);
        assert_eq!(
            TzMode::parse("Europe/Amsterdam"),
            TzMode::Named("Europe/Amsterdam".to_string())
        );
    }

    #[test]
    fn display_log_mode_keeps_the_offset_from_the_log() {
        let ts = Ts::parse("2026-01-15T10:30:45.123456+01:00").expect("parses");
        assert_eq!(
            display(&ts, &TzMode::Log).expect("geen fout"),
            "2026-01-15T10:30:45.123456+01:00"
        );
    }

    #[test]
    fn display_named_mode_converts_to_the_iana_zone() {
        // 2026-01-15T10:30:45+01:00 (CET) is 2026-01-15T09:30:45Z, en op dat moment is
        // het in New York 04:30:45 EST (-05:00) — geen DST in januari.
        let ts = Ts::parse("2026-01-15T10:30:45.000000+01:00").expect("parses");
        assert_eq!(
            display(&ts, &TzMode::Named("America/New_York".to_string())).expect("geen fout"),
            "2026-01-15T04:30:45-05:00"
        );
    }

    #[test]
    fn display_named_mode_rejects_unknown_zone_name() {
        let ts = Ts::parse("2026-01-15T10:30:45.000000+01:00").expect("parses");
        assert!(display(&ts, &TzMode::Named("Nergens/Bestaat".to_string())).is_err());
    }

    #[test]
    fn display_no_zone_ignores_the_mode() {
        let ts = Ts::parse("2015-05-01 12:34:56").expect("parses");
        let expected = "2015-05-01T12:34:56 zonder zone";
        assert_eq!(display(&ts, &TzMode::Log).expect("geen fout"), expected);
        assert_eq!(display(&ts, &TzMode::Local).expect("geen fout"), expected);
        assert_eq!(
            display(&ts, &TzMode::Named("America/New_York".to_string())).expect("geen fout"),
            expected
        );
    }
}
