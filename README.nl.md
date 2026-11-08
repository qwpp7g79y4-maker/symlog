# symlog

Nederlandse versie van [README.md](README.md) (Engels, volledig).

Een terminal-logviewer voor Symfony/Monolog-logs, met vim-toetsen. `symlog` leest een
logbestand alleen (read-only), draait volledig lokaal en doet geen netwerkverzoeken.

## Installatie

```sh
cargo install symlog
cargo install --git https://github.com/qwpp7g79y4-maker/symlog
```

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
symlog <PATH> [-f|--follow] [--level LEVEL] [--channel CHANNELS]... [--plain] [--no-color] [--no-redact] [--tui]
```

`<PATH>` is verplicht; er is geen automatisch `var/log/...`-pad en geen stdin (`-`).

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
| `<PATH>` | pad naar het logbestand (verplicht) |
| `-f`, `--follow` | zonder tty: ontbreekt het pad, dan komt `wacht op <pad>...` op stderr en blijft het proces wachten (geen exit 1). Verschijnt het bestand, dan gaan records naar stdout en blijft het proces draaien. Een leeg bestand geeft geen uitvoer en stopt niet; een later binnenkomende regel verschijnt alsnog |
| `--level <LEVEL>` | niveaufilter: een lijst (`error,critical`) of een ondergrens (`>=warning`) |
| `--channel <CHANNELS>` | kanaalfilter, herhaalbaar; `!kanaal` sluit dat kanaal uit |
| `--plain` | forceer plain-uitvoer, ook op een tty |
| `--no-color` | geen kleur, ook op een tty |
| `--no-redact` | toon geheimen ongemaskeerd; standaard worden ze `***` |
| `--tui` | forceer de TUI; zonder tty op stdout geeft dit exit 3 |
| `-h`, `--help` | help |
| `-V`, `--version` | versie |

## Exitcodes

0 is geslaagd, 1 is een ontbrekend pad (de enige bevinding die symlog meldt; er is geen drempel), 2 is ongeldige invoer. Code 3 en 4 zijn extra en ongewijzigd. `--version` en `-V` geven exit 0.

| Code | Betekenis |
|---|---|
| 0 | normaal afgesloten (een leeg bestand zonder `--follow` geeft niets en exit 0) |
| 1 | pad bestaat niet en `--follow` staat uit |
| 2 | gebruiksfout: ontbrekend argument, ongeldige `--level`, of het pad is een map |
| 3 | `--tui` gevraagd zonder tty op stdout |
| 4 | onherstelbare I/O-fout bij het lezen (ook: geen leesrechten) |

## Kwaliteit

437 geslaagde tests, 0 gefaald, 1 genegeerd (`cargo test --no-fail-fast`), gemeten op
commit `f5ab1a4` plus de quickstart-tests. Zie `QUALITY.md`.

## Licentie

`MIT OR Apache-2.0`, zie `LICENSE-MIT` en `LICENSE-APACHE`.
