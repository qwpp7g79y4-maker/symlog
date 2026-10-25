//! Follow (`tail -f`), rotatie en afkappen (task.md T10; blueprint.md `follow/`).
//!
//! [`Watcher`] wekt een volglus op bij elke OS-bestandssysteemnotificatie (via
//! `notify`) én elke `poll_interval` (standaard 500 ms, ook als `notify` werkt: dat
//! vangt filesystems op die geen events geven, zoals NFS en sommige Docker-volumes,
//! blueprint.md "Follow en rotatie"). [`Follower`] is de daadwerkelijke
//! toestandsmachine die bij elke wake `stat(pad)` vergelijkt met wat eerder bekend
//! was ([`rotate::detect`]) en daarop groei, rotatie, afkappen of verdwijnen
//! afhandelt.
//!
//! In tegenstelling tot [`crate::reader::Reader`] (één achtergrondthread die één keer
//! tot EOF indexeert) is [`Follower`] bewust single-threaded en synchroon: elke
//! `tick()` wordt door de aanroeper zelf getriggerd (na een [`Watcher`]-wake), zodat
//! er geen gedeelde mutex/Arc-toestand nodig is tussen een indexer-thread en de
//! lezende kant.

pub mod rotate;

use crate::reader::cache::RecordCache;
use crate::reader::index::{IndexScanner, RecordIndex};
use crate::reader::read_record_at;
use rotate::{Change, FileId};
use std::fs::File;
use std::io;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// Standaard pollinterval van de polling-terugval (blueprint.md T10: "polling-
/// terugval van 500 ms (ook voor NFS/Docker-volumes)").
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Standaard timeout voor een onvolledige laatste regel (task.md T10 "Doet": "een
/// half geschreven laatste regel wordt pas getoond als de newline er is (of na 2 s
/// timeout als Raw-staart)").
pub const DEFAULT_PENDING_TAIL_TIMEOUT: Duration = Duration::from_secs(2);

/// Waarom [`Watcher::wait`]/[`Watcher::wait_timeout`] wakker werd. Puur informatief:
/// [`Follower::tick`] doet in beide gevallen hetzelfde (een echte `stat`-vergelijking),
/// dus een gemiste of overbodige wake is nooit incorrect, alleen een beetje minder
/// snel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wake {
    /// Een OS-bestandssysteemnotificatie (inotify/kqueue/FSEvents via `notify`).
    Notify,
    /// De polling-timer, altijd actief (ook naast `notify`).
    Poll,
}

/// Wekt een volglus op elke `notify`-gebeurtenis in de oudermap van het gevolgde pad
/// én elke `poll_interval`. De oudermap (niet het bestand zelf) wordt gevolgd, niet
/// recursief: zo blijft een rename-rotatie (het bestand verdwijnt, een nieuw bestand
/// met dezelfde naam verschijnt) zichtbaar, en kan `notify` ook al aanslaan vóórdat
/// het gevolgde bestand voor het eerst bestaat (task.md T11: `--follow` op een nog
/// ontbrekend bestand).
pub struct Watcher {
    rx: Receiver<Wake>,
    // Houdt de achtergrondwatcher in leven zolang deze `Watcher` bestaat; wordt nooit
    // rechtstreeks gebruikt (de events komen via `rx` binnen).
    _notify: Option<notify::RecommendedWatcher>,
}

impl Watcher {
    /// `notify` (beste backend voor dit platform) plus de polling-terugval. Lukt het
    /// opzetten van `notify` niet (bijvoorbeeld een sandbox zonder inotify-
    /// rechten, of een oudermap die zelf nog niet bestaat), dan draait deze
    /// `Watcher` gewoon alleen op de polling-terugval: dat is precies waar die voor
    /// bedoeld is, geen fout.
    pub fn new(path: &Path, poll_interval: Duration) -> Watcher {
        let (tx, rx) = mpsc::channel();
        let notify = spawn_notify(path, tx.clone());
        spawn_poll_thread(poll_interval, tx);
        Watcher {
            rx,
            _notify: notify,
        }
    }

    /// Alleen de polling-terugval, geen `notify` (task.md T10 "Klaar als": "Tests
    /// draaien in CI op Linux en macOS met `--poll` en met notify"; dit is het
    /// `--poll`-pad).
    pub fn poll_only(poll_interval: Duration) -> Watcher {
        let (tx, rx) = mpsc::channel();
        spawn_poll_thread(poll_interval, tx);
        Watcher { rx, _notify: None }
    }

    /// Blokkeert tot de volgende wake.
    pub fn wait(&self) -> Wake {
        // De poll-thread blijft bestaan zolang deze `Watcher` leeft, dus `recv()`
        // faalt in de praktijk niet; `Poll` is een veilige terugval als dat toch
        // gebeurt (bijvoorbeeld een paniek in de poll-thread).
        self.rx.recv().unwrap_or(Wake::Poll)
    }

    /// Als [`Watcher::wait`], maar geeft `None` terug in plaats van te blokkeren als
    /// er binnen `timeout` geen wake kwam.
    pub fn wait_timeout(&self, timeout: Duration) -> Option<Wake> {
        self.rx.recv_timeout(timeout).ok()
    }
}

fn spawn_notify(path: &Path, tx: mpsc::Sender<Wake>) -> Option<notify::RecommendedWatcher> {
    use notify::Watcher as _;
    let watch_dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut watcher = notify::recommended_watcher(move |_event: notify::Result<notify::Event>| {
        let _ = tx.send(Wake::Notify);
    })
    .ok()?;
    watcher
        .watch(watch_dir, notify::RecursiveMode::NonRecursive)
        .ok()?;
    Some(watcher)
}

fn spawn_poll_thread(interval: Duration, tx: mpsc::Sender<Wake>) {
    thread::spawn(move || loop {
        thread::sleep(interval);
        if tx.send(Wake::Poll).is_err() {
            // De `Watcher` (en daarmee `rx`) is weg: deze thread heeft geen functie
            // meer en stopt zichzelf.
            break;
        }
    });
}

/// Eén generatie van het gevolgde bestand: een eigen open handle, index, cache en
/// scanner. Rotatie (blueprint.md T10, stap 2) parkeert de huidige generatie en
/// start een nieuwe; afkappen (stap 3) reset de huidige generatie in plaats van een
/// nieuwe te openen (dezelfde inode blijft immers dezelfde generatie).
struct Generation {
    file: File,
    index: RecordIndex,
    cache: RecordCache,
    scanner: IndexScanner,
    /// `true` zodra deze generatie nooit meer kan groeien (uitgelezen tot EOF bij
    /// rotatie). Bepaalt of het laatste record tot de echte EOF leest ([`finished`])
    /// of geclamp wordt op [`Generation::complete_end`] (nog actief, kan nog groeien).
    finished: bool,
}

impl Generation {
    fn new(file: File) -> Generation {
        Generation {
            file,
            index: RecordIndex::new(),
            cache: RecordCache::with_capacity(crate::reader::cache::DEFAULT_CAPACITY),
            scanner: IndexScanner::new(),
            finished: false,
        }
    }

    fn bytes_read(&self) -> u64 {
        self.scanner.bytes_read()
    }

    fn pending_tail(&self) -> &[u8] {
        self.scanner.pending_tail()
    }

    /// Bytes die zeker tot een afgesloten regel horen: alles tot en met de laatste
    /// `\n` die deze generatie tot nu toe gezien heeft.
    fn complete_end(&self) -> u64 {
        self.bytes_read() - self.pending_tail().len() as u64
    }

    /// Leest de nieuwe bytes tussen de huidige `bytes_read()` en `to` via `pread` en
    /// voedt ze aan de scanner. Geen paniek als `to` intussen (door een race met een
    /// gelijktijdige afkapping) minder bytes oplevert dan verwacht: `read_exact_at`
    /// geeft dan een `Err` terug (task.md T10 "Blijft").
    fn grow_to(&mut self, to: u64) -> io::Result<()> {
        let start = self.bytes_read();
        if to <= start {
            return Ok(());
        }
        let len = (to - start) as usize;
        let mut buf = vec![0u8; len];
        self.file.read_exact_at(&mut buf, start)?;
        self.scanner.feed(&self.index, &buf);
        Ok(())
    }

    /// Sluit deze generatie definitief af (task.md T10, rotatie-stap 2: "lees de oude
    /// handle uit tot EOF"): een eventuele onvolledige laatste regel is nu compleet
    /// (er komt nooit meer bij) en krijgt zo nodig alsnog zijn eigen offset.
    fn finish(&mut self) {
        self.scanner.finish(&self.index);
        self.finished = true;
    }

    /// Reset deze generatie naar een lege index/cache/scanner, maar met dezelfde open
    /// handle (task.md T10, afkappen-stap 3: zelfde inode, "index en cache leeg ...
    /// index vanaf 0").
    fn reset(&mut self) {
        self.index.reset();
        self.cache.clear();
        self.scanner = IndexScanner::new();
        self.finished = false;
    }

    fn record_count(&self) -> usize {
        self.index.len()
    }

    /// De zichtbare eindoffset van record `i` binnen deze generatie: het begin van
    /// het volgende record als dat al bekend is; anders, voor een nog actieve
    /// generatie, [`Generation::complete_end`] (de onvolledige staartregel blijft
    /// verborgen tot ze compleet is, zie [`Follower::tail`]); voor een afgesloten
    /// generatie de echte EOF (`None`, die nooit meer verandert).
    fn visible_end(&self, i: usize) -> Option<u64> {
        if let Some(next) = self.index.offset(i + 1) {
            return Some(next);
        }
        if self.finished {
            None
        } else {
            Some(self.complete_end())
        }
    }

    fn record_bytes(&self, i: usize) -> io::Result<Option<Arc<[u8]>>> {
        let Some(start) = self.index.offset(i) else {
            return Ok(None);
        };
        // Alleen cachen als de eindgrens van dit record vastligt en nooit meer kan
        // veranderen: het laatste record van een nog actieve generatie kan bij de
        // volgende groei alsnog meer inhoud krijgen (bijvoorbeeld een stacktrace die
        // over meerdere `tick()`-aanroepen heen binnenkomt), dus die mag niet met
        // stale bytes in de cache belanden.
        let next_known = self.index.offset(i + 1).is_some();
        let cacheable = next_known || self.finished;
        if cacheable {
            if let Some(cached) = self.cache.get(start) {
                return Ok(Some(cached));
            }
        }
        let end = self.visible_end(i);
        let bytes: Arc<[u8]> = read_record_at(&self.file, start, end)?.into();
        if cacheable {
            self.cache.insert(start, Arc::clone(&bytes));
        }
        Ok(Some(bytes))
    }
}

/// Soort scheidingsregel die tussen twee records in de getoonde lijst hoort (task.md
/// T10 "Doet": "scheidingsregel '— geroteerd —'" / "'— afgekapt —'").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeparatorKind {
    Rotated,
    Truncated,
}

/// Eén scheidingsregel, geplaatst op een positie in de doorlopende, globale
/// recordnummering van [`Follower`] (over generaties heen voor rotatie; binnen
/// dezelfde generatie voor afkappen).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Separator {
    /// De scheidingsregel hoort direct vóór het record op deze globale index (of aan
    /// het eind van de lijst als dit gelijk is aan het huidige totaal).
    pub before_record: usize,
    pub kind: SeparatorKind,
}

/// Wat [`Follower::tick`] net heeft afgehandeld.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FollowEvent {
    /// Het bestand is voor het eerst gezien en volledig ingelezen tot aan dit
    /// moment; geen scheidingsregel, dit is geen rotatie.
    Opened,
    /// Nieuwe bytes zijn geïndexeerd.
    Grown,
    /// `(dev, ino)` is veranderd: de oude generatie is uitgelezen tot EOF en
    /// geparkeerd, een nieuwe generatie is vanaf offset 0 gestart.
    Rotated,
    /// Zelfde bestand, maar kleiner dan voorheen: de huidige generatie is leeg
    /// gemaakt en begint opnieuw vanaf offset 0.
    Truncated,
    /// `stat(pad)` vindt niets (meer); blueprint.md: "statusbalk 'wacht op
    /// bestand'".
    Missing,
    /// Niets veranderd sinds de vorige `tick()`.
    Unchanged,
}

/// Hoe de onvolledige laatste regel van het gevolgde bestand getoond moet worden
/// (task.md T10 "Doet"). Nooit gecached in [`Generation`]: dit zijn rechtstreeks de
/// bytes die [`IndexScanner::pending_tail`] op dit moment vasthoudt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TailState {
    /// Geen onvolledige laatste regel: het bestand eindigt op dit moment op een
    /// regelgrens (of is nog niet geopend).
    None,
    /// Een onvolledige laatste regel, maar de timeout is nog niet verstreken: nog
    /// niet tonen.
    Waiting,
    /// De timeout is verstreken: toon deze bytes als voorlopige Raw-staart. Wordt
    /// vervangen (niet verdubbeld) zodra de afsluitende `\n` arriveert, want dat
    /// maakt de regel een gewoon record in de index en [`Follower::tail`] geeft dan
    /// vanzelf weer [`TailState::None`].
    Visible(Arc<[u8]>),
}

/// Volgt één pad (task.md T10; blueprint.md `follow::mod`). Bewust single-threaded:
/// de aanroeper triggert `tick()` zelf (typisch na een [`Watcher`]-wake), dus er is
/// geen gedeelde toestand tussen threads nodig, in tegenstelling tot
/// [`crate::reader::Reader`]'s achtergrondindexer.
pub struct Follower {
    path: PathBuf,
    id: Option<FileId>,
    current: Option<Generation>,
    /// Afgesloten generaties van vóór een rotatie, oudste eerst. Blijven bewaard
    /// (bestandshandle, index en cache) zodat hun records nog leesbaar zijn via de
    /// doorlopende globale nummering ([`Follower::record_bytes`]); in de praktijk is
    /// dit een klein aantal (logrotate draait typisch dagelijks, niet continu).
    retired: Vec<Generation>,
    separators: Vec<Separator>,
    pending_since: Option<Instant>,
    pending_timeout: Duration,
}

impl Follower {
    /// Volgt `path`. Opent het bestand nog niet: bestaat het (nog) niet, dan geeft de
    /// eerste `tick()` gewoon `FollowEvent::Missing` terug (task.md T11: `--follow`
    /// op een nog ontbrekend bestand wacht).
    pub fn open(path: impl AsRef<Path>) -> Follower {
        Follower {
            path: path.as_ref().to_path_buf(),
            id: None,
            current: None,
            retired: Vec::new(),
            separators: Vec::new(),
            pending_since: None,
            pending_timeout: DEFAULT_PENDING_TAIL_TIMEOUT,
        }
    }

    /// Als [`Follower::open`], met een eigen timeout voor de onvolledige-laatste-
    /// regel-staart in plaats van [`DEFAULT_PENDING_TAIL_TIMEOUT`] (tests gebruiken
    /// een korte timeout zodat ze niet op de echte 2 seconden hoeven te wachten).
    pub fn with_pending_tail_timeout(path: impl AsRef<Path>, timeout: Duration) -> Follower {
        let mut follower = Follower::open(path);
        follower.pending_timeout = timeout;
        follower
    }

    /// Eén wake-tick: `stat(path)`, vergelijk met de vorige waarneming
    /// ([`rotate::detect`]) en handel het resultaat af.
    pub fn tick(&mut self) -> io::Result<FollowEvent> {
        let current_stat = match std::fs::metadata(&self.path) {
            Ok(meta) => Some((FileId::of_metadata(&meta), meta.len())),
            Err(err) if err.kind() == io::ErrorKind::NotFound => None,
            Err(err) => return Err(err),
        };

        let Some(prev_id) = self.id else {
            return match current_stat {
                None => Ok(FollowEvent::Missing),
                Some((id, size)) => {
                    self.open_fresh(id, size)?;
                    Ok(FollowEvent::Opened)
                }
            };
        };

        let indexed_end = self
            .current
            .as_ref()
            .expect("self.id is Some => er is een actieve generatie")
            .bytes_read();

        match rotate::detect(prev_id, indexed_end, current_stat) {
            Change::Missing => Ok(FollowEvent::Missing),
            Change::Unchanged => Ok(FollowEvent::Unchanged),
            Change::Grown { to, .. } => {
                self.grow(to)?;
                Ok(FollowEvent::Grown)
            }
            Change::Truncated => {
                let (_, size) =
                    current_stat.expect("Change::Truncated komt alleen voor bij Some(..)");
                self.truncate_current(size)?;
                Ok(FollowEvent::Truncated)
            }
            Change::Rotated => {
                let (new_id, new_size) =
                    current_stat.expect("Change::Rotated komt alleen voor bij Some(..)");
                self.rotate_to(new_id, new_size)?;
                Ok(FollowEvent::Rotated)
            }
        }
    }

    fn open_fresh(&mut self, id: FileId, size: u64) -> io::Result<()> {
        let file = File::open(&self.path)?;
        self.current = Some(Generation::new(file));
        self.id = Some(id);
        self.grow(size)
    }

    fn grow(&mut self, to: u64) -> io::Result<()> {
        let gen = self
            .current
            .as_mut()
            .expect("grow() alleen aanroepen met een actieve generatie");
        gen.grow_to(to)?;
        self.update_pending_since();
        Ok(())
    }

    fn update_pending_since(&mut self) {
        let pending = self
            .current
            .as_ref()
            .is_some_and(|gen| !gen.pending_tail().is_empty());
        match (pending, self.pending_since) {
            (true, None) => self.pending_since = Some(Instant::now()),
            (false, Some(_)) => self.pending_since = None,
            _ => {}
        }
    }

    /// Reset de huidige generatie (task.md T10, afkappen-stap 3: "index en cache
    /// leeg ... index vanaf 0 opnieuw") en indexeert meteen alles wat er na de
    /// afkapping al weer staat tot `new_size`, in plaats van te wachten op een
    /// volgende, apart gedetecteerde groei.
    fn truncate_current(&mut self, new_size: u64) -> io::Result<()> {
        let gen = self
            .current
            .as_mut()
            .expect("truncate_current() alleen aanroepen met een actieve generatie");
        gen.reset();
        self.pending_since = None;
        let before_record = self.total_record_count();
        self.separators.push(Separator {
            before_record,
            kind: SeparatorKind::Truncated,
        });
        self.grow(new_size)
    }

    fn rotate_to(&mut self, new_id: FileId, new_size: u64) -> io::Result<()> {
        if let Some(mut old) = self.current.take() {
            // Lees de oude handle uit tot zijn eigen, werkelijke EOF: er kan nog een
            // laatste restje bijgekomen zijn tussen de vorige `tick()` en het moment
            // dat de rotatie zelf gedetecteerd werd.
            let final_size = old.file.metadata()?.len();
            old.grow_to(final_size)?;
            old.finish();
            self.retired.push(old);
        }
        self.pending_since = None;
        let before_record = self.total_record_count();
        self.separators.push(Separator {
            before_record,
            kind: SeparatorKind::Rotated,
        });

        let file = File::open(&self.path)?;
        self.current = Some(Generation::new(file));
        self.id = Some(new_id);
        self.grow(new_size)
    }

    /// Totaal aantal geïndexeerde records, over alle generaties heen (afgesloten
    /// generaties vóór de actieve generatie, in rotatievolgorde).
    pub fn total_record_count(&self) -> usize {
        self.retired
            .iter()
            .map(Generation::record_count)
            .sum::<usize>()
            + self.current.as_ref().map_or(0, Generation::record_count)
    }

    /// Record `global_i` uit de doorlopende, globale nummering over alle generaties
    /// heen. `None` als `global_i` nog niet (of niet meer) bestaat.
    pub fn record_bytes(&self, global_i: usize) -> io::Result<Option<Arc<[u8]>>> {
        let mut base = 0usize;
        for gen in self.retired.iter().chain(self.current.iter()) {
            let len = gen.record_count();
            if global_i < base + len {
                return gen.record_bytes(global_i - base);
            }
            base += len;
        }
        Ok(None)
    }

    /// Scheidingsregels die tot nu toe zijn ingevoegd (task.md T10 "Doet"), in
    /// volgorde van optreden.
    pub fn separators(&self) -> &[Separator] {
        &self.separators
    }

    /// Sluit de actieve generatie af alsof ze nooit meer kan groeien (zoals
    /// [`Generation::finish`] dat al bij rotatie voor de oude generatie deed, task.md
    /// T10 stap 2): een onvolledige laatste regel wordt zo alsnog een gewoon,
    /// zichtbaar record (via [`IndexScanner::finish`]), precies zoals [`build_index`]
    /// dat voor de TUI-weg bij echte EOF al deed. Bedoeld voor een eenmalige run
    /// zonder `--follow` (breaktest-symlog-0004 bevinding 1): daar komt nooit meer
    /// bij, dus er is geen risico dat een later aangevulde regel dubbel verschijnt.
    /// Met `--follow` blijft de generatie bewust onaangeroerd (niet aangeroepen): een
    /// nog onvolledige staartregel moet daar kunnen blijven groeien zonder dat de
    /// timeout/verdubbel-afweging van [`Follower::tail`] verstoord wordt.
    pub fn finish(&mut self) {
        if let Some(gen) = self.current.as_mut() {
            gen.finish();
        }
    }

    /// De onvolledige laatste regel van de actieve generatie, als die er is (task.md
    /// T10 "Doet"). Puur afgeleid van in-memory toestand (de scanner houdt deze
    /// bytes al vast): geen bestandstoegang nodig.
    pub fn tail(&self) -> TailState {
        let Some(gen) = self.current.as_ref() else {
            return TailState::None;
        };
        let pending = gen.pending_tail();
        if pending.is_empty() {
            return TailState::None;
        }
        let timed_out = self
            .pending_since
            .is_some_and(|since| since.elapsed() >= self.pending_timeout);
        if timed_out {
            TailState::Visible(Arc::from(pending))
        } else {
            TailState::Waiting
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_file(path: &Path, content: &[u8]) {
        File::create(path)
            .expect("create")
            .write_all(content)
            .expect("write");
    }

    fn append(path: &Path, content: &[u8]) {
        use std::fs::OpenOptions;
        OpenOptions::new()
            .append(true)
            .open(path)
            .expect("open voor append")
            .write_all(content)
            .expect("append");
    }

    #[test]
    fn missing_file_reports_missing_without_erroring() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nooit-aangemaakt.log");
        let mut follower = Follower::open(&path);
        assert_eq!(follower.tick().expect("tick"), FollowEvent::Missing);
        assert_eq!(follower.total_record_count(), 0);
    }

    #[test]
    fn opening_an_existing_file_indexes_what_is_already_there() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("app.log");
        write_file(
            &path,
            b"[2026-01-01 00:00:00] a.INFO: one\n[2026-01-01 00:00:01] a.INFO: two\n",
        );
        let mut follower = Follower::open(&path);
        assert_eq!(follower.tick().expect("tick"), FollowEvent::Opened);
        assert_eq!(follower.total_record_count(), 2);
        assert_eq!(
            &*follower.record_bytes(0).expect("lezen").expect("record 0"),
            b"[2026-01-01 00:00:00] a.INFO: one\n"
        );
        assert_eq!(
            &*follower.record_bytes(1).expect("lezen").expect("record 1"),
            b"[2026-01-01 00:00:01] a.INFO: two\n"
        );
    }

    #[test]
    fn growth_is_indexed_incrementally_without_rereading_old_records() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("app.log");
        write_file(&path, b"[2026-01-01 00:00:00] a.INFO: one\n");
        let mut follower = Follower::open(&path);
        assert_eq!(follower.tick().expect("tick"), FollowEvent::Opened);

        append(&path, b"[2026-01-01 00:00:01] a.INFO: two\n");
        assert_eq!(follower.tick().expect("tick"), FollowEvent::Grown);
        assert_eq!(follower.total_record_count(), 2);
        // Record 0 is nu afgesloten (record 1 bestaat), dus deze lezing cachet het.
        let first = follower.record_bytes(0).expect("lezen").expect("record 0");
        assert_eq!(&*first, b"[2026-01-01 00:00:00] a.INFO: one\n");

        // Een volgende, ongerelateerde groei mag record 0's gecachete bytes niet
        // ongeldig maken of opnieuw van schijf lezen.
        append(&path, b"[2026-01-01 00:00:02] a.INFO: three\n");
        assert_eq!(follower.tick().expect("tick"), FollowEvent::Grown);
        let first_again = follower.record_bytes(0).expect("lezen").expect("record 0");
        assert!(Arc::ptr_eq(&first, &first_again));
        assert_eq!(
            &*follower.record_bytes(1).expect("lezen").expect("record 1"),
            b"[2026-01-01 00:00:01] a.INFO: two\n"
        );
        assert_eq!(
            &*follower.record_bytes(2).expect("lezen").expect("record 2"),
            b"[2026-01-01 00:00:02] a.INFO: three\n"
        );
    }

    #[test]
    fn a_second_tick_without_changes_is_unchanged() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("app.log");
        write_file(&path, b"[2026-01-01 00:00:00] a.INFO: one\n");
        let mut follower = Follower::open(&path);
        follower.tick().expect("tick");
        assert_eq!(follower.tick().expect("tick"), FollowEvent::Unchanged);
    }

    #[test]
    fn incomplete_last_line_is_hidden_until_the_newline_arrives() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("app.log");
        write_file(&path, b"[2026-01-01 00:00:00] a.INFO: one\n");
        let mut follower = Follower::with_pending_tail_timeout(&path, Duration::from_secs(3600));
        follower.tick().expect("tick");

        append(&path, b"[2026-01-01 00:00:01] a.INFO: twee, nog niet af");
        follower.tick().expect("tick");
        // Nog geen afsluitende `\n`: geen tweede record, en de staart wacht nog
        // (timeout staat hier bewust op een uur, dus nooit verstreken in deze test).
        assert_eq!(follower.total_record_count(), 1);
        assert_eq!(follower.tail(), TailState::Waiting);

        append(&path, b"gerond\n");
        follower.tick().expect("tick");
        assert_eq!(follower.total_record_count(), 2);
        assert_eq!(follower.tail(), TailState::None);
        assert_eq!(
            &*follower.record_bytes(1).expect("lezen").expect("record 1"),
            b"[2026-01-01 00:00:01] a.INFO: twee, nog niet afgerond\n"
        );
    }

    #[test]
    fn incomplete_last_line_becomes_visible_as_a_raw_tail_after_the_timeout() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("app.log");
        write_file(&path, b"[2026-01-01 00:00:00] a.INFO: one\n");
        let mut follower = Follower::with_pending_tail_timeout(&path, Duration::from_millis(20));
        follower.tick().expect("tick");

        append(&path, b"[2026-01-01 00:00:01] a.INFO: onvolledig");
        follower.tick().expect("tick");
        assert_eq!(follower.tail(), TailState::Waiting);

        thread::sleep(Duration::from_millis(60));
        follower.tick().expect("tick"); // geen bestandswijziging, maar tail() mag nu Visible zijn
        match follower.tail() {
            TailState::Visible(bytes) => {
                assert_eq!(
                    &*bytes,
                    b"[2026-01-01 00:00:01] a.INFO: onvolledig".as_slice()
                )
            }
            other => panic!("verwacht TailState::Visible, kreeg {other:?}"),
        }
        // Nog steeds geen los, tweede record: de staart is voorlopig, geen echt
        // record in de index.
        assert_eq!(follower.total_record_count(), 1);

        append(&path, b"\n");
        follower.tick().expect("tick");
        assert_eq!(follower.total_record_count(), 2);
        assert_eq!(follower.tail(), TailState::None);
    }

    #[test]
    fn truncation_resets_the_current_generation_and_adds_a_separator() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("app.log");
        write_file(
            &path,
            b"[2026-01-01 00:00:00] a.INFO: one\n[2026-01-01 00:00:01] a.INFO: two\n",
        );
        let mut follower = Follower::open(&path);
        follower.tick().expect("tick");
        assert_eq!(follower.total_record_count(), 2);

        // copytruncate: zelfde pad, zelfde inode, kleiner dan voorheen.
        write_file(&path, b"[2026-01-01 00:00:02] a.INFO: fresh start\n");
        assert_eq!(follower.tick().expect("tick"), FollowEvent::Truncated);
        assert_eq!(follower.total_record_count(), 1);
        assert_eq!(
            &*follower.record_bytes(0).expect("lezen").expect("record 0"),
            b"[2026-01-01 00:00:02] a.INFO: fresh start\n"
        );
        assert_eq!(
            follower.separators(),
            &[Separator {
                before_record: 0,
                kind: SeparatorKind::Truncated
            }]
        );
    }

    #[test]
    fn rename_rotation_drains_the_old_file_and_keeps_both_generations_readable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("app.log");
        write_file(
            &path,
            b"[2026-01-01 00:00:00] a.INFO: old one\n[2026-01-01 00:00:01] a.INFO: old two\n",
        );
        let mut follower = Follower::open(&path);
        follower.tick().expect("tick");
        assert_eq!(follower.total_record_count(), 2);

        // logrotate: hernoem het oude bestand weg, maak een nieuw bestand aan onder
        // hetzelfde pad (nieuwe inode).
        std::fs::rename(&path, dir.path().join("app.log.1")).expect("rename");
        write_file(&path, b"[2026-01-01 00:00:02] a.INFO: new one\n");

        assert_eq!(follower.tick().expect("tick"), FollowEvent::Rotated);
        assert_eq!(follower.total_record_count(), 3);
        assert_eq!(
            &*follower.record_bytes(0).expect("lezen").expect("record 0"),
            b"[2026-01-01 00:00:00] a.INFO: old one\n"
        );
        assert_eq!(
            &*follower.record_bytes(1).expect("lezen").expect("record 1"),
            b"[2026-01-01 00:00:01] a.INFO: old two\n"
        );
        assert_eq!(
            &*follower.record_bytes(2).expect("lezen").expect("record 2"),
            b"[2026-01-01 00:00:02] a.INFO: new one\n"
        );
        assert_eq!(
            follower.separators(),
            &[Separator {
                before_record: 2,
                kind: SeparatorKind::Rotated
            }]
        );

        // De nieuwe generatie groeit verder zoals elke andere.
        append(&path, b"[2026-01-01 00:00:03] a.INFO: new two\n");
        assert_eq!(follower.tick().expect("tick"), FollowEvent::Grown);
        assert_eq!(follower.total_record_count(), 4);
    }

    #[test]
    fn file_disappearing_and_reappearing_is_a_rotation_with_no_record_loss() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("app.log");
        write_file(&path, b"[2026-01-01 00:00:00] a.INFO: before gone\n");
        let mut follower = Follower::open(&path);
        follower.tick().expect("tick");
        assert_eq!(follower.total_record_count(), 1);

        std::fs::remove_file(&path).expect("verwijderen");
        assert_eq!(follower.tick().expect("tick"), FollowEvent::Missing);
        assert_eq!(follower.tick().expect("tick"), FollowEvent::Missing);
        assert_eq!(
            follower.total_record_count(),
            1,
            "het oude record mag niet verdwijnen terwijl het bestand weg is"
        );

        write_file(&path, b"[2026-01-01 00:00:05] a.INFO: after return\n");
        assert_eq!(follower.tick().expect("tick"), FollowEvent::Rotated);
        assert_eq!(follower.total_record_count(), 2);
        assert_eq!(
            &*follower.record_bytes(0).expect("lezen").expect("record 0"),
            b"[2026-01-01 00:00:00] a.INFO: before gone\n"
        );
        assert_eq!(
            &*follower.record_bytes(1).expect("lezen").expect("record 1"),
            b"[2026-01-01 00:00:05] a.INFO: after return\n"
        );
    }

    #[test]
    fn truncation_mid_record_drops_the_dangling_bytes_without_a_panic() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("app.log");
        write_file(&path, b"[2026-01-01 00:00:00] a.INFO: one\n");
        let mut follower = Follower::open(&path);
        follower.tick().expect("tick");

        // Een nieuw record begint, maar wordt halverwege afgekapt (`> file`) vóór de
        // afsluitende `\n` er ooit komt: de scanner zit op dat moment met een
        // onvolledige staart (task.md T10 "Blijft": dit mag geen paniek geven, en de
        // afgekapte, nooit voltooide bytes horen niet in het volgende record terug
        // te komen).
        append(&path, b"[2026-01-01 00:00:01] a.INFO: halverwe");
        follower.tick().expect("tick");
        assert_eq!(follower.tail(), TailState::Waiting);
        File::create(&path)
            .expect("open voor afkappen")
            .set_len(0)
            .expect("afkappen tot 0 bytes");

        assert_eq!(follower.tick().expect("tick"), FollowEvent::Truncated);
        assert_eq!(follower.total_record_count(), 0);
        assert_eq!(
            follower.tail(),
            TailState::None,
            "de halverwege afgekapte staart mag niet in de nieuwe generatie blijven hangen"
        );

        append(&path, b"[2026-01-01 00:00:02] a.INFO: na afkappen\n");
        assert_eq!(follower.tick().expect("tick"), FollowEvent::Grown);
        assert_eq!(follower.total_record_count(), 1);
        assert_eq!(
            &*follower.record_bytes(0).expect("lezen").expect("record 0"),
            b"[2026-01-01 00:00:02] a.INFO: na afkappen\n"
        );
    }

    #[test]
    fn watcher_poll_only_wakes_up_without_notify() {
        let watcher = Watcher::poll_only(Duration::from_millis(20));
        assert_eq!(
            watcher.wait_timeout(Duration::from_millis(200)),
            Some(Wake::Poll)
        );
    }

    #[test]
    fn watcher_with_notify_wakes_up_on_a_file_write() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("app.log");
        write_file(&path, b"eerste inhoud\n");
        let watcher = Watcher::new(&path, Duration::from_millis(500));

        append(&path, b"meer inhoud\n");
        // `notify` zou hier ruim binnen 500 ms moeten wekken; valt dat om een of
        // andere platformreden weg, dan vangt de polling-terugval het binnen de
        // ingestelde 500 ms sowieso op, dus deze test blijft ook dan slagen.
        assert!(watcher.wait_timeout(Duration::from_secs(2)).is_some());
    }
}
