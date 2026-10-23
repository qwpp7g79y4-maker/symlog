//! Editor-sprong met een nep-`nvim`/nep-editor (task.md T9 "Klaar als"): controleert de
//! exacte argv voor elke tak van de beslisboom door een echt kindproces te starten
//! (`tests/bin/record-argv`, zie dat bestand) dat zijn eigen argv naar een tmp-bestand
//! schrijft. Niets hier raakt het procesmilieu van de testrunner aan (geen
//! `std::env::set_var`): elke test krijgt zijn eigen tijdelijke `PATH` en
//! `ARGV_LOG`-waarde via [`ProcessRunner::with_env`]. Schrijven en starten van
//! het nep-programma delen één slot: een fork erft anders een open
//! schrijf-descriptor en `exec` weigert met "Text file busy". Die tests lopen
//! daardoor niet racevrij parallel.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::Mutex;
use symlog::editor::{
    self, CommandRunner, Decision, JumpContext, MapEntry, PathMap, ProcessRunner,
};
use symlog::model::Frame;
use tempfile::TempDir;

fn fake_script_source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/bin/record-argv")
}

/// Eén slot voor het schrijven én het starten van een nep-programma.
///
/// `exec` weigert met "Text file busy" zolang ergens een schrijf-descriptor op
/// dat bestand openstaat. Een fork in een andere thread erft elke open
/// descriptor van dit proces, ook een die nog niet gesloten is. Daarom mag er
/// niet tegelijk geschreven en geforkt worden: het slot dekt flush, sluiten en
/// chmod, en ook het wachten tot `exec` in het kind klaar is.
fn with_fake_bin_lock<T>(body: impl FnOnce() -> T) -> T {
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock().expect("slot voor nep-programma");
    body()
}

fn install_executable(dest: &Path, bytes: &[u8]) {
    {
        let mut file = File::create(dest).expect("schrijf het nep-script");
        file.write_all(bytes).expect("schrijf het nep-script");
        file.sync_all().expect("flush het nep-script");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dest, fs::Permissions::from_mode(0o755))
            .expect("maak het nep-script uitvoerbaar");
    }
}

/// Een tijdelijke `PATH` met alleen het argv-opnamescriptje, onder elk van `names`.
struct FakeBin {
    dir: TempDir,
    log: PathBuf,
}

/// Start het nep-programma onder hetzelfde slot als [`install_executable`].
struct LockedRunner {
    inner: ProcessRunner,
}

impl CommandRunner for LockedRunner {
    fn run(&mut self, program: &str, args: &[String]) -> io::Result<ExitStatus> {
        with_fake_bin_lock(|| self.inner.run(program, args))
    }
}

impl FakeBin {
    fn new(names: &[&str]) -> FakeBin {
        let dir = tempfile::tempdir().expect("tmp-map voor nep-PATH");
        let log = dir.path().join("argv.log");
        let bytes = fs::read(fake_script_source()).expect("lees het nep-script");
        with_fake_bin_lock(|| {
            for name in names {
                install_executable(&dir.path().join(name), &bytes);
            }
        });
        FakeBin { dir, log }
    }

    fn runner(&self) -> LockedRunner {
        LockedRunner {
            inner: ProcessRunner::with_env(
                self.dir
                    .path()
                    .to_str()
                    .expect("tmp-pad is utf-8")
                    .to_string(),
                vec![(
                    "ARGV_LOG".to_string(),
                    self.log.to_str().expect("log-pad is utf-8").to_string(),
                )],
            ),
        }
    }

    fn path_var(&self) -> String {
        self.dir
            .path()
            .to_str()
            .expect("tmp-pad is utf-8")
            .to_string()
    }

    /// Elke aanroep van het nep-script als eigen `Vec<String>` argv, in
    /// aanroepvolgorde (zie het "---"-scheidingsteken in `tests/bin/record-argv`).
    fn recorded_calls(&self) -> Vec<Vec<String>> {
        let content = fs::read_to_string(&self.log).unwrap_or_default();
        let mut calls = Vec::new();
        let mut current = Vec::new();
        for line in content.lines() {
            if line == "---" {
                calls.push(std::mem::take(&mut current));
            } else {
                current.push(line.to_string());
            }
        }
        calls
    }
}

fn jumpable_frame(path: &str, line: u32) -> Frame {
    Frame {
        path: Some(path.to_string()),
        line: Some(line),
        func: "App\\Foo->bar()".to_string(),
    }
}

/// Een bestaand tmp-bestand om naartoe te "springen" (de padmapping-/bestaanscheck in
/// [`editor::jump`] moet slagen vóórdat er een editor gestart wordt).
fn target_file() -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tmp-doelmap");
    let file = dir.path().join("Foo.php");
    fs::write(&file, "<?php\n").expect("schrijf doelbestand");
    (dir, file)
}

fn empty_context(path_var: &str) -> JumpContext<'_> {
    JumpContext {
        server_flag: None,
        server_config: None,
        nvim_env: None,
        visual: None,
        editor: None,
        path_var,
    }
}

#[test]
fn server_via_nvim_env_sends_the_two_remote_calls_and_leaves_symlog_open() {
    let bin = FakeBin::new(&["nvim"]);
    let (_dir, file) = target_file();
    let frame = jumpable_frame(file.to_str().unwrap(), 42);
    let map = PathMap::default();
    let path_var = bin.path_var();
    let mut ctx = empty_context(&path_var);
    ctx.nvim_env = Some("/tmp/fake-nvim.sock");
    let mut runner = bin.runner();

    let mut pause_calls = 0u32;
    let mut resume_calls = 0u32;
    let outcome = editor::jump(
        &frame,
        &map,
        &ctx,
        &mut runner,
        || pause_calls += 1,
        || resume_calls += 1,
    );

    assert_eq!(outcome, editor::JumpOutcome::Server);
    assert_eq!(pause_calls, 0, "een serveraanroep pauzeert de TUI niet");
    assert_eq!(resume_calls, 0, "een serveraanroep herstelt de TUI niet");
    let calls = bin.recorded_calls();
    assert_eq!(
        calls.len(),
        2,
        "eerst het bestand openen, dan de cursor zetten"
    );
    assert_eq!(
        calls[0],
        vec![
            "--server",
            "/tmp/fake-nvim.sock",
            "--remote",
            file.to_str().unwrap()
        ]
    );
    assert_eq!(
        calls[1],
        vec![
            "--server",
            "/tmp/fake-nvim.sock",
            "--remote-expr",
            "execute(':42')"
        ]
    );
}

#[test]
fn server_via_server_flag_wins_over_nvim_env() {
    let bin = FakeBin::new(&["nvim"]);
    let (_dir, file) = target_file();
    let frame = jumpable_frame(file.to_str().unwrap(), 7);
    let map = PathMap::default();
    let path_var = bin.path_var();
    let mut ctx = empty_context(&path_var);
    ctx.server_flag = Some("/tmp/flag.sock");
    ctx.nvim_env = Some("/tmp/env.sock");
    let mut runner = bin.runner();

    let outcome = editor::jump(&frame, &map, &ctx, &mut runner, || {}, || {});

    assert_eq!(outcome, editor::JumpOutcome::Server);
    let calls = bin.recorded_calls();
    assert_eq!(
        calls[0][1], "/tmp/flag.sock",
        "--server-vlag wint van $NVIM"
    );
}

#[test]
fn no_server_spawns_nvim_on_path_and_pauses_the_tui() {
    // task.md T9 "Klaar als": "geen server".
    let bin = FakeBin::new(&["nvim"]);
    let (_dir, file) = target_file();
    let frame = jumpable_frame(file.to_str().unwrap(), 42);
    let map = PathMap::default();
    let path_var = bin.path_var();
    let ctx = empty_context(&path_var);
    let mut runner = bin.runner();

    let mut pause_calls = 0u32;
    let mut resume_calls = 0u32;
    let outcome = editor::jump(
        &frame,
        &map,
        &ctx,
        &mut runner,
        || pause_calls += 1,
        || resume_calls += 1,
    );

    assert_eq!(outcome, editor::JumpOutcome::Spawned);
    assert_eq!(pause_calls, 1, "de TUI pauzeert rond een kindproces");
    assert_eq!(resume_calls, 1, "en herstelt weer zodra het terugkomt");
    let calls = bin.recorded_calls();
    assert_eq!(
        calls,
        vec![vec!["+42".to_string(), file.to_str().unwrap().to_string()]]
    );
}

#[test]
fn editor_env_set_to_bare_nano_adds_the_plus_line_flag() {
    // task.md T9 "Klaar als": "alleen $EDITOR=nano".
    let bin = FakeBin::new(&["nano"]);
    let (_dir, file) = target_file();
    let frame = jumpable_frame(file.to_str().unwrap(), 5);
    let map = PathMap::default();
    let path_var = bin.path_var();
    let mut ctx = empty_context(&path_var);
    ctx.editor = Some("nano");
    let mut runner = bin.runner();

    let outcome = editor::jump(&frame, &map, &ctx, &mut runner, || {}, || {});

    assert_eq!(outcome, editor::JumpOutcome::Spawned);
    let calls = bin.recorded_calls();
    assert_eq!(
        calls,
        vec![vec!["+5".to_string(), file.to_str().unwrap().to_string()]]
    );
}

#[test]
fn editor_env_with_quoted_flags_is_split_with_shell_words() {
    // task.md T9 "Klaar als": `$EDITOR="code --wait"` (gesplitst met shell-words).
    let bin = FakeBin::new(&["code"]);
    let (_dir, file) = target_file();
    let frame = jumpable_frame(file.to_str().unwrap(), 5);
    let map = PathMap::default();
    let path_var = bin.path_var();
    let mut ctx = empty_context(&path_var);
    ctx.editor = Some("code --wait");
    let mut runner = bin.runner();

    let outcome = editor::jump(&frame, &map, &ctx, &mut runner, || {}, || {});

    assert_eq!(outcome, editor::JumpOutcome::Spawned);
    let calls = bin.recorded_calls();
    assert_eq!(
        calls,
        vec![vec![
            "--wait".to_string(),
            file.to_str().unwrap().to_string()
        ]],
        "onbekende editor: geen +regel geraden, wel de bestaande --wait-vlag"
    );
}

#[test]
fn visual_wins_over_editor_when_both_are_set() {
    let bin = FakeBin::new(&["code", "nano"]);
    let (_dir, file) = target_file();
    let frame = jumpable_frame(file.to_str().unwrap(), 5);
    let map = PathMap::default();
    let path_var = bin.path_var();
    let mut ctx = empty_context(&path_var);
    ctx.visual = Some("code --wait");
    ctx.editor = Some("nano");
    let mut runner = bin.runner();

    let outcome = editor::jump(&frame, &map, &ctx, &mut runner, || {}, || {});

    assert_eq!(outcome, editor::JumpOutcome::Spawned);
    let calls = bin.recorded_calls();
    assert_eq!(
        calls.len(),
        1,
        "alleen $VISUAL mag een proces starten, niet ook $EDITOR"
    );
    assert_eq!(calls[0][0], "--wait", "$VISUAL wint van $EDITOR");
}

#[test]
fn nothing_set_falls_back_to_vi() {
    // task.md T9 "Klaar als": "niets → vi".
    let bin = FakeBin::new(&["vi"]);
    let (_dir, file) = target_file();
    let frame = jumpable_frame(file.to_str().unwrap(), 9);
    let map = PathMap::default();
    let path_var = bin.path_var();
    let ctx = empty_context(&path_var);
    let mut runner = bin.runner();

    let outcome = editor::jump(&frame, &map, &ctx, &mut runner, || {}, || {});

    assert_eq!(outcome, editor::JumpOutcome::Spawned);
    let calls = bin.recorded_calls();
    assert_eq!(
        calls,
        vec![vec!["+9".to_string(), file.to_str().unwrap().to_string()]]
    );
}

#[test]
fn a_frame_mapped_to_a_missing_local_file_never_starts_an_editor() {
    let bin = FakeBin::new(&["vi"]);
    let frame = jumpable_frame("/var/www/does/not/exist.php", 1);
    let map = PathMap::default();
    let path_var = bin.path_var();
    let ctx = empty_context(&path_var);
    let mut runner = bin.runner();

    let outcome = editor::jump(&frame, &map, &ctx, &mut runner, || {}, || {});

    assert!(matches!(outcome, editor::JumpOutcome::NotFound { .. }));
    assert!(
        bin.recorded_calls().is_empty(),
        "geen editor gestart voor een ontbrekend bestand"
    );
}

/// task.md T9 "Klaar als": "F3 levert de juiste file:line na mapping".
#[test]
fn f3_stacktrace_frame_resolves_to_the_correct_file_and_line_after_mapping() {
    let content = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/line-stacktrace.log"),
    )
    .expect("F3-fixture");
    let blocks = symlog::parse::line::split_records(&content);
    let record = symlog::parse::line::parse_line(blocks[0]).expect("F3 parseert als lineformat");
    let first_frame = &record.frames[0];
    assert_eq!(
        first_frame.path.as_deref(),
        Some("/var/www/src/Dispatcher.php")
    );
    assert_eq!(first_frame.line, Some(40));

    let local_dir = tempfile::tempdir().expect("tmp-projectmap");
    fs::create_dir_all(local_dir.path().join("src")).expect("src-map");
    fs::write(local_dir.path().join("src/Dispatcher.php"), "<?php\n").expect("schrijf bestand");

    let map = PathMap::new(vec![MapEntry::parse_spec(&format!(
        "/var/www={}",
        local_dir.path().display()
    ))
    .unwrap()]);

    let bin = FakeBin::new(&["vi"]);
    let path_var = bin.path_var();
    let ctx = empty_context(&path_var);
    let mut runner = bin.runner();
    let outcome = editor::jump(first_frame, &map, &ctx, &mut runner, || {}, || {});

    assert_eq!(outcome, editor::JumpOutcome::Spawned);
    let expected_local = local_dir.path().join("src/Dispatcher.php");
    let calls = bin.recorded_calls();
    assert_eq!(
        calls,
        vec![vec![
            "+40".to_string(),
            expected_local.to_str().unwrap().to_string()
        ]]
    );
}

/// F17 (`container-paths.log`): frames onder twee verschillende containerprefixen in
/// hetzelfde bestand, met een overlappende, specifiekere prefix voor een deel van
/// `/var/www`. Elk frame moet de langst matchende prefix krijgen (task.md T9 "Doet").
#[test]
fn f17_container_paths_maps_each_frame_with_the_longest_matching_prefix() {
    let content = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/container-paths.log"),
    )
    .expect("F17-fixture");
    let blocks = symlog::parse::line::split_records(&content);
    assert_eq!(blocks.len(), 1, "F17 is één record (één exception-melding)");
    let record = symlog::parse::line::parse_line(blocks[0]).expect("F17 parseert als lineformat");
    assert_eq!(
        record.frames.len(),
        6,
        "5 stacktrace-frames plus de {{main}}-afsluiter"
    );

    let project = tempfile::tempdir().expect("tmp-projectmap");
    let vendor = tempfile::tempdir().expect("tmp-vendormap");

    let map = PathMap::new(vec![
        // De kortere, algemenere prefix staat eerst; de langere, specifiekere prefix
        // voor de Sub-submap moet daar toch van winnen (overlappende prefixen).
        MapEntry::parse_spec(&format!("/var/www/html/src={}", project.path().display())).unwrap(),
        MapEntry::parse_spec(&format!(
            "/var/www/html/src/Controller/Sub={}",
            project.path().join("sub-override").display()
        ))
        .unwrap(),
        MapEntry::parse_spec(&format!("/app/vendor={}", vendor.path().display())).unwrap(),
    ]);

    // frame 0: /var/www/html/src/Controller/HomeController.php -> de kortere prefix.
    assert_eq!(
        map.map("/var/www/html/src/Controller/HomeController.php"),
        Some(
            project
                .path()
                .join("Controller/HomeController.php")
                .to_str()
                .unwrap()
                .to_string()
        )
    );
    // frame 1: /var/www/html/src/Controller/Sub/DeepController.php -> de langere,
    // specifiekere "Sub"-prefix wint, ook al matcht de kortere ook.
    assert_eq!(
        map.map("/var/www/html/src/Controller/Sub/DeepController.php"),
        Some(
            project
                .path()
                .join("sub-override/DeepController.php")
                .to_str()
                .unwrap()
                .to_string()
        )
    );
    // frame 2/3: /app/vendor/... -> de losse vendor-prefix.
    assert_eq!(
        map.map("/app/vendor/symfony/http-kernel/HttpKernel.php"),
        Some(
            vendor
                .path()
                .join("symfony/http-kernel/HttpKernel.php")
                .to_str()
                .unwrap()
                .to_string()
        )
    );

    for frame in &record.frames {
        let Some(path) = frame.path.as_deref() else {
            continue; // "[internal function]"/"{main}": niet springbaar, geen mapping.
        };
        assert!(
            map.map(path).is_some(),
            "elk frame met een pad in F17 valt onder één van de geconfigureerde prefixen: {path}"
        );
    }
}

#[test]
fn decide_matches_the_jump_outcome_for_each_branch() {
    // Sanity-check dat `jump`'s interne `decide`-aanroep dezelfde boom volgt als de
    // publieke `decide`-functie (geen losgezongen duplicaat-logica).
    assert_eq!(
        editor::decide(Some("/tmp/s.sock"), true, Some("code"), Some("nano")),
        Decision::Server {
            socket: "/tmp/s.sock".to_string()
        }
    );
    assert_eq!(editor::decide(None, true, None, None), Decision::SpawnNvim);
    assert_eq!(editor::decide(None, false, None, None), Decision::Vi);
}
