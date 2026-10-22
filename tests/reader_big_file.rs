//! task.md T2, "Klaar als": `G` op een bestand van 1 GiB (gegenereerd in tmp) toont
//! binnen het prestatiedoel de laatste record.
//!
//! De formele meting van de prestatiedoelen (release-build, CI-hardware) is T12;
//! deze test is een regressiebewaker die vastlegt dat het lezen van het laatste
//! record via de offset-index gebeurt (een paar bytes lezen), niet via een volledige
//! scan van het bestand.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::{Duration, Instant};

use symlog::reader::Reader;

const ONE_GIB: u64 = 1024 * 1024 * 1024;
const LAST_RECORD: &[u8] = b"[2026-01-15T23:59:59.000000+01:00] app.CRITICAL: LAST-RECORD-MARKER\n";

#[test]
fn jumping_to_last_record_of_1gib_file_is_fast() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("big.log");
    generate_1gib_log(&path);

    let mut reader = Reader::open(&path).expect("open");
    reader.wait_until_indexed().expect("index opbouwen");
    assert!(reader.record_count() > 1);

    let start = Instant::now();
    let last = reader
        .last_record_bytes()
        .expect("laatste record lezen")
        .expect("er is minstens één record");
    let elapsed = start.elapsed();

    assert_eq!(&*last, LAST_RECORD);
    assert!(
        elapsed < Duration::from_millis(200),
        "laatste record lezen duurde {elapsed:?}; de offset-index zou dit O(1) moeten maken"
    );
}

fn generate_1gib_log(path: &std::path::Path) {
    let file = File::create(path).expect("create");
    let mut writer = BufWriter::new(file);
    // Elke regel is zelf een geldige recordkop (task.md T4: de index groepeert per
    // record, niet per regel meer), zodat dit bestand echt uit veel records bestaat
    // en de test hieronder de offset-index raakt in plaats van één groot record.
    let line = format!(
        "[2026-01-15T10:30:45.000000+01:00] app.INFO: {}",
        "x".repeat(85)
    );
    let line_with_newline = format!("{line}\n");
    let mut written: u64 = 0;
    while written < ONE_GIB {
        writer
            .write_all(line_with_newline.as_bytes())
            .expect("write regel");
        written += line_with_newline.len() as u64;
    }
    writer.write_all(LAST_RECORD).expect("write marker");
    writer.flush().expect("flush");
}
