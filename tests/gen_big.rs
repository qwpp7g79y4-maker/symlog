//! task.md T12, "Grote bestanden en geheugendoel (benchmark)".
//!
//! Genereert een groot, gemengd lijn/JSON-logbestand met 5% stacktraces (de
//! `write_mixed_log`-generator hieronder, wat task.md T12 "Doet" aanduidt als
//! `tests/gen_big.rs`) en meet daarop de prestatiedoelen uit blueprint.md
//! "Prestatiedoel": piek-RSS, eerste scherm, volledige index, `G` na index, en het
//! `--level error`-filter (met een gelijktijdige "toets" om te bewijzen dat filteren
//! niet blokkeert). `peak_rss_bytes` leest `/proc/self/status`'s `VmHWM` op Linux en
//! `getrusage(RUSAGE_SELF)` op macOS (task.md T12 "Doet"), zonder een nieuwe crate: dat
//! zou tegen "Geen nieuwe crate buiten de lijst zonder melding" ingaan, en `libc` staat
//! niet in de gescreende lijst van blueprint.md, ook al komt die al transitief mee via
//! `notify`.
//!
//! Task.md T12 "Blijft": de 2 GiB-fixture wordt gegenereerd, nooit ingecheckt, en de
//! bijbehorende test ([`bench_mem_2gib_full_fixture`]) is `#[ignore]` (handmatig onder
//! de geheugenwachter). De 256 MiB-CI-variant ([`bench_mem_256mib_ci_fixture`]) draait
//! wél standaard mee met `cargo test`, met **evenredig** verlaagde tijdsbudgetten (een
//! achtste van de 2 GiB-doelen: 1/8 van 2 GiB ≈ 256 MiB); het piek-RSS-budget en de
//! grootte-onafhankelijke budgetten (eerste scherm, `G`, toetsrespons) blijven gelijk.
//! De harde getallen uit blueprint.md gelden expliciet alleen "in release" (en debug
//! heeft geen vergelijkbare optimalisaties), dus [`run_benchmark`] handhaaft de
//! prestatiebudgetten alleen zonder `debug_assertions`; de juistheidscontroles
//! (recordaantallen, geen verlies, exact de verwachte ERROR-matches) gelden altijd.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use symlog::filter::{FilterSpec, LevelFilter};
use symlog::parse::parse_record_lossy;
use symlog::parse::raw::ParsedRecord;
use symlog::reader::Reader;

const CHANNELS: &[&str] = &[
    "app",
    "request",
    "doctrine",
    "security",
    "php",
    "console",
    "messenger",
    "deprecation",
];
const LEVEL_NAMES: &[&str] = &[
    "DEBUG",
    "INFO",
    "NOTICE",
    "WARNING",
    "ERROR",
    "CRITICAL",
    "ALERT",
    "EMERGENCY",
];
const LEVEL_INTS: &[i64] = &[100, 200, 250, 300, 400, 500, 550, 600];

/// Elk 20e record (5%) krijgt een stacktrace (task.md T12 "Doet": "5% stacktraces").
const STACKTRACE_EVERY: usize = 20;
/// Microseconden tussen twee opeenvolgende records; bij het grootste geplande bestand
/// (2 GiB, ruwweg tientallen miljoenen records) blijft de teller ruim binnen één dag
/// (zie [`format_datetime`]), dus geen kalenderoverloop mogelijk.
const MICROS_PER_RECORD: u64 = 1500;
/// Opvultekst zodat een gewoon record ongeveer even lang is als blueprint.md's eigen
/// aanname voor het geheugenbudget ("index ≈ 8 B/record (bij ~10 M records ≈ 80 MB)",
/// dus zo'n 200 bytes/record voor een 2 GiB-bestand).
const FILLER_LEN: usize = 120;
const LAST_RECORD_MESSAGE: &str = "LAST-RECORD-MARKER";

/// Resultaat van [`write_mixed_log`]: wat de rest van de benchmark nodig heeft om de
/// index en het filterresultaat tegen de echte, gegenereerde inhoud te controleren
/// (geen giswerk over hoeveel records of ERROR-matches er "zouden moeten" zijn).
struct GeneratedLog {
    /// Totaal aantal records, inclusief het laatste markerrecord.
    records: usize,
    /// Aantal records met niveau ERROR (voor de `--level error`-filtertest).
    error_records: usize,
}

/// Formatteert een strikt stijgende tijd binnen één dag als Monolog-datetime met een
/// vaste offset (`+01:00`). Puur integer-rekenwerk: elk veld blijft door de modulo
/// altijd in zijn geldige bereik, dus dit kan nooit een ongeldige kalenderdatum
/// opleveren (in tegenstelling tot echte kalenderwiskunde) en heeft geen tijdzone- of
/// kalendercrate nodig — task.md T12 heeft alleen een syntactisch geldige, oplopende
/// datetime-kop nodig (voor `looks_like_datetime`/`Ts::parse`), geen echte kalenderdata.
fn format_datetime(micros_since_midnight: u64) -> String {
    let hour = (micros_since_midnight / 3_600_000_000) % 24;
    let minute = (micros_since_midnight / 60_000_000) % 60;
    let second = (micros_since_midnight / 1_000_000) % 60;
    let micros = micros_since_midnight % 1_000_000;
    format!("2026-03-01T{hour:02}:{minute:02}:{second:02}.{micros:06}+01:00")
}

fn line_record(dt: &str, channel: &str, level_name: &str, idx: usize, filler: &str) -> String {
    format!("[{dt}] {channel}.{level_name}: generated record #{idx} {filler} [] []\n")
}

fn json_record(
    dt: &str,
    channel: &str,
    level_name: &str,
    level_int: i64,
    idx: usize,
    filler: &str,
) -> String {
    let value = serde_json::json!({
        "message": format!("generated record #{idx} {filler}"),
        "level": level_int,
        "level_name": level_name,
        "channel": channel,
        "datetime": dt,
    });
    format!("{value}\n")
}

/// Zes "gewone" frames plus een `{main}`-afsluiter (task.md T4-stijl), met context en
/// extra op de laatste fysieke regel (zoals F3, `tests/fixtures/line-stacktrace.log`):
/// `%message%` is een kale stringvervanging, dus de trailing JSON hoort aan het eind
/// van het hele, meerregelige blok, niet na de eerste regel.
fn line_stacktrace_record(dt: &str, channel: &str, level_name: &str, idx: usize) -> String {
    let mut block = format!(
        "[{dt}] {channel}.{level_name}: Uncaught RuntimeException: generated failure #{idx} in /var/www/src/Worker.php:88\nStack trace:\n"
    );
    for frame in 0..6 {
        block.push_str(&format!(
            "#{frame} /var/www/src/Dispatcher.php({line}): App\\Worker->runStep({frame})\n",
            line = 40 + frame,
        ));
    }
    block.push_str("#6 {main} [] []\n");
    block
}

/// JSON-tegenhanger van [`line_stacktrace_record`]: `context.exception.file` plus
/// `trace` (task.md T4, dezelfde framesyntax). `file` levert zelf ook een frame op
/// ([`crate::parse::trace::frames_from_exception_json`] in de hoofdcrate), dus dit
/// record heeft één frame meer dan de lijnformaat-variant.
fn json_stacktrace_record(
    dt: &str,
    channel: &str,
    level_name: &str,
    level_int: i64,
    idx: usize,
) -> String {
    let trace: Vec<String> = (0..6)
        .map(|frame| {
            format!(
                "#{frame} /var/www/src/Dispatcher.php({line}): App\\Worker->run()",
                line = 40 + frame
            )
        })
        .chain(std::iter::once("#6 {main}".to_string()))
        .collect();
    let value = serde_json::json!({
        "message": format!("Uncaught RuntimeException: generated failure #{idx}"),
        "level": level_int,
        "level_name": level_name,
        "channel": channel,
        "datetime": dt,
        "context": {
            "exception": {
                "file": "/var/www/src/Worker.php:88",
                "trace": trace,
            }
        }
    });
    format!("{value}\n")
}

/// Schrijft een gemengd lijn/JSON-logbestand van minstens `target_bytes` naar `path`
/// (task.md T12 "Doet"), met 5% stacktraces en een uniek, herkenbaar laatste record
/// (voor de `G`-sprongtest). Levert de werkelijke aantallen terug ([`GeneratedLog`]) in
/// plaats van vooraf te schatten, zodat de benchmark zijn eigen uitkomst tegen de echte
/// inhoud controleert.
fn write_mixed_log(path: &Path, target_bytes: u64) -> GeneratedLog {
    let file = File::create(path).expect("maak fixture-bestand aan");
    let mut writer = BufWriter::with_capacity(1 << 20, file);
    let filler = "x".repeat(FILLER_LEN);

    let mut written: u64 = 0;
    let mut idx: usize = 0;
    let mut error_records = 0usize;

    while written < target_bytes {
        let channel = CHANNELS[idx % CHANNELS.len()];
        let level_pos = idx % LEVEL_NAMES.len();
        let level_name = LEVEL_NAMES[level_pos];
        let level_int = LEVEL_INTS[level_pos];
        if level_name == "ERROR" {
            error_records += 1;
        }
        let dt = format_datetime(idx as u64 * MICROS_PER_RECORD);
        let has_stacktrace = idx.is_multiple_of(STACKTRACE_EVERY);
        // `idx % 2` kiezen voor de stacktrace-variant zou altijd dezelfde kant kiezen
        // (STACKTRACE_EVERY is even, dus `idx % STACKTRACE_EVERY == 0` impliceert
        // `idx % 2 == 0`); de teller van het aantal stacktrace-records zelf wisselt wel
        // evenwichtig tussen lijn- en JSON-vorm.
        let record = if has_stacktrace {
            if (idx / STACKTRACE_EVERY).is_multiple_of(2) {
                line_stacktrace_record(&dt, channel, level_name, idx)
            } else {
                json_stacktrace_record(&dt, channel, level_name, level_int, idx)
            }
        } else if idx.is_multiple_of(2) {
            line_record(&dt, channel, level_name, idx, &filler)
        } else {
            json_record(&dt, channel, level_name, level_int, idx, &filler)
        };

        writer.write_all(record.as_bytes()).expect("schrijf record");
        written += record.len() as u64;
        idx += 1;
    }

    let marker_dt = format_datetime(idx as u64 * MICROS_PER_RECORD);
    let marker = format!("[{marker_dt}] app.CRITICAL: {LAST_RECORD_MESSAGE} [] []\n");
    writer.write_all(marker.as_bytes()).expect("schrijf marker");
    writer.flush().expect("flush fixture-bestand");

    GeneratedLog {
        records: idx + 1,
        error_records,
    }
}

/// Piek-RSS van dit proces in bytes (task.md T12 "Doet"). Linux leest de "high water
/// mark" rechtstreeks uit `/proc/self/status` (`VmHWM`, in kB); macOS heeft geen
/// `/proc` en gebruikt daarom `getrusage(RUSAGE_SELF)` (`ru_maxrss`, al in **bytes** op
/// Darwin — anders dan Linux' eigen `getrusage`, dat in kB rapporteert, zie de
/// moduledoc hierboven voor waarom dit geen `libc`-afhankelijkheid gebruikt).
fn peak_rss_bytes() -> u64 {
    peak_rss_bytes_impl()
}

#[cfg(target_os = "linux")]
fn peak_rss_bytes_impl() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").expect("/proc/self/status lezen");
    for line in status.lines() {
        if line.starts_with("VmHWM:") {
            let kb: u64 = line
                .split_whitespace()
                .nth(1)
                .and_then(|s| s.parse().ok())
                .expect("VmHWM-waarde parsen");
            return kb * 1024;
        }
    }
    panic!("VmHWM niet gevonden in /proc/self/status");
}

#[cfg(target_os = "macos")]
fn peak_rss_bytes_impl() -> u64 {
    // Minimale, met de hand geschreven FFI-binding voor `getrusage(2)`: geen
    // `libc`-crate (niet in de gescreende lijst, zie de moduledoc). `timeval` is op
    // zowel Linux als Darwin 16 bytes groot (op Darwin is `tv_usec` een `i32` met 4
    // bytes opvulling erna, maar de totale structgrootte en dus de offset van
    // `ru_maxrss` erna komt overeen), dus twee `i64`-velden volstaan hier: alleen de
    // offset van `ru_maxrss` telt, `tv_usec` zelf wordt nergens gelezen. Deze
    // Darwin-tak is op dit platform (Linux) nooit gecompileerd of uitgevoerd, dus
    // zonder echte macOS-hardware ter controle — door de eigenaar te bevestigen op
    // macOS-CI (zie docs/perf.md).
    #[repr(C)]
    struct Timeval {
        tv_sec: i64,
        tv_usec: i64,
    }
    #[repr(C)]
    struct RUsage {
        ru_utime: Timeval,
        ru_stime: Timeval,
        ru_maxrss: i64,
        ru_ixrss: i64,
        ru_idrss: i64,
        ru_isrss: i64,
        ru_minflt: i64,
        ru_majflt: i64,
        ru_nswap: i64,
        ru_inblock: i64,
        ru_oublock: i64,
        ru_msgsnd: i64,
        ru_msgrcv: i64,
        ru_nsignals: i64,
        ru_nvcsw: i64,
        ru_nivcsw: i64,
    }
    const RUSAGE_SELF: i32 = 0;
    extern "C" {
        fn getrusage(who: i32, usage: *mut RUsage) -> i32;
    }
    let mut usage: RUsage = unsafe { std::mem::zeroed() };
    let rc = unsafe { getrusage(RUSAGE_SELF, &mut usage) };
    assert_eq!(rc, 0, "getrusage(RUSAGE_SELF) mislukte");
    usage.ru_maxrss as u64
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn peak_rss_bytes_impl() -> u64 {
    panic!("piek-RSS-meting is alleen geïmplementeerd voor Linux en macOS (task.md open punt 8: Windows wordt niet ondersteund)");
}

/// Prestatiedoelen uit blueprint.md "Prestatiedoel", voor één fixturegrootte.
struct Budgets {
    peak_rss_bytes: u64,
    first_screen: Duration,
    full_index: Duration,
    jump_to_last: Duration,
    level_filter: Duration,
    keypress_latency: Duration,
}

/// De letterlijke doelen uit blueprint.md, voor de 2 GiB-fixture in release.
fn release_budgets_2gib() -> Budgets {
    Budgets {
        peak_rss_bytes: 150 * 1024 * 1024,
        first_screen: Duration::from_millis(200),
        full_index: Duration::from_secs(10),
        jump_to_last: Duration::from_millis(50),
        level_filter: Duration::from_secs(15),
        keypress_latency: Duration::from_millis(100),
    }
}

/// De 256 MiB-CI-variant (task.md T12 "Blijft": "een evenredig doel"): tijdsbudgetten
/// die met de bestandsgrootte meeschalen (hier een achtste van de 2 GiB-doelen) worden
/// verlaagd; piek-RSS en de grootte-onafhankelijke budgetten (eerste scherm, `G` na
/// index, toetsrespons tijdens filteren) blijven gelijk aan de 2 GiB-doelen.
fn ci_budgets_256mib() -> Budgets {
    let full = release_budgets_2gib();
    Budgets {
        peak_rss_bytes: full.peak_rss_bytes,
        first_screen: full.first_screen,
        full_index: full.full_index / 8,
        jump_to_last: full.jump_to_last,
        level_filter: full.level_filter / 8,
        keypress_latency: full.keypress_latency,
    }
}

/// Genereert de fixture, meet alle prestatiedoelen uit blueprint.md en controleert ze
/// altijd op juistheid (recordaantallen, geen verlies, exact de verwachte
/// ERROR-matches). De harde getallen in `budgets` gelden expliciet "in release"
/// (task.md T12 "Klaar als"); zonder optimalisaties (`cfg!(debug_assertions)`) print dit
/// de gemeten cijfers nog steeds (zichtbaar met `--nocapture`), maar handhaaft ze niet
/// als harde assert, om een trage debug-build niet kunstmatig te laten falen.
fn run_benchmark(label: &str, target_bytes: u64, budgets: &Budgets) {
    let enforce_perf_budgets = !cfg!(debug_assertions);

    let dir = tempfile::tempdir().expect("tempdir voor de fixture");
    let path = dir.path().join("big.log");

    let gen_start = Instant::now();
    let generated = write_mixed_log(&path, target_bytes);
    let fixture_bytes = std::fs::metadata(&path)
        .expect("metadata van de fixture")
        .len();
    eprintln!(
        "[{label}] fixture gegenereerd: {} records ({} ERROR), {fixture_bytes} bytes in {:?}",
        generated.records,
        generated.error_records,
        gen_start.elapsed()
    );

    // Eerste scherm (blueprint.md "Prestatiedoel": "< 200 ms, ongeacht grootte"): de
    // achtergrondindexer hoeft alleen het eerste venster te vinden, niet het hele
    // bestand, dus dit meet vanaf `Reader::open` tot het eerste scherm gelezen én
    // geparst is — niet tot de volledige index klaar is (dat is de volgende meting).
    const WINDOW: usize = 50;
    let first_screen_start = Instant::now();
    let mut reader = Reader::open(&path).expect("open reader");
    while reader.record_count() < WINDOW {
        std::thread::sleep(Duration::from_millis(1));
    }
    for i in 0..WINDOW {
        reader
            .record_bytes(i)
            .expect("lees venster-record")
            .expect("venster-record moet bestaan, de index meldde het al");
    }
    let first_screen_elapsed = first_screen_start.elapsed();

    let index_start = Instant::now();
    reader.wait_until_indexed().expect("index opbouwen");
    let full_index_elapsed = index_start.elapsed();

    assert_eq!(
        reader.record_count(),
        generated.records,
        "[{label}] de index moet elk gegenereerd record vinden, geen verlies of dubbelen"
    );

    let jump_start = Instant::now();
    let last = reader
        .last_record_bytes()
        .expect("laatste record lezen")
        .expect("er is minstens één record");
    let jump_elapsed = jump_start.elapsed();
    assert!(
        String::from_utf8_lossy(&last).contains(LAST_RECORD_MESSAGE),
        "[{label}] G moet het echte laatste record tonen, niet een ander record"
    );

    let spec = FilterSpec {
        levels: Some(LevelFilter::parse("error").expect("niveau 'error' parseert")),
        ..Default::default()
    };

    // Toetsrespons tijdens filteren (blueprint.md "Prestatiedoel": "< 100 ms"): het
    // filter draait op een eigen thread (zoals de echte TUI dat doet, zie `filter.rs`'
    // moduledoc), terwijl de "hoofddraad" herhaaldelijk record 0 leest — dezelfde
    // cache-mutex die het filter zelf ook per record kort vasthoudt, dus dit meet
    // echte lock-contentie, niet een stroman die niets deelt met de filterthread.
    let mut max_keypress_latency = Duration::ZERO;
    let filter_start = Instant::now();
    let filtered = std::thread::scope(|scope| {
        let handle = scope.spawn(|| symlog::filter::filtered_offsets(&reader, &spec));
        while !handle.is_finished() {
            let tick = Instant::now();
            let _ = reader.record_bytes(0);
            max_keypress_latency = max_keypress_latency.max(tick.elapsed());
            std::thread::sleep(Duration::from_millis(5));
        }
        handle
            .join()
            .expect("filter-thread paniekte niet")
            .expect("filteren faalt niet")
    });
    let filter_elapsed = filter_start.elapsed();

    assert_eq!(
        filtered.len(),
        generated.error_records,
        "[{label}] --level error moet precies de gegenereerde ERROR-records vinden"
    );

    let peak_rss = peak_rss_bytes();
    eprintln!(
        "[{label}] piek-RSS={peak_rss} bytes, eerste scherm={first_screen_elapsed:?}, \
         volledige index={full_index_elapsed:?}, G-na-index={jump_elapsed:?}, \
         filter(--level error)={filter_elapsed:?} over {} matches, \
         traagste 'toets' tijdens filteren={max_keypress_latency:?}, \
         budgetten gehandhaafd={enforce_perf_budgets}",
        filtered.len()
    );

    if !enforce_perf_budgets {
        return;
    }

    assert!(
        peak_rss <= budgets.peak_rss_bytes,
        "[{label}] piek-RSS {peak_rss} bytes overschrijdt het budget van {} bytes",
        budgets.peak_rss_bytes
    );
    assert!(
        first_screen_elapsed < budgets.first_screen,
        "[{label}] eerste scherm duurde {first_screen_elapsed:?}, budget is {:?}",
        budgets.first_screen
    );
    assert!(
        full_index_elapsed < budgets.full_index,
        "[{label}] volledige index duurde {full_index_elapsed:?}, budget is {:?}",
        budgets.full_index
    );
    assert!(
        jump_elapsed < budgets.jump_to_last,
        "[{label}] G na index duurde {jump_elapsed:?}, budget is {:?}",
        budgets.jump_to_last
    );
    assert!(
        filter_elapsed < budgets.level_filter,
        "[{label}] filter --level error duurde {filter_elapsed:?}, budget is {:?}",
        budgets.level_filter
    );
    assert!(
        max_keypress_latency < budgets.keypress_latency,
        "[{label}] traagste 'toets' tijdens filteren duurde {max_keypress_latency:?}, budget is {:?}",
        budgets.keypress_latency
    );
}

/// Task.md T12 "Klaar als", letterlijk op de 2 GiB-fixture in release. Task.md T12
/// "Blijft": handmatig draaien onder de geheugenwachter (`cargo test --release
/// --ignored bench_mem_2gib`), nooit automatisch in de gewone testrun.
#[test]
#[ignore = "2 GiB-fixture: handmatig draaien onder de geheugenwachter, zie task.md T12"]
fn bench_mem_2gib_full_fixture() {
    run_benchmark(
        "bench_mem_2gib",
        2 * 1024 * 1024 * 1024,
        &release_budgets_2gib(),
    );
}

/// Task.md T12 "Blijft": "In CI draait een 256 MB-variant met een evenredig doel."
/// Deze test is bewust niet `#[ignore]`, zodat zowel de gewone testrun (debug, alleen
/// juistheid) als de losse `bench-mem`-CI-job (`cargo test --release bench_mem`, met
/// gehandhaafde budgetten) hem meenemen.
#[test]
fn bench_mem_256mib_ci_fixture() {
    run_benchmark("bench_mem_256mib", 256 * 1024 * 1024, &ci_budgets_256mib());
}

/// Edge case (blueprint.md): elk gegenereerd record moet als `Record` parsen, nooit als
/// Raw-terugval — anders zou de `--level error`-telling in [`run_benchmark`] stil
/// kunnen afwijken (een Raw-record negeert het niveaufilter altijd wanneer er geen
/// zoekterm is, zie `filter.rs`' moduledoc), zonder dat de oorzaak duidelijk is.
#[test]
fn generated_fixture_has_no_raw_fallback_records() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("tiny.log");
    let generated = write_mixed_log(&path, 64 * 1024);

    let mut reader = Reader::open(&path).expect("open reader");
    reader.wait_until_indexed().expect("index opbouwen");
    assert_eq!(reader.record_count(), generated.records);

    for i in 0..reader.record_count() {
        let bytes = reader
            .record_bytes(i)
            .expect("lees record")
            .expect("record bestaat");
        let parsed = parse_record_lossy(&bytes);
        assert!(
            parsed.as_raw().is_none(),
            "record {i} viel terug op Raw in plaats van te parsen: {:?}",
            String::from_utf8_lossy(&bytes)
        );
    }
}

/// Edge case (blueprint.md T4): de gegenereerde stacktrace-records (5%, zowel
/// lijnformaat als JSON) moeten echt frames opleveren, met het verwachte aantal voor
/// elke vorm (lijnformaat: 6 padframes + `{main}` = 7; JSON: ook de `file`-werplocatie
/// erbij, dus 8).
#[test]
fn generated_stacktrace_records_produce_frames() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("tiny-stacktrace.log");
    // STACKTRACE_EVERY=20, dus 64 KiB bevat ruim voldoende stacktrace-records van
    // beide vormen (lijn en JSON wisselen elkaar af, zie write_mixed_log).
    write_mixed_log(&path, 64 * 1024);

    let mut reader = Reader::open(&path).expect("open reader");
    reader.wait_until_indexed().expect("index opbouwen");

    let mut saw_line_stacktrace = false;
    let mut saw_json_stacktrace = false;
    for i in 0..reader.record_count() {
        let bytes = reader
            .record_bytes(i)
            .expect("lees record")
            .expect("record bestaat");
        let parsed = parse_record_lossy(&bytes);
        let ParsedRecord::Record(record) = &parsed else {
            continue;
        };
        match record.frames.len() {
            7 => saw_line_stacktrace = true,
            8 => saw_json_stacktrace = true,
            0 => {}
            other => panic!("onverwacht aantal frames voor record {i}: {other}"),
        }
    }
    assert!(
        saw_line_stacktrace,
        "fixture moet minstens één lijnformaat-stacktrace bevatten (7 frames)"
    );
    assert!(
        saw_json_stacktrace,
        "fixture moet minstens één JSON-stacktrace bevatten (8 frames)"
    );
}

/// De kop-scan in `filtered_offsets` (niveaufilter zonder zoekterm) moet dezelfde
/// offsets opleveren als de volledige parse, ook voorbij de leesbatch van 4096
/// records. Dit is geen prestatietest: het budget blijft bij
/// [`bench_mem_256mib_ci_fixture`].
#[test]
fn level_filter_matches_full_parse_across_read_batches() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("batch.log");
    let generated = write_mixed_log(&path, 1_500_000);
    let mut reader = Reader::open(&path).expect("open");
    reader.wait_until_indexed().expect("index");
    assert!(
        reader.record_count() > 4096,
        "de fixture moet minstens twee leesbatches beslaan"
    );

    let spec = FilterSpec {
        levels: Some(LevelFilter::parse("error").expect("error")),
        ..Default::default()
    };
    let fast = symlog::filter::filtered_offsets(&reader, &spec).expect("snel pad");
    let mut slow = Vec::new();
    for i in 0..reader.record_count() {
        let bytes = reader.record_bytes(i).expect("io").expect("record");
        let offset = reader.index().offset(i).expect("offset");
        let parsed = parse_record_lossy(&bytes);
        if spec.matches_parsed(&parsed) {
            slow.push(offset);
        }
    }
    assert_eq!(fast, slow);
    assert_eq!(fast.len(), generated.error_records);

    let search = FilterSpec {
        levels: Some(LevelFilter::parse(">=warning").expect("warning")),
        search: Some(regex::Regex::new("generated failure").expect("re")),
        ..Default::default()
    };
    let fast_search = symlog::filter::filtered_offsets(&reader, &search).expect("zoekpad");
    let mut slow_search = Vec::new();
    for i in 0..reader.record_count() {
        let bytes = reader.record_bytes(i).expect("io").expect("record");
        let offset = reader.index().offset(i).expect("offset");
        if search.matches_parsed(&parse_record_lossy(&bytes)) {
            slow_search.push(offset);
        }
    }
    assert_eq!(fast_search, slow_search);
    assert!(!slow_search.is_empty());
}
