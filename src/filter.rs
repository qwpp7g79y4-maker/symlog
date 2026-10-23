//! Filters: kanaal, niveau en regex-zoeken (task.md T7; blueprint.md `filter.rs`).
//!
//! De drie filters zijn onafhankelijk opbouwbaar en worden samen toegepast met
//! [`FilterSpec::matches`]: een record moet aan **alle** ingestelde filters voldoen
//! (kanaal *en* niveau *en* zoekterm), niet aan één ervan. Een filter dat niet is
//! ingesteld (bijvoorbeeld geen `--level`) telt niet mee, zodat `FilterSpec::default()`
//! alles doorlaat.
//!
//! [`FilterSpec::matches`] werkt op een geparste [`crate::model::Record`].
//! [`FilterSpec::matches_parsed`] is de tegenhanger voor
//! [`crate::parse::raw::ParsedRecord`] (dus óók Raw-records, task.md T6): een
//! Raw-record valt nooit weg door het kanaal- of niveaufilter ("ze vallen niet weg bij
//! een niveaufilter tenzij `--hide-raw`", task.md T6 "Doet"), maar blijft wel
//! onderworpen aan de zoekterm (Raw-records zijn "doorzoekbaar", task.md T6).
//!
//! De achtergrond-filterthread met annulering bij een nieuwe `FilterSpec`
//! (blueprint.md: "generatienummer") hoort bij de TUI-event-loop (T8) die de
//! `FilterSpec` daadwerkelijk live wijzigt; dat threadmodel komt pas zinvol tot zijn
//! recht zodra er een consument is die hem aanstuurt. [`filtered_offsets`] hier is de
//! pure, synchrone kern die T8 in zo'n achtergrondthread kan aanroepen: hij werkt op
//! een snapshot van de huidige index ([`crate::reader::Reader::record_count`]) en is
//! daardoor vrij herhaalbaar aan te roepen terwijl de achtergrondindexer nog groeit
//! (task.md T7 "Doet": "een gefilterde offset-lijst die incrementeel wordt
//! opgebouwd").

use crate::model::{Level, Record};
use crate::parse::raw::ParsedRecord;
use crate::reader::Reader;
use regex::Regex;
use std::io;

/// Kanaalfilter (task.md T7): een lijst van namen om te tonen, en een lijst van namen
/// om uit te sluiten (`!doctrine`). Beide kunnen tegelijk gebruikt worden; uitsluiten
/// wint altijd, ook als een kanaal toevallig in beide lijsten voorkomt.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChannelFilter {
    include: Vec<String>,
    exclude: Vec<String>,
}

impl ChannelFilter {
    /// Bouwt een filter uit CLI-specs zoals `["app", "!doctrine"]` (task.md T7: "ook
    /// `!doctrine` om uit te sluiten"). Een spec die met `!` begint, sluit uit; alle
    /// andere specs sluiten in.
    pub fn from_specs<I, S>(specs: I) -> ChannelFilter
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut include = Vec::new();
        let mut exclude = Vec::new();
        for spec in specs {
            let spec = spec.as_ref();
            match spec.strip_prefix('!') {
                Some(excluded) => exclude.push(excluded.to_string()),
                None => include.push(spec.to_string()),
            }
        }
        ChannelFilter { include, exclude }
    }

    /// Of `channel` door dit filter komt: niet uitgesloten, en (als er expliciete
    /// inclusies zijn) één van die inclusies.
    pub fn matches(&self, channel: &str) -> bool {
        if self.exclude.iter().any(|c| c == channel) {
            return false;
        }
        self.include.is_empty() || self.include.iter().any(|c| c == channel)
    }
}

/// Niveaufilter (task.md T7): een vaste lijst (`error,critical`) of een ondergrens
/// (`>=warning`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LevelFilter {
    /// `>=L`: `L` en elk ernstiger niveau.
    AtLeast(Level),
    /// Een expliciete, kommagescheiden lijst van niveaus.
    OneOf(Vec<Level>),
}

impl LevelFilter {
    /// Parseert de CLI-vorm `>=warning` of `error,critical` (hoofdletterongevoelig,
    /// zie [`Level::parse`]). `None` als geen van beide vormen herkend wordt, of als
    /// de lijst leeg is of een onbekend niveau bevat.
    pub fn parse(spec: &str) -> Option<LevelFilter> {
        if let Some(rest) = spec.strip_prefix(">=") {
            return Level::parse(rest.trim()).map(LevelFilter::AtLeast);
        }
        let mut levels = Vec::new();
        for part in spec.split(',') {
            levels.push(Level::parse(part.trim())?);
        }
        if levels.is_empty() {
            return None;
        }
        Some(LevelFilter::OneOf(levels))
    }

    /// Of `level` door dit filter komt (steunt op de ernstvolgorde van [`Level`] voor
    /// `AtLeast`, zie de doc-comment daar).
    pub fn matches(&self, level: Level) -> bool {
        match self {
            LevelFilter::AtLeast(min) => level >= *min,
            LevelFilter::OneOf(levels) => levels.contains(&level),
        }
    }
}

/// Alle filters samen (task.md T7; blueprint.md `filter.rs`). `Default` laat alles
/// door: geen kanaal-, niveau- of zoekfilter ingesteld.
#[derive(Debug, Clone, Default)]
pub struct FilterSpec {
    pub channels: ChannelFilter,
    pub levels: Option<LevelFilter>,
    pub search: Option<Regex>,
}

impl FilterSpec {
    /// Of `record` aan **alle** ingestelde filters voldoet.
    pub fn matches(&self, record: &Record) -> bool {
        self.channels.matches(&record.channel)
            && self.levels.as_ref().is_none_or(|l| l.matches(record.level))
            && self
                .search
                .as_ref()
                .is_none_or(|re| self.search_matches(re, record))
    }

    /// Regex-zoeken over message + context (task.md T7 "Doet"), niet over `extra`.
    /// Context wordt als JSON-tekst doorzocht (niet alleen stringwaarden), zodat een
    /// zoekterm ook een sleutel of een geneste waarde kan raken.
    fn search_matches(&self, re: &Regex, record: &Record) -> bool {
        re.is_match(&record.message) || re.is_match(&record.context.to_string())
    }

    /// Of `parsed` aan alle ingestelde filters voldoet. Een [`ParsedRecord::Raw`]
    /// (task.md T6) wordt nooit door het kanaal- of niveaufilter weggefilterd — het
    /// heeft geen kanaal/niveau om op te filteren, en Raw-records blijven altijd
    /// zichtbaar tenzij `--hide-raw` (dat de TUI, T8, apart afdwingt) — maar is wel
    /// onderworpen aan de zoekterm, over de ruwe (gesaneerde) tekst.
    pub fn matches_parsed(&self, parsed: &ParsedRecord) -> bool {
        match parsed {
            ParsedRecord::Record(record) => self.matches(record),
            ParsedRecord::Raw(raw) => self.search.as_ref().is_none_or(|re| re.is_match(&raw.text)),
        }
    }
}

/// Past `spec` toe op alle records die de achtergrondindexer van `reader` tot nu toe
/// gevonden heeft, en levert de offsets van de records die matchen (task.md T7
/// "Doet": "een gefilterde offset-lijst"). Pure, synchrone snapshot-functie: opnieuw
/// aanroepen terwijl de index nog groeit, levert een uitgebreide lijst op zonder eerder
/// werk te herhalen op een manier die de aanroeper zelf niet in de hand heeft (zie de
/// moduledoc hierboven voor de T8-achtergrondthread die dit incrementeel aanstuurt).
pub fn filtered_offsets(reader: &Reader, spec: &FilterSpec) -> io::Result<Vec<u64>> {
    let mut offsets = Vec::new();
    for i in 0..reader.record_count() {
        let Some(bytes) = reader.record_bytes(i)? else {
            break;
        };
        let Some(offset) = reader.index().offset(i) else {
            break;
        };
        let parsed = crate::parse::parse_record_lossy(&bytes);
        if spec.matches_parsed(&parsed) {
            offsets.push(offset);
        }
    }
    Ok(offsets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::line::parse_line;
    use std::fs;
    use std::path::Path;

    fn f1_records() -> Vec<Record> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/line-basic.log");
        let content = fs::read_to_string(&path).expect("F1 fixture");
        crate::parse::line::split_records(&content)
            .into_iter()
            .map(|block| parse_line(block).expect("F1 parseert altijd als lineformat"))
            .collect()
    }

    // --- kanaalfilter ---

    #[test]
    fn channel_filter_default_matches_everything() {
        let filter = ChannelFilter::default();
        for record in f1_records() {
            assert!(filter.matches(&record.channel));
        }
    }

    #[test]
    fn channel_filter_includes_a_single_channel() {
        let filter = ChannelFilter::from_specs(["app"]);
        let matched: Vec<_> = f1_records()
            .into_iter()
            .filter(|r| filter.matches(&r.channel))
            .map(|r| r.channel)
            .collect();
        assert_eq!(matched, vec!["app", "app"]);
    }

    #[test]
    fn channel_filter_includes_multiple_channels() {
        let filter = ChannelFilter::from_specs(["app", "security"]);
        let matched: usize = f1_records()
            .into_iter()
            .filter(|r| filter.matches(&r.channel))
            .count();
        assert_eq!(matched, 3); // app, security, app
    }

    #[test]
    fn channel_filter_excludes_a_channel() {
        let filter = ChannelFilter::from_specs(["!doctrine"]);
        let matched: usize = f1_records()
            .into_iter()
            .filter(|r| filter.matches(&r.channel))
            .count();
        assert_eq!(matched, 7); // alle 8 behalve doctrine
    }

    #[test]
    fn channel_filter_exclude_wins_over_include() {
        let filter = ChannelFilter::from_specs(["app", "!app"]);
        assert!(!filter.matches("app"));
    }

    // --- niveaufilter ---

    #[test]
    fn level_filter_parses_a_list() {
        let filter = LevelFilter::parse("error,critical").expect("parseert");
        assert_eq!(
            filter,
            LevelFilter::OneOf(vec![Level::Error, Level::Critical])
        );
        assert!(filter.matches(Level::Error));
        assert!(filter.matches(Level::Critical));
        assert!(!filter.matches(Level::Warning));
    }

    #[test]
    fn level_filter_parses_a_list_case_insensitively() {
        let filter = LevelFilter::parse("Error, CRITICAL").expect("parseert");
        assert!(filter.matches(Level::Error));
        assert!(filter.matches(Level::Critical));
    }

    #[test]
    fn level_filter_parses_at_least() {
        let filter = LevelFilter::parse(">=warning").expect("parseert");
        assert_eq!(filter, LevelFilter::AtLeast(Level::Warning));
        assert!(!filter.matches(Level::Notice));
        assert!(filter.matches(Level::Warning));
        assert!(filter.matches(Level::Error));
        assert!(filter.matches(Level::Emergency));
    }

    #[test]
    fn level_filter_rejects_unknown_level_and_empty_list() {
        assert_eq!(LevelFilter::parse(">=trace"), None);
        assert_eq!(LevelFilter::parse("trace"), None);
        assert_eq!(LevelFilter::parse(""), None);
        assert_eq!(LevelFilter::parse(","), None);
    }

    // --- zoekfilter ---

    #[test]
    fn search_matches_in_message() {
        let spec = FilterSpec {
            search: Some(Regex::new("cache").unwrap()),
            ..Default::default()
        };
        let matched: usize = f1_records().into_iter().filter(|r| spec.matches(r)).count();
        assert_eq!(matched, 1); // "User cache warmed"
    }

    #[test]
    fn search_matches_in_context() {
        let spec = FilterSpec {
            search: Some(Regex::new("demo").unwrap()),
            ..Default::default()
        };
        let matched: usize = f1_records().into_iter().filter(|r| spec.matches(r)).count();
        assert_eq!(matched, 1); // context: {"username":"demo"}
    }

    #[test]
    fn search_matches_nothing_for_an_absent_term() {
        let spec = FilterSpec {
            search: Some(Regex::new("nergens-te-vinden").unwrap()),
            ..Default::default()
        };
        assert!(f1_records().into_iter().all(|r| !spec.matches(&r)));
    }

    // --- combinaties: kanaal x niveau x zoek (task.md T7 "Klaar als") ---

    #[test]
    fn combination_channel_and_level() {
        let spec = FilterSpec {
            channels: ChannelFilter::from_specs(["app"]),
            levels: Some(LevelFilter::parse(">=alert").unwrap()),
            search: None,
        };
        let matched: Vec<_> = f1_records()
            .into_iter()
            .filter(|r| spec.matches(r))
            .map(|r| r.level)
            .collect();
        // app.DEBUG valt af op niveau, app.ALERT blijft over.
        assert_eq!(matched, vec![Level::Alert]);
    }

    #[test]
    fn combination_channel_and_search() {
        let spec = FilterSpec {
            channels: ChannelFilter::from_specs(["request"]),
            levels: None,
            search: Some(Regex::new("unreachable").unwrap()),
        };
        let matched: usize = f1_records().into_iter().filter(|r| spec.matches(r)).count();
        assert_eq!(matched, 1); // request.EMERGENCY: Service unreachable
    }

    #[test]
    fn combination_level_and_search() {
        let spec = FilterSpec {
            channels: ChannelFilter::default(),
            levels: Some(LevelFilter::parse(">=error").unwrap()),
            search: Some(Regex::new("sync").unwrap()),
        };
        let matched: usize = f1_records().into_iter().filter(|r| spec.matches(r)).count();
        assert_eq!(matched, 1); // console.CRITICAL: Command failed {"command":"app:sync"}
    }

    #[test]
    fn combination_channel_level_and_search_all_match() {
        let spec = FilterSpec {
            channels: ChannelFilter::from_specs(["console"]),
            levels: Some(LevelFilter::parse("critical").unwrap()),
            search: Some(Regex::new("app:sync").unwrap()),
        };
        let matched: usize = f1_records().into_iter().filter(|r| spec.matches(r)).count();
        assert_eq!(matched, 1);
    }

    #[test]
    fn combination_channel_level_and_search_one_mismatch_excludes_the_record() {
        // Zelfde record als de vorige test, maar nu met een kanaal dat niet past: geen
        // van de drie filters mag los voldoende zijn.
        let spec = FilterSpec {
            channels: ChannelFilter::from_specs(["app"]),
            levels: Some(LevelFilter::parse("critical").unwrap()),
            search: Some(Regex::new("app:sync").unwrap()),
        };
        assert!(f1_records().into_iter().all(|r| !spec.matches(&r)));
    }

    #[test]
    fn default_filter_spec_matches_every_f1_record() {
        let spec = FilterSpec::default();
        assert!(f1_records().into_iter().all(|r| spec.matches(&r)));
    }

    // --- Raw-records (task.md T6/T7): nooit weggefilterd door kanaal/niveau ---

    #[test]
    fn matches_parsed_never_drops_raw_on_channel_or_level_filters() {
        let spec = FilterSpec {
            channels: ChannelFilter::from_specs(["app"]),
            levels: Some(LevelFilter::parse("critical").unwrap()),
            search: None,
        };
        let raw = crate::parse::parse_record_lossy(b"geen enkele kop herkend");
        assert!(raw.as_raw().is_some());
        assert!(spec.matches_parsed(&raw));
    }

    #[test]
    fn matches_parsed_still_applies_search_to_raw_text() {
        let spec = FilterSpec {
            search: Some(Regex::new("herkend").unwrap()),
            ..Default::default()
        };
        let raw = crate::parse::parse_record_lossy(b"geen enkele kop herkend");
        assert!(raw.as_raw().is_some());
        assert!(spec.matches_parsed(&raw));

        let other = crate::parse::parse_record_lossy(b"iets anders zonder die term");
        assert!(other.as_raw().is_some());
        assert!(!spec.matches_parsed(&other));
    }

    #[test]
    fn matches_parsed_applies_all_filters_to_a_regular_record() {
        let spec = FilterSpec {
            channels: ChannelFilter::from_specs(["!doctrine"]),
            levels: None,
            search: None,
        };
        for record in f1_records() {
            let parsed = ParsedRecord::Record(record.clone());
            assert_eq!(spec.matches_parsed(&parsed), spec.matches(&record));
        }
    }
}
