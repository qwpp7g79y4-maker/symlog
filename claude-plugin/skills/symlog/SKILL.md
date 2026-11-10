---
name: symlog
description: Read or follow Symfony or Monolog log files in the terminal, with secrets masked. Use when the user mentions a Symfony or Monolog log, var/log, following a log in the terminal, or symlog.
---

# symlog

Use the local `symlog` CLI (version 0.1.1). It reads and follows Symfony or Monolog logs in the terminal, and masks secrets in the displayed output. It does not modify the file on disk.

## Install

If `symlog` is not on `PATH`:

```sh
cargo install symlog
```

From a git checkout you can also use `cargo install --git https://github.com/qwpp7g79y4-maker/symlog` or `cargo install --path .`. Building from source needs Rust 1.88 or newer.

Do not invent flags. If you are unsure, run `symlog --help`. The usage text from `--help` (see also `tests/golden/help.txt`) is:

```
Usage: symlog [OPTIONS] <PATH>

Arguments:
  <PATH>  Pad naar het logbestand

Options:
  -f, --follow              Blijf het bestand volgen (`tail -f`), ook als het nog niet bestaat
      --level <LEVEL>       Niveaufilter: een lijst (`error,critical`) of een ondergrens (`>=warning`)
      --channel <CHANNELS>  Kanaalfilter (herhaalbaar); `!kanaal` sluit dat kanaal juist uit
      --plain               Forceer plain-uitvoer, ook op een tty
      --no-color            Geen kleur, ook op een tty
      --no-redact           Toon geheimen ongemaskeerd. Standaard worden ze vervangen door `***`
      --tui                 Forceer de TUI; zonder tty op stdout geeft dit exit 3
  -h, --help                Print help
  -V, --version             Print version
```

`<PATH>` is required. There is no default `var/log/...` path and no stdin support (`-`) in this version.

When Claude should read the log as text, always pass `--plain` (and `--no-color` if you want plain text without ANSI codes). Do not pass `--tui` unless the user is at an interactive terminal and asked for the TUI. `--tui` without a tty on stdout exits 3.

## Examples from the project README

Plain output on a fixture (needs a clone):

```sh
symlog tests/fixtures/line-basic.log --plain --no-color
```

Filter examples from `--help`:

```sh
symlog /path/to/app.log --plain --level error,critical
symlog /path/to/app.log --plain --level '>=warning'
symlog /path/to/app.log --plain --channel request --channel '!doctrine'
symlog -f /path/to/app.log --plain
```

## Secret redaction

By default, displayed values are replaced with `***` for:

- the value after `password`, `passwd`, `token`, `secret`, `api_key`, and `apikey`
- the token after `Authorization: Bearer`
- passwords in URL userinfo, and several well-known token patterns (`sk_live_` / `sk_test_`, `AKIA`, `ghp_`, `github_pat_`, `xoxb-`, JWTs)

`--no-redact` shows the original text. Do not use `--no-redact` unless the user explicitly asks to see unmasked secrets. Never copy unmasked secrets into the conversation.

## Exit codes

- 0 — normal exit (an empty file without `--follow` exits 0 and prints nothing)
- 1 — path does not exist and `--follow` is off
- 2 — usage error: missing required argument, invalid `--level`, or the path is a directory
- 3 — `--tui` requested without a tty on stdout
- 4 — unrecoverable I/O error while reading, including no read permission

With `--follow`, a missing path does not exit 1: the process waits.
