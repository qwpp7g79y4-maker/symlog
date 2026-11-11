# symlog

Nederlandse versie van [README.md](README.md) (Engels, volledig).

Een terminal-logviewer voor Symfony/Monolog-logs, met vim-toetsen. `symlog` leest een
logbestand alleen (read-only), draait volledig lokaal en doet geen netwerkverzoeken.

[![crates.io](https://img.shields.io/crates/v/symlog.svg)](https://crates.io/crates/symlog) [![downloads](https://img.shields.io/crates/d/symlog.svg)](https://crates.io/crates/symlog) [![CI](https://github.com/qwpp7g79y4-maker/symlog/actions/workflows/ci.yml/badge.svg)](https://github.com/qwpp7g79y4-maker/symlog/actions/workflows/ci.yml) [![PEPAI gecontroleerd](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Fqwpp7g79y4-maker%2Fsymlog%2Fmain%2F.github%2Fbadge.json)](QUALITY.md) [![MSRV 1.88](https://img.shields.io/badge/MSRV-1.88-blue.svg)](https://github.com/qwpp7g79y4-maker/symlog/blob/main/Cargo.toml) [![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#licentie)

## Installatie

Versie 1.0.0 staat op [crates.io](https://crates.io/crates/symlog). Met Rust 1.88 of nieuwer:

```sh
cargo install symlog
symlog --help
```

`cargo install --locked symlog` zet de lockfile vast.

Voor de laatste `main`: `cargo install --locked --git https://github.com/qwpp7g79y4-maker/symlog`.

Of vanuit een lokale checkout: `cargo install --path .`

## Aan de slag in 5 minuten

Voer dit uit vanuit een checkout van deze repository (met de fixtures in
`tests/fixtures`; geen systeemlogs, geen netwerk, geen TUI). `tests/quickstart_readme.rs`
draait elke `symlog`-regel hieronder letterlijk.

```sh
symlog tests/fixtures/line-basic.log --plain --no-color
symlog tests/fixtures/line-basic.log --plain --no-color --level error
symlog tests/fixtures/line-basic.log --plain --no-color --level error,critical
symlog tests/fixtures/line-basic.log --plain --no-color --channel request
symlog tests/fixtures/json-basic.ndjson --plain --no-color
```

De eerste regel toont alle records, één per regel. De volgende houden alleen een level,
een lijst levels of een kanaal over. De laatste leest een JsonFormatter-bestand (NDJSON)
op dezelfde manier.

## Gebruik

```
symlog [PATH] [-f|--follow] [--level LEVEL] [--channel CHANNELS]... [--plain] [--no-color] [--no-redact] [--tui] [--output PATH] [--quiet | --verbose]
symlog --completions <bash|zsh>
```

`<PATH>` is verplicht; er is geen automatisch `var/log/...`-pad. `-` leest stdin in één doorgang en schrijft elk record plain uit zodra het compleet is. `--follow` of `--tui` samen met `-` is een gebruiksfout (exit 2).

## Voorbeeld

Echte uitvoer op `tests/fixtures/line-basic.log` (plain-modus, bijvoorbeeld via een pipe
of `--plain`):

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

In `examples/` staan twee realistische invoerbestanden (`symfony-prod.log`,
`api-json.ndjson`) met de verwachte `--plain --no-color`-uitvoer ernaast (`*.expected`);
`tests/examples_golden.rs` vergelijkt ze.

## Opties

Exacte lijst uit `symlog --help` (zie `tests/golden/help.txt`):

| Optie | Doet |
|---|---|
| `[PATH]` | pad naar het logbestand (verplicht, behalve met `--completions`) |
| `-f`, `--follow` | zonder tty: ontbreekt het pad, dan komt `wacht op <pad>...` op stderr en blijft het proces wachten (geen exit 1). Verschijnt het bestand, dan gaan records naar stdout en blijft het proces draaien. Een leeg bestand geeft geen uitvoer en stopt niet; een later binnenkomende regel verschijnt alsnog |
| `--level <LEVEL>` | niveaufilter: een lijst (`error,critical`) of een ondergrens (`>=warning`) |
| `--channel <CHANNELS>` | kanaalfilter, herhaalbaar; `!kanaal` sluit dat kanaal uit |
| `--plain` | forceer plain-uitvoer, ook op een tty |
| `--no-color` | geen kleur, ook op een tty |
| `--no-redact` | toon geheimen ongemaskeerd; standaard worden ze `***` |
| `--tui` | forceer de TUI; zonder tty op stdout geeft dit exit 3 |
| `--output <PATH>` | schrijf de plain-uitvoer naar dit bestand in plaats van stdout. Een bestaand bestand wordt vervangen. Een ontbrekende bovenliggende map is een fout en wordt niet aangemaakt. Niet samen met `--tui` of `--completions` |
| `--quiet` | verberg de hintregel, ook op een terminal. Geen contextregel. Een pipe laat beide al weg |
| `--verbose` | toon de hintregel, en als de uitvoer geschreven is één `context:`-regel op stderr, ook bij een pipe. Niet samen met `--quiet` |
| `--completions <SHELL>` | print een shell-completionscript (`bash` of `zsh`) en eindig met exit 0; `<PATH>` is dan niet nodig |
| `-h`, `--help` | help |
| `-V`, `--version` | versie |

## Exitcodes

0 is geslaagd, 1 is een ontbrekend pad (de enige bevinding die symlog meldt; er is geen drempel), 2 is ongeldige invoer. Code 3 en 4 zijn extra en ongewijzigd. SIGINT eindigt met exit 130: stderr is `onderbroken door SIGINT`, en een eenmalig `--output`-bestand wordt niet vervangen door een half rapport. `--version` en `-V` tonen `symlog 1.0.0` en geven exit 0. Een debug-build toont daarachter de git-sha, alleen als git die bij de build zelf geeft. Een release-build toont die sha niet. Op een terminal print een fout ook een `hint:`-regel met de vervolgstap die al in die fout staat. Een pipe, een redirect of `--quiet` houdt alleen de foutregel. `--verbose` toont de hint ook bij een pipe, en als de uitvoer geschreven is één `context:`-regel op stderr (`records`). De plain-uitvoer op stdout blijft gelijk. `--quiet` samen met `--verbose` is exit 2.

| Code | Betekenis |
|---|---|
| 0 | normaal afgesloten (een leeg bestand zonder `--follow` geeft niets en exit 0) |
| 1 | pad bestaat niet en `--follow` staat uit |
| 2 | gebruiksfout: ontbrekend argument, ongeldige `--level`, het pad is een map, een gewoon bestand boven 268435456 bytes (256 MiB), of `--quiet` samen met `--verbose`. Een bestand boven die grens wordt niet ingelezen |
| 3 | `--tui` gevraagd zonder tty op stdout |
| 4 | onherstelbare I/O-fout bij het lezen (ook: geen leesrechten), of een mislukte schrijfactie van `--output`. Een ontbrekende bovenliggende map wordt niet aangemaakt |
| 130 | SIGINT. Geen half eenmalig rapport. Een bestaand `--output`-bestand blijft staan als het nieuwe rapport niet af was. Stderr is `onderbroken door SIGINT`. Gewone codes 0, 1, 2, 3 en 4 blijven |

## Kwaliteit

462 geslaagde tests, 0 gefaald, 1 genegeerd (`cargo test --no-fail-fast`), gemeten op
deze versiewijziging (ouder `ae22844`) op linux x86_64. `tests/sigint.rs` wordt alleen
op dat doel gebouwd. Zie `QUALITY.md`.

## Licentie

`MIT OR Apache-2.0`, zie `LICENSE-MIT` en `LICENSE-APACHE`.
