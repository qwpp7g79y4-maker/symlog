# symlog

[![crates.io](https://img.shields.io/crates/v/symlog.svg)](https://crates.io/crates/symlog)

Een terminal-logviewer voor Symfony/Monolog-logs, met vim-achtige toetsen. `symlog`
leest een logbestand alleen (read-only), werkt volledig lokaal en doet geen
netwerkverkeer.

> **Status:** versie 0.1.0, gepubliceerd op [crates.io](https://crates.io/crates/symlog) en getagd als v0.1.0. Deze
> README beschrijft precies wat de huidige binary doet; zie
> ["Open punten"](#open-punten) voor wat er bewust nog ontbreekt.

## Wat het doet

- Leest het Monolog LineFormatter- en JsonFormatter-formaat (LineFormatter voor
  Monolog 1, 2 en 3, JsonFormatter als NDJSON), ook door elkaar in één bestand: het
  formaat wordt per record bepaald, niet per bestand.
- Herkent multiline-stacktraces (ook over meerdere `[previous exception]`-blokken) en
  JSON-exceptions, en toont de frames (bestand, regel, functie) gestructureerd onder
  het bericht.
- Toont kapotte regels, ongeldige JSON en ongeldige UTF-8 als leesbare "raw"-records
  in plaats van te crashen; ANSI-escapes en NUL-bytes in de logtekst worden
  geneutraliseerd voor weergave.
- Filtert op kanaal (`--channel`, herhaalbaar, `!kanaal` sluit uit) en op niveau
  (`--level error,critical` of `--level >=warning`); in de TUI ook interactief via de
  toetsen `c` en `l`.
- In de TUI: `/` zoekt met een reguliere expressie in bericht en context en springt
  naar de volgende/vorige match (`n`/`N`).
- Toont brede tekens (CJK, emoji, inclusief ZWJ-reeksen) en combinerende accenten met
  de juiste kolombreedte, en kapt nooit midden in een teken af.
- Volgt een bestand live (`--follow`, zoals `tail -f`) in **plain-uitvoer**: groei,
  rename-rotatie (bijvoorbeeld `logrotate`) en afkappen (`copytruncate`) worden
  gedetecteerd, met een scheidingsregel (`— geroteerd —` / `— afgekapt —`) in de
  uitvoer. Zie ["Open punten"](#open-punten) voor de beperking hiervan in de TUI.
- Werkt zonder terminal (pipe, redirect, CI) als plain-tekstuitvoer: één regel per
  record, frames ingesprongen, kleur alleen op een echte tty en zonder `NO_COLOR`.
- Is aantoonbaar read-only: een testrun op alle fixtures met `chmod a-w` laat mtimes en
  hashes ongewijzigd, en een `strace`-test op Linux CI faalt als het proces ooit een
  logbestand met schrijfrechten of een netwerk-socket opent. Die test is één keer
  handmatig uitgeprobeerd met een bewust kapotte variant van `Reader::open`
  (`OpenOptions::new().read(true).write(true).open(...)` in plaats van `File::open`):
  check 1 faalde toen op de hash-vergelijking. De open zelf schrijft niets, maar laat
  zien dat de test een schrijfbare handle wél zou opmerken zodra er geschreven wordt.
  Die variant staat niet in de broncode; om de proef te herhalen, open dan het bestand in
  `Reader::open` tijdelijk op die manier.

## Installatie

```sh
cargo install symlog
cargo install --git https://github.com/qwpp7g79y4-maker/symlog
```

Of vanuit een lokale checkout: `cargo install --path .`

## Gebruik

```
symlog <PAD> [-f|--follow] [--level LEVEL] [--channel KANAAL]... [--plain] [--no-color] [--no-redact] [--tui]
```

`<PAD>` is verplicht; er is geen automatische `var/log/...`-standaardwaarde en geen
stdin-steun (`-`) in deze versie.

### Voorbeeld

Echte uitvoer op `tests/fixtures/line-basic.log` (plain-modus, bijvoorbeeld via een
pipe of `--plain`):

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

Dezelfde fixture in de interactieve TUI (80×24, geen filter):

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

## Toetsen (TUI)

| Toets | Doet |
|---|---|
| `j` / `k` / `↓` / `↑` | selectie één regel omlaag/omhoog |
| `gg` | naar de eerste regel |
| `G` | naar de laatste regel |
| `Ctrl-d` / `Ctrl-u` | een halve pagina omlaag/omhoog |
| `/` | zoekprompt openen (reguliere expressie); `Enter` bevestigt, `Esc` annuleert, `Backspace` wist een teken |
| `n` / `N` | naar de volgende/vorige zoekmatch |
| `c` | kanaalkiezer (multi-select; `j`/`k`/`↓`/`↑` verplaatst, `Spatie` toggelt, `Enter` bevestigt, `Esc` annuleert) |
| `l` | niveaukiezer (zelfde bediening als `c`) |
| `F` | follow-indicator aan/uit (zie ["Open punten"](#open-punten)) |
| `o` | toont bestand:regel van het geselecteerde frame in de statusbalk |
| `y` | markeert bestand:regel van het geselecteerde frame om bij afsluiten naar stdout te printen |
| `Enter` / `Tab` | detailpaneel (context, extra, frames) open/dicht |
| `?` | help-overlay (elke toets sluit hem weer) |
| `q` / `Ctrl-c` | afsluiten |

Er wordt niet horizontaal gescrold. In de normale modus doet `h` niets; `l` opent de
niveaukiezer. Tekst die breder is dan het venster wordt afgekapt.

## Vlaggen

Exacte lijst uit `symlog --help` (zie `tests/golden/help.txt`):

| Vlag | Doet |
|---|---|
| `<PATH>` | pad naar het logbestand (verplicht) |
| `-f`, `--follow` | zonder tty: ontbreekt het pad, dan `wacht op <pad>...` op stderr en het proces blijft wachten (geen exit 1). Verschijnt het bestand, dan komen de records op stdout en het proces blijft lopen. Een leeg bestand geeft geen uitvoer en stopt niet; een regel die er later bij komt, verschijnt alsnog |
| `--level <LEVEL>` | niveaufilter: een lijst (`error,critical`) of een ondergrens (`>=warning`) |
| `--channel <KANAAL>` | kanaalfilter, herhaalbaar; `!kanaal` sluit dat kanaal juist uit |
| `--plain` | forceer plain-uitvoer, ook op een tty |
| `--no-color` | geen kleur, ook op een tty |
| `--no-redact` | toon geheimen ongemaskeerd; standaard worden ze `***` |
| `--tui` | forceer de TUI; zonder tty op stdout geeft dit exit 3 |
| `-h`, `--help` | help |
| `-V`, `--version` | versie |

## Geheimen

Standaard vervangt symlog in de plain-uitvoer en in de TUI (lijst, zoekresultaten en
detail) deze waarden door `***`:

- de waarde na `password`, `passwd`, `token`, `secret`, `api_key` en `apikey`
  (`=` of `:`, hoofdletterongevoelig, ook als JSON-veld);
- het token na `Authorization: Bearer`, ook als JSON-veld
  (`"Authorization": "Bearer …"`);
- het wachtwoord in URL-userinfo na `://` (`user:wachtwoord@host`). Staat in de
  gebruikersnaam een hoog-risico token (`ghp_`, `github_pat_`, `xoxb-`,
  `sk_live_` / `sk_test_`, `AKIA` of een JWT), ook ná een voorvoegsel
  (`x-access-token-ghp_…`, `svc-AKIA…`, `bearer-<jwt>`), dan worden gebruikersnaam
  én wachtwoord `***` (`scheme://***:***@host`). Een gewone naam blijft staan.
  `scheme://<token>@host` (geen wachtwoord) valt onder het token zelf;
- een sleutel `sk_live_` of `sk_test_` gevolgd door minstens één letter of cijfer;
- een AWS-toegangssleutel `AKIA` plus precies 16 hoofdletters of cijfers;
- een GitHub-token `ghp_` plus minstens 36 letters of cijfers;
- een fijnmazig GitHub-token `github_pat_` plus minstens 82 letters, cijfers of `_`;
- een Slack-bottoken `xoxb-` met een lichaam van minstens 10 tekens (`[A-Za-z0-9-]`);
- een JWT: drie base64url-delen van minstens 16 tekens, gescheiden door één punt,
  waarvan het eerste met `eyJ` begint.

`--no-redact` toont de oorspronkelijke tekst. Het logbestand op schijf blijft
ongewijzigd.

## Exitcodes

| Code | Betekenis |
|---|---|
| 0 | normaal afgesloten. Een leeg bestand zonder `--follow` geeft 0 en geen uitvoer. Met `--follow` sluit een leeg bestand niet vanzelf af |
| 1 | pad ontbreekt en `--follow` staat uit: stderr meldt dat het pad niet bestaat, code 1. Met `--follow` wacht het proces en is de code niet 1 |
| 2 | gebruiksfout: ontbrekend verplicht argument, ongeldig `--level`, of pad is een map |
| 3 | `--tui` gevraagd zonder tty op stdout |
| 4 | I/O-fout tijdens lezen die niet herstelbaar is (inclusief: geen leesrechten) |

## Wat het NIET doet

- Geen logverzameling, geen server, geen netwerk; alleen een lokaal bestand op PAD.
- Geen stdin-invoer (`-`) in deze versie.
- Schrijft, roteert of kapt nooit zelf een logbestand.
- Geen aangepaste `LineFormatter`-formaatstrings; zulke regels worden als ruwe regels
  getoond.
- Geen pretty-printed (meerregelige) JSON-records.
- Geen gzip-archieven (`*.log.gz`).
- Geen hersortering op tijd; de volgorde is die van het bestand.
- Springt niet naar een editor en rekent geen tijdzones om (zie
  ["Open punten"](#open-punten)).
- Opent containers niet zelf; geef het pad van het gemounte volume.

## Alternatieven

- **lnav** — krachtige algemene logviewer met SQL; Monolog-formaat via een eigen
  formaatdefinitie *(niet geverifieerd door dit project)*.
- **Symfony Profiler / web debug toolbar** — per request in de browser, niet voor
  bestanden achteraf.
- **`tail -f` + `grep` / `jq`** — altijd beschikbaar, geen stacktrace-frames.
- **`less +F`** — volgen en zoeken, geen parsing.
- Andere TUI-logviewers (bijvoorbeeld tailspin, hl) *(niet geverifieerd voor
  Monolog)*.

## Open punten

De volgende onderdelen zijn als losse, zelfstandig geteste modules gebouwd, maar zijn
**nog niet aangesloten** op de CLI of de interactieve TUI-lus van deze binary:

- **Editor-sprong**: met één toets (`o`/`Enter`) naar `bestand:regel` in Neovim, een
  andere editor (`$VISUAL`/`$EDITOR`) of `vi` springen. De module hiervoor bestaat en
  is op zichzelf getest (argv-tests met een nep-editor), maar de TUI roept hem nog
  niet aan: `o` toont alleen `bestand:regel` in de statusbalk.
- **Padmapping** (containerpad naar lokaal pad, bijvoorbeeld `/var/www` → een lokale
  projectmap): de mappingslogica bestaat en is los getest, maar er is geen `--map`-
  vlag of config om hem te vullen.
- **Tijdzone-omrekening**: de weergavelogica voor "zoals in de log" / lokaal / een
  IANA-naam bestaat, maar er is geen `--tz`-vlag; de TUI en de plain-uitvoer tonen
  altijd de datumtijd-tekst zoals die in de log stond.
- **Config-bestand** (`$XDG_CONFIG_HOME/symlog/config.toml`): nog niet gebouwd.
- **`--follow` in de TUI**: `F` zet alleen een status-indicator aan; de interactieve
  TUI-lus wacht op toetsaanslagen en leest het bestand niet opnieuw terwijl hij open
  staat. Live volgen werkt wel volledig in plain-uitvoer (`--follow` zonder `--tui`,
  of zonder tty).
- Geen `--format`/`--grep`/`--server`/`--editor` vlaggen: die bestaan niet in deze
  binary.

Dit is bewust gedocumenteerd in plaats van verzwegen; de bovenstaande claims in dit
bestand noemen alleen gedrag dat echt aangesloten is.

## Claims en de tests die ze dekken

| Claim hierboven | Fixture(s) | Test(s) |
|---|---|---|
| LineFormatter (Monolog 1–3) en JsonFormatter, ook gemengd | F1, F2, F5, F6, F11 | `tests/line_parser_golden.rs`, `tests/json_golden.rs` |
| Multiline-stacktraces en JSON-exceptions als frames | F3, F4, F12 | `tests/multiline_golden.rs` |
| Kapotte regels, ongeldige JSON/UTF-8, ANSI/NUL-neutralisatie zonder crash | F7, F8, F15 | `tests/raw_golden.rs`, `tests/raw_fuzz.rs` |
| Kanaal- en niveaufilter (CLI en TUI-kiezers) | F1 | `src/filter.rs` (unit-tests), `src/main.rs::tests::level_and_channel_filters_apply_to_the_plain_output`, `src/tui/keys.rs::tests::channel_picker_keys_move_toggle_and_commit` |
| TUI-regexzoeken (`/ n N`) | — | `src/tui/keys.rs::tests::full_keystroke_scenario_from_task_md` |
| Brede tekens (CJK/emoji/ZWJ/combining) met juiste kolombreedte | F10 | `src/tui/view.rs::tests::snapshot_f10_wide_80x24` (en 40×10, 200×50), `src/tui/width.rs` (unit-tests) |
| Live volgen in plain-uitvoer: groei, rename-rotatie, copytruncate | F13 (gegenereerd) | `tests/follow_rotation.rs`, `tests/plain_cli.rs::plain_follow_streams_new_lines_as_the_file_grows` |
| Plain-uitvoer zonder tty, filters toegepast | F1 | `tests/plain_cli.rs::f1_plain_output_matches_golden`, `::level_and_channel_filtered_plain_output_matches_golden` |
| Leeg bestand: exit 0; ontbrekend bestand: exit 1; map: exit 2; `--tui` zonder tty: exit 3; geen leesrechten: exit 4 | F14 | `tests/startup_cli.rs`, `tests/plain_cli.rs::tui_flag_without_a_tty_exits_three` |
| Read-only: geen schrijf- of netwerkoproepen | alle fixtures | `tests/read_only_proof.rs` |
| `--help`-tekst | — | `tests/help_golden.rs` |
| Geheimen in plain-uitvoer en TUI worden `***`; `--no-redact` toont het origineel | — | `tests/redact.rs` |

## Ontwikkelen

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo deny check
cargo package --allow-dirty
```

De `bench-mem`-CI-job draait de prestatiemeting uit `docs/perf.md`; zie dat bestand
voor de huidige cijfers tegenover de doelen uit `blueprint.md`.

## Licentie

`MIT OR Apache-2.0`, zie `LICENSE-MIT` en `LICENSE-APACHE`.
