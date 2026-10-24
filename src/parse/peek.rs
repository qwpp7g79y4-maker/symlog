//! Kop-only classificatie voor het niveau- en kanaalfilter.
//!
//! [`crate::filter::filtered_offsets`] hoeft bij een filter zonder zoekterm alleen
//! niveau en kanaal te weten. De volledige parse bouwt daarvoor alsnog context-JSON,
//! frames en een tijdzone (`Ts`) — dat is het dure deel van `--level error` op een
//! groot bestand (task.md T12, `docs/perf.md`). Deze module herhaalt alleen de
//! beslisregels die bepalen of [`crate::parse::parse_record`] slaagt en welk niveau
//! en kanaal daar dan uit komen.
//!
//! [`Peek::Unsure`] betekent: dit blok is te ongewoon om hier zeker over te zijn
//! (bijvoorbeeld een UTF-16-surrogaatpaar). De aanroeper valt dan terug op de
//! volledige parse, zodat het resultaat hetzelfde blijft.

use std::borrow::Cow;

use crate::model::Level;
use crate::parse::line;

#[cfg(test)]
use crate::parse::parse_record_lossy;
#[cfg(test)]
use crate::parse::raw::ParsedRecord;

const MAX_DEPTH: usize = 128;

enum Fail {
    /// Zeker geen geldig record (ongeldige JSON, of een kop die de parser afwijst).
    Invalid,
    /// Niet zeker; de volledige parse moet beslissen.
    Unsure,
}

/// Uitkomst van [`classify`], met dezelfde filterbetekenis als een geparst record.
#[derive(Debug)]
pub(crate) enum Peek<'a> {
    /// De volledige parser zou een record met dit niveau opleveren.
    /// `channel` is alleen gevuld als `need_channel` waar was; anders een lege
    /// leenstring die de aanroeper niet gebruikt.
    Parsed { level: Level, channel: Cow<'a, str> },
    /// De volledige parser zou op Raw uitkomen. Een niveau- of kanaalfilter laat
    /// Raw staan (task.md T6).
    Raw,
    /// Geen zekere uitspraak. De aanroeper parset het blok volledig.
    Unsure,
}

/// Classificeert `bytes` zoals [`parse_record_lossy`], maar zonder recordopbouw.
/// `need_channel` vraagt het kanaal erbij; zonder die vraag wordt het kanaal wel
/// gecontroleerd (het moet een string zijn bij JSON) maar niet bewaard.
pub(crate) fn classify(bytes: &[u8], need_channel: bool) -> Peek<'_> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Peek::Unsure;
    };
    match text.as_bytes().first().copied() {
        Some(b'[') => match line::peek_level_channel(text) {
            Ok(Some(found)) => Peek::Parsed {
                level: found.level,
                channel: Cow::Borrowed(found.channel),
            },
            Ok(None) => Peek::Raw,
            Err(()) => Peek::Unsure,
        },
        Some(b'{') => match peek_json(text, need_channel) {
            Ok(Some((level, channel))) => Peek::Parsed { level, channel },
            Ok(None) => Peek::Raw,
            Err(Fail::Invalid) => Peek::Raw,
            Err(Fail::Unsure) => Peek::Unsure,
        },
        _ => Peek::Raw,
    }
}

/// Of `text` precies één JSON-waarde is (plus witruimte eromheen), zoals
/// `serde_json::from_str` die accepteert. `Err(())` als de scanner het niet zeker
/// weet; de lijnparser valt dan terug op de volledige parse.
pub(crate) fn accepts_one_json_value(text: &str) -> Result<bool, ()> {
    if text == "[]" || text == "{}" {
        return Ok(true);
    }
    let mut scan = Scan {
        b: text.as_bytes(),
        i: 0,
        depth: 0,
    };
    match scan.one_value_to_end() {
        Ok(()) => Ok(true),
        Err(Fail::Invalid) => Ok(false),
        Err(Fail::Unsure) => Err(()),
    }
}

struct FoundJson<'a> {
    level: Level,
    channel: Cow<'a, str>,
}

fn peek_json(text: &str, need_channel: bool) -> Result<Option<(Level, Cow<'_, str>)>, Fail> {
    let text = strip_line_ending(text);
    let mut scan = Scan {
        b: text.as_bytes(),
        i: 0,
        depth: 0,
    };
    scan.skip_ws();
    if scan.peek() != Some(b'{') {
        return Err(Fail::Invalid);
    }
    let found = scan.parse_object(need_channel)?;
    scan.skip_ws();
    if scan.i != scan.b.len() {
        return Err(Fail::Invalid);
    }
    Ok(found.map(|f| (f.level, f.channel)))
}

struct Scan<'a> {
    b: &'a [u8],
    i: usize,
    depth: usize,
}

impl<'a> Scan<'a> {
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn one_value_to_end(&mut self) -> Result<(), Fail> {
        self.skip_ws();
        self.skip_value()?;
        self.skip_ws();
        if self.i != self.b.len() {
            return Err(Fail::Invalid);
        }
        Ok(())
    }

    fn parse_object(&mut self, need_channel: bool) -> Result<Option<FoundJson<'a>>, Fail> {
        if !self.eat(b'{') {
            return Err(Fail::Invalid);
        }
        self.enter()?;
        self.skip_ws();
        if self.eat(b'}') {
            self.depth -= 1;
            return Ok(None);
        }

        let mut message_ok = false;
        let mut channel_ok = false;
        let mut datetime_ok = false;
        let mut level_name: Option<Level> = None;
        let mut level_int: Option<i64> = None;
        let mut channel = Cow::Borrowed("");

        loop {
            let key = self.parse_key()?;
            self.skip_ws();
            if !self.eat(b':') {
                return Err(Fail::Invalid);
            }
            match key {
                Key::Message => message_ok = self.consume_string_value(None)?,
                Key::Channel => {
                    let mut captured = None;
                    channel_ok = self.consume_string_value(if need_channel {
                        Some(&mut captured)
                    } else {
                        None
                    })?;
                    if let Some(text) = captured {
                        channel = Cow::Owned(text);
                    }
                }
                Key::Datetime => datetime_ok = self.consume_string_value(None)?,
                Key::LevelName => level_name = self.consume_level_name()?,
                Key::Level => level_int = self.consume_level_int()?,
                Key::Other => self.skip_value()?,
            }
            self.skip_ws();
            if self.eat(b'}') {
                self.depth -= 1;
                break;
            }
            if !self.eat(b',') {
                return Err(Fail::Invalid);
            }
            self.skip_ws();
            if self.peek() == Some(b'}') {
                return Err(Fail::Invalid);
            }
        }

        if !message_ok || !channel_ok || !datetime_ok {
            return Ok(None);
        }
        let level = if let Some(level) = level_name {
            level
        } else if let Some(n) = level_int {
            match Level::from_monolog_int(n) {
                Some(level) => level,
                None => return Ok(None),
            }
        } else {
            return Ok(None);
        };
        if !need_channel {
            channel = Cow::Borrowed("");
        }
        Ok(Some(FoundJson { level, channel }))
    }

    fn enter(&mut self) -> Result<(), Fail> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            Err(Fail::Unsure)
        } else {
            Ok(())
        }
    }

    fn parse_key(&mut self) -> Result<Key, Fail> {
        self.skip_ws();
        if !self.eat(b'"') {
            return Err(Fail::Invalid);
        }
        let start = self.i;
        while self.i < self.b.len() {
            let c = self.b[self.i];
            if c == b'"' {
                let raw = &self.b[start..self.i];
                self.i += 1;
                return Ok(Key::from_bytes(raw));
            }
            if c == b'\\' || !(0x20..0x80).contains(&c) {
                self.i = start - 1;
                let owned = self.read_string()?;
                return Ok(Key::from_bytes(owned.as_bytes()));
            }
            self.i += 1;
        }
        Err(Fail::Invalid)
    }

    /// Leest een stringwaarde. `out` vangt de gedecodeerde tekst als die gevraagd
    /// wordt. `Ok(false)` als de waarde geen string is (de waarde is dan wel
    /// verbruikt).
    fn consume_string_value(&mut self, out: Option<&mut Option<String>>) -> Result<bool, Fail> {
        self.skip_ws();
        if self.peek() != Some(b'"') {
            self.skip_value()?;
            return Ok(false);
        }
        if let Some(slot) = out {
            *slot = Some(self.read_string()?);
        } else {
            self.skip_string()?;
        }
        Ok(true)
    }

    fn consume_level_name(&mut self) -> Result<Option<Level>, Fail> {
        self.skip_ws();
        if self.peek() != Some(b'"') {
            self.skip_value()?;
            return Ok(None);
        }
        self.i += 1;
        let start = self.i;
        while self.i < self.b.len() {
            let c = self.b[self.i];
            if c == b'"' {
                let raw = &self.b[start..self.i];
                self.i += 1;
                let text = std::str::from_utf8(raw).map_err(|_| Fail::Invalid)?;
                return Ok(Level::parse(text));
            }
            if c == b'\\' || !(0x20..0x80).contains(&c) {
                self.i = start - 1;
                let owned = self.read_string()?;
                return Ok(Level::parse(&owned));
            }
            self.i += 1;
        }
        Err(Fail::Invalid)
    }

    fn consume_level_int(&mut self) -> Result<Option<i64>, Fail> {
        self.skip_ws();
        match self.peek() {
            Some(b'-' | b'0'..=b'9') => match self.parse_number()? {
                Num::Int(n) => Ok(Some(n)),
                Num::NotInt => Ok(None),
            },
            _ => {
                self.skip_value()?;
                Ok(None)
            }
        }
    }

    fn skip_value(&mut self) -> Result<(), Fail> {
        self.skip_ws();
        match self.peek() {
            Some(b'"') => self.skip_string(),
            Some(b'{') => self.skip_container(b'{', b'}'),
            Some(b'[') => self.skip_container(b'[', b']'),
            Some(b't') => self.eat_lit(b"true"),
            Some(b'f') => self.eat_lit(b"false"),
            Some(b'n') => self.eat_lit(b"null"),
            Some(b'-' | b'0'..=b'9') => self.parse_number().map(|_| ()),
            _ => Err(Fail::Invalid),
        }
    }

    fn skip_container(&mut self, open: u8, close: u8) -> Result<(), Fail> {
        if !self.eat(open) {
            return Err(Fail::Invalid);
        }
        self.enter()?;
        self.skip_ws();
        if self.eat(close) {
            self.depth -= 1;
            return Ok(());
        }
        loop {
            if open == b'{' {
                self.skip_string()?;
                self.skip_ws();
                if !self.eat(b':') {
                    return Err(Fail::Invalid);
                }
            }
            self.skip_value()?;
            self.skip_ws();
            if self.eat(close) {
                self.depth -= 1;
                return Ok(());
            }
            if !self.eat(b',') {
                return Err(Fail::Invalid);
            }
            self.skip_ws();
            if self.peek() == Some(close) {
                return Err(Fail::Invalid);
            }
        }
    }

    fn skip_string(&mut self) -> Result<(), Fail> {
        if !self.eat(b'"') {
            return Err(Fail::Invalid);
        }
        while self.i < self.b.len() {
            let c = self.b[self.i];
            if c == b'"' {
                self.i += 1;
                return Ok(());
            }
            if c == b'\\' {
                self.i += 1;
                self.escape_char()?;
                continue;
            }
            if c < 0x20 {
                return Err(Fail::Invalid);
            }
            let w = utf8_width(c).ok_or(Fail::Invalid)?;
            if self.i + w > self.b.len() {
                return Err(Fail::Invalid);
            }
            if w > 1 && std::str::from_utf8(&self.b[self.i..self.i + w]).is_err() {
                return Err(Fail::Invalid);
            }
            self.i += w;
        }
        Err(Fail::Invalid)
    }

    fn read_string(&mut self) -> Result<String, Fail> {
        if !self.eat(b'"') {
            return Err(Fail::Invalid);
        }
        let mut out = String::new();
        while self.i < self.b.len() {
            let c = self.b[self.i];
            if c == b'"' {
                self.i += 1;
                return Ok(out);
            }
            if c == b'\\' {
                self.i += 1;
                out.push(self.escape_char()?);
                continue;
            }
            if c < 0x20 {
                return Err(Fail::Invalid);
            }
            let w = utf8_width(c).ok_or(Fail::Invalid)?;
            if self.i + w > self.b.len() {
                return Err(Fail::Invalid);
            }
            let text =
                std::str::from_utf8(&self.b[self.i..self.i + w]).map_err(|_| Fail::Invalid)?;
            out.push_str(text);
            self.i += w;
        }
        Err(Fail::Invalid)
    }

    fn escape_char(&mut self) -> Result<char, Fail> {
        let Some(e) = self.peek() else {
            return Err(Fail::Invalid);
        };
        self.i += 1;
        match e {
            b'"' | b'\\' | b'/' => Ok(e as char),
            b'b' => Ok('\u{0008}'),
            b'f' => Ok('\u{000c}'),
            b'n' => Ok('\n'),
            b'r' => Ok('\r'),
            b't' => Ok('\t'),
            b'u' => self.unicode_escape(),
            _ => Err(Fail::Invalid),
        }
    }

    fn unicode_escape(&mut self) -> Result<char, Fail> {
        let unit = self.hex4()?;
        if (0xD800..=0xDFFF).contains(&unit) {
            return Err(Fail::Unsure);
        }
        char::from_u32(u32::from(unit)).ok_or(Fail::Invalid)
    }

    fn hex4(&mut self) -> Result<u16, Fail> {
        let mut value = 0u16;
        for _ in 0..4 {
            let Some(c) = self.peek() else {
                return Err(Fail::Invalid);
            };
            self.i += 1;
            let digit = match c {
                b'0'..=b'9' => c - b'0',
                b'a'..=b'f' => c - b'a' + 10,
                b'A'..=b'F' => c - b'A' + 10,
                _ => return Err(Fail::Invalid),
            };
            value = value * 16 + u16::from(digit);
        }
        Ok(value)
    }

    fn eat_lit(&mut self, lit: &[u8]) -> Result<(), Fail> {
        if self.b[self.i..].starts_with(lit) {
            self.i += lit.len();
            Ok(())
        } else {
            Err(Fail::Invalid)
        }
    }

    fn parse_number(&mut self) -> Result<Num, Fail> {
        let start = self.i;
        if self.peek() == Some(b'-') {
            self.i += 1;
        }
        match self.peek() {
            Some(b'0') => self.i += 1,
            Some(b'1'..=b'9') => {
                self.i += 1;
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.i += 1;
                }
            }
            _ => return Err(Fail::Invalid),
        }
        let mut is_float = false;
        if self.peek() == Some(b'.') {
            is_float = true;
            self.i += 1;
            let frac = self.i;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.i += 1;
            }
            if self.i == frac {
                return Err(Fail::Invalid);
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            is_float = true;
            self.i += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.i += 1;
            }
            let exp = self.i;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.i += 1;
            }
            if self.i == exp {
                return Err(Fail::Invalid);
            }
        }
        if self.i - start > 64 {
            return Err(Fail::Unsure);
        }
        if is_float {
            return Ok(Num::NotInt);
        }
        let text = std::str::from_utf8(&self.b[start..self.i]).map_err(|_| Fail::Invalid)?;
        match text.parse::<i64>() {
            Ok(n) => Ok(Num::Int(n)),
            Err(_) => Ok(Num::NotInt),
        }
    }
}

enum Num {
    Int(i64),
    NotInt,
}

enum Key {
    Message,
    Channel,
    Datetime,
    LevelName,
    Level,
    Other,
}

impl Key {
    fn from_bytes(raw: &[u8]) -> Self {
        match raw {
            b"message" => Key::Message,
            b"channel" => Key::Channel,
            b"datetime" => Key::Datetime,
            b"level_name" => Key::LevelName,
            b"level" => Key::Level,
            _ => Key::Other,
        }
    }
}

fn utf8_width(first: u8) -> Option<usize> {
    match first {
        0x00..=0x7F => Some(1),
        0xC2..=0xDF => Some(2),
        0xE0..=0xEF => Some(3),
        0xF0..=0xF4 => Some(4),
        _ => None,
    }
}

fn strip_line_ending(line: &str) -> &str {
    let line = line.strip_suffix('\n').unwrap_or(line);
    line.strip_suffix('\r').unwrap_or(line)
}

/// Zelfde uitkomst als het niveau- en kanaalfilter op een volledig geparst blok,
/// zodat een afwijking in de kop-scan in tests zichtbaar is. `None` als de scan
/// terugvalt op de volledige parse (dan is er niets te vergelijken).
#[cfg(test)]
fn assert_classify_agrees(bytes: &[u8]) {
    let parsed = parse_record_lossy(bytes);
    for need_channel in [false, true] {
        match classify(bytes, need_channel) {
            Peek::Unsure => {}
            Peek::Raw => {
                assert!(
                    parsed.as_raw().is_some(),
                    "scan zei Raw, parser niet, need_channel={need_channel}: {:?}",
                    String::from_utf8_lossy(bytes)
                );
            }
            Peek::Parsed { level, channel } => match &parsed {
                ParsedRecord::Record(record) => {
                    assert_eq!(record.level, level, "{}", String::from_utf8_lossy(bytes));
                    if need_channel {
                        assert_eq!(record.channel, channel.as_ref());
                    }
                }
                ParsedRecord::Raw(raw) => panic!(
                    "scan zei record {level:?}, parser Raw ({}): {}",
                    raw.reason.as_str(),
                    String::from_utf8_lossy(bytes)
                ),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Level;
    use crate::parse::json::parse_json;
    use crate::parse::line::parse_line;

    #[test]
    fn json_acceptor_matches_serde_or_defers() {
        let samples = [
            "[]",
            "{}",
            " [ ] ",
            "{ }",
            "null",
            "true",
            "false",
            "0",
            "-1",
            "400",
            "400.0",
            "1e2",
            "1.5e+1",
            "01",
            "\"a\"",
            "\"a\\nb\"",
            "\"\\u0041\"",
            "\"\\uD800\"",
            "{\"a\":1}",
            "{\"a\":1,\"b\":[true,false,null]}",
            "[1, 2, 3]",
            "",
            " ",
            "{",
            "}",
            "[",
            "{\"a\":1,}",
            "[1,]",
            "{'a':1}",
            "\"unterminated",
            "\"\n\"",
            "{\"a\":1} trailing",
            "[][]",
            "0.5",
            "-0",
            "9223372036854775807",
            "9223372036854775808",
        ];
        for sample in samples {
            let serde_ok = serde_json::from_str::<serde_json::Value>(sample).is_ok();
            if let Ok(got) = accepts_one_json_value(sample) {
                assert_eq!(got, serde_ok, "{sample:?}");
            }
        }
    }

    #[test]
    fn normal_monolog_headers_are_decided_without_fallback() {
        let line = "[2026-03-01T00:00:00.000000+01:00] app.ERROR: hallo [] []\n";
        assert!(matches!(
            classify(line.as_bytes(), false),
            Peek::Parsed {
                level: Level::Error,
                ..
            }
        ));
        let parsed = parse_line(line).expect("lijn");
        assert_eq!(parsed.level, Level::Error);
        assert_eq!(parsed.channel, "app");

        let json = "{\"message\":\"generated\",\"level\":400,\"level_name\":\"ERROR\",\"channel\":\"request\",\"datetime\":\"2026-03-01T00:00:00.000000+01:00\"}\n";
        match classify(json.as_bytes(), true) {
            Peek::Parsed { level, channel } => {
                assert_eq!(level, Level::Error);
                assert_eq!(channel.as_ref(), "request");
            }
            other => panic!("JSON-kop hoort zeker te zijn, was {other:?}"),
        }
        assert_eq!(parse_json(json).expect("json").channel, "request");
    }

    #[test]
    fn classify_agrees_on_crafted_edges() {
        let samples = [
            "[2026-03-01T00:00:00.000000+01:00] app.DEBUG: x [] []\n",
            "[2026-03-01T00:00:00.000000+01:00] app.ERROR: kapot {\"n\":1\n",
            "[2026-01-15T10:30:45.123456+01:00] app.WARNING: bericht [] []\n",
            "[not-a-date] app.INFO: nee [] []\n",
            "geen kop\n",
            "{\"message\":\"boot\",\"level\":200,\"level_name\":\"INFO\",\"channel\":\"app\",\"datetime\":\"2026-01-15T10:00:00.000000+01:00\"}\n",
            "{\"message\":\"boot\",\"level\":400,\"level_name\":\"nonsense\",\"channel\":\"app\",\"datetime\":\"2026-01-15T10:00:00.000000+01:00\"}\n",
            "{\"message\":\"boot\",\"level\":400.0,\"channel\":\"app\",\"datetime\":\"2026-01-15T10:00:00.000000+01:00\"}\n",
            "{\"message\":\"boot\",\"level_name\":\"error\",\"channel\":\"app\",\"datetime\":\"2026-01-15T10:00:00.000000+01:00\"}\n",
            "{\"level_name\":\"DEBUG\",\"level_name\":\"ERROR\",\"level\":100,\"message\":\"m\",\"channel\":\"app\",\"datetime\":\"t\"}\n",
            "{\"message\":\"m\",\"level\":100,\"channel\":\"app\",\"datetime\":\"t\",\"level\":400}\n",
            "{\"message\":\"zie level_name ERROR\",\"level\":200,\"level_name\":\"INFO\",\"channel\":\"app\",\"datetime\":\"2026-01-15T10:00:00+01:00\"}\n",
            "{\"message\":\"m\",\"level\":400,\"channel\":\"app\\u002dweb\",\"datetime\":\"t\"}\n",
            "{\"message\":1,\"level\":400,\"channel\":\"app\",\"datetime\":\"t\"}\n",
            "{\"message\":\"m\",\"level\":400,\"channel\":\"app\"}\n",
            "{'message': 'nee'}\n",
            "{\"message\":\"m\",\"level\":400,\"level_name\":\"ERROR\",\"channel\":\"app\",\"datetime\":\"t\",\"context\":{\"exception\":{\"trace\":[\"#0 {main}\"]}}}\n",
            "[2026-03-01T00:00:01.500000+01:00] php.CRITICAL: Uncaught RuntimeException: x\nStack trace:\n#0 /var/www/src/Dispatcher.php(40): App\\Worker->run()\n#1 {main} [] []\n",
            "{\"message\":\"\",\"level\":\"400\",\"channel\":\"app\",\"datetime\":\"t\"}\n",
            "[]\n",
            "{\"message\":\"m\",\"level\":999,\"channel\":\"app\",\"datetime\":\"t\"}\n",
            "{\"message\":\"m\",\"level\":400,\"channel\":\"app\",\"datetime\":\"t\",\"extra\":{\"n\":1}}\n",
        ];
        for sample in samples {
            assert_classify_agrees(sample.as_bytes());
        }
    }

    #[test]
    fn classify_agrees_on_fixture_files() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        for entry in std::fs::read_dir(&dir).expect("fixtures") {
            let path = entry.expect("entry").path();
            let ext = path.extension().and_then(|e| e.to_str());
            if ext != Some("log") && ext != Some("ndjson") {
                continue;
            }
            let bytes = std::fs::read(&path).expect("lees fixture");
            if bytes.is_empty() {
                assert_classify_agrees(&bytes);
                continue;
            }
            let mut reader = crate::reader::Reader::open(&path).expect("open");
            reader.wait_until_indexed().expect("index");
            for i in 0..reader.record_count() {
                let rec = reader.record_bytes(i).expect("io").expect("record");
                assert_classify_agrees(&rec);
            }
        }
    }
}
