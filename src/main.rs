//! CLI-ingang (task.md T13: modus kiezen tussen TUI en plain-uitvoer op basis van of
//! stdout een tty is). T11 legde de opstartcontroles (leeg/ontbrekend bestand) vast;
//! dit bestand verbindt die voor het eerst met de echte leespijplijn ([`symlog::plain`]
//! voor niet-TTY-uitvoer, [`symlog::tui`] voor een echte terminal), zie
//! [`symlog::tui`]'s moduledoc ("die main-loop hoort bij de CLI-laag ... task.md T13").
//!
//! `--format`, `--grep`, `--tz`, `--map`, `--server`, `--editor` en `--config` uit
//! task.md's CLI-schets zijn hier nog niet aangesloten: T13 vraagt zelf alleen om de
//! moduskeuze en plain-uitvoer, met filters beperkt tot wat de "Klaar als"-tests nodig
//! hebben (`--level`, `--channel`). Stdin (`-`) is ook nog niet aangesloten: dat vraagt
//! stdin-steun in [`symlog::reader`] die er nog niet is.

use clap::Parser;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::layout::Alignment;
use ratatui::widgets::Paragraph;
use ratatui::Terminal;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use symlog::filter::{ChannelFilter, FilterSpec, LevelFilter};
use symlog::follow::{Follower, Watcher, DEFAULT_POLL_INTERVAL};
use symlog::plain::{self, PlainPrinter};
use symlog::reader::Reader;
use symlog::startup::{self, Outcome};
use symlog::tui::{self, keys, view, App};

type CrosstermTerminal = Terminal<CrosstermBackend<io::Stdout>>;

#[derive(Parser)]
#[command(name = "symlog", version)]
struct Cli {
    /// Pad naar het logbestand.
    path: PathBuf,
    /// Blijf het bestand volgen (`tail -f`), ook als het nog niet bestaat.
    #[arg(short = 'f', long)]
    follow: bool,
    /// Niveaufilter: een lijst (`error,critical`) of een ondergrens (`>=warning`).
    #[arg(long)]
    level: Option<String>,
    /// Kanaalfilter (herhaalbaar); `!kanaal` sluit dat kanaal juist uit.
    #[arg(long = "channel")]
    channels: Vec<String>,
    /// Forceer plain-uitvoer, ook op een tty.
    #[arg(long)]
    plain: bool,
    /// Geen kleur, ook op een tty.
    #[arg(long)]
    no_color: bool,
    /// Forceer de TUI; zonder tty op stdout geeft dit exit 3 (task.md T13 "Doet").
    #[arg(long)]
    tui: bool,
}

/// CLI-brede opties voor [`run`], gebundeld zodat de functiehandtekening niet aan
/// clippy's `too_many_arguments` komt (task.md T13: elke optie hier komt rechtstreeks
/// van [`Cli`] of van de tty-detectie in `main`).
struct RunOptions<'a> {
    follow: bool,
    want_tui: bool,
    want_plain: bool,
    stdout_is_tty: bool,
    colors_enabled: bool,
    filter: &'a FilterSpec,
}

fn main() {
    let cli = Cli::parse();
    let filter = match build_filter(cli.level.as_deref(), &cli.channels) {
        Ok(filter) => filter,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(startup::EXIT_USAGE);
        }
    };
    let stdout_is_tty = io::stdout().is_terminal();
    let no_color_env_set = std::env::var_os("NO_COLOR").is_some();
    let opts = RunOptions {
        follow: cli.follow,
        want_tui: cli.tui,
        want_plain: cli.plain,
        stdout_is_tty,
        colors_enabled: plain::colors_enabled(stdout_is_tty, cli.no_color, no_color_env_set),
        filter: &filter,
    };
    let code = run(&cli.path, &opts, &mut io::stderr(), &mut io::stdout());
    std::process::exit(code);
}

/// Bouwt het filter uit `--level`/`--channel` (task.md T13 "Klaar als": `--level error
/// --channel request`). `Err` bij een onherkend `--level`-spec: een gebruiksfout
/// (exitcode 2), geen paniek en geen stille misinterpretatie.
fn build_filter(level: Option<&str>, channels: &[String]) -> Result<FilterSpec, String> {
    let levels = match level {
        None => None,
        Some(spec) => match LevelFilter::parse(spec) {
            Some(parsed) => Some(parsed),
            None => return Err(format!("ongeldig --level: {spec:?}")),
        },
    };
    Ok(FilterSpec {
        channels: ChannelFilter::from_specs(channels),
        levels,
        search: None,
    })
}

/// De eigenlijke opstart- en modus-logica, los van `process::exit` en de echte
/// stdout/stderr zodat dit zonder een subproces getest kan worden (zie
/// `tests/startup_cli.rs`/`tests/plain_cli.rs` voor de subproces-tests die ook de
/// echte exitcode en tekst van de gecompileerde binary controleren).
fn run(path: &Path, opts: &RunOptions, err_out: &mut impl Write, out: &mut impl Write) -> i32 {
    if opts.want_tui && !opts.stdout_is_tty {
        let _ = writeln!(
            err_out,
            "--tui vereist een terminal op stdout; zonder tty is er alleen --plain \
             (of de automatische terugval daarop)"
        );
        return startup::EXIT_TUI_REQUIRES_TTY;
    }

    let classification = startup::classify(path);
    let classification = if opts.follow {
        match classification {
            startup::Classification::Missing => startup::wait_until_present(path, || {
                let _ = writeln!(err_out, "wacht op {}...", path.display());
            }),
            other => other,
        }
    } else {
        classification
    };

    match startup::outcome_for(path, classification) {
        Outcome::Exit { message, code } => {
            let _ = writeln!(err_out, "{message}");
            code
        }
        Outcome::Empty | Outcome::Ready => {
            let use_plain = opts.want_plain || !opts.stdout_is_tty;
            if use_plain {
                run_plain_mode(
                    path,
                    opts.follow,
                    opts.filter,
                    opts.colors_enabled,
                    out,
                    err_out,
                )
            } else {
                run_tui_mode(path, opts.follow, opts.filter)
            }
        }
    }
}

/// Plain-uitvoer (task.md T13 "Doet"): dumpt wat er nu al staat, en blijft bij
/// `--follow` daarna net als [`Follower`] zelf voor altijd verder streamen (net als
/// `tail -f`; stopt alleen doordat het proces beëindigd wordt, zie
/// `tests/startup_cli.rs`'s `missing_file_with_follow_waits_then_keeps_following`, die
/// dat al voor de oude, ongekoppelde `follow_forever` controleerde).
fn run_plain_mode(
    path: &Path,
    follow: bool,
    filter: &FilterSpec,
    colors_enabled: bool,
    out: &mut impl Write,
    err_out: &mut impl Write,
) -> i32 {
    let mut follower = Follower::open(path);
    let mut printer = PlainPrinter::new();

    if let Err(err) = follower.tick() {
        let _ = writeln!(err_out, "{}: {err}", path.display());
        return startup::EXIT_IO_ERROR;
    }
    if let Err(err) = printer.flush(&follower, filter, colors_enabled, out) {
        let _ = writeln!(err_out, "{}: {err}", path.display());
        if !follow {
            return startup::EXIT_IO_ERROR;
        }
    }

    if !follow {
        return 0;
    }

    let watcher = Watcher::new(path, DEFAULT_POLL_INTERVAL);
    loop {
        watcher.wait();
        if let Err(err) = follower.tick() {
            // Herstelbaar (task.md T10 "Blijft"): de volgende tick detecteert een
            // eventuele afkapping of rotatie gewoon opnieuw via `stat`, dus dit stopt
            // het volgen niet.
            let _ = writeln!(err_out, "{}: {err}", path.display());
            continue;
        }
        if let Err(err) = printer.flush(&follower, filter, colors_enabled, out) {
            let _ = writeln!(err_out, "{}: {err}", path.display());
        }
    }
}

/// Interactieve TUI (task.md T13 "Doet"): leest het hele bestand (via
/// [`Reader::wait_until_indexed`]) en bouwt daar [`App`] op, net als T8's eigen tests
/// al deden; deze functie is de eerste plek die dat met een echte terminal verbindt.
/// Een leeg bestand (task.md T11 "Doet": "TUI opent met de melding 'log is leeg'")
/// toont alleen die melding. `--follow` zet hier alleen de follow-indicator aan
/// ([`App::toggle_follow`]): live bijwerken tijdens de TUI-sessie zelf hoort niet bij
/// task.md T13 "Doet" (dat vraagt alleen "`--follow` werkt ook in plain"), dus dat
/// blijft het bestaande, niet-levende follow-statusveld uit T8.
fn run_tui_mode(path: &Path, follow: bool, filter: &FilterSpec) -> i32 {
    let mut reader = match Reader::open(path) {
        Ok(reader) => reader,
        Err(err) => {
            eprintln!("{}: {err}", path.display());
            return startup::EXIT_IO_ERROR;
        }
    };
    if let Err(err) = reader.wait_until_indexed() {
        eprintln!("{}: {err}", path.display());
        return startup::EXIT_IO_ERROR;
    }

    let mut records = Vec::with_capacity(reader.record_count());
    for i in 0..reader.record_count() {
        match reader.record_bytes(i) {
            Ok(Some(bytes)) => records.push(symlog::parse::parse_record_lossy(&bytes)),
            Ok(None) => break,
            Err(err) => {
                eprintln!("{}: {err}", path.display());
                return startup::EXIT_IO_ERROR;
            }
        }
    }

    if records.is_empty() {
        return match run_in_terminal(run_empty_message_loop) {
            Ok(()) => 0,
            Err(err) => {
                eprintln!("{err}");
                startup::EXIT_IO_ERROR
            }
        };
    }

    let mut app = App::new(records);
    app.set_filter(filter.clone());
    if follow {
        app.toggle_follow();
    }

    match run_in_terminal(|terminal| run_app_loop(terminal, &mut app)) {
        Ok(()) => {
            if let Some(copy) = app.copy_on_exit() {
                println!("{copy}");
            }
            0
        }
        Err(err) => {
            eprintln!("{err}");
            startup::EXIT_IO_ERROR
        }
    }
}

/// Zet raw mode en het alternate screen aan, voert `body` uit en herstelt de terminal
/// altijd weer (ook als `body` of het aanzetten zelf faalt), task.md T8 "Doet": "Bij
/// panic wordt de terminal hersteld" (de paniekhaak via [`tui::install_panic_hook`]
/// dekt de paniek-kant; dit dekt de normale, foutloze en fout-eindigende paden).
fn run_in_terminal<F>(body: F) -> io::Result<()>
where
    F: FnOnce(&mut CrosstermTerminal) -> io::Result<()>,
{
    tui::install_panic_hook();
    enable_raw_mode()?;
    if let Err(err) = execute!(io::stdout(), EnterAlternateScreen) {
        let _ = disable_raw_mode();
        return Err(err);
    }

    let result = match Terminal::new(CrosstermBackend::new(io::stdout())) {
        Ok(mut terminal) => {
            let result = body(&mut terminal);
            // Best-effort: een fout hier mag de echte fout uit `body` niet verdringen.
            let _ = terminal.show_cursor();
            result
        }
        Err(err) => Err(err),
    };

    let _ = execute!(io::stdout(), LeaveAlternateScreen);
    let _ = disable_raw_mode();
    result
}

fn run_app_loop(terminal: &mut CrosstermTerminal, app: &mut App) -> io::Result<()> {
    loop {
        terminal.draw(|frame| view::render(frame, app))?;
        if app.should_quit() {
            return Ok(());
        }
        if let Event::Key(key) = event::read()? {
            keys::handle_key(app, key);
        }
    }
}

/// Task.md T11 "Doet": "TUI opent met de melding 'log is leeg'"; elke toets sluit af.
fn run_empty_message_loop(terminal: &mut CrosstermTerminal) -> io::Result<()> {
    loop {
        terminal.draw(render_empty_message)?;
        if let Event::Key(_) = event::read()? {
            return Ok(());
        }
    }
}

fn render_empty_message(frame: &mut ratatui::Frame) {
    let area = frame.area();
    frame.render_widget(
        Paragraph::new("log is leeg").alignment(Alignment::Center),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use symlog::model::Level;

    fn write_file(path: &Path, content: &[u8]) {
        std::fs::File::create(path)
            .expect("create")
            .write_all(content)
            .expect("write");
    }

    fn plain_opts<'a>(filter: &'a FilterSpec, follow: bool) -> RunOptions<'a> {
        RunOptions {
            follow,
            want_tui: false,
            want_plain: true,
            stdout_is_tty: false,
            colors_enabled: false,
            filter,
        }
    }

    #[test]
    fn empty_file_exits_zero_without_any_output() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("empty.log");
        write_file(&path, b"");

        let filter = FilterSpec::default();
        let opts = plain_opts(&filter, false);
        let mut err = Vec::new();
        let mut out = Vec::new();
        let code = run(&path, &opts, &mut err, &mut out);
        assert_eq!(code, 0);
        assert!(err.is_empty());
        assert!(out.is_empty());
    }

    #[test]
    fn missing_file_without_follow_exits_one_with_a_hint() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("weg.log");

        let filter = FilterSpec::default();
        let opts = plain_opts(&filter, false);
        let mut err = Vec::new();
        let mut out = Vec::new();
        let code = run(&path, &opts, &mut err, &mut out);
        assert_eq!(code, startup::EXIT_MISSING);
        let message = String::from_utf8(err).expect("utf8");
        assert!(message.contains(&path.display().to_string()));
        assert!(message.contains("bestaat niet"));
    }

    #[test]
    fn directory_as_path_exits_two() {
        let dir = tempfile::tempdir().expect("tempdir");

        let filter = FilterSpec::default();
        let opts = plain_opts(&filter, false);
        let mut err = Vec::new();
        let mut out = Vec::new();
        let code = run(dir.path(), &opts, &mut err, &mut out);
        assert_eq!(code, startup::EXIT_USAGE);
        assert!(String::from_utf8(err).expect("utf8").contains("map"));
    }

    #[test]
    fn non_empty_file_without_follow_exits_zero_and_prints_the_plain_line() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("app.log");
        write_file(&path, b"[2026-01-01 00:00:00] a.INFO: one [] []\n");

        let filter = FilterSpec::default();
        let opts = plain_opts(&filter, false);
        let mut err = Vec::new();
        let mut out = Vec::new();
        let code = run(&path, &opts, &mut err, &mut out);
        assert_eq!(code, 0);
        assert!(err.is_empty());
        assert_eq!(
            String::from_utf8(out).expect("utf8"),
            "[2026-01-01 00:00:00] a.INFO: one\n"
        );
    }

    #[test]
    fn tui_without_a_tty_exits_three_with_an_explanation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("app.log");
        write_file(&path, b"[2026-01-01 00:00:00] a.INFO: one [] []\n");

        let filter = FilterSpec::default();
        let opts = RunOptions {
            follow: false,
            want_tui: true,
            want_plain: false,
            stdout_is_tty: false,
            colors_enabled: false,
            filter: &filter,
        };
        let mut err = Vec::new();
        let mut out = Vec::new();
        let code = run(&path, &opts, &mut err, &mut out);
        assert_eq!(code, startup::EXIT_TUI_REQUIRES_TTY);
        assert!(out.is_empty());
        assert!(!err.is_empty());
    }

    #[test]
    fn tui_without_a_tty_is_checked_before_the_path_is_even_looked_at() {
        // Een niet-bestaand pad mag de `--tui`-zonder-tty-fout niet overschaduwen met
        // de "bestaat niet"-fout: de vlagcombinatie zelf is al ongeldig.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("bestaat-niet.log");

        let filter = FilterSpec::default();
        let opts = RunOptions {
            follow: false,
            want_tui: true,
            want_plain: false,
            stdout_is_tty: false,
            colors_enabled: false,
            filter: &filter,
        };
        let mut err = Vec::new();
        let mut out = Vec::new();
        let code = run(&path, &opts, &mut err, &mut out);
        assert_eq!(code, startup::EXIT_TUI_REQUIRES_TTY);
    }

    #[test]
    fn level_and_channel_filters_apply_to_the_plain_output() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("app.log");
        write_file(
            &path,
            b"[2026-01-01 00:00:00] app.ERROR: kept [] []\n\
              [2026-01-01 00:00:01] app.DEBUG: dropped by level [] []\n\
              [2026-01-01 00:00:02] request.ERROR: dropped by channel [] []\n",
        );

        let filter = build_filter(Some("error"), &["app".to_string()]).expect("filter");
        let opts = plain_opts(&filter, false);
        let mut err = Vec::new();
        let mut out = Vec::new();
        let code = run(&path, &opts, &mut err, &mut out);
        assert_eq!(code, 0);
        assert_eq!(
            String::from_utf8(out).expect("utf8"),
            "[2026-01-01 00:00:00] app.ERROR: kept\n"
        );
    }

    #[test]
    fn build_filter_rejects_an_unknown_level_spec() {
        assert!(build_filter(Some("trace"), &[]).is_err());
        assert!(build_filter(Some(">=warning"), &[]).is_ok());
        assert!(build_filter(None, &[]).is_ok());
    }

    #[test]
    fn build_filter_combines_level_and_channel() {
        let filter = build_filter(
            Some(">=error"),
            &["app".to_string(), "!request".to_string()],
        )
        .expect("filter");
        assert!(filter.channels.matches("app"));
        assert!(!filter.channels.matches("request"));
        assert!(filter.levels.expect("level").matches(Level::Error));
    }
}
