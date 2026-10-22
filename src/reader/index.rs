//! Offset-index: alleen record-startoffsets (u64), geen kopie van de inhoud
//! (task.md T2, blueprint.md `reader::index`).
//!
//! Een record begint bij offset 0 en bij elke byte direct na een `\n`. De laatste
//! regel van een bestand dat zelf met een `\n` eindigt, levert geen extra leeg
//! record op.

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

    /// Verwijdert de laatst gepubliceerde offset als die gelijk is aan `value`.
    /// Gebruikt aan het eind van [`build_index`] om een leeg record na een
    /// afsluitende `\n` ongedaan te maken.
    fn drop_trailing_if(&self, value: u64) {
        let mut offsets = self.offsets.write().expect("index-lock vergiftigd");
        if offsets.last() == Some(&value) {
            offsets.pop();
        }
    }

    fn mark_done(&self) {
        self.done.store(true, Ordering::Release);
    }
}

/// Scant `reader` in blokken van `block_size` bytes en publiceert record-startoffsets
/// in `index`. Werkt op elke `Read`-bron (bestand of, in tests, een `Cursor`), zodat
/// de blokgrens-logica deterministisch getest kan worden.
pub fn build_index<R: Read>(
    mut reader: R,
    index: &RecordIndex,
    block_size: usize,
) -> io::Result<()> {
    assert!(block_size > 0, "block_size moet positief zijn");
    let mut buf = vec![0u8; block_size];
    let mut pos: u64 = 0;
    let mut seen_any_byte = false;

    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        let chunk = &buf[..n];
        let mut batch = Vec::new();
        if !seen_any_byte {
            batch.push(pos);
            seen_any_byte = true;
        }
        for nl in memchr_iter(b'\n', chunk) {
            batch.push(pos + nl as u64 + 1);
        }
        pos += n as u64;
        index.extend(&batch, pos);
    }

    index.drop_trailing_if(pos);
    index.mark_done();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

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
    fn single_line_without_trailing_newline() {
        // "Blijft" uit T2: een bestand zonder slot-newline levert geen fantoom-record.
        assert_eq!(offsets_of(b"line1\nline2", DEFAULT_BLOCK_SIZE), vec![0, 6]);
    }

    #[test]
    fn trailing_newline_does_not_add_empty_record() {
        assert_eq!(
            offsets_of(b"line1\nline2\n", DEFAULT_BLOCK_SIZE),
            vec![0, 6]
        );
    }

    #[test]
    fn record_boundary_exactly_on_block_edge() {
        // Blok 1 is precies "AAAAAAA\n" (8 bytes): de newline is de laatste byte
        // van het eerste blok bij block_size=8.
        let content = b"AAAAAAA\nBBBBBBB\n";
        assert_eq!(offsets_of(content, 8), vec![0, 8]);
    }

    #[test]
    fn crlf_split_across_block_boundary() {
        // "ABC\r" is precies het eerste blok (4 bytes); "\n" is de eerste byte
        // van het tweede blok. De CR en LF horen zo bij hetzelfde record, ook al
        // scant de indexer ze in twee verschillende blokken.
        let content = b"ABC\r\nXY";
        assert_eq!(offsets_of(content, 4), vec![0, 5]);
    }

    #[test]
    fn many_small_blocks_find_same_offsets_as_one_big_block() {
        let content = b"a\nbb\nccc\nd\n\ne\n";
        let whole = offsets_of(content, DEFAULT_BLOCK_SIZE);
        let one_byte_at_a_time = offsets_of(content, 1);
        assert_eq!(whole, one_byte_at_a_time);
    }
}
