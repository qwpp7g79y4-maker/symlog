//! Offset-index: alleen record-startoffsets (u64), geen kopie van de inhoud
//! (task.md T2/T4/T5, blueprint.md `reader::index`).
//!
//! Een record begint bij offset 0 en bij elke regel die een geldige recordkop is:
//! `[` + herkende datetime voor LineFormatter (task.md T4), of `{` met een
//! `"datetime"`-veld voor JsonFormatter (task.md T5) — zie
//! [`crate::parse::is_record_header`], die beide koppen combineert zodat een gemengd
//! bestand (T5, F6) ook via de streaming-indexer correct in records valt. Andere
//! regels zijn een vervolg van het lopende record (bijvoorbeeld een stacktrace-regel):
//! ze leveren geen nieuwe offset op. Dit maakt een record inherent meerdere fysieke
//! regels lang mogelijk. De laatste regel van een bestand dat zelf met een `\n`
//! eindigt, levert nooit een extra leeg record op, want een lege regel is nooit een
//! geldige recordkop.

use crate::parse::is_record_header;
use memchr::memchr_iter;
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::RwLock;

/// Scangrootte voor de achtergrondindexer ("blokken van 1 MiB", task.md T2).
pub const DEFAULT_BLOCK_SIZE: usize = 1024 * 1024;

/// Index van record-startoffsets, incrementeel gevuld door [`build_index`].
///
/// Groeit alleen (geen verwijderingen tijdens het indexeren) en wordt per blok in
/// één keer uitgebreid, zodat lezers niet per record op een lock wachten.
#[derive(Debug, Default)]
pub struct RecordIndex {
    offsets: RwLock<Vec<u64>>,
    bytes_scanned: AtomicU64,
    done: AtomicBool,
}

impl RecordIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Aantal tot nu toe gevonden record-starts.
    pub fn len(&self) -> usize {
        self.offsets.read().expect("index-lock vergiftigd").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Startoffset van record `i`, of `None` als die (nog) niet bekend is.
    pub fn offset(&self, i: usize) -> Option<u64> {
        self.offsets
            .read()
            .expect("index-lock vergiftigd")
            .get(i)
            .copied()
    }

    /// Bytes die de achtergrondindexer al gescand heeft; basis voor een
    /// voortgangsbalk (de balk zelf komt in T8).
    pub fn bytes_scanned(&self) -> u64 {
        self.bytes_scanned.load(Ordering::Acquire)
    }

    /// Of de achtergrondindexer het bestand (of de stream) volledig gescand heeft.
    pub fn is_done(&self) -> bool {
        self.done.load(Ordering::Acquire)
    }

    fn extend(&self, new_offsets: &[u64], scanned_so_far: u64) {
        if !new_offsets.is_empty() {
            self.offsets
                .write()
                .expect("index-lock vergiftigd")
                .extend_from_slice(new_offsets);
        }
        self.bytes_scanned.store(scanned_so_far, Ordering::Release);
    }

    fn mark_done(&self) {
        self.done.store(true, Ordering::Release);
    }

    /// Leegt de index volledig (task.md T10, "Afkappen"/"Rotatie": "index en cache
    /// leeg" / "index vanaf 0"). Gebruikt door [`crate::follow`] als het gevolgde
    /// bestand afgekapt is of (voor een nieuwe generatie na rotatie) opnieuw vanaf
    /// offset 0 begint; niet gebruikt door de eenmalige achtergrondindexer van
    /// [`crate::reader::Reader`], die geen reden heeft om te resetten.
    pub fn reset(&self) {
        self.offsets.write().expect("index-lock vergiftigd").clear();
        self.bytes_scanned.store(0, Ordering::Release);
        self.done.store(false, Ordering::Release);
    }
}

/// Scant een logbestand incrementeel, blok voor blok, en publiceert record-
/// startoffsets in een [`RecordIndex`]. [`build_index`] voert dit in één keer tot EOF
/// uit (task.md T2); [`crate::follow`] (T10) houdt in plaats daarvan zelf een
/// `IndexScanner` vast en roept [`IndexScanner::feed`] telkens aan zodra het gevolgde
/// bestand gegroeid is, zonder steeds vanaf offset 0 opnieuw te beginnen.
///
/// Een regel kan zelf over een bloklezing/`feed`-aanroep heen lopen (bijvoorbeeld een
/// lange stacktrace-regel, of de datetime-kop precies op de blokgrens); `carry` houdt
/// de bytes van de huidige, nog niet afgesloten regel vast totdat de afsluitende `\n`
/// (of EOF, via [`IndexScanner::finish`]) gevonden is, zodat [`is_record_header`]
/// altijd de volledige regel ziet en niet een toevallig stukje ervan.
#[derive(Debug, Default)]
pub struct IndexScanner {
    pos: u64,
    line_start: u64,
    carry: Vec<u8>,
    seen_any_byte: bool,
}

impl IndexScanner {
    /// Een nieuwe scanner die bij offset 0 van een (nog lege) bron begint.
    pub fn new() -> Self {
        Self::default()
    }

    /// Aantal bytes dat tot nu toe aan [`IndexScanner::feed`] is gegeven, inclusief een
    /// eventuele nog niet afgesloten staartregel (zie [`IndexScanner::pending_tail`]).
    /// Dit is de volgende offset waar een toekomstige `feed`-aanroep rechtstreeks op
    /// moet aansluiten (geen gat, geen overlap).
    pub fn bytes_read(&self) -> u64 {
        self.pos
    }

    /// Bytes van de huidige, nog niet door een `\n` afgesloten regel (leeg als het
    /// bestand op dit moment op een regelgrens eindigt). Task.md T10 "Doet": deze
    /// bytes worden pas een zichtbaar record zodra de afsluitende `\n` arriveert, of na
    /// een timeout als voorlopige Raw-staart (zie [`crate::follow`]).
    pub fn pending_tail(&self) -> &[u8] {
        &self.carry
    }

    /// Verwerkt `chunk`, die rechtstreeks aansluit op alles wat deze scanner al eerder
    /// gezien heeft (geen gat, geen overlap), en publiceert nieuwe record-startoffsets
    /// in `index`.
    pub fn feed(&mut self, index: &RecordIndex, chunk: &[u8]) {
        let mut batch = Vec::new();
        if !self.seen_any_byte && !chunk.is_empty() {
            // Offset 0 is altijd een recordstart, ongeacht de inhoud (task.md T2
            // "Blijft"); T6 toont zo'n record evt. als Raw als de kop niet klopt.
            batch.push(0);
            self.seen_any_byte = true;
        }

        let mut chunk_pos = 0usize;
        for nl in memchr_iter(b'\n', chunk) {
            self.carry.extend_from_slice(&chunk[chunk_pos..nl]);
            // `line_start != 0` slaat alleen de allereerste regel over: offset 0 is al
            // hierboven gepubliceerd en mag niet nogmaals in `batch` terechtkomen.
            if self.line_start != 0 && is_record_header(&self.carry) {
                batch.push(self.line_start);
            }
            self.line_start = self.pos + nl as u64 + 1;
            self.carry.clear();
            chunk_pos = nl + 1;
        }
        self.carry.extend_from_slice(&chunk[chunk_pos..]);
        self.pos += chunk.len() as u64;
        index.extend(&batch, self.pos);
    }

    /// Sluit de scan definitief af: de laatste regel zonder afsluitende `\n` (als die
    /// er is) is nu compleet (echte EOF) en kan dus alsnog op een geldige recordkop
    /// gecontroleerd worden. Na `finish` mag deze scanner niet meer verder gevoed
    /// worden (gebruikt door [`build_index`] bij bestands-EOF en door
    /// [`crate::follow`] bij het afsluiten van een afgesloten generatie, bijvoorbeeld
    /// na rotatie).
    pub fn finish(&mut self, index: &RecordIndex) {
        if !self.carry.is_empty() && self.line_start != 0 && is_record_header(&self.carry) {
            index.extend(&[self.line_start], self.pos);
        }
        index.mark_done();
    }
}

/// Scant `reader` in blokken van `block_size` bytes en publiceert record-startoffsets
/// in `index`. Werkt op elke `Read`-bron (bestand of, in tests, een `Cursor`), zodat
/// de blokgrens-logica deterministisch getest kan worden. Dunne laag boven
/// [`IndexScanner`]: leest tot EOF in één aanroep (task.md T2), in tegenstelling tot
/// [`crate::follow`] dat de scanner tussen afzonderlijke groei-aanroepen in leven
/// houdt.
pub fn build_index<R: Read>(
    mut reader: R,
    index: &RecordIndex,
    block_size: usize,
) -> io::Result<()> {
    assert!(block_size > 0, "block_size moet positief zijn");
    let mut buf = vec![0u8; block_size];
    let mut scanner = IndexScanner::new();

    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        scanner.feed(index, &buf[..n]);
    }

    scanner.finish(index);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Minimale geldige recordkop (Monolog 1-stijl, geen offset): 21 bytes
    /// (`[` + 19 datetime-bytes + `]`). Alleen de kop zelf wordt door de indexer
    /// gecontroleerd (blueprint.md), dus de rest van de regel is vrije tekst.
    const H0: &[u8] = b"[2026-01-01 00:00:00]";
    const H1: &[u8] = b"[2026-01-01 00:00:01]";

    fn offsets_of(content: &[u8], block_size: usize) -> Vec<u64> {
        let index = RecordIndex::new();
        build_index(Cursor::new(content), &index, block_size).expect("build_index");
        assert!(index.is_done());
        assert_eq!(index.bytes_scanned(), content.len() as u64);
        let offsets = index.offsets.read().expect("index-lock vergiftigd");
        offsets.clone()
    }

    #[test]
    fn empty_file_has_no_records() {
        assert_eq!(offsets_of(b"", DEFAULT_BLOCK_SIZE), Vec::<u64>::new());
    }

    #[test]
    fn two_records_the_last_without_trailing_newline() {
        // "Blijft" uit T2: een bestand zonder slot-newline levert geen fantoom-record,
        // en het laatste (onvolledige) record wordt nog steeds als recordstart herkend.
        let mut content = H0.to_vec();
        content.extend_from_slice(b" one\n");
        let second_start = content.len() as u64;
        content.extend_from_slice(H1);
        content.extend_from_slice(b" two");
        assert_eq!(
            offsets_of(&content, DEFAULT_BLOCK_SIZE),
            vec![0, second_start]
        );
    }

    #[test]
    fn trailing_newline_does_not_add_empty_record() {
        let mut content = H0.to_vec();
        content.extend_from_slice(b" one\n");
        let second_start = content.len() as u64;
        content.extend_from_slice(H1);
        content.extend_from_slice(b" two\n");
        assert_eq!(
            offsets_of(&content, DEFAULT_BLOCK_SIZE),
            vec![0, second_start]
        );
    }

    #[test]
    fn continuation_line_does_not_start_a_new_record() {
        // Kernfunctie van T4: een vervolgregel (bijvoorbeeld een stacktrace-regel)
        // die niet met een geldige kop begint, hoort bij het lopende record.
        let mut content = H0.to_vec();
        content.extend_from_slice(b" boom\nStack trace:\n#0 /a.php(1): foo()\n#1 {main}\n");
        let second_start = content.len() as u64;
        content.extend_from_slice(H1);
        content.extend_from_slice(b" next\n");
        assert_eq!(
            offsets_of(&content, DEFAULT_BLOCK_SIZE),
            vec![0, second_start]
        );
    }

    #[test]
    fn continuation_line_starting_with_bracket_is_not_a_false_header() {
        // Spiegelt F12 (task.md T4): een vervolgregel die toevallig met `[` begint
        // (`[previous exception] ...`) maar geen geldige datetime heeft, mag geen
        // nieuw record openen.
        let mut content = H0.to_vec();
        content.extend_from_slice(b" boom\n");
        content.extend_from_slice(
            b"[previous exception] [object] (LogicException: oorzaak at /a.php:1)\n",
        );
        content.extend_from_slice(b"#0 /a.php(1): foo()\n");
        let second_start = content.len() as u64;
        content.extend_from_slice(H1);
        content.extend_from_slice(b" next\n");
        assert_eq!(
            offsets_of(&content, DEFAULT_BLOCK_SIZE),
            vec![0, second_start]
        );
    }

    #[test]
    fn record_boundary_exactly_on_block_edge() {
        // Regel 1 is precies "[2026-01-01 00:00:00]\n" (22 bytes): de newline is de
        // laatste byte van het eerste blok bij block_size=22.
        let mut content = H0.to_vec();
        content.push(b'\n');
        assert_eq!(content.len(), 22);
        let second_start = content.len() as u64;
        content.extend_from_slice(H1);
        content.push(b'\n');
        assert_eq!(offsets_of(&content, 22), vec![0, second_start]);
    }

    #[test]
    fn crlf_split_across_block_boundary() {
        // De `\r` is precies het laatste byte van het eerste blok; de `\n` is het
        // eerste byte van het tweede blok. De CR en LF horen zo bij hetzelfde record,
        // ook al scant de indexer ze in twee verschillende blokken, en de CR mag de
        // datetime-kopcontrole niet verstoren (die stopt bij de eerste `]`, vóór de CR).
        let mut content = H0.to_vec();
        content.push(b'\r');
        let block_size = content.len();
        content.push(b'\n');
        let second_start = content.len() as u64;
        content.extend_from_slice(H1);
        content.extend_from_slice(b"\r\n");
        assert_eq!(offsets_of(&content, block_size), vec![0, second_start]);
    }

    #[test]
    fn header_split_across_block_boundary() {
        // De datetime-kop van het tweede record wordt over twee bloklezingen heen
        // gescand (block_size=32 valt middenin "[2026-01-01 00:00:01]"); de `carry`-
        // buffer in build_index moet de regel toch in zijn geheel zien.
        let mut content = H0.to_vec();
        content.push(b'\n');
        let second_start = content.len() as u64;
        content.extend_from_slice(H1);
        content.push(b'\n');
        // block_size=32 valt binnen het haakje van het tweede record ([22..43)):
        // precies de situatie die deze test moet dekken.
        assert!((second_start as usize..second_start as usize + 21).contains(&32));
        assert_eq!(offsets_of(&content, 32), vec![0, second_start]);
    }

    #[test]
    fn many_small_blocks_find_same_offsets_as_one_big_block() {
        let mut content = H0.to_vec();
        content.extend_from_slice(b" a\ncontinuation one\ncontinuation two\n");
        let second_start = content.len() as u64;
        content.extend_from_slice(H1);
        content.extend_from_slice(b" b\n\nlast continuation without newline");
        let whole = offsets_of(&content, DEFAULT_BLOCK_SIZE);
        let one_byte_at_a_time = offsets_of(&content, 1);
        assert_eq!(whole, one_byte_at_a_time);
        assert_eq!(whole, vec![0, second_start]);
    }

    #[test]
    fn json_record_header_opens_a_new_record_in_a_mixed_file() {
        // Task.md T5 (F6): een JSON-record na een lijnformaat-record moet zijn eigen
        // offset krijgen, niet als vervolgregel bij het vorige record getrokken worden.
        let mut content = H0.to_vec();
        content.extend_from_slice(b" line record\n");
        let second_start = content.len() as u64;
        content.extend_from_slice(
            br#"{"message":"json record","level":200,"channel":"app","datetime":"2026-01-01T00:00:02+00:00"}"#,
        );
        content.push(b'\n');
        assert_eq!(
            offsets_of(&content, DEFAULT_BLOCK_SIZE),
            vec![0, second_start]
        );
    }

    #[test]
    fn continuation_line_starting_with_brace_is_not_a_false_json_header() {
        // Spiegelt de `[`-variant hierboven: een vervolgregel van een lijnformaat-
        // stacktrace die toevallig met `{` begint maar geen geldig JSON-object met een
        // `"datetime"`-veld is, mag geen nieuw record openen.
        let mut content = H0.to_vec();
        content.extend_from_slice(b" boom\n");
        content.extend_from_slice(b"{not json, just a stray brace in the message}\n");
        content.extend_from_slice(br#"{"no_datetime_field": true}"#);
        content.push(b'\n');
        let second_start = content.len() as u64;
        content.extend_from_slice(H1);
        content.extend_from_slice(b" next\n");
        assert_eq!(
            offsets_of(&content, DEFAULT_BLOCK_SIZE),
            vec![0, second_start]
        );
    }

    #[test]
    fn line_record_follows_json_record_in_a_mixed_file() {
        // De omgekeerde volgorde van de eerste test hierboven: na een JSON-record moet
        // een lijnformaat-kop ook zijn eigen offset krijgen.
        let mut content: Vec<u8> =
            br#"{"message":"json first","level":200,"channel":"app","datetime":"2026-01-01T00:00:00+00:00"}"#
                .to_vec();
        content.push(b'\n');
        let second_start = content.len() as u64;
        content.extend_from_slice(H1);
        content.extend_from_slice(b" line second\n");
        assert_eq!(
            offsets_of(&content, DEFAULT_BLOCK_SIZE),
            vec![0, second_start]
        );
    }

    #[test]
    fn pretty_printed_json_record_is_not_merged_across_lines() {
        // Task.md T5 "Blijft": een pretty-printed (meerregelig) JSON-record wordt in
        // v1 niet samengevoegd. De openende `{` alleen heeft geen `"datetime"`-veld op
        // die regel, dus elke vervolgregel van het pretty-printed blok blijft gewoon
        // bij het voorgaande record horen in plaats van een eigen (kapot) record te
        // worden; een echt volgend record krijgt wel zijn eigen offset.
        let mut content = H0.to_vec();
        content.extend_from_slice(b" before pretty json\n");
        content.extend_from_slice(b"{\n  \"message\": \"pretty\",\n  \"level\": 200\n}\n");
        let second_start = content.len() as u64;
        content.extend_from_slice(H1);
        content.extend_from_slice(b" after pretty json\n");
        assert_eq!(
            offsets_of(&content, DEFAULT_BLOCK_SIZE),
            vec![0, second_start]
        );
    }

    #[test]
    fn index_scanner_fed_in_separate_calls_matches_build_index() {
        // Task.md T10: `IndexScanner` is wat `crate::follow` tussen groei-aanroepen in
        // levend houdt, dus moet het feeden in losse stukjes (zoals follow dat doet, één
        // stuk per gedetecteerde groei) exact dezelfde offsets geven als één
        // `build_index`-aanroep op dezelfde inhoud in zijn geheel.
        let mut content = H0.to_vec();
        content.extend_from_slice(b" a\ncontinuation\n");
        let second_start = content.len() as u64;
        content.extend_from_slice(H1);
        content.extend_from_slice(b" b\n");

        let whole = offsets_of(&content, DEFAULT_BLOCK_SIZE);
        assert_eq!(whole, vec![0, second_start]);

        let index = RecordIndex::new();
        let mut scanner = IndexScanner::new();
        // Groei in drie willekeurige, ongelijke stukken, zoals opeenvolgende
        // `tick()`-aanroepen tijdens follow elk een ander aantal nieuwe bytes zouden
        // kunnen aanleveren.
        let mid = second_start as usize + 2;
        scanner.feed(&index, &content[..5]);
        scanner.feed(&index, &content[5..mid]);
        scanner.feed(&index, &content[mid..]);
        scanner.finish(&index);

        let offsets: Vec<u64> = (0..index.len()).filter_map(|i| index.offset(i)).collect();
        assert_eq!(offsets, whole);
        assert!(index.is_done());
    }

    #[test]
    fn index_scanner_pending_tail_is_empty_only_on_a_line_boundary() {
        let index = RecordIndex::new();
        let mut scanner = IndexScanner::new();
        let first_line = b"[2026-01-01 00:00:00] a.INFO: compleet\n";
        scanner.feed(&index, first_line);
        assert!(scanner.pending_tail().is_empty());
        assert_eq!(scanner.bytes_read(), first_line.len() as u64);

        scanner.feed(&index, b"[2026-01-01 00:00:01] a.INFO: onvolledig");
        assert_eq!(
            scanner.pending_tail(),
            b"[2026-01-01 00:00:01] a.INFO: onvolledig"
        );
        assert_eq!(index.len(), 1); // de onvolledige regel heeft nog geen eigen offset

        scanner.feed(&index, b"\n");
        assert!(scanner.pending_tail().is_empty());
        assert_eq!(index.len(), 2); // nu wel: de `\n` maakte de regel compleet
    }

    #[test]
    fn reset_clears_offsets_bytes_scanned_and_done() {
        let index = RecordIndex::new();
        build_index(
            Cursor::new(b"[2026-01-01 00:00:00] a.INFO: one\n".as_slice()),
            &index,
            DEFAULT_BLOCK_SIZE,
        )
        .expect("build_index");
        assert_eq!(index.len(), 1);
        assert!(index.is_done());

        index.reset();
        assert_eq!(index.len(), 0);
        assert_eq!(index.bytes_scanned(), 0);
        assert!(!index.is_done());
    }
}
