# symlog

[![crates.io](https://img.shields.io/crates/v/symlog.svg)](https://crates.io/crates/symlog)

A terminal log viewer for Symfony/Monolog logs, with vim-like keys. `symlog` reads
a log file only (read-only), runs entirely locally, and makes no network requests.

> **Status:** 0.1.1 is published on [crates.io](https://crates.io/crates/symlog) and
> tagged as v0.1.1. This README describes exactly what the current binary does; see
> [Open items](#open-items) for what is deliberately still missing.

Nederlandse versie: [README.nl.md](README.nl.md). The help text and a few lines the binary
prints are Dutch (`wacht op <pad>...`, `— geroteerd —`, `— afgekapt —`).

## What it does

- Reads Monolog LineFormatter and JsonFormatter (LineFormatter for Monolog 1, 2 and
  3, JsonFormatter as NDJSON), including mixed in one file: the format is decided
  per record, not per file.
- Recognizes multiline stack traces (including across several `[previous exception]`
  blocks) and JSON exceptions, and shows the frames (file, line, function) structured
  under the message.
- Shows broken lines, invalid JSON and invalid UTF-8 as readable "raw" records
  instead of crashing; ANSI escapes and NUL bytes in the log text are neutralized
  before display.
- Filters by channel (`--channel`, repeatable; `!channel` excludes) and by level
  (`--level error,critical` or `--level error,critical`); in the TUI also interactively
  with the keys `c` and `l`.
- In the TUI: `/` searches message and context with a regular expression and jumps
  to the next or previous match (`n`/`N`).
- Renders wide characters (CJK, emoji, including ZWJ sequences) and combining marks
  at the correct column width, and never truncates in the middle of a character.
- Follows a file live (`--follow`, like `tail -f`) in **plain output**: growth,
  rename rotation (for example `logrotate`) and truncation (`copytruncate`) are
  detected, with a separator line (`— geroteerd —` / `— afgekapt —`) in the output.
  See [Open items](#open-items) for the limit of this in the TUI.
- Works without a terminal (pipe, redirect, CI) as plain text: one line per record,
  frames indented, color only on a real tty and when `NO_COLOR` is unset.
- Is demonstrably read-only: a test run over all fixtures with `chmod a-w` leaves
  mtimes and hashes unchanged, and a `strace` test on Linux CI fails if the process
  ever opens a log file with write permission or opens a network socket. That test
  was tried once by hand with a deliberately broken variant of `Reader::open`
  (`OpenOptions::new().read(true).write(true).open(...)` instead of `File::open`):
  check 1 then failed on the hash comparison. The open itself writes nothing, but it
  shows that the test would notice a writable handle as soon as something is written.
  That variant is not in the source; to repeat the check, open the file in
  `Reader::open` that way temporarily.

## Installation

```sh
cargo install symlog
cargo install --git https://github.com/qwpp7g79y4-maker/symlog
```

Or from a local checkout: `cargo install --path .`

## Aan de slag in 5 minuten

Run these from a checkout of this repository (they use the fixtures in `tests/fixtures`;
no system logs, no network, no TUI). `tests/quickstart_readme.rs` runs every `symlog`
line below literally.

```sh
symlog tests/fixtures/line-basic.log --plain --no-color
symlog tests/fixtures/line-basic.log --plain --no-color --level error
symlog tests/fixtures/line-basic.log --plain --no-color --level error,critical
symlog tests/fixtures/line-basic.log --plain --no-color --channel request
symlog tests/fixtures/json-basic.ndjson --plain --no-color
```

The first line prints every record, one per line. The next ones keep only a level, a
level range or a channel. The last one reads a JsonFormatter (NDJSON) file the same way.

## Usage

```
symlog [PATH] [-f|--follow] [--level LEVEL] [--channel CHANNELS]... [--plain] [--no-color] [--no-redact] [--tui] [--output PATH] [--quiet | --verbose]
symlog --completions <bash|zsh>
```

`<PATH>` is required; there is no automatic `var/log/...` default and no stdin
support (`-`) in this version.

## Example

Real output on `tests/fixtures/line-basic.log` (plain mode, for example via a pipe
or `--plain`):

```
$ symlog tests/fixtures/line-basic.log --plain --no-color
[2026-01-15T10:30:45.123456+01:00] app.DEBUG: User cache warmed
[2026-01-15T10:30:46.000000+01:00] request.INFO: Incoming GET /health
[2026-01-15T10:30:47.500000+01:00] doctrine.NOTICE: Slow query detected
[2026-01-15T10:30:48.250000+01:00] security.WARNING: Failed login attempt
[2026-01-15T10:30:49.000000+01:00] php.ERROR: Undefined array key "foo"
[2026-01-15T10:30:50.750000+01:00] console.CRITICAL: Command failed
[2026-01-15T10:30:51.333333+01:00] app.ALERT: Disk usage above threshold
[2026-01-15T10:30:52.999999+01:00] request.EMERGENCY: Service unreachable
```

The same fixture in the interactive TUI (80×24, no filter):

```
 symlog — 1/8 records
10:30:45 DEBUG     app      User cache warmed
10:30:46 INFO      request  Incoming GET /health
10:30:47 NOTICE    doctrine Slow query detected
10:30:48 WARNING   security Failed login attempt
10:30:49 ERROR     php      Undefined array key "foo"
10:30:50 CRITICAL  console  Command failed
10:30:51 ALERT     app      Disk usage above threshold
10:30:52 EMERGENCY request  Service unreachable

j/k gg/G Ctrl-d/u / n/N c l F o y ? q
```

`examples/` holds two realistic inputs (`symfony-prod.log`, `api-json.ndjson`) with the
expected `--plain --no-color` output next to them (`*.expected`); `tests/examples_golden.rs`
compares them.

## Keys (TUI)

| Key | Does |
|---|---|
| `j` / `k` / `↓` / `↑` | move the selection one row down/up |
| `gg` | go to the first row |
| `G` | go to the last row |
| `Ctrl-d` / `Ctrl-u` | half a page down/up |
| `/` | open the search prompt (regular expression); `Enter` confirms, `Esc` cancels, `Backspace` deletes one character |
| `n` / `N` | go to the next/previous search match |
| `c` | channel picker (multi-select; `j`/`k`/`↓`/`↑` moves, `Space` toggles, `Enter` confirms, `Esc` cancels) |
| `l` | level picker (same controls as `c`) |
| `F` | toggle the follow indicator (see [Open items](#open-items)) |
| `o` | shows file:line of the selected frame in the status bar |
| `y` | marks file:line of the selected frame to print to stdout on exit |
| `Enter` / `Tab` | open/close the detail pane (context, extra, frames) |
| `?` | help overlay (any key closes it again) |
| `q` / `Ctrl-c` | quit |

There is no horizontal scrolling. In normal mode `h` does nothing; `l` opens the
level picker. Text wider than the window is truncated.

## Options

Exact list from `symlog --help` (see `tests/golden/help.txt`):

| Flag | Does |
|---|---|
| `[PATH]` | path to the log file (required, except with `--completions`) |
| `-f`, `--follow` | without a tty: if the path is missing, `wacht op <pad>...` is printed on stderr and the process keeps waiting (not exit 1). When the file appears, records go to stdout and the process keeps running. An empty file produces no output and does not stop; a line that arrives later still appears |
| `--level <LEVEL>` | level filter: a list (`error,critical`) or a lower bound (`>=warning`) |
| `--channel <CHANNELS>` | channel filter, repeatable; `!channel` excludes that channel |
| `--plain` | force plain output, even on a tty |
| `--no-color` | no color, even on a tty |
| `--no-redact` | show secrets unmasked; by default they become `***` |
| `--tui` | force the TUI; without a tty on stdout this exits 3 |
| `--output <PATH>` | write the plain output to this file instead of stdout. An existing file is replaced. A missing parent directory is an error and is not created. Not together with `--tui` or `--completions` |
| `--quiet` | hide the hint line, even on a terminal. No context line. A pipe already omits both |
| `--verbose` | print the hint line, and when output was written one `context:` line on stderr, even on a pipe. Not with `--quiet` |
| `--completions <SHELL>` | print a shell completion script (`bash` or `zsh`) and exit 0; `<PATH>` is then not needed |
| `-h`, `--help` | help |
| `-V`, `--version` | version |

## Secrets

By default, in plain output and in the TUI (list, search results and detail), symlog
replaces these values with `***`:

- the value after `password`, `passwd`, `token`, `secret`, `api_key` and `apikey`
  (`=` or `:`, case-insensitive, including as a JSON field);
- the token after `Authorization: Bearer`, including as a JSON field
  (`"Authorization": "Bearer …"`);
- the password in URL userinfo after `://` (`user:password@host`). If the username
  contains a high-risk token (`ghp_`, `github_pat_`, `xoxb-`, `sk_live_` /
  `sk_test_`, `AKIA`, or a JWT), including after a prefix (`x-access-token-ghp_…`,
  `svc-AKIA…`, `bearer-<jwt>`), both username and password become `***`
  (`scheme://***:***@host`). An ordinary name is left as-is.
  `scheme://<token>@host` (no password) is covered by the token itself;
- a key `sk_live_` or `sk_test_` followed by at least one letter or digit;
- an AWS access key `AKIA` plus exactly 16 uppercase letters or digits;
- a GitHub token `ghp_` plus at least 36 letters or digits;
- a fine-grained GitHub token `github_pat_` plus at least 82 letters, digits or `_`;
- a Slack bot token `xoxb-` with a body of at least 10 characters (`[A-Za-z0-9-]`);
- a JWT: three base64url segments of at least 16 characters, separated by a single
  dot, the first of which starts with `eyJ`.

`--no-redact` shows the original text. The log file on disk stays unchanged.

## Exit codes

0 is success, 1 is a missing path (the only finding symlog reports; it has no threshold), 2 is invalid input. Codes 3 and 4 are extra and unchanged. SIGINT exits 130: stderr is `onderbroken door SIGINT`, and a one-shot `--output` file is not replaced by a half-written report. `--version` and `-V` print `symlog 1.0.0` and exit 0. A debug build also prints the git sha when the build got it from git itself. A release build does not print that sha. On a terminal, an error also prints a `hint:` line with the next step already in that error. A pipe, a redirect, or `--quiet` keeps the error line only. `--verbose` prints the hint even on a pipe, and when output was written one `context:` line on stderr (`records`). Plain output on stdout is unchanged. `--quiet` together with `--verbose` is exit 2.

| Code | Meaning |
|---|---|
| 0 | exited normally. An empty file without `--follow` exits 0 and prints nothing. With `--follow`, an empty file does not exit by itself |
| 1 | the path is missing and `--follow` is off: stderr says the path does not exist, code 1. With `--follow` the process waits and the code is not 1 |
| 2 | usage error: missing required argument, invalid `--level`, the path is a directory, a regular file over 268435456 bytes (256 MiB), or `--quiet` together with `--verbose`. A file over that limit is not read |
| 3 | `--tui` requested without a tty on stdout |
| 4 | I/O error while reading that cannot be recovered (including: no read permission), or a failed write of `--output`. A missing parent directory is not created |
| 130 | SIGINT. No half-written one-shot report. An existing `--output` file stays when the new report was not finished. Stderr is `onderbroken door SIGINT`. Ordinary codes 0, 1, 2, 3 and 4 are unchanged |

## What it does not do

- No log collection, no server, no network; only a local file at PATH.
- No stdin input (`-`) in this version.
- Never writes, rotates or truncates the log file itself. `--output PATH` writes the plain output to that other path. A one-shot run replaces an existing file there only after the new report is complete. SIGINT exits 130 and leaves the previous file in place. A missing parent directory is not created.
- No custom `LineFormatter` format strings; such lines are shown as raw lines.
- No pretty-printed (multi-line) JSON records.
- No gzip archives (`*.log.gz`).
- No reordering by time; the order is the file order.
- Does not jump to an editor and does not convert time zones (see
  [Open items](#open-items)).
- Does not open containers itself; pass the path of the mounted volume.

## Alternatives

- **lnav** — powerful general log viewer with SQL; Monolog format via its own format
  definition *(not verified by this project)*.
- **Symfony Profiler / web debug toolbar** — per request in the browser, not for
  files after the fact.
- **`tail -f` + `grep` / `jq`** — always available, no stack-trace frames.
- **`less +F`** — follow and search, no parsing.
- Other TUI log viewers (for example tailspin, hl) *(not verified for Monolog)*.

## Open items

The following pieces are built as separate, independently tested modules, but are
**not yet wired** into the CLI or the interactive TUI loop of this binary:

- **Editor jump**: one key (`o`/`Enter`) to open `file:line` in Neovim, another
  editor (`$VISUAL`/`$EDITOR`) or `vi`. The module exists and is tested on its own
  (argv tests with a fake editor), but the TUI does not call it yet: `o` only shows
  `file:line` in the status bar.
- **Path mapping** (container path to a local path, for example `/var/www` → a local
  project directory): the mapping logic exists and is tested on its own, but there
  is no `--map` flag or config to fill it.
- **Time-zone conversion**: the display logic for "as in the log" / local / an IANA
  name exists, but there is no `--tz` flag; the TUI and plain output always show the
  datetime text as it stood in the log.
- **Config file** (`$XDG_CONFIG_HOME/symlog/config.toml`): not built yet.
- **`--follow` in the TUI**: `F` only toggles a status indicator; the interactive TUI
  loop waits for keystrokes and does not re-read the file while it is open. Live
  follow works fully in plain output (`--follow` without `--tui`, or without a tty).
- No `--format`/`--grep`/`--server`/`--editor` flags: they do not exist in this
  binary.

This is documented on purpose instead of left unsaid; the claims above name only
behavior that is actually wired up.

## Claims and the tests that cover them

| Claim above | Fixture(s) | Test(s) |
|---|---|---|
| LineFormatter (Monolog 1–3) and JsonFormatter, including mixed | F1, F2, F5, F6, F11 | `tests/line_parser_golden.rs`, `tests/json_golden.rs` |
| Multiline stack traces and JSON exceptions as frames | F3, F4, F12 | `tests/multiline_golden.rs` |
| Broken lines, invalid JSON/UTF-8, ANSI/NUL neutralization without a crash | F7, F8, F15 | `tests/raw_golden.rs`, `tests/raw_fuzz.rs` |
| Channel and level filters (CLI and TUI pickers) | F1 | `src/filter.rs` (unit tests), `src/main.rs::tests::level_and_channel_filters_apply_to_the_plain_output`, `src/tui/keys.rs::tests::channel_picker_keys_move_toggle_and_commit` |
| TUI regex search (`/ n N`) | — | `src/tui/keys.rs::tests::full_keystroke_scenario_from_task_md` |
| Wide characters (CJK/emoji/ZWJ/combining) at the correct column width | F10 | `src/tui/view.rs::tests::snapshot_f10_wide_80x24` (and 40×10, 200×50), `src/tui/width.rs` (unit tests) |
| Live follow in plain output: growth, rename rotation, copytruncate | F13 (generated) | `tests/follow_rotation.rs`, `tests/plain_cli.rs::plain_follow_streams_new_lines_as_the_file_grows` |
| Plain output without a tty, filters applied | F1 | `tests/plain_cli.rs::f1_plain_output_matches_golden`, `::level_and_channel_filtered_plain_output_matches_golden` |
| Empty file: exit 0; missing file: exit 1; directory: exit 2; `--tui` without a tty: exit 3; no read permission: exit 4 | F14 | `tests/startup_cli.rs`, `tests/plain_cli.rs::tui_flag_without_a_tty_exits_three` |
| A regular file over 268435456 bytes exits 2 and is not read. The measured 100 MiB input stays under this limit | sparse file in `/tmp`, not committed | `tests/input_limit.rs` |
| SIGINT exits 130 and does not replace an existing `--output` file with a partial report | linux x86_64 | `tests/sigint.rs` |
| Read-only: no write or network calls | all fixtures | `tests/read_only_proof.rs` |
| `--help` text | — | `tests/help_golden.rs` |
| `--version` and `-V` print `symlog 1.0.0` and exit 0. A debug build also prints the git sha when git itself provided it. A release build does not | — | `tests/version_cli.rs`, `src/main.rs::tests::release_version_omits_the_sha` |
| Secrets in plain output and the TUI become `***`; `--no-redact` shows the original | — | `tests/redact.rs` |

## Development

The suite (`cargo test --no-fail-fast`) has 462 passing tests, 0 failing and 1 ignored,
measured on this version change (parent `ae22844`) on linux x86_64. `tests/sigint.rs` is
built only on that target.
See `QUALITY.md` for the quality gates.

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo deny check
cargo package --allow-dirty
```

The `bench-mem` CI job runs the performance measurement from `docs/perf.md`; see
that file for the current numbers against the targets in `blueprint.md`.

## License

`MIT OR Apache-2.0`, see `LICENSE-MIT` and `LICENSE-APACHE`.
