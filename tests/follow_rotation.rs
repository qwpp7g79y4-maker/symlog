//! Integratietests voor `follow` (task.md T10 "Klaar als", fixture F13): een echte
//! schrijver-thread in een tmp-map, gevolgd via [`Watcher`] + [`Follower`] zoals een
//! latere CLI-laag dat zou doen, voor groei, rename-rotatie, copytruncate, afkappen
//! midden in een record en verdwijnen-en-teruggkomen. Elk scenario draait twee keer:
//! met alleen de polling-terugval (`--poll`) en met `notify` erbij (task.md T10
//! "Klaar als": "Tests draaien in CI op Linux en macOS met `--poll` en met notify"),
//! en controleert dat er geen records verloren gaan of verdubbelen.
//!
//! De losse detectie- en toestandslogica staat al grondig unit-getest in
//! `src/follow/mod.rs`; dit bestand bewijst dat die logica ook correct samenwerkt met
//! een [`Watcher`] die écht op een achtergrondthread/`notify` draait, terwijl een
//! onafhankelijke schrijver-thread tegelijk het bestand bewerkt.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use symlog::follow::{FollowEvent, Follower, SeparatorKind, TailState, Watcher};

fn make_watcher(path: &Path, use_notify: bool) -> Watcher {
    if use_notify {
        Watcher::new(path, Duration::from_millis(50))
    } else {
        Watcher::poll_only(Duration::from_millis(20))
    }
}

fn write_file(path: &Path, content: &[u8]) {
    File::create(path)
        .expect("create")
        .write_all(content)
        .expect("write");
}

fn append(path: &Path, content: &[u8]) {
    OpenOptions::new()
        .append(true)
        .open(path)
        .expect("open voor append")
        .write_all(content)
        .expect("append");
}

/// Roept `tick()` net zo vaak aan tot `done` waar is, met tussendoor een wake (of
/// timeout na 50 ms) van `watcher`. Faalt met een duidelijke paniek (geen hang) als
/// `done` niet binnen 5 s waar wordt: in CI betekent dat een echte regressie, geen
/// race die nog een kans verdient.
fn pump_until(follower: &mut Follower, watcher: &Watcher, mut done: impl FnMut(&Follower) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        follower.tick().expect("tick");
        if done(follower) {
            return;
        }
        if Instant::now() >= deadline {
            panic!("voorwaarde niet bereikt binnen 5s (mogelijke follow-regressie)");
        }
        watcher.wait_timeout(Duration::from_millis(50));
    }
}

fn all_records(follower: &Follower) -> Vec<Vec<u8>> {
    (0..follower.total_record_count())
        .map(|i| {
            follower
                .record_bytes(i)
                .expect("lezen")
                .expect("record binnen total_record_count() moet bestaan")
                .to_vec()
        })
        .collect()
}

fn growth_scenario(use_notify: bool) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("app.log");
    write_file(&path, b"[2026-01-01 00:00:00] a.INFO: eerste\n");

    let mut follower = Follower::open(&path);
    let watcher = make_watcher(&path, use_notify);
    pump_until(&mut follower, &watcher, |f| f.total_record_count() == 1);

    let writer_path = path.clone();
    let writer = thread::spawn(move || {
        for i in 1..=20u32 {
            thread::sleep(Duration::from_millis(10));
            append(
                &writer_path,
                format!("[2026-01-01 00:00:{i:02}] a.INFO: regel {i}\n").as_bytes(),
            );
        }
    });
    writer.join().expect("schrijver-thread");

    pump_until(&mut follower, &watcher, |f| f.total_record_count() == 21);

    let records = all_records(&follower);
    assert_eq!(
        records.len(),
        21,
        "geen verloren of dubbele records bij groei"
    );
    let unique: std::collections::HashSet<_> = records.iter().collect();
    assert_eq!(
        unique.len(),
        21,
        "elke gegroeide regel moet precies één keer voorkomen"
    );
    assert_eq!(records[0], b"[2026-01-01 00:00:00] a.INFO: eerste\n");
    assert_eq!(records[20], b"[2026-01-01 00:00:20] a.INFO: regel 20\n");
    assert!(
        follower.separators().is_empty(),
        "groei zet geen scheidingsregel"
    );
}

#[test]
fn growth_with_poll() {
    growth_scenario(false);
}

#[test]
fn growth_with_notify() {
    growth_scenario(true);
}

fn rename_rotation_scenario(use_notify: bool) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("app.log");
    write_file(
        &path,
        b"[2026-01-01 00:00:00] a.INFO: oud een\n[2026-01-01 00:00:01] a.INFO: oud twee\n",
    );

    let mut follower = Follower::open(&path);
    let watcher = make_watcher(&path, use_notify);
    pump_until(&mut follower, &watcher, |f| f.total_record_count() == 2);

    let dir_path = dir.path().to_path_buf();
    let writer = thread::spawn(move || {
        let target = dir_path.join("app.log");
        thread::sleep(Duration::from_millis(30));
        fs::rename(&target, dir_path.join("app.log.1")).expect("rename (logrotate)");
        write_file(&target, b"[2026-01-01 00:00:02] a.INFO: nieuw een\n");
        thread::sleep(Duration::from_millis(30));
        append(&target, b"[2026-01-01 00:00:03] a.INFO: nieuw twee\n");
    });
    writer.join().expect("schrijver-thread");

    pump_until(&mut follower, &watcher, |f| f.total_record_count() == 4);

    let records = all_records(&follower);
    assert_eq!(
        records,
        vec![
            b"[2026-01-01 00:00:00] a.INFO: oud een\n".to_vec(),
            b"[2026-01-01 00:00:01] a.INFO: oud twee\n".to_vec(),
            b"[2026-01-01 00:00:02] a.INFO: nieuw een\n".to_vec(),
            b"[2026-01-01 00:00:03] a.INFO: nieuw twee\n".to_vec(),
        ],
        "geen verloren of dubbele records over de rotatiegrens heen"
    );
    assert_eq!(follower.separators().len(), 1);
    assert_eq!(follower.separators()[0].before_record, 2);
    assert_eq!(follower.separators()[0].kind, SeparatorKind::Rotated);
}

#[test]
fn rename_rotation_with_poll() {
    rename_rotation_scenario(false);
}

#[test]
fn rename_rotation_with_notify() {
    rename_rotation_scenario(true);
}

fn copytruncate_scenario(use_notify: bool) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("app.log");
    write_file(
        &path,
        b"[2026-01-01 00:00:00] a.INFO: oud een\n[2026-01-01 00:00:01] a.INFO: oud twee\n",
    );

    let mut follower = Follower::open(&path);
    let watcher = make_watcher(&path, use_notify);
    pump_until(&mut follower, &watcher, |f| f.total_record_count() == 2);

    // De twee schrijfacties zijn hier bewust gescheiden door een rendez-vous met de
    // hoofdthread: het bestand wordt kort kleiner (de echte afkapping) vóórdat het
    // weer groter wordt dan de oude grootte. Zonder deze synchronisatie zou de
    // schrijver-thread de afkapping al helemaal kunnen inhalen vóórdat ook maar één
    // `tick()` die kans heeft gezien, waardoor alleen de netto groottewijziging
    // overblijft (precies dezelfde inherente blinde vlek die `tail -f` zelf ook heeft
    // bij stat-gebaseerde detectie: zonder een waarneming van de tussenliggende dip
    // is "kort kleiner, toen weer groter" niet te onderscheiden van gewone groei).
    let (truncated_tx, truncated_rx) = std::sync::mpsc::channel::<()>();
    let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
    let writer_path = path.clone();
    let writer = thread::spawn(move || {
        // copytruncate: de inode blijft gelijk, de inhoud wordt in-place vervangen
        // door iets kleiners.
        write_file(
            &writer_path,
            b"[2026-01-01 00:00:02] a.INFO: na copytruncate\n",
        );
        truncated_tx.send(()).expect("signaal afkapping");
        go_rx.recv().expect("ga-signaal van hoofdthread");
        append(&writer_path, b"[2026-01-01 00:00:03] a.INFO: erna\n");
    });

    truncated_rx.recv().expect("signaal afkapping");
    pump_until(&mut follower, &watcher, |f| f.total_record_count() == 1);
    go_tx.send(()).expect("ga-signaal");
    writer.join().expect("schrijver-thread");

    pump_until(&mut follower, &watcher, |f| f.total_record_count() == 2);

    let records = all_records(&follower);
    assert_eq!(
        records,
        vec![
            b"[2026-01-01 00:00:02] a.INFO: na copytruncate\n".to_vec(),
            b"[2026-01-01 00:00:03] a.INFO: erna\n".to_vec(),
        ],
        "de oude records horen na copytruncate niet meer mee te tellen"
    );
    assert_eq!(follower.separators().len(), 1);
    assert_eq!(follower.separators()[0].before_record, 0);
    assert_eq!(follower.separators()[0].kind, SeparatorKind::Truncated);
}

#[test]
fn copytruncate_with_poll() {
    copytruncate_scenario(false);
}

#[test]
fn copytruncate_with_notify() {
    copytruncate_scenario(true);
}

fn truncate_mid_record_scenario(use_notify: bool) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("app.log");
    write_file(&path, b"[2026-01-01 00:00:00] a.INFO: compleet\n");

    let mut follower = Follower::with_pending_tail_timeout(&path, Duration::from_secs(3600));
    let watcher = make_watcher(&path, use_notify);
    pump_until(&mut follower, &watcher, |f| f.total_record_count() == 1);

    // Net als bij copytruncate hierboven: elke stap wordt pas gezet nadat de
    // hoofdthread de vorige stap heeft waargenomen, anders zou de schrijver-thread de
    // afkapping (en het stuk onvolledige regel dat erbij verloren gaat) voorbij
    // kunnen lopen vóórdat een `tick()` die ooit ziet.
    let (wrote_partial_tx, wrote_partial_rx) = std::sync::mpsc::channel::<()>();
    let (go_truncate_tx, go_truncate_rx) = std::sync::mpsc::channel::<()>();
    let (truncated_tx, truncated_rx) = std::sync::mpsc::channel::<()>();
    let (go_append_tx, go_append_rx) = std::sync::mpsc::channel::<()>();
    let writer_path = path.clone();
    let writer = thread::spawn(move || {
        // Een nieuw record begint te schrijven, maar wordt halverwege afgekapt vóór
        // de afsluitende `\n` er ooit komt.
        append(&writer_path, b"[2026-01-01 00:00:01] a.INFO: halverwe");
        wrote_partial_tx
            .send(())
            .expect("signaal onvolledige regel");
        go_truncate_rx.recv().expect("ga-signaal van hoofdthread");

        File::create(&writer_path)
            .expect("afkappen")
            .set_len(0)
            .expect("afkappen tot 0 bytes");
        truncated_tx.send(()).expect("signaal afkapping");
        go_append_rx.recv().expect("ga-signaal van hoofdthread");

        append(&writer_path, b"[2026-01-01 00:00:02] a.INFO: na afkappen\n");
    });

    wrote_partial_rx.recv().expect("signaal onvolledige regel");
    pump_until(&mut follower, &watcher, |f| f.tail() != TailState::None);
    go_truncate_tx.send(()).expect("ga-signaal");

    truncated_rx.recv().expect("signaal afkapping");
    pump_until(&mut follower, &watcher, |f| f.total_record_count() == 0);
    go_append_tx.send(()).expect("ga-signaal");
    writer.join().expect("schrijver-thread");

    pump_until(&mut follower, &watcher, |f| f.total_record_count() == 1);

    let records = all_records(&follower);
    assert_eq!(
        records,
        vec![b"[2026-01-01 00:00:02] a.INFO: na afkappen\n".to_vec()],
        "de halverwege afgekapte regel mag nooit als (half) record verschijnen"
    );
    assert_eq!(follower.separators().len(), 1);
    assert_eq!(follower.separators()[0].kind, SeparatorKind::Truncated);
}

#[test]
fn truncate_mid_record_with_poll() {
    truncate_mid_record_scenario(false);
}

#[test]
fn truncate_mid_record_with_notify() {
    truncate_mid_record_scenario(true);
}

fn delete_and_return_scenario(use_notify: bool) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("app.log");
    write_file(&path, b"[2026-01-01 00:00:00] a.INFO: voor verwijderen\n");

    let mut follower = Follower::open(&path);
    let watcher = make_watcher(&path, use_notify);
    pump_until(&mut follower, &watcher, |f| f.total_record_count() == 1);

    let writer_path = path.clone();
    let writer = thread::spawn(move || {
        thread::sleep(Duration::from_millis(30));
        fs::remove_file(&writer_path).expect("verwijderen");
        thread::sleep(Duration::from_millis(80));
        write_file(
            &writer_path,
            b"[2026-01-01 00:00:05] a.INFO: na terugkomst\n",
        );
    });

    // Terwijl de schrijver het bestand weg laat, moet elke tick "Missing" zijn en mag
    // het bestaande record niet verdwijnen.
    let missing_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if follower.tick().expect("tick") == FollowEvent::Missing {
            break;
        }
        if Instant::now() >= missing_deadline {
            panic!("het bestand had intussen al verwijderd moeten zijn");
        }
        watcher.wait_timeout(Duration::from_millis(20));
    }
    assert_eq!(follower.total_record_count(), 1);

    writer.join().expect("schrijver-thread");
    pump_until(&mut follower, &watcher, |f| f.total_record_count() == 2);

    let records = all_records(&follower);
    assert_eq!(
        records,
        vec![
            b"[2026-01-01 00:00:00] a.INFO: voor verwijderen\n".to_vec(),
            b"[2026-01-01 00:00:05] a.INFO: na terugkomst\n".to_vec(),
        ],
        "het record van vóór het verdwijnen mag niet verloren gaan"
    );
    assert_eq!(follower.separators().len(), 1);
    assert_eq!(follower.separators()[0].kind, SeparatorKind::Rotated);
}

#[test]
fn delete_and_return_with_poll() {
    delete_and_return_scenario(false);
}

#[test]
fn delete_and_return_with_notify() {
    delete_and_return_scenario(true);
}
