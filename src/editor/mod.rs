//! Editor-sprong: Neovim-server, Neovim als kindproces, `$VISUAL`/`$EDITOR` en `vi`
//! (task.md T9; blueprint.md `editor/`).
//!
//! [`decide`] is de pure beslisboom uit task.md T9 "Doet" ("Volgorde: 1. Draaiende
//! nvim-server ... 2. Geen server ... 3. Geen nvim op PATH ..."): een beslissing op
//! *configuratie* (is er een serverreferentie? staat `nvim` op PATH? is `$VISUAL` of
//! `$EDITOR` gezet?), niet op runtime-gedrag. Of de gekozen aanpak ook echt werkt (de
//! server antwoordt, de editor start zonder fout) blijkt pas in [`execute`]/[`jump`],
//! via de exitcode/foutresultaten van de aanroep; mislukt dat, dan is de uitkomst
//! [`JumpOutcome::Failed`] en blijft symlog gewoon draaien (blueprint.md
//! "Foutafhandeling": "Editor faalt ... → statusbalk, symlog blijft draaien"), er is
//! geen automatische terugval naar de volgende stap in de volgorde.
//!
//! [`CommandRunner`] maakt het daadwerkelijk starten van een proces testbaar zonder een
//! echte `nvim`/`$EDITOR` nodig te hebben: de tests in `tests/editor_argv.rs` gebruiken
//! [`ProcessRunner`] met een `PATH` die alleen naar de nep-editorscripts in
//! `tests/bin` wijst (task.md T9 "Klaar als": "scriptje in `tests/bin` dat argv naar
//! een bestand schrijft in tmp"), zodat de precieze argv van een echt gestart proces
//! gecontroleerd kan worden zonder het systeem-PATH van de testrunner aan te raken
//! (geen gedeelde, racy `std::env::set_var` tussen parallelle tests).

pub mod fallback;
pub mod nvim;
pub mod pathmap;

pub use pathmap::{MapEntry, PathMap};

use crate::model;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

/// Resultaat van een editor-sprong (task.md T9 "Doet"/"Blijft").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JumpOutcome {
    /// Via een draaiende nvim-server: symlog blijft gewoon open, geen pauze/herstel
    /// van de TUI nodig (task.md T9 "Doet": "symlog blijft open").
    Server,
    /// Een editor is als kindproces gestart en weer afgesloten (nvim zonder server,
    /// of de `$VISUAL`/`$EDITOR`/`vi`-terugval); de aanroeper pauzeert en herstelt de
    /// TUI eromheen (blueprint.md T9: "TUI pauzeren ... wachten, TUI herstellen").
    Spawned,
    /// Het gemapte (of ongemapte) bestand bestaat lokaal niet: geen editor gestart
    /// (task.md T9 "Doet": "bestaat het gemapte bestand niet, dan een melding in de
    /// statusbalk en geen start van de editor").
    NotFound { message: String },
    /// De editor kon niet (goed) starten; symlog blijft gewoon draaien
    /// (blueprint.md "Foutafhandeling").
    Failed { message: String },
}

/// De vier takken van task.md T9 "Doet"'s beslisboom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Stap 1: een draaiende nvim-server (`--server`, config, of `$NVIM`).
    Server { socket: String },
    /// Stap 2: geen server, maar `nvim` staat op PATH.
    SpawnNvim,
    /// Stap 3: geen server, geen `nvim` op PATH, maar `$VISUAL` of `$EDITOR` is gezet.
    Fallback { spec: String },
    /// Stap 3, laatste terugval: niets van het bovenstaande.
    Vi,
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|s| !s.is_empty())
}

/// De pure beslisboom (task.md T9 "Doet"). `server` is al samengevoegd uit
/// `--server`/config/`$NVIM` (zie [`nvim::resolve_server`]); `nvim_on_path` is het
/// resultaat van een eerdere, los uitgevoerde PATH-zoekactie ([`find_on_path`]), zodat
/// deze functie zelf geen bestandssysteem aanraakt en dus zonder PATH/tmp opgezet te
/// hoeven worden getest kan worden.
pub fn decide(
    server: Option<&str>,
    nvim_on_path: bool,
    visual: Option<&str>,
    editor: Option<&str>,
) -> Decision {
    if let Some(socket) = non_empty(server) {
        return Decision::Server {
            socket: socket.to_string(),
        };
    }
    if nvim_on_path {
        return Decision::SpawnNvim;
    }
    if let Some(spec) = non_empty(visual) {
        return Decision::Fallback {
            spec: spec.to_string(),
        };
    }
    if let Some(spec) = non_empty(editor) {
        return Decision::Fallback {
            spec: spec.to_string(),
        };
    }
    Decision::Vi
}

/// Zoekt `name` als uitvoerbaar bestand in elke map van `path_var` (`:`-gescheiden,
/// net als de echte `$PATH`), eerste match wint. Een eigen, kleine implementatie in
/// plaats van op het echte procesmilieu te steunen: zo kan [`decide`]'s
/// PATH-afhankelijke input (en [`ProcessRunner`]'s eigen programmaresolutie)
/// deterministisch getest worden met een `path_var` die alleen naar een tmp-map met
/// nepscripts wijst, zonder het gedeelde procesmilieu te wijzigen.
pub fn find_on_path(name: &str, path_var: &str) -> Option<PathBuf> {
    path_var
        .split(':')
        .filter(|dir| !dir.is_empty())
        .map(|dir| Path::new(dir).join(name))
        .find(|candidate| is_executable_file(candidate))
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable_file(_path: &Path) -> bool {
    // task.md open punt 8: Windows wordt niet ondersteund in v1 (alleen Linux/macOS in
    // CI); "niet gevonden" is hier het veilige antwoord in plaats van een platform
    // waarvoor deze check geen betekenis heeft.
    false
}

/// Start, of beëindigt, één programma (task.md T9 "Doet": "Argumenten worden als
/// losse argv doorgegeven, nooit via een shell"). Een trait in plaats van direct
/// `std::process::Command` zodat [`execute`]/[`jump`] zonder een echte `nvim`/editor
/// getest kunnen worden.
pub trait CommandRunner {
    fn run(&mut self, program: &str, args: &[String]) -> io::Result<ExitStatus>;
}

/// De echte `CommandRunner`: start `program` als kindproces, nooit via een shell.
/// `path_var` is de `$PATH`-waarde waarmee een programmanaam zonder `/` wordt
/// opgezocht ([`find_on_path`]) vóórdat het proces gestart wordt — dus niet het
/// gedeelde procesmilieu van de testrunner, wat `tests/editor_argv.rs` toelaat om
/// parallel te draaien zonder race op een proces-globale `$PATH`.
pub struct ProcessRunner {
    path_var: String,
    extra_env: Vec<(String, String)>,
}

impl ProcessRunner {
    /// Bouwt een `ProcessRunner` die het echte procesmilieu (`$PATH`) gebruikt; dit is
    /// het pad dat de (nog niet aangesloten) CLI-laag in productie zou gebruiken.
    pub fn from_system_env() -> ProcessRunner {
        ProcessRunner {
            path_var: std::env::var("PATH").unwrap_or_default(),
            extra_env: Vec::new(),
        }
    }

    /// Bouwt een `ProcessRunner` met een expliciete `$PATH`-waarde en extra
    /// omgevingsvariabelen voor het kindproces (`tests/editor_argv.rs`: een tmp-map
    /// met nepscripts als enige `$PATH`, plus een `ARGV_LOG`-variabele die het
    /// nepscript vertelt waar de ontvangen argv naar te schrijven).
    pub fn with_env(
        path_var: impl Into<String>,
        extra_env: Vec<(String, String)>,
    ) -> ProcessRunner {
        ProcessRunner {
            path_var: path_var.into(),
            extra_env,
        }
    }

    fn resolve_program(&self, program: &str) -> PathBuf {
        if program.contains('/') {
            return PathBuf::from(program);
        }
        find_on_path(program, &self.path_var).unwrap_or_else(|| PathBuf::from(program))
    }
}

impl CommandRunner for ProcessRunner {
    fn run(&mut self, program: &str, args: &[String]) -> io::Result<ExitStatus> {
        let resolved = self.resolve_program(program);
        let mut cmd = Command::new(resolved);
        cmd.args(args);
        // Het kindproces zoekt zijn eigen PATH-afhankelijke dingen (zou hier niet
        // relevant zijn, nvim/editors doen dat niet) met dezelfde PATH als deze
        // runner, niet het geërfde procesmilieu: consistent met `resolve_program`
        // hierboven, en test-deterministisch (geen afhankelijkheid van de omgeving
        // waarin `cargo test` zelf draait).
        cmd.env("PATH", &self.path_var);
        for (key, value) in &self.extra_env {
            cmd.env(key, value);
        }
        cmd.status()
    }
}

/// Start `program` als kindproces en wacht erop af, met de TUI gepauzeerd/herstel
/// eromheen (blueprint.md T9: "TUI pauzeren ... spawn ... wachten, TUI herstellen").
/// Gedeeld door [`Decision::SpawnNvim`], [`Decision::Fallback`] en [`Decision::Vi`]:
/// geen van die drie heeft een server, dus alle drie onderbreken de TUI op dezelfde
/// manier.
fn run_as_child<R: CommandRunner>(
    program: &str,
    args: &[String],
    runner: &mut R,
    pause: &mut dyn FnMut(),
    resume: &mut dyn FnMut(),
) -> JumpOutcome {
    pause();
    let result = runner.run(program, args);
    resume();
    match result {
        Ok(status) if status.success() => JumpOutcome::Spawned,
        Ok(status) => JumpOutcome::Failed {
            message: format!("{program} gaf exitcode {:?}", status.code()),
        },
        Err(err) => JumpOutcome::Failed {
            message: format!("{program} starten mislukt: {err}"),
        },
    }
}

/// Voert `decision` uit (task.md T9 "Doet"): de daadwerkelijke proceskant van
/// [`decide`]'s beslissing. `file`/`line` zijn al door de padmapping heen (zie
/// [`jump`]).
pub fn execute<R: CommandRunner>(
    decision: &Decision,
    file: &str,
    line: u32,
    runner: &mut R,
    mut pause: impl FnMut(),
    mut resume: impl FnMut(),
) -> JumpOutcome {
    match decision {
        Decision::Server { socket } => {
            let open = nvim::remote_open_argv(socket, file);
            let goto = nvim::remote_goto_line_argv(socket, line);
            let opened = runner.run("nvim", &open);
            let moved = runner.run("nvim", &goto);
            match (opened, moved) {
                (Ok(s1), Ok(s2)) if s1.success() && s2.success() => JumpOutcome::Server,
                _ => JumpOutcome::Failed {
                    message: format!("nvim-server {socket} reageert niet"),
                },
            }
        }
        Decision::SpawnNvim => {
            let args = vec![format!("+{line}"), file.to_string()];
            run_as_child("nvim", &args, runner, &mut pause, &mut resume)
        }
        Decision::Fallback { spec } => match fallback::build_command(spec, file, line) {
            Ok(cmd) => run_as_child(&cmd.program, &cmd.args, runner, &mut pause, &mut resume),
            Err(err) => JumpOutcome::Failed {
                message: format!("ongeldige editor-opdracht {spec:?}: {err}"),
            },
        },
        Decision::Vi => {
            let cmd = fallback::vi_command(file, line);
            run_as_child(&cmd.program, &cmd.args, runner, &mut pause, &mut resume)
        }
    }
}

/// Bronnen voor een editor-sprong, nog niet aangesloten op een echte CLI/config-laag
/// (task.md T9 "Doet"): de drie serverbronnen, `$VISUAL`/`$EDITOR` en de `$PATH`
/// waarmee `nvim` gezocht wordt. Een toekomstige CLI-laag vult dit met
/// `std::env::var`/config/vlaggen; deze struct raakt zelf geen omgeving aan, zodat
/// `tests/editor_argv.rs` elk scenario met expliciete waarden kan opzetten.
pub struct JumpContext<'a> {
    pub server_flag: Option<&'a str>,
    pub server_config: Option<&'a str>,
    pub nvim_env: Option<&'a str>,
    pub visual: Option<&'a str>,
    pub editor: Option<&'a str>,
    pub path_var: &'a str,
}

/// De volledige editor-sprong (task.md T9; blueprint.md's datastroomdiagram: "'o' op
/// frame → pathmap → EditorPlan → nvim remote / kindproces"): padmapping, bestaan
/// controleren, de beslisboom en de uitvoering, in één aanroep.
pub fn jump<R: CommandRunner>(
    frame: &model::Frame,
    map: &PathMap,
    ctx: &JumpContext<'_>,
    runner: &mut R,
    pause: impl FnMut(),
    resume: impl FnMut(),
) -> JumpOutcome {
    let (Some(path), Some(line)) = (frame.path.as_deref(), frame.line) else {
        return JumpOutcome::NotFound {
            message: "frame niet springbaar (geen bestand:regel)".to_string(),
        };
    };
    let local = map.map(path).unwrap_or_else(|| path.to_string());
    if !Path::new(&local).is_file() {
        return JumpOutcome::NotFound {
            message: format!("bestand niet gevonden: {local}"),
        };
    }

    let server = nvim::resolve_server(ctx.server_flag, ctx.server_config, ctx.nvim_env);
    let nvim_on_path = find_on_path("nvim", ctx.path_var).is_some();
    let decision = decide(server, nvim_on_path, ctx.visual, ctx.editor);
    execute(&decision, &local, line, runner, pause, resume)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decide_prefers_server_over_everything_else() {
        assert_eq!(
            decide(Some("/tmp/nvim.sock"), true, Some("code"), Some("nano")),
            Decision::Server {
                socket: "/tmp/nvim.sock".to_string()
            }
        );
    }

    #[test]
    fn decide_prefers_nvim_on_path_over_visual_and_editor() {
        assert_eq!(
            decide(None, true, Some("code"), Some("nano")),
            Decision::SpawnNvim
        );
    }

    #[test]
    fn decide_prefers_visual_over_editor() {
        assert_eq!(
            decide(None, false, Some("code --wait"), Some("nano")),
            Decision::Fallback {
                spec: "code --wait".to_string()
            }
        );
    }

    #[test]
    fn decide_falls_back_to_editor_without_visual() {
        assert_eq!(
            decide(None, false, None, Some("nano")),
            Decision::Fallback {
                spec: "nano".to_string()
            }
        );
    }

    #[test]
    fn decide_falls_back_to_vi_when_nothing_is_set() {
        assert_eq!(decide(None, false, None, None), Decision::Vi);
    }

    #[test]
    fn decide_treats_blank_visual_and_editor_as_unset() {
        assert_eq!(decide(None, false, Some(""), Some("")), Decision::Vi);
    }

    #[test]
    fn find_on_path_locates_an_executable_script() {
        let dir = tempfile::tempdir().expect("tmp");
        let script = dir.path().join("fake-nvim");
        std::fs::write(&script, "#!/bin/sh\nexit 0\n").expect("schrijven");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path_var = dir.path().to_str().unwrap();
        assert_eq!(find_on_path("fake-nvim", path_var), Some(script));
    }

    #[test]
    fn find_on_path_ignores_a_non_executable_file() {
        let dir = tempfile::tempdir().expect("tmp");
        let script = dir.path().join("not-executable");
        std::fs::write(&script, "nope").expect("schrijven");
        let path_var = dir.path().to_str().unwrap();
        assert_eq!(find_on_path("not-executable", path_var), None);
    }

    #[test]
    fn find_on_path_returns_none_for_an_empty_path() {
        assert_eq!(find_on_path("nvim", ""), None);
    }

    #[test]
    fn jump_on_a_non_jumpable_frame_is_not_found_without_touching_the_filesystem() {
        let frame = model::Frame {
            path: None,
            line: None,
            func: "[internal function]".to_string(),
        };
        let map = PathMap::default();
        let ctx = JumpContext {
            server_flag: None,
            server_config: None,
            nvim_env: None,
            visual: None,
            editor: None,
            path_var: "",
        };
        struct NeverRun;
        impl CommandRunner for NeverRun {
            fn run(&mut self, _program: &str, _args: &[String]) -> io::Result<ExitStatus> {
                panic!("mag niet aangeroepen worden voor een niet-springbaar frame");
            }
        }
        let mut runner = NeverRun;
        let outcome = jump(&frame, &map, &ctx, &mut runner, || {}, || {});
        assert_eq!(
            outcome,
            JumpOutcome::NotFound {
                message: "frame niet springbaar (geen bestand:regel)".to_string()
            }
        );
    }

    #[test]
    fn jump_on_a_missing_local_file_is_not_found_without_starting_an_editor() {
        let frame = model::Frame {
            path: Some("/does/not/exist/Foo.php".to_string()),
            line: Some(10),
            func: "App\\Foo->bar()".to_string(),
        };
        let map = PathMap::default();
        let ctx = JumpContext {
            server_flag: None,
            server_config: None,
            nvim_env: None,
            visual: None,
            editor: None,
            path_var: "",
        };
        struct NeverRun;
        impl CommandRunner for NeverRun {
            fn run(&mut self, _program: &str, _args: &[String]) -> io::Result<ExitStatus> {
                panic!("mag niet aangeroepen worden als het bestand niet bestaat");
            }
        }
        let mut runner = NeverRun;
        let outcome = jump(&frame, &map, &ctx, &mut runner, || {}, || {});
        assert_eq!(
            outcome,
            JumpOutcome::NotFound {
                message: "bestand niet gevonden: /does/not/exist/Foo.php".to_string()
            }
        );
    }

    #[test]
    fn jump_maps_the_container_path_before_checking_existence() {
        let dir = tempfile::tempdir().expect("tmp");
        let local_file = dir.path().join("Foo.php");
        std::fs::write(&local_file, "<?php\n").expect("schrijven");

        let frame = model::Frame {
            path: Some("/var/www/Foo.php".to_string()),
            line: Some(10),
            func: "App\\Foo->bar()".to_string(),
        };
        let map = PathMap::new(vec![MapEntry::parse_spec(&format!(
            "/var/www={}",
            dir.path().display()
        ))
        .unwrap()]);
        let ctx = JumpContext {
            server_flag: None,
            server_config: None,
            nvim_env: None,
            visual: None,
            editor: None,
            path_var: "",
        };
        struct RecordLastCall(Option<(String, Vec<String>)>);
        impl CommandRunner for RecordLastCall {
            fn run(&mut self, program: &str, args: &[String]) -> io::Result<ExitStatus> {
                self.0 = Some((program.to_string(), args.to_vec()));
                #[cfg(unix)]
                {
                    use std::os::unix::process::ExitStatusExt;
                    Ok(ExitStatus::from_raw(0))
                }
                #[cfg(not(unix))]
                Err(io::Error::other("alleen op unix"))
            }
        }
        let mut runner = RecordLastCall(None);
        let outcome = jump(&frame, &map, &ctx, &mut runner, || {}, || {});
        assert_eq!(
            outcome,
            JumpOutcome::Spawned,
            "geen server/nvim/editor: terugval op vi"
        );
        let (program, args) = runner.0.expect("vi moet aangeroepen zijn");
        assert_eq!(program, "vi");
        assert_eq!(args[1], local_file.to_str().unwrap());
    }
}
