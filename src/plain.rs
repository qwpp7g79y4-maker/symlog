//! Niet-TTY-uitvoer (task.md T13; blueprint.md `plain.rs`).
//!
//! [`render_line`] is de CLI-brede opvolger van [`crate::parse::raw::ParsedRecord::to_plain_line`]
//! (die zelf al zei: "de volledige plain-modus met filters en kolomopmaak komt in
//! T13"): één regel per record, met frames ingesprongen op eigen regels in plaats van
//! de ruwe vervolgregels uit het record zelf (task.md T13 "Doet"). Voor een
//! LineFormatter-record met een stacktrace bevat `record.message` de ruwe,
//! meerregelige Monolog-tekst al (task.md T4: `%message%` is een kale
//! stringvervanging); die ruwe tekst wordt hier bewust niet herhaald naast de
//! structured [`crate::model::Frame`]-lijst (dat zou dezelfde bestand/regel/functie-
//! informatie dubbel tonen), dus de koptekst toont alleen de eerste regel van
//! `message` en de frames komen daaronder, in hetzelfde ingesprongen formaat voor
//! beide bronformaten (LineFormatter én JsonFormatter, task.md T4/T5). Dat laat alleen
//! de "losse" omringende tekst weg die geen frame is (bijvoorbeeld Monolog's eigen
//! `Stack trace:`- en `[previous exception]`-regels); die blijft wel zichtbaar in de
//! TUI (task.md T8's detailpaneel toont `message` in zijn geheel).
//!
//! [`ParsedRecord::Raw`] krijgt dezelfde markering als
//! [`crate::parse::raw::RawRecord::to_plain_line`] (task.md T6): een kapotte regel heeft
//! geen frames en geen kanaal/niveau om los te koppen. Geheimen in die tekst worden
//! hier alsnog gemaskeerd; `to_plain_line` zelf blijft de ongemaskeerde golden-vorm.
//!
//! [`PlainPrinter`] is de incrementele kern die zowel het eenmalige dump (geen
//! `--follow`) als de streaminglus (`--follow` streamt, task.md T13 "Doet") voedt: hij
//! onthoudt tot welk record en welke scheidingsregel ([`crate::follow::Separator`],
//! task.md T10) al afgedrukt zijn, en [`PlainPrinter::flush`] is veilig herhaald aan te
//! roepen terwijl [`crate::follow::Follower`] groeit.

use crate::filter::FilterSpec;
use crate::follow::{Follower, Separator, SeparatorKind};
use crate::model::{Frame, Level, Record};
use crate::parse::raw::ParsedRecord;
use std::io::{self, Write};

/// ANSI SGR-kleurcode per niveau (task.md T13 "Doet": "kleur alleen bij TTY en zonder
/// `NO_COLOR`"). Dezelfde achtliggende paletkeuze als [`crate::tui::view`]'s
/// `level_style` (niet hergebruikt: dat geeft een `ratatui::style::Style` voor een
/// backend-buffer, hier gaat de tekst rechtstreeks met ruwe ANSI-codes naar stdout).
fn level_color_code(level: Level) -> &'static str {
    match level {
        Level::Debug => "90",
        Level::Info => "34",
        Level::Notice => "36",
        Level::Warning => "33",
        Level::Error => "1;31",
        Level::Critical => "1;91",
        Level::Alert => "1;35",
        Level::Emergency => "1;95",
    }
}

fn colorize(text: &str, level: Level, colors_enabled: bool) -> String {
    if !colors_enabled {
        return text.to_string();
    }
    format!("\u{1b}[{}m{text}\u{1b}[0m", level_color_code(level))
}

/// Of kleur aan mag (task.md T13 "Doet"): alleen als stdout een tty is, `--no-color`
/// niet gezet is, en `NO_COLOR` niet gezet is (<https://no-color.org/>: élke waarde,
/// ook leeg, schakelt uit). Puur, met de omgeving als parameter in plaats van zelf
/// gelezen, zodat dit zonder write naar de echte omgeving getest kan worden.
pub fn colors_enabled(stdout_is_tty: bool, no_color_flag: bool, no_color_env_set: bool) -> bool {
    stdout_is_tty && !no_color_flag && !no_color_env_set
}

/// Eén frame, ingesprongen en genummerd zoals Monolog/PHP's eigen `#N`-notatie, maar
/// met de al gescheiden velden (task.md T4) in plaats van een los te parsen string.
fn frame_line(index: usize, frame: &Frame) -> String {
    match (&frame.path, frame.line) {
        (Some(path), Some(line)) => format!("    #{index} {path}:{line}: {}", frame.func),
        _ => format!("    #{index} {}", frame.func),
    }
}

/// Eerste fysieke regel van `message` (zie de moduledoc hierboven voor waarom de rest
/// van een meerregelig bericht hier niet herhaald wordt).
fn first_line(message: &str) -> &str {
    message.split('\n').next().unwrap_or(message)
}

fn render_record(record: &Record, colors_enabled: bool, redact: bool) -> String {
    let header = format!(
        "[{}] {}.{}: {}",
        crate::redact::for_display(&record.datetime, redact),
        crate::redact::for_display(&record.channel, redact),
        record.level.as_str(),
        crate::redact::for_display(first_line(&record.message), redact)
    );
    let mut text = colorize(&header, record.level, colors_enabled);
    for (i, frame) in record.frames.iter().enumerate() {
        text.push('\n');
        text.push_str(&crate::redact::for_display(&frame_line(i, frame), redact));
    }
    text
}

/// Eén of meer regels (koptekst plus ingesprongen frames) voor `parsed` (task.md T13
/// "Doet"). Bevat geen afsluitende `\n`; de aanroeper (bijvoorbeeld [`PlainPrinter`])
/// voegt die toe.
pub fn render_line(parsed: &ParsedRecord, colors_enabled: bool) -> String {
    render_line_with(parsed, colors_enabled, true)
}

fn render_line_with(parsed: &ParsedRecord, colors_enabled: bool, redact: bool) -> String {
    match parsed {
        ParsedRecord::Record(record) => render_record(record, colors_enabled, redact),
        ParsedRecord::Raw(raw) => format!(
            "RAW[{}] {}",
            raw.reason.as_str(),
            crate::redact::for_display(&raw.text, redact)
        ),
    }
}

/// Scheidingsregel-tekst (task.md T10 "Doet": exacte teksten "— geroteerd —" en
/// "— afgekapt —").
fn separator_line(kind: SeparatorKind) -> &'static str {
    match kind {
        SeparatorKind::Rotated => "— geroteerd —",
        SeparatorKind::Truncated => "— afgekapt —",
    }
}

/// Incrementele plain-afdrukker over een [`Follower`] (task.md T13 "Doet": "`--follow`
/// werkt ook in plain (streamt)"). Onthoudt tot welk (doorlopend genummerd) record en
/// welke scheidingsregel al geschreven zijn, zodat herhaald aanroepen na een nieuwe
/// `tick()` alleen het nieuwe stuk schrijft, nooit dubbel.
#[derive(Debug)]
pub struct PlainPrinter {
    next_record: usize,
    next_separator: usize,
    /// Standaard aan. `--no-redact` zet dit uit; het bronbestand blijft hoe dan ook
    /// onaangeroerd.
    redact: bool,
}

impl Default for PlainPrinter {
    fn default() -> Self {
        Self {
            next_record: 0,
            next_separator: 0,
            redact: true,
        }
    }
}

impl PlainPrinter {
    pub fn new() -> PlainPrinter {
        PlainPrinter::default()
    }

    /// Zet maskeren aan of uit voor de volgende `flush`-aanroepen.
    pub fn set_redact(&mut self, redact: bool) {
        self.redact = redact;
    }

    /// Schrijft alle records en scheidingsregels die sinds de vorige aanroep nieuw
    /// beschikbaar zijn gekomen in `follower`, gefilterd door `spec`. Records die niet
    /// aan `spec` voldoen worden overgeslagen maar tellen wel mee voor de voortgang
    /// (ze worden niet opnieuw bekeken bij een volgende `flush`).
    pub fn flush(
        &mut self,
        follower: &Follower,
        spec: &FilterSpec,
        colors_enabled: bool,
        out: &mut impl Write,
    ) -> io::Result<()> {
        let total = follower.total_record_count();
        let separators = follower.separators();
        loop {
            while self.next_separator < separators.len()
                && due(&separators[self.next_separator], self.next_record)
            {
                let separator = &separators[self.next_separator];
                writeln!(out, "{}", separator_line(separator.kind))?;
                // Bij afkappen (task.md T10) telt `Follower` de globale recordindex
                // vanaf `before_record` opnieuw: zonder deze regel zou `next_record`
                // op zijn oude, hogere stand blijven staan en de nieuwe generatie
                // (die weer bij index `before_record` begint) voorgoed overslaan.
                self.next_record = separator.before_record;
                self.next_separator += 1;
            }
            if self.next_record >= total {
                break;
            }
            let Some(bytes) = follower.record_bytes(self.next_record)? else {
                break;
            };
            let parsed = crate::parse::parse_record_lossy(&bytes);
            if spec.matches_parsed(&parsed) {
                writeln!(
                    out,
                    "{}",
                    render_line_with(&parsed, colors_enabled, self.redact)
                )?;
            }
            self.next_record += 1;
        }
        Ok(())
    }
}

fn due(separator: &Separator, next_record: usize) -> bool {
    separator.before_record <= next_record
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::{ChannelFilter, LevelFilter};
    use crate::model::Ts;
    use std::fs;
    use std::path::Path;
    use std::time::Duration;

    fn frame(path: Option<&str>, line: Option<u32>, func: &str) -> Frame {
        Frame {
            path: path.map(str::to_string),
            line,
            func: func.to_string(),
        }
    }

    fn sample_record(message: &str, frames: Vec<Frame>) -> Record {
        Record {
            datetime: "2026-01-15T10:30:45.000000+01:00".to_string(),
            ts: Ts::parse("2026-01-15T10:30:45.000000+01:00"),
            channel: "app".to_string(),
            level: Level::Error,
            message: message.to_string(),
            context: serde_json::json!([]),
            extra: serde_json::json!([]),
            frames,
        }
    }

    #[test]
    fn colors_enabled_requires_a_tty_and_no_no_color() {
        assert!(colors_enabled(true, false, false));
        assert!(!colors_enabled(false, false, false), "geen tty");
        assert!(!colors_enabled(true, true, false), "--no-color");
        assert!(!colors_enabled(true, false, true), "NO_COLOR gezet");
    }

    #[test]
    fn render_record_without_frames_is_a_single_line() {
        let record = sample_record("boom", Vec::new());
        let line = render_line(&ParsedRecord::Record(record), false);
        assert_eq!(line, "[2026-01-15T10:30:45.000000+01:00] app.ERROR: boom");
    }

    #[test]
    fn render_record_with_frames_indents_each_frame_on_its_own_line() {
        let record = sample_record(
            "Uncaught RuntimeException: boom\nStack trace:\n#0 /a/b.php(12): f()\n#1 {main}",
            vec![
                frame(Some("/a/b.php"), Some(12), "f()"),
                frame(None, None, "{main}"),
            ],
        );
        let line = render_line(&ParsedRecord::Record(record), false);
        assert_eq!(
            line,
            "[2026-01-15T10:30:45.000000+01:00] app.ERROR: Uncaught RuntimeException: boom\n\
             \x20   #0 /a/b.php:12: f()\n\
             \x20   #1 {main}"
        );
    }

    #[test]
    fn render_record_colorizes_the_header_only_when_enabled() {
        let record = sample_record("boom", Vec::new());
        let parsed = ParsedRecord::Record(record);
        let colored = render_line(&parsed, true);
        assert!(colored.starts_with("\u{1b}[1;31m"));
        assert!(colored.ends_with("\u{1b}[0m"));
        let plain = render_line(&parsed, false);
        assert!(!plain.contains('\u{1b}'));
    }

    #[test]
    fn render_raw_is_unaffected_by_color() {
        let raw = crate::parse::parse_record_lossy(b"geen enkele kop herkend");
        assert_eq!(render_line(&raw, true), render_line(&raw, false));
    }

    #[test]
    fn render_record_sanitizes_ansi_and_nul_in_the_message() {
        let record = sample_record("a\u{0}b\u{1b}[31mc", Vec::new());
        let line = render_line(&ParsedRecord::Record(record), false);
        assert!(!line.contains('\u{0}'));
        assert!(!line.contains('\u{1b}'));
    }

    fn write_file(path: &Path, content: &[u8]) {
        fs::File::create(path)
            .expect("create")
            .write_all(content)
            .expect("write");
    }

    #[test]
    fn plain_printer_prints_each_record_exactly_once_across_flushes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("app.log");
        write_file(&path, b"[2026-01-01 00:00:00] a.INFO: one [] []\n");

        let mut follower = Follower::open(&path);
        follower.tick().expect("tick");
        let mut printer = PlainPrinter::new();
        let mut out = Vec::new();
        printer
            .flush(&follower, &FilterSpec::default(), false, &mut out)
            .expect("flush");
        // Niets nieuws: een tweede flush zonder groei mag niets extra schrijven.
        printer
            .flush(&follower, &FilterSpec::default(), false, &mut out)
            .expect("flush");
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "[2026-01-01 00:00:00] a.INFO: one\n"
        );
    }

    #[test]
    fn plain_printer_streams_growth_without_repeating_earlier_records() {
        use std::fs::OpenOptions;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("app.log");
        write_file(&path, b"[2026-01-01 00:00:00] a.INFO: one [] []\n");

        let mut follower = Follower::open(&path);
        follower.tick().expect("tick");
        let mut printer = PlainPrinter::new();
        let mut out = Vec::new();
        printer
            .flush(&follower, &FilterSpec::default(), false, &mut out)
            .expect("flush");

        OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open voor append")
            .write_all(b"[2026-01-01 00:00:01] a.INFO: two [] []\n")
            .expect("append");
        follower.tick().expect("tick");
        printer
            .flush(&follower, &FilterSpec::default(), false, &mut out)
            .expect("flush");

        assert_eq!(
            String::from_utf8(out).unwrap(),
            "[2026-01-01 00:00:00] a.INFO: one\n[2026-01-01 00:00:01] a.INFO: two\n"
        );
    }

    #[test]
    fn plain_printer_applies_the_filter_but_still_advances_past_skipped_records() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("app.log");
        write_file(
            &path,
            b"[2026-01-01 00:00:00] a.DEBUG: skip me [] []\n\
              [2026-01-01 00:00:01] a.ERROR: keep me [] []\n",
        );

        let mut follower = Follower::open(&path);
        follower.tick().expect("tick");
        let spec = FilterSpec {
            channels: ChannelFilter::default(),
            levels: Some(LevelFilter::parse(">=error").unwrap()),
            search: None,
        };
        let mut printer = PlainPrinter::new();
        let mut out = Vec::new();
        printer
            .flush(&follower, &spec, false, &mut out)
            .expect("flush");
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "[2026-01-01 00:00:01] a.ERROR: keep me\n"
        );
    }

    #[test]
    fn plain_printer_shows_a_separator_line_on_truncation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("app.log");
        // De eerste regel moet strikt meer bytes zijn dan de tweede: `Follower` herkent
        // afkappen aan "zelfde inode, size < eerder geïndexeerde eind" (blueprint.md
        // T10), niet aan de inhoud. Een `write_file` (die intern `File::create`, dus
        // afkapt-en-herschrijft) met een *langere* nieuwe regel zou voor de follower
        // als gewone groei lijken, niet als afkapping.
        write_file(
            &path,
            b"[2026-01-01 00:00:00] a.INFO: one two three four [] []\n",
        );

        let mut follower = Follower::open(&path);
        follower.tick().expect("tick");
        let mut printer = PlainPrinter::new();
        let mut out = Vec::new();
        printer
            .flush(&follower, &FilterSpec::default(), false, &mut out)
            .expect("flush");

        write_file(&path, b"[2026-01-01 00:00:02] a.INFO: x [] []\n");
        follower.tick().expect("tick");
        printer
            .flush(&follower, &FilterSpec::default(), false, &mut out)
            .expect("flush");

        assert_eq!(
            String::from_utf8(out).unwrap(),
            "[2026-01-01 00:00:00] a.INFO: one two three four\n\
             — afgekapt —\n\
             [2026-01-01 00:00:02] a.INFO: x\n"
        );
    }

    #[test]
    fn plain_printer_shows_a_separator_line_on_rotation_even_with_no_new_record_yet() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("app.log");
        write_file(&path, b"[2026-01-01 00:00:00] a.INFO: one [] []\n");

        let mut follower = Follower::with_pending_tail_timeout(&path, Duration::from_secs(3600));
        follower.tick().expect("tick");
        let mut printer = PlainPrinter::new();
        let mut out = Vec::new();
        printer
            .flush(&follower, &FilterSpec::default(), false, &mut out)
            .expect("flush");

        std::fs::rename(&path, dir.path().join("app.log.1")).expect("rename");
        write_file(&path, b"");
        follower.tick().expect("tick");
        // Nog geen enkel nieuw record in de nieuwe generatie, maar de scheidingsregel
        // hoort al wel getoond te worden (hij markeert de knip, niet het eerste nieuwe
        // record).
        printer
            .flush(&follower, &FilterSpec::default(), false, &mut out)
            .expect("flush");

        assert_eq!(
            String::from_utf8(out).unwrap(),
            "[2026-01-01 00:00:00] a.INFO: one\n— geroteerd —\n"
        );
    }
}
