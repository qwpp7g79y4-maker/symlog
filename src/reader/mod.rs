//! `reader`: leest een logbestand streaming, zonder het volledig te laden
//! (task.md T2). Opent het bestand alleen read-only (`File::open`, nooit met
//! schrijfrechten) en bouwt in een achtergrondthread een offset-index van
//! record-starts (zie [`index`]). Alleen records die daadwerkelijk opgevraagd
//! worden, worden via `pread`/seek gelezen en begrensd gecached (zie [`cache`]).
//!
//! mmap (feature `mmap`) staat hier bewust niet achter: de standaard is pread,
//! omdat afkappen tijdens een mmap SIGBUS kan geven (blueprint.md, T10).

pub mod cache;
pub mod index;

use cache::RecordCache;
use index::{build_index, RecordIndex, DEFAULT_BLOCK_SIZE};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

/// Leest een logbestand read-only; de achtergrondindexer start meteen bij
/// [`Reader::open`] en de index groeit terwijl eerdere records al opvraagbaar zijn.
pub struct Reader {
    file: File,
    index: Arc<RecordIndex>,
    cache: RecordCache,
    indexer: Option<JoinHandle<io::Result<()>>>,
}

impl Reader {
    /// Opent `path` read-only en start de achtergrondindexer.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file = File::open(&path)?;
        let index = Arc::new(RecordIndex::new());
        let indexer = spawn_indexer(path, Arc::clone(&index));
        Ok(Self {
            file,
            index,
            cache: RecordCache::with_capacity(cache::DEFAULT_CAPACITY),
            indexer: Some(indexer),
        })
    }

    /// Gedeelde index, voor bijvoorbeeld een voortgangsbalk (de balk zelf komt in T8).
    pub fn index(&self) -> &Arc<RecordIndex> {
        &self.index
    }

    /// Aantal tot nu toe geïndexeerde records. Groeit terwijl de achtergrondindexer
    /// nog aan het werk is; de TUI (T8) mag dit tussentijds al tonen.
    pub fn record_count(&self) -> usize {
        self.index.len()
    }

    /// Wacht tot de achtergrondindexer klaar is. Bedoeld voor tests en kleine
    /// bestanden; de TUI zelf hoeft hier niet op te wachten (task.md T2: "de TUI is
    /// bruikbaar vóór de index klaar is").
    pub fn wait_until_indexed(&mut self) -> io::Result<()> {
        let Some(handle) = self.indexer.take() else {
            return Ok(());
        };
        match handle.join() {
            Ok(result) => result,
            Err(panic) => {
                let msg = panic
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| panic.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "onbekende paniek".to_string());
                Err(io::Error::other(format!("indexer paniekte: {msg}")))
            }
        }
    }

    /// Ruwe bytes van record `i` (inclusief regeleinde), gelezen via `pread`/seek op
    /// offset en gecached in een begrensde LRU. `None` als record `i` nog niet
    /// geïndexeerd is.
    pub fn record_bytes(&self, i: usize) -> io::Result<Option<Arc<[u8]>>> {
        let Some(start) = self.index.offset(i) else {
            return Ok(None);
        };
        if let Some(cached) = self.cache.get(start) {
            return Ok(Some(cached));
        }
        let end = self.index.offset(i + 1);
        let bytes: Arc<[u8]> = self.read_record_at(start, end)?.into();
        self.cache.insert(start, Arc::clone(&bytes));
        Ok(Some(bytes))
    }

    /// Laatste geïndexeerde record op dit moment, of `None` als er nog geen enkel
    /// record gevonden is.
    pub fn last_record_bytes(&self) -> io::Result<Option<Arc<[u8]>>> {
        let count = self.record_count();
        if count == 0 {
            return Ok(None);
        }
        self.record_bytes(count - 1)
    }

    /// Leest de bytes van `start` tot `end` (exclusief), of tot EOF als `end`
    /// onbekend is (het laatste record). Gebruikt `read_at` (pread) zodat dit nooit
    /// de leespositie van andere lezers van hetzelfde bestand verstoort.
    fn read_record_at(&self, start: u64, end: Option<u64>) -> io::Result<Vec<u8>> {
        read_record_at(&self.file, start, end)
    }

    /// Zoals [`read_record_at`], maar hergebruikt `buf`. Bedoeld voor het filter,
    /// dat veel records achter elkaar leest en de LRU-cache niet per record wil
    /// vullen (die verdringt zichzelf toch bij een volledige scan).
    pub(crate) fn read_into(
        &self,
        start: u64,
        end: Option<u64>,
        buf: &mut Vec<u8>,
    ) -> io::Result<()> {
        match end {
            Some(end) if end < start => Err(io::Error::other("recordbereik loopt terug")),
            Some(end) => {
                let len = usize::try_from(end - start)
                    .map_err(|_| io::Error::other("recordbereik past niet in het geheugen"))?;
                buf.clear();
                buf.resize(len, 0);
                self.file.read_exact_at(buf, start)?;
                Ok(())
            }
            None => {
                let mut tail = self.file.try_clone()?;
                tail.seek(SeekFrom::Start(start))?;
                buf.clear();
                tail.read_to_end(buf)?;
                Ok(())
            }
        }
    }
}

/// Leest de bytes van `start` tot `end` (exclusief) uit `file`, of tot EOF als `end`
/// onbekend is. Vrije functie (niet een methode op [`Reader`]) zodat
/// [`crate::follow`] (T10) dezelfde pread-gebaseerde leeslogica hergebruikt voor zijn
/// eigen, per generatie bijgehouden bestandshandles.
///
/// Geeft een `io::Error` terug (nooit een paniek) als `file` intussen minder bytes
/// bevat dan `end` verwacht, bijvoorbeeld door afkappen tussen het bepalen van de
/// recordgrenzen en deze lezing (task.md T10, "Blijft": "ook buiten follow wordt
/// afkappen tijdens het lezen gedetecteerd ... en netjes afgehandeld in plaats van
/// een panic").
pub(crate) fn read_record_at(file: &File, start: u64, end: Option<u64>) -> io::Result<Vec<u8>> {
    match end {
        Some(end) => {
            let len = (end - start) as usize;
            let mut buf = vec![0u8; len];
            file.read_exact_at(&mut buf, start)?;
            Ok(buf)
        }
        None => {
            let mut tail = file.try_clone()?;
            tail.seek(SeekFrom::Start(start))?;
            let mut buf = Vec::new();
            tail.read_to_end(&mut buf)?;
            Ok(buf)
        }
    }
}

fn spawn_indexer(path: PathBuf, index: Arc<RecordIndex>) -> JoinHandle<io::Result<()>> {
    thread::spawn(move || {
        let file = File::open(&path)?;
        build_index(file, &index, DEFAULT_BLOCK_SIZE)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp_log(content: &[u8]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("test.log");
        File::create(&path)
            .expect("create")
            .write_all(content)
            .expect("write");
        (dir, path)
    }

    #[test]
    fn opens_read_only_and_indexes_records() {
        let (_dir, path) = write_temp_log(
            b"[2026-01-01 00:00:00] a.INFO: one\n\
              [2026-01-01 00:00:01] a.INFO: two\n\
              [2026-01-01 00:00:02] a.INFO: three\n",
        );
        let mut reader = Reader::open(&path).expect("open");
        reader.wait_until_indexed().expect("index");

        assert_eq!(reader.record_count(), 3);
        assert_eq!(
            &*reader.record_bytes(0).unwrap().unwrap(),
            b"[2026-01-01 00:00:00] a.INFO: one\n"
        );
        assert_eq!(
            &*reader.record_bytes(1).unwrap().unwrap(),
            b"[2026-01-01 00:00:01] a.INFO: two\n"
        );
        assert_eq!(
            &*reader.record_bytes(2).unwrap().unwrap(),
            b"[2026-01-01 00:00:02] a.INFO: three\n"
        );
        assert!(reader.record_bytes(3).unwrap().is_none());
    }

    #[test]
    fn last_record_without_trailing_newline() {
        let (_dir, path) = write_temp_log(
            b"[2026-01-01 00:00:00] a.INFO: first\n[2026-01-01 00:00:01] a.INFO: second",
        );
        let mut reader = Reader::open(&path).expect("open");
        reader.wait_until_indexed().expect("index");

        let last = reader.last_record_bytes().expect("read").expect("some");
        assert_eq!(&*last, b"[2026-01-01 00:00:01] a.INFO: second");
    }

    #[test]
    fn empty_file_has_no_last_record() {
        let (_dir, path) = write_temp_log(b"");
        let mut reader = Reader::open(&path).expect("open");
        reader.wait_until_indexed().expect("index");

        assert_eq!(reader.record_count(), 0);
        assert!(reader.last_record_bytes().expect("read").is_none());
    }

    #[test]
    fn repeated_reads_come_from_cache() {
        let (_dir, path) = write_temp_log(b"[2026-01-01 00:00:00] a.INFO: only record\n");
        let mut reader = Reader::open(&path).expect("open");
        reader.wait_until_indexed().expect("index");

        let first = reader.record_bytes(0).unwrap().unwrap();
        let second = reader.record_bytes(0).unwrap().unwrap();
        // Zelfde Arc-allocatie: de tweede lezing kwam uit de cache, niet van schijf.
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn multiline_record_is_read_as_a_single_record() {
        // Task.md T4: een stacktrace-vervolgregel hoort bij het voorgaande record, de
        // index mag er geen nieuw record op openen.
        let (_dir, path) = write_temp_log(
            b"[2026-01-01 00:00:00] a.ERROR: boom\nStack trace:\n#0 {main}\n\
              [2026-01-01 00:00:01] a.INFO: next\n",
        );
        let mut reader = Reader::open(&path).expect("open");
        reader.wait_until_indexed().expect("index");

        assert_eq!(reader.record_count(), 2);
        assert_eq!(
            &*reader.record_bytes(0).unwrap().unwrap(),
            b"[2026-01-01 00:00:00] a.ERROR: boom\nStack trace:\n#0 {main}\n"
        );
        assert_eq!(
            &*reader.record_bytes(1).unwrap().unwrap(),
            b"[2026-01-01 00:00:01] a.INFO: next\n"
        );
    }

    #[test]
    fn truncation_during_read_is_an_error_not_a_panic() {
        // Task.md T10 "Blijft": "ook buiten follow wordt afkappen tijdens het lezen
        // gedetecteerd (read geeft minder bytes dan de index verwacht) en netjes
        // afgehandeld in plaats van een panic." De index verwacht hier twee records,
        // maar het bestand is na het indexeren afgekapt tot vóór het eerste record is
        // afgelopen: de bytes die de index belooft (tot offset[1]) bestaan niet meer.
        let (_dir, path) = write_temp_log(
            b"[2026-01-01 00:00:00] a.INFO: first record, lang genoeg om af te kappen\n\
              [2026-01-01 00:00:01] a.INFO: second\n",
        );
        let mut reader = Reader::open(&path).expect("open");
        reader.wait_until_indexed().expect("index");
        assert_eq!(reader.record_count(), 2);

        File::create(&path)
            .expect("open voor afkappen")
            .set_len(10)
            .expect("afkappen tot 10 bytes");

        let result = reader.record_bytes(0);
        assert!(
            result.is_err(),
            "minder bytes dan de index verwacht moet een Err geven, geen paniek"
        );
    }
}
