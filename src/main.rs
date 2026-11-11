//! CLI-ingang (task.md T13: modus kiezen tussen TUI en plain-uitvoer op basis van of
//! stdout een tty is). T11 legde de opstartcontroles (leeg/ontbrekend bestand) vast;
//! dit bestand verbindt die voor het eerst met de echte leespijplijn ([`symlog::plain`]
//! voor niet-TTY-uitvoer, [`symlog::tui`] voor een echte terminal), zie
//! [`symlog::tui`]'s moduledoc ("die main-loop hoort bij de CLI-laag ... task.md T13").
//!
//! `--format`, `--grep`, `--tz`, `--map`, `--server`, `--editor` en `--config` uit
//! task.md's CLI-schets zijn hier nog niet aangesloten: T13 vraagt zelf alleen om de
//! moduskeuze en plain-uitvoer, met filters beperkt tot wat de "Klaar als"-tests nodig
//! hebben (`--level`, `--channel`). Stdin (`-`) loopt via [`symlog::plain::stream_plain`]
//! (W-05), los van [`symlog::reader`].

mod interrupt;

use clap::{CommandFactory, FromArgMatches, Parser, ValueEnum};
use clap_complete::{generate, Shell};
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
use std::time::Duration;
use symlog::filter::{ChannelFilter, FilterSpec, LevelFilter};
use symlog::follow::{Follower, Watcher, DEFAULT_POLL_INTERVAL};
use symlog::plain::{self, PlainPrinter};
use symlog::reader::Reader;
use symlog::startup::{self, Outcome};
use symlog::tui::{self, keys, view, App};

type CrosstermTerminal = Terminal<CrosstermBackend<io::Stdout>>;

#[derive(Parser)]
#[command(name = "symlog")]
struct Cli {
    /// Pad naar het logbestand, of `-` om van stdin te lezen (streaming, alleen plain;
    /// de invoer wordt niet in het geheugen verzameld).
    #[arg(required_unless_present = "completions")]
    path: Option<PathBuf>,
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
    /// Toon geheimen ongemaskeerd. Standaard worden ze vervangen door `***`.
    #[arg(long)]
    no_redact: bool,
    /// Forceer de TUI; zonder tty op stdout geeft dit exit 3.
    #[arg(long)]
    tui: bool,
    /// Schrijf de plain-uitvoer naar dit bestand in plaats van stdout. Een bestaand bestand wordt vervangen. Een ontbrekende bovenliggende map is een fout en wordt niet aangemaakt.
    #[arg(long, value_name = "PATH", conflicts_with_all = ["tui", "completions"])]
    output: Option<PathBuf>,
    /// Verberg de hintregel. Geen contextregel. Een pipe laat beide al weg.
    #[arg(long, conflicts_with = "verbose")]
    quiet: bool,
    /// Toon de hintregel, en bij succes één contextregel op stderr. Niet samen met --quiet.
    #[arg(long, conflicts_with = "quiet")]
    verbose: bool,
    /// Print een shell-completionscript (bash of zsh) en stop.
    #[arg(long, value_enum, value_name = "SHELL")]
    completions: Option<CompletionShell>,
}

/// Shells waarvoor `--completions` een script kan printen.
#[derive(Clone, Copy, ValueEnum)]
enum CompletionShell {
    Bash,
    Zsh,
}

impl From<CompletionShell> for Shell {
    fn from(shell: CompletionShell) -> Self {
        match shell {
            CompletionShell::Bash => Shell::Bash,
            CompletionShell::Zsh => Shell::Zsh,
        }
    }
}

/// Schrijft het completionscript naar stdout. Een mislukte schrijfactie geeft
/// exit 4 en een melding, geen paniek.
fn print_completions(shell: CompletionShell, extra: bool) -> i32 {
    let mut script = Vec::new();
    generate(
        Shell::from(shell),
        &mut cli_command(),
        "symlog",
        &mut script,
    );
    let mut stdout = io::stdout().lock();
    match stdout.write_all(&script).and_then(|()| stdout.flush()) {
        Ok(()) if interrupt::interrupted() => interrupt::EXIT_INTERRUPTED,
        Ok(()) => 0,
        Err(_) if interrupt::interrupted() => interrupt::EXIT_INTERRUPTED,
        Err(err) => {
            emit_message(
                &format!(
                    "kan het completionscript niet schrijven: {err}; controleer of stdout nog open staat en schijfruimte heeft, of schrijf naar een bestand"
                ),
                extra,
            );
            startup::EXIT_IO_ERROR
        }
    }
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
    /// Maskeer geheimen in de uitvoer. Standaard aan; `--no-redact` zet het uit.
    redact: bool,
    /// Eén `context:`-regel op stderr nadat de uitvoer geschreven is.
    verbose: bool,
    /// Hintregel bij een fout. Uit op een pipe, aan op een terminal, tenzij de vlaggen anders zeggen.
    extra: bool,
    filter: &'a FilterSpec,
}

/// Hintregels zijn extra context.
///
/// Een terminal toont ze. Een pipe of een redirect niet, zodat vastgelegde stderr
/// de foutregel blijft. `--verbose` toont ze toch. `--quiet` verbergt ze ook op
/// een terminal. De plain-uitvoer op stdout blijft gelijk. Beide vlaggen samen
/// zijn een clap-gebruiksfout en komen nooit bij een rapport.
fn show_extra(quiet: bool, verbose: bool, terminal: bool) -> bool {
    if quiet {
        false
    } else if verbose {
        true
    } else {
        terminal
    }
}

/// De vervolgstap staat al in de fout, na `; ` of `. `.
fn next_step(message: &str) -> Option<&str> {
    let semi = message.find("; ").map(|index| index + 2);
    let dot = message.find(". ").map(|index| index + 2);
    let start = match (semi, dot) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }?;
    let tail = message[start..].trim();
    if tail.is_empty() {
        None
    } else {
        Some(tail)
    }
}

fn emit_to(err_out: &mut impl Write, message: &str, extra: bool) {
    let _ = writeln!(err_out, "{message}");
    if extra {
        if let Some(hint) = next_step(message) {
            let _ = writeln!(err_out, "hint: {hint}");
        }
    }
}

fn emit_message(message: &str, extra: bool) {
    emit_to(&mut io::stderr(), message, extra);
}

fn emit_context(err_out: &mut impl Write, records: impl std::fmt::Display, verbose: bool) {
    if verbose {
        let _ = writeln!(err_out, "context: records {records}");
    }
}

/// `--version` en `-V`. De crate-versie altijd. In een debug-build ook de
/// git-sha, alleen als de build die van git kreeg. Een release-build niet.
fn cli_version() -> &'static str {
    static VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    VERSION
        .get_or_init(|| {
            format_cli_version(
                env!("CARGO_PKG_VERSION"),
                option_env!("SYMLOG_GIT_SHA"),
                cfg!(debug_assertions),
            )
        })
        .as_str()
}

/// `debug` toont `sha` alleen als het een objectnaam is zoals `git rev-parse`
/// die print. Andere tekst, en elke release-build, blijft bij de crate-versie.
fn format_cli_version(crate_version: &str, sha: Option<&str>, debug: bool) -> String {
    if debug {
        if let Some(sha) = sha.filter(|candidate| is_git_object_name(candidate)) {
            return format!("{crate_version} {sha}");
        }
    }
    crate_version.to_string()
}

/// 40 (SHA-1) of 64 (SHA-256) kleine hex-tekens.
fn is_git_object_name(text: &str) -> bool {
    matches!(text.len(), 40 | 64)
        && text
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn cli_command() -> clap::Command {
    Cli::command().version(cli_version())
}

fn parse_cli() -> Cli {
    let matches = cli_command().get_matches();
    match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(err) => err.exit(),
    }
}

fn main() {
    interrupt::install();
    let cli = parse_cli();
    let extra = show_extra(cli.quiet, cli.verbose, io::stderr().is_terminal());
    if let Some(shell) = cli.completions {
        exit_or_interrupt(print_completions(shell, extra), extra);
    }
    let Some(path) = cli.path.as_deref() else {
        // `required_unless_present` laat dit niet toe; dit is alleen een vangnet.
        emit_message(
            "het pad naar het logbestand ontbreekt; geef een pad of `-` mee, zie `symlog --help`",
            extra,
        );
        exit_or_interrupt(startup::EXIT_USAGE, extra);
    };
    let filter = match build_filter(cli.level.as_deref(), &cli.channels) {
        Ok(filter) => filter,
        Err(message) => {
            emit_message(&message, extra);
            exit_or_interrupt(startup::EXIT_USAGE, extra);
        }
    };
    let to_file = cli.output.is_some();
    let stdout_is_tty = !to_file && io::stdout().is_terminal();
    let no_color_env_set = std::env::var_os("NO_COLOR").is_some();
    let opts = RunOptions {
        follow: cli.follow,
        want_tui: cli.tui,
        want_plain: cli.plain || to_file,
        stdout_is_tty,
        colors_enabled: plain::colors_enabled(stdout_is_tty, cli.no_color, no_color_env_set),
        redact: !cli.no_redact,
        verbose: cli.verbose,
        extra,
        filter: &filter,
    };
    if let Some(out_path) = cli.output.as_deref() {
        let code = if opts.follow {
            write_follow_output(out_path, path, &opts, extra)
        } else {
            publish_report(out_path, path, &opts, extra)
        };
        exit_or_interrupt(code, extra);
    }
    let code = run(path, &opts, &mut io::stderr(), &mut io::stdout());
    exit_or_interrupt(code, extra);
}

/// Stopt het proces. SIGINT eindigt met 130 en één stderr-regel, ook als `code`
/// al een gewone fout was. De gewone codes worden niet hernummerd.
fn exit_or_interrupt(code: i32, extra: bool) -> ! {
    if code == interrupt::EXIT_INTERRUPTED || interrupt::interrupted() {
        emit_message(interrupt::INTERRUPTED_MESSAGE, extra);
        std::process::exit(interrupt::EXIT_INTERRUPTED);
    }
    std::process::exit(code);
}

/// Tijdelijk rapport naast `path`. Verdwijnt bij drop, ook na een geslaagde
/// hernoeming (dan bestaat de tijdelijke naam niet meer).
struct TempReport(PathBuf);

impl Drop for TempReport {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Pad van een tijdelijk bestand naast `path`, zodat een af rapport `path` in
/// één hernoeming vervangt. SIGINT vóór die hernoeming verwijdert het tijdelijke
/// bestand en laat `path` zoals het was.
fn temp_beside(path: &Path) -> PathBuf {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut name = std::ffi::OsString::from(".");
    name.push(path.file_name().unwrap_or(std::ffi::OsStr::new("rapport")));
    name.push(format!(".symlog-{}.tmp", std::process::id()));
    parent.join(name)
}

/// Schrijft een eenmalig rapport naar een tijdelijk bestand en publiceert het
/// pas als het af is. Een ontbrekende bovenliggende map blijft exit 4 en wordt
/// niet aangemaakt. SIGINT laat het vorige bestand staan en geeft 130.
fn publish_report(path: &Path, log_path: &Path, opts: &RunOptions<'_>, extra: bool) -> i32 {
    if interrupt::interrupted() {
        return interrupt::EXIT_INTERRUPTED;
    }
    let tmp = temp_beside(path);
    let mut file = match std::fs::File::create(&tmp) {
        Ok(file) => file,
        Err(err) => {
            if interrupt::interrupted() {
                return interrupt::EXIT_INTERRUPTED;
            }
            emit_message(&output_write_error(path, &err), extra);
            return startup::EXIT_IO_ERROR;
        }
    };
    let _guard = TempReport(tmp.clone());
    let code = run(log_path, opts, &mut io::stderr(), &mut file);
    if let Err(err) = file.flush() {
        if interrupt::interrupted() {
            return interrupt::EXIT_INTERRUPTED;
        }
        emit_message(&output_write_error(path, &err), extra);
        return startup::EXIT_IO_ERROR;
    }
    drop(file);
    if code != 0 || interrupt::interrupted() {
        if interrupt::interrupted() || code == interrupt::EXIT_INTERRUPTED {
            return interrupt::EXIT_INTERRUPTED;
        }
        return code;
    }
    let renamed = interrupt::with_sigint_blocked(|| {
        if interrupt::interrupted() {
            return None;
        }
        Some(std::fs::rename(&tmp, path))
    });
    match renamed {
        None => interrupt::EXIT_INTERRUPTED,
        Some(Ok(())) if interrupt::interrupted() => interrupt::EXIT_INTERRUPTED,
        Some(Ok(())) => 0,
        Some(Err(_)) if interrupt::interrupted() => interrupt::EXIT_INTERRUPTED,
        Some(Err(err)) => {
            emit_message(&output_write_error(path, &err), extra);
            startup::EXIT_IO_ERROR
        }
    }
}

/// `--follow` schrijft meteen naar het doel, omdat de stroom niet op een
/// eindig rapport wacht. SIGINT stopt de stroom met 130.
fn write_follow_output(path: &Path, log_path: &Path, opts: &RunOptions<'_>, extra: bool) -> i32 {
    if interrupt::interrupted() {
        return interrupt::EXIT_INTERRUPTED;
    }
    match std::fs::File::create(path) {
        Ok(mut file) => {
            let code = run(log_path, opts, &mut io::stderr(), &mut file);
            if let Err(err) = file.flush() {
                if interrupt::interrupted() || code == interrupt::EXIT_INTERRUPTED {
                    return interrupt::EXIT_INTERRUPTED;
                }
                emit_message(&output_write_error(path, &err), extra);
                return startup::EXIT_IO_ERROR;
            }
            if interrupt::interrupted() {
                return interrupt::EXIT_INTERRUPTED;
            }
            code
        }
        Err(err) => {
            if interrupt::interrupted() {
                return interrupt::EXIT_INTERRUPTED;
            }
            emit_message(&output_write_error(path, &err), extra);
            startup::EXIT_IO_ERROR
        }
    }
}

/// Fouttekst als `--output` niet geschreven kan worden. Exitcode blijft 4,
/// dezelfde code als een andere mislukte schrijfactie. De map wordt niet aangemaakt.
fn output_write_error(path: &Path, err: &io::Error) -> String {
    format!(
        "{}: schrijven mislukt ({err}); controleer of de bovenliggende map bestaat en beschrijfbaar is; symlog maakt die map niet aan",
        path.display()
    )
}

/// Bouwt het filter uit `--level`/`--channel` (task.md T13 "Klaar als": `--level error
/// --channel request`). `Err` bij een onherkend `--level`-spec: een gebruiksfout
/// (exitcode 2), geen paniek en geen stille misinterpretatie.
fn build_filter(level: Option<&str>, channels: &[String]) -> Result<FilterSpec, String> {
    let levels = match level {
        None => None,
        Some(spec) => match LevelFilter::parse(spec) {
            Some(parsed) => Some(parsed),
            None => return Err(format!(
                "ongeldig --level: {spec:?}; gebruik een lijst van niveaus (`error,critical`) of een ondergrens (`>=warning`); \
                 geldige niveaus: debug, info, notice, warning, error, critical, alert, emergency"
            )),
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
    if interrupt::interrupted() {
        return interrupt::EXIT_INTERRUPTED;
    }
    if opts.want_tui && !opts.stdout_is_tty {
        emit_to(
            err_out,
            "--tui vereist een terminal op stdout; zonder tty is er alleen --plain \
             (of de automatische terugval daarop); laat --tui weg of start symlog in een echte terminal",
            opts.extra,
        );
        return startup::EXIT_TUI_REQUIRES_TTY;
    }

    if path == Path::new("-") {
        return run_stdin_mode(opts, err_out, out);
    }

    let classification = startup::classify(path);
    let classification = if opts.follow {
        match classification {
            startup::Classification::Missing => {
                match startup::wait_until_present_or_stop(
                    path,
                    || {
                        let _ = writeln!(err_out, "wacht op {}...", path.display());
                    },
                    interrupt::interrupted,
                ) {
                    Some(found) => found,
                    None => return interrupt::EXIT_INTERRUPTED,
                }
            }
            other => other,
        }
    } else {
        classification
    };

    match startup::outcome_for(path, classification) {
        Outcome::Exit { message, code } => {
            emit_to(err_out, &message, opts.extra);
            code
        }
        Outcome::Empty | Outcome::Ready => {
            let use_plain = opts.want_plain || !opts.stdout_is_tty;
            if use_plain {
                run_plain_mode(path, opts, out, err_out)
            } else {
                run_tui_mode(path, opts)
            }
        }
    }
}

/// Stdin-streaming (W-05): `symlog -` leest stdin in één doorgang en schrijft elk
/// record plain uit zodra het compleet is ([`plain::stream_plain`]). Stdin is niet
/// terug te spoelen en niet te volgen, dus `--follow` en `--tui` zijn hier een
/// gebruiksfout (exit 2).
fn run_stdin_mode(opts: &RunOptions, err_out: &mut impl Write, out: &mut impl Write) -> i32 {
    if opts.follow || opts.want_tui {
        emit_to(
            err_out,
            "stdin (`-`) werkt alleen als streaming plain-uitvoer; --follow en --tui zijn niet beschikbaar; \
             laat --follow en --tui weg, of geef een bestandspad op in plaats van `-`",
            opts.extra,
        );
        return startup::EXIT_USAGE;
    }
    let stdin = io::stdin();
    let writer = io::BufWriter::with_capacity(64 * 1024, out);
    match plain::stream_plain(
        stdin.lock(),
        opts.filter,
        opts.colors_enabled,
        opts.redact,
        writer,
    ) {
        Ok(_) if interrupt::interrupted() => interrupt::EXIT_INTERRUPTED,
        Ok(count) => {
            emit_context(err_out, count, opts.verbose);
            0
        }
        Err(_) if interrupt::interrupted() => interrupt::EXIT_INTERRUPTED,
        // `symlog - | head`: de lezer sloot de pijp, dat is geen fout.
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => 0,
        Err(err) => {
            emit_to(
                err_out,
                &format!(
                    "stdin: lezen of schrijven mislukt ({err}); controleer of de invoerpijp en de uitvoer (schijf of pijp) nog open staan"
                ),
                opts.extra,
            );
            startup::EXIT_IO_ERROR
        }
    }
}

/// Foutmelding als het uitschrijven van plain-uitvoer faalt. De fout kan van het lezen
/// van het logbestand komen of van het schrijven naar stdout (schijf vol, gesloten
/// pijp); de tekst noemt beide en zegt wat je controleert.
fn flush_failure_message(path: &Path, err: &io::Error) -> String {
    format!(
        "{}: lezen of schrijven mislukt ({err}); controleer of het bestand nog bestaat en leesbaar is, \
         en of de uitvoer (schijf of pijp) nog open staat",
        path.display()
    )
}

/// Plain-uitvoer (task.md T13 "Doet"): dumpt wat er nu al staat, en blijft bij
/// `--follow` daarna net als [`Follower`] zelf voor altijd verder streamen (net als
/// `tail -f`; stopt alleen doordat het proces beëindigd wordt, zie
/// `tests/startup_cli.rs`'s `missing_file_with_follow_waits_then_keeps_following`, die
/// dat al voor de oude, ongekoppelde `follow_forever` controleerde).
fn run_plain_mode(
    path: &Path,
    opts: &RunOptions,
    out: &mut impl Write,
    err_out: &mut impl Write,
) -> i32 {
    let mut follower = Follower::open(path);
    let mut printer = PlainPrinter::new();
    printer.set_redact(opts.redact);

    if let Err(err) = follower.tick() {
        if interrupt::interrupted() {
            return interrupt::EXIT_INTERRUPTED;
        }
        emit_to(
            err_out,
            &format!(
                "{}: lezen mislukt ({err}); controleer of het bestand nog bestaat en leesbaar is",
                path.display()
            ),
            opts.extra,
        );
        return startup::EXIT_IO_ERROR;
    }
    if !opts.follow {
        // Een eenmalige run groeit nooit meer verder: sluit de generatie af zodat een
        // staartregel zonder afsluitende `\n` alsnog verschijnt (geredigeerd), net als
        // de TUI-weg via `IndexScanner::finish` (breaktest-symlog-0004 bevinding 1).
        // Met `--follow` blijft dit ongewijzigd: `Follower::finish` wordt dan bewust
        // niet aangeroepen, zie het commentaar daar.
        follower.finish();
    }
    if let Err(err) = printer.flush(&follower, opts.filter, opts.colors_enabled, out) {
        if interrupt::interrupted() {
            return interrupt::EXIT_INTERRUPTED;
        }
        emit_to(err_out, &flush_failure_message(path, &err), opts.extra);
        if !opts.follow {
            return startup::EXIT_IO_ERROR;
        }
    } else if interrupt::interrupted() {
        return interrupt::EXIT_INTERRUPTED;
    } else {
        emit_context(err_out, follower.total_record_count(), opts.verbose);
        if !opts.follow {
            return 0;
        }
    }

    let watcher = Watcher::new(path, DEFAULT_POLL_INTERVAL);
    loop {
        if interrupt::interrupted() {
            return interrupt::EXIT_INTERRUPTED;
        }
        watcher.wait();
        if interrupt::interrupted() {
            return interrupt::EXIT_INTERRUPTED;
        }
        if let Err(err) = follower.tick() {
            if interrupt::interrupted() {
                return interrupt::EXIT_INTERRUPTED;
            }
            // Herstelbaar (task.md T10 "Blijft"): de volgende tick detecteert een
            // eventuele afkapping of rotatie gewoon opnieuw via `stat`, dus dit stopt
            // het volgen niet.
            emit_to(
                err_out,
                &format!(
                    "{}: lezen mislukt ({err}); controleer of het bestand nog bestaat en leesbaar is",
                    path.display()
                ),
                opts.extra,
            );
            continue;
        }
        if let Err(err) = printer.flush(&follower, opts.filter, opts.colors_enabled, out) {
            if interrupt::interrupted() {
                return interrupt::EXIT_INTERRUPTED;
            }
            emit_to(err_out, &flush_failure_message(path, &err), opts.extra);
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
fn run_tui_mode(path: &Path, opts: &RunOptions) -> i32 {
    let mut reader = match Reader::open(path) {
        Ok(reader) => reader,
        Err(err) => return read_error(path, &err, opts.extra),
    };
    if let Err(err) = reader.wait_until_indexed() {
        return read_error(path, &err, opts.extra);
    }

    let mut records = Vec::with_capacity(reader.record_count());
    for i in 0..reader.record_count() {
        match reader.record_bytes(i) {
            Ok(Some(bytes)) => records.push(symlog::parse::parse_record_lossy(&bytes)),
            Ok(None) => break,
            Err(err) => return read_error(path, &err, opts.extra),
        }
    }

    let record_count = records.len();
    if records.is_empty() {
        return match run_in_terminal(run_empty_message_loop) {
            Ok(()) if interrupt::interrupted() => interrupt::EXIT_INTERRUPTED,
            Ok(()) => {
                emit_context(&mut io::stderr(), record_count, opts.verbose);
                0
            }
            Err(_) if interrupt::interrupted() => interrupt::EXIT_INTERRUPTED,
            Err(err) => terminal_error(&err, opts.extra),
        };
    }

    let mut app = App::new(records);
    app.set_redact(opts.redact);
    app.set_filter(opts.filter.clone());
    if opts.follow {
        app.toggle_follow();
    }

    match run_in_terminal(|terminal| run_app_loop(terminal, &mut app)) {
        Ok(()) if interrupt::interrupted() => interrupt::EXIT_INTERRUPTED,
        Ok(()) => {
            if let Some(copy) = app.copy_on_exit() {
                println!("{}", symlog::redact::for_display(copy, app.redact()));
            }
            emit_context(&mut io::stderr(), record_count, opts.verbose);
            0
        }
        Err(_) if interrupt::interrupted() => interrupt::EXIT_INTERRUPTED,
        Err(err) => terminal_error(&err, opts.extra),
    }
}

fn read_error(path: &Path, err: &io::Error, extra: bool) -> i32 {
    if interrupt::interrupted() {
        return interrupt::EXIT_INTERRUPTED;
    }
    emit_message(
        &format!(
            "{}: lezen mislukt ({err}); controleer of het bestand nog bestaat en leesbaar is",
            path.display()
        ),
        extra,
    );
    startup::EXIT_IO_ERROR
}

fn terminal_error(err: &io::Error, extra: bool) -> i32 {
    if interrupt::interrupted() {
        return interrupt::EXIT_INTERRUPTED;
    }
    emit_message(
        &format!("terminalfout: {err}; start symlog in een echte terminal of gebruik --plain"),
        extra,
    );
    startup::EXIT_IO_ERROR
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

/// Hoe lang de TUI wacht op een toets voordat hij SIGINT opnieuw bekijkt.
const EVENT_WAIT: Duration = Duration::from_millis(200);

fn event_ready() -> io::Result<bool> {
    match event::poll(EVENT_WAIT) {
        Ok(ready) => Ok(ready),
        Err(_) if interrupt::interrupted() => Ok(false),
        Err(err) => Err(err),
    }
}

fn run_app_loop(terminal: &mut CrosstermTerminal, app: &mut App) -> io::Result<()> {
    terminal.draw(|frame| view::render(frame, app))?;
    loop {
        if interrupt::interrupted() || app.should_quit() {
            return Ok(());
        }
        if !event_ready()? {
            continue;
        }
        if let Event::Key(key) = event::read()? {
            keys::handle_key(app, key);
        }
        terminal.draw(|frame| view::render(frame, app))?;
    }
}

/// Task.md T11 "Doet": "TUI opent met de melding 'log is leeg'"; elke toets sluit af.
fn run_empty_message_loop(terminal: &mut CrosstermTerminal) -> io::Result<()> {
    terminal.draw(render_empty_message)?;
    loop {
        if interrupt::interrupted() {
            return Ok(());
        }
        if !event_ready()? {
            continue;
        }
        if let Event::Key(_) = event::read()? {
            return Ok(());
        }
        terminal.draw(render_empty_message)?;
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

    #[test]
    fn debug_version_appends_a_git_object_name() {
        let sha1 = "ab".repeat(20);
        let sha256 = "cd".repeat(32);
        assert_eq!(
            format_cli_version("1.0.0", Some(&sha1), true),
            format!("1.0.0 {sha1}")
        );
        assert_eq!(
            format_cli_version("1.0.0", Some(&sha256), true),
            format!("1.0.0 {sha256}")
        );
    }

    #[test]
    fn version_omits_a_sha_git_did_not_print() {
        let short = "ab".repeat(19);
        let upper = "ab".repeat(20).to_ascii_uppercase();
        let not_hex = "zz".repeat(20);
        let samples = [
            "",
            "HEAD",
            "main",
            "abc",
            short.as_str(),
            upper.as_str(),
            not_hex.as_str(),
        ];
        for sha in samples {
            assert_eq!(format_cli_version("1.0.0", Some(sha), true), "1.0.0");
            assert_eq!(format_cli_version("1.0.0", Some(sha), false), "1.0.0");
        }
        assert_eq!(format_cli_version("1.0.0", None, true), "1.0.0");
        assert_eq!(format_cli_version("1.0.0", None, false), "1.0.0");
    }

    #[test]
    fn release_version_omits_the_sha() {
        let sha1 = "ab".repeat(20);
        let sha256 = "cd".repeat(32);
        assert_eq!(format_cli_version("1.0.0", Some(&sha1), false), "1.0.0");
        assert_eq!(format_cli_version("1.0.0", Some(&sha256), false), "1.0.0");
        assert_eq!(env!("CARGO_PKG_VERSION"), "1.0.0");
    }

    #[test]
    fn extra_follows_quiet_verbose_and_the_terminal() {
        assert!(!show_extra(true, false, true));
        assert!(show_extra(false, true, false));
        assert!(!show_extra(false, false, false));
        assert!(show_extra(false, false, true));
        assert!(!show_extra(true, true, true));
    }

    #[test]
    fn next_step_is_the_clause_after_the_first_separator() {
        assert_eq!(
            next_step("ontbreekt. Geef een pad op"),
            Some("Geef een pad op")
        );
        assert_eq!(
            next_step("lezen mislukt; controleer het bestand"),
            Some("controleer het bestand")
        );
        assert_eq!(next_step("geen scheiding"), None);
    }

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
            redact: true,
            verbose: false,
            extra: false,
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
            redact: true,
            verbose: false,
            extra: false,
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
            redact: true,
            verbose: false,
            extra: false,
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
