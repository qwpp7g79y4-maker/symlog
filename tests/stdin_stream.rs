//! W-05: `symlog -` leest stdin als stroom. De test genereert zelf een stroom van
//! 1 GiB (nooit als bestand op schijf, nooit als één blok in het geheugen) en
//! controleert uitvoer én piekgeheugen van het proces.

use assert_cmd::Command;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::process::{Command as StdCommand, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const ONE_GIB: u64 = 1024 * 1024 * 1024;
/// Elke 1000e record is CRITICAL, de rest INFO.
const CRITICAL_EVERY: u64 = 1000;
/// Piekgeheugen (VmHWM) waaronder het proces moet blijven; ver onder 1 GiB.
const MAX_PEAK_KIB: u64 = 128 * 1024;
const TIME_LIMIT: Duration = Duration::from_secs(600);

fn write_record(buf: &mut Vec<u8>, i: u64) {
    let level = if i.is_multiple_of(CRITICAL_EVERY) {
        "CRITICAL"
    } else {
        "INFO"
    };
    buf.clear();
    buf.extend_from_slice(
        format!("[2026-01-15T10:00:00.000000+01:00] app.{level}: record {i}\n").as_bytes(),
    );
    // Drie vervolgregels van ~1 KiB maken van elk record ~3 KiB.
    for k in 0..3 {
        buf.extend_from_slice(format!("    continuation {k} of {i} ").as_bytes());
        buf.resize(buf.len() + 1000, b'.');
        // Context en extra staan op de laatste regel van het record.
        buf.extend_from_slice(if k == 2 { b" [] []\n" } else { b"\n" });
    }
}

/// Schrijft records tot minstens `target` bytes, in kleine stukken. Geeft het aantal
/// records terug.
fn generate(mut sink: impl Write, target: u64) -> u64 {
    let mut buf = Vec::with_capacity(4096);
    let (mut written, mut i) = (0u64, 0u64);
    while written < target {
        write_record(&mut buf, i);
        if sink.write_all(&buf).is_err() {
            break;
        }
        written += buf.len() as u64;
        i += 1;
    }
    let _ = sink.flush();
    i
}

fn peak_kib(pid: u32) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

#[test]
fn small_stdin_stream_is_filtered_like_a_file() {
    let mut input = Vec::new();
    generate(&mut input, 20_000);
    Command::cargo_bin("symlog")
        .unwrap()
        .args(["-", "--level", "critical"])
        .write_stdin(input)
        .assert()
        .success()
        .stdout(predicates::str::contains("app.CRITICAL: record 0\n"));
}

#[test]
fn follow_with_stdin_is_a_usage_error_and_tui_needs_a_tty() {
    Command::cargo_bin("symlog")
        .unwrap()
        .args(["-", "--follow"])
        .write_stdin("")
        .assert()
        .code(2);
    // Zonder tty op stdout wint de bestaande `--tui`-controle (exit 3).
    Command::cargo_bin("symlog")
        .unwrap()
        .args(["-", "--tui"])
        .write_stdin("")
        .assert()
        .code(3);
}

#[test]
fn one_gib_stream_on_stdin_stays_within_bounded_memory() {
    let mut child = StdCommand::new(env!("CARGO_BIN_EXE_symlog"))
        .args(["-", "--level", "critical"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn symlog");
    let pid = child.id();
    let stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();

    let start = Instant::now();
    let producer =
        thread::spawn(move || generate(BufWriter::with_capacity(64 * 1024, stdin), ONE_GIB));
    let err_reader = thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    let out_reader = thread::spawn(move || {
        let mut lines = Vec::new();
        let mut reader = BufReader::new(&mut stdout);
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap_or(0) > 0 {
            lines.push(std::mem::take(&mut line));
        }
        lines
    });

    // Piekgeheugen bijhouden terwijl het proces draait.
    let mut peak = 0u64;
    let status = loop {
        if let Some(p) = peak_kib(pid) {
            peak = peak.max(p);
        }
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if start.elapsed() > TIME_LIMIT {
            let _ = child.kill();
            panic!("1 GiB-stroom duurde langer dan {TIME_LIMIT:?}");
        }
        thread::sleep(Duration::from_millis(100));
    };

    let records = producer.join().unwrap();
    let lines = out_reader.join().unwrap();
    let stderr = err_reader.join().unwrap();
    assert!(status.success(), "exit {status:?}, stderr: {stderr}");

    let expected = records.div_ceil(CRITICAL_EVERY);
    assert_eq!(lines.len() as u64, expected, "aantal CRITICAL-regels");
    assert!(
        lines[0].contains("app.CRITICAL: record 0\n"),
        "{}",
        lines[0]
    );
    let last = (records - 1) / CRITICAL_EVERY * CRITICAL_EVERY;
    assert!(
        lines.last().unwrap().ends_with(&format!("record {last}\n")),
        "laatste regel: {}",
        lines.last().unwrap()
    );
    if cfg!(target_os = "linux") {
        assert!(peak > 0, "geen VmHWM gemeten");
        assert!(
            peak <= MAX_PEAK_KIB,
            "piekgeheugen {peak} KiB > {MAX_PEAK_KIB} KiB"
        );
    }
}
