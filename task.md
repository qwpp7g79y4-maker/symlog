# symlog: bouwklare taakbrief (22 okt 2026)

> Formaat gelijk aan /workspace/concepten/cron-ledger-task.md: **Doet / Klaar als / Blijft** per taak T1..Tn. Architectuur: `blueprint.md` in deze map.
> Dit bestand wordt publiek. Er staan geen persoonsnamen of handles in, en dat moet zo blijven.


## Wat het is
Een standalone Rust-binary: een terminal-logviewer voor Symfony/Monolog-logs, met vim-toetsen. Hij leest de logs alleen (read-only), werkt lokaal en gebruikt geen netwerk.
- Navigatie: `j/k`, `gg/G`, `Ctrl-d/Ctrl-u`, `/zoek`, `n/N`, `q`.
- Filteren op kanaal (`c`) en op niveau (`l`). Niveaus zijn te combineren, bijvoorbeeld `error,critical` of `>=warning`.
- `Enter` of `o` op een stacktrace-frame opent bestand:regel in de eigen Neovim van de gebruiker. Ontbreekt die, dan volgt een terugval op `$EDITOR` en daarna op `vi`.
- Containerpaden worden omgezet naar lokale paden, bijvoorbeeld `/var/www` naar de projectmap.

Licentie: `MIT OR Apache-2.0`. Rust stable, edition 2021, MSRV wordt vastgelegd in T1.

## CLI-schets
```
symlog [PAD|-] [--follow|-f] [--format auto|line|json] [--channel C]... [--level L[,L]|>=L]
       [--grep REGEX] [--tz log|local|<IANA>] [--map /var/www=/home/u/proj]...
       [--server SOCK] [--editor CMD] [--plain] [--no-color] [--config PAD]
```
- Zonder PAD: `var/log/<APP_ENV|dev>.log` in de huidige map, als die bestaat; anders exit 2 met uitleg.
- `-` = stdin (alleen plain of TUI zonder follow-rotatie).
- Config: `$XDG_CONFIG_HOME/symlog/config.toml` (keys `map`, `editor`, `server`, `tz`). Flags gaan voor config.
- Exitcodes: **0** normaal afgesloten; **1** bestand ontbreekt (zonder `--follow`); **2** gebruiksfout (flags, config); **3** geen TTY en `--plain` niet mogelijk (zie T13); **4** I/O-fout tijdens lezen die niet herstelbaar is. Exacte codes staan als open punt.

## Taken

### T1: Repo-skelet, licentie en CI
- **Doet:** cargo-project `symlog` (lib + bin). LICENSE-MIT, LICENSE-APACHE, `license = "MIT OR Apache-2.0"`, `rust-version`. GitHub Actions `ci.yml` (ubuntu-latest, macos-latest): `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`. Een aparte job `bench-mem` draait de geheugentest uit T5 (release-build). `deny.toml` met licentie-allowlist en een verbod op netwerk-crates (reqwest, hyper, ureq, tokio met net-feature). `docs/crates.md` met de read-only check per crate.
- **Klaar als:** CI is groen op een lege lib-test, `cargo deny check` is groen, en `cargo tree -e normal` bevat geen netwerk-crate.
- **Blijft:** één binary, geen daemon, geen netwerk, geen telemetrie.

### T2: Reader en offset-index (streaming, geen volledige load)
- **Doet:** module `reader`. Opent het bestand read-only (`File::open`, nooit met schrijfrechten). Bouwt in een achtergrondthread een index van **record-startoffsets** (u64) in blokken van 1 MiB met `memchr`. Records worden lazy gelezen via `pread`/seek op offset; alleen het zichtbare venster + een kleine LRU-cache (standaard 4096 records) staat geparsed in het geheugen. mmap is optioneel achter een feature (`mmap`), standaard uit, omdat afkappen tijdens mmap SIGBUS kan geven (zie blueprint). Voortgang van het indexeren is zichtbaar in de statusbalk; de TUI is bruikbaar vóór de index klaar is.
- **Klaar als:** unit-tests voor index-opbouw over blokgrenzen (record precies op de grens, CRLF over de grens, bestand zonder slot-newline), en een test dat `G` op een bestand van 1 GB (gegenereerd in tmp) binnen het prestatiedoel de laatste record toont.
- **Blijft:** de index bevat alleen offsets (8 B per record + recordkop-cache). Geen kopie van de inhoud.

### T3: Monolog LineFormatter-parser
- **Doet:** parseert het standaardformaat `[%datetime%] %channel%.%level_name%: %message% %context% %extra%`, met datetime in de varianten `Y-m-d\TH:i:s.uP` (Monolog 2/3 default), `Y-m-d H:i:s` (Monolog 1) en met/zonder microseconden. Context en extra aan het eind van de regel zijn JSON (`{...}` of `[]`); de parser vindt de grens van achteren via een gebalanceerde JSON-scan, zodat `{` of `]` in de message zelf geen probleem is. Niveaus: DEBUG, INFO, NOTICE, WARNING, ERROR, CRITICAL, ALERT, EMERGENCY (hoofdletterongevoelig). Kanalen zoals `app`, `request`, `doctrine`, `security`, `php`, `console`, `messenger`, `deprecation`.
- **Klaar als:** fixtures F1, F2 en F11 parsen naar golden records (`tests/golden/line_*.json`), inclusief een message met `{` erin, een lege context `[] []`, en context met geneste JSON.
- **Blijft:** een aangepast formaat (eigen LineFormatter-string) wordt in v1 niet geconfigureerd; zulke regels vallen terug op "onbekend formaat" (T6) en blijven zichtbaar.

### T4: Multiline-records en stacktraces
- **Doet:** een record loopt door tot de volgende regel die met een geldige recordkop begint (`[` + datetime voor lijnformaat, `{` met `"datetime"`-veld voor JSON). Ondersteunt LineFormatter met `includeStacktraces` (regels `#0 /pad/File.php(123): ...`), PHP-fatal-uitvoer (`Stack trace:`), `[previous exception]`-blokken en de JSON-velden `context.exception.trace` (array) en `context.exception.file` (`/pad:regel`). Elk frame wordt een `Frame { path, line, func }`.
- **Klaar als:** F3 (stacktrace in lijnformaat, 40 frames, met previous exception), F4 (exception in JSON) en F12 (`#0`-regel die toevallig met `[` begint maar geen datetime heeft) geven golden frames. Een unit-test dekt een record dat over een index-blokgrens loopt.
- **Blijft:** frames zonder regelnummer (`[internal function]`) worden getoond maar zijn niet "springbaar".

### T5: Monolog JsonFormatter (NDJSON) en gemengde bestanden
- **Doet:** parseert per regel JSON met `message`, `context`, `level` (int) en/of `level_name`, `channel`, `datetime`, `extra`. Formaat wordt per record bepaald, niet per bestand, zodat een **gemengd** bestand (lijn + JSON door elkaar, bijvoorbeeld na een configwijziging) werkt. `--format` dwingt één parser af. Gebruikt `serde_json` met borrowed strings waar mogelijk.
- **Klaar als:** F5 (zuiver NDJSON) en F6 (gemengd, 50/50 door elkaar) geven golden records; `--format line` op F5 toont alles als onbekend formaat zonder crash.
- **Blijft:** JSON-records die over meerdere regels lopen (pretty-printed) worden in v1 niet samengevoegd; zie open punten.

### T6: Kapotte regels, ongeldige JSON, ongeldige UTF-8
- **Doet:** een regel die geen enkel formaat haalt, wordt een record `Raw` met de originele bytes (lossy UTF-8 via `String::from_utf8_lossy`, U+FFFD). Ongeldige JSON (afgekapt, enkele quotes) wordt `Raw` met reden. Raw-records zijn zichtbaar, doorzoekbaar en worden met een eigen markering getoond; ze vallen niet weg bij een niveaufilter tenzij `--hide-raw`. NUL-bytes en ANSI-escapes in de logtekst worden geneutraliseerd vóór weergave (geen terminal-injectie).
- **Klaar als:** F7 (kapotte regels + halve JSON), F8 (ongeldige UTF-8-bytes, Latin-1-regel) en F15 (ANSI-escape en NUL in message) tonen zonder panic, met golden plain-uitvoer. Een fuzz/property-test (`proptest`, 10 000 cases) op de parser vindt geen panic.
- **Blijft:** de originele bytes worden nooit aangepast op schijf.

### T7: Model, tijdzones en filters
- **Doet:** `Record { offset, ts: Option<Zoned/OffsetDateTime>, channel, level, message, context, extra, frames, raw }`. De tijd behoudt de **offset uit de log**; `--tz local` of `--tz Europe/Amsterdam` toont omgerekend, met de offset zichtbaar. Een tijd zonder offset (Monolog 1) wordt als "zonder zone" getoond en niet stil als UTC aangenomen. Filters: kanaal (meerdere, ook `!doctrine` om uit te sluiten), niveau (lijst of `>=`), regex-zoeken (`regex` crate) over message+context. Filters zijn combineerbaar; het filterresultaat is een gefilterde offset-lijst die incrementeel wordt opgebouwd.
- **Klaar als:** F9 (tijden met +02:00, +01:00, Z en zonder offset, over de DST-wissel 25 okt 2026) toont golden tijden in `log`, `local` (TZ=Europe/Amsterdam in de test) en `America/New_York`; filtertests dekken elke combinatie (kanaal × niveau × zoek) op F1.
- **Blijft:** sortering blijft in bestandsvolgorde; er wordt niet hersorteerd op tijd.

### T8: TUI (ratatui + crossterm) en vim-toetsen
- **Doet:** lijstweergave (tijd, niveau in kleur, kanaal, message afgekapt) en een detailpaneel (`Enter`/`Tab`) met context, extra en frames. Toetsen: `j/k`, `gg/G`, `Ctrl-d/Ctrl-u`, `/`, `n/N`, `c` (kanaalkiezer), `l` (niveaukiezer), `F` (follow aan/uit), `o` (open frame), `y` (kopieer bestand:regel naar stdout bij afsluiten, optioneel), `?` (help), `q`/`Ctrl-c`. Breedte van tekens via `unicode-width` (emoji en CJK = 2 kolommen, combining = 0); afkappen gebeurt nooit midden in een grafeem (`unicode-segmentation`). **Resize**: herteken bij `Event::Resize`, selectie blijft op hetzelfde record. Bij panic wordt de terminal hersteld (panic-hook).
- **Klaar als:** snapshot-tests met `ratatui::backend::TestBackend` op 80×24, 40×10 en 200×50 voor F1, F10 (emoji/CJK/combining) en F3; een resize-test (80×24 → 30×8 → 120×40) houdt de selectie vast en geeft geen panic; een test toetst de toetsenreeks `gg`, `G`, `/foo`, `n`, `N` op een gescripte event-stroom.
- **Blijft:** geen muis vereist (optioneel aan), geen kleur bij `NO_COLOR`.

### T9: Editor-sprong (Neovim, $EDITOR, vi) en padmapping
- **Doet:** module `editor`. Volgorde:
  1. Draaiende nvim-server: socket uit `--server`, config, of `$NVIM` (gezet binnen een nvim-terminal). Commando: `nvim --server <sock> --remote <file>` gevolgd door `nvim --server <sock> --remote-send '<C-\><C-N>:<line><CR>'` (of `--remote-expr` met `cursor()`; keuze in blueprint). symlog blijft open.
  2. Geen server: TUI tijdelijk verlaten (raw mode uit, alternate screen uit), `nvim +<line> <file>` als kindproces, wachten, TUI herstellen.
  3. Geen nvim op PATH: `$VISUAL`/`$EDITOR` met `+<line>` als de editor dat kent (vi, vim, nvim, nano, emacs, micro, kak, hx via `file:line`), anders alleen het bestand; daarna `vi`.
  Argumenten worden als losse argv doorgegeven, nooit via een shell. Padmapping: `--map /var/www=/home/u/proj` (meerdere, langste prefix wint), ook uit config; bestaat het gemapte bestand niet, dan een melding in de statusbalk en geen start van de editor.
- **Klaar als:** tests met een nep-`nvim`/nep-editor (scriptje in `tests/bin` dat argv naar een bestand schrijft in tmp) controleren de exacte argv voor: server via `$NVIM`, server via `--server`, geen server, alleen `$EDITOR=nano`, `$EDITOR="code --wait"` (gesplitst met `shell-words`), niets → `vi`; padmapping-tests met overlappende prefixen; F3 levert de juiste file:line na mapping.
- **Blijft:** de logbestanden worden nooit door symlog geschreven; de editor opent bronbestanden, niet de log.

### T10: Follow (tail -f), rotatie en afkappen
- **Doet:** `--follow` of `F` volgt het bestand. Mechanisme: `notify` (inotify/kqueue/FSEvents) met een polling-terugval van 500 ms (ook voor NFS/Docker-volumes). Bij elke wake: `metadata()` van het pad vergelijken met de open handle.
  - **Groei**: nieuwe bytes indexeren vanaf de laatste offset; een half geschreven laatste regel wordt pas getoond als de newline er is (of na 2 s timeout als Raw-staart).
  - **Rotatie** (inode/dev anders, bijvoorbeeld logrotate of Monolog RotatingFileHandler): oude handle uitlezen tot EOF, dan het nieuwe bestand openen vanaf 0; scheidingsregel "— geroteerd —" in de lijst.
  - **Afkappen** (zelfde inode, size < offset, bijvoorbeeld `copytruncate` of `> file`): index vanaf 0 opnieuw, scheidingsregel "— afgekapt —", selectie naar het eind als follow aanstaat.
  - **Bestand verdwijnt**: statusbalk "wacht op bestand", blijven pollen.
  Ook buiten follow wordt afkappen tijdens het lezen gedetecteerd (read geeft minder bytes dan de index verwacht) en netjes afgehandeld in plaats van een panic.
- **Klaar als:** integratietests (in tmp, met een schrijver-thread) voor F13: groei, rotatie via rename + nieuw bestand, copytruncate, afkappen midden in een record, verwijderen en terugkomen; elk scenario controleert de getoonde records en dat er geen dubbele of verloren records zijn. Tests draaien in CI op Linux en macOS met `--poll` en met notify.
- **Blijft:** symlog roteert of kapt zelf nooit iets af.

### T11: Leeg en ontbrekend bestand
- **Doet:** leeg bestand: TUI opent met de melding "log is leeg" (plain: niets, exit 0). Ontbrekend bestand zonder `--follow`: melding met het pad en een hint (bijvoorbeeld `var/log/dev.log bestaat niet; draait de app in een container? gebruik --map of geef het pad`), exit 1. Met `--follow`: wachten tot het bestand verschijnt ("wacht op …"), daarna normaal volgen. Geen leesrechten: duidelijke melding, exit 4. Pad is een map: exit 2.
- **Klaar als:** `assert_cmd`-tests voor F14 (leeg), ontbrekend, ontbrekend + `--follow` (bestand verschijnt na 1 s in de test), geen rechten (overgeslagen als de test als root draait), map als pad; elke test controleert de exitcode en de stderr-tekst.
- **Blijft:** de exitcodes staan in de README-tabel.

### T12: Grote bestanden en geheugendoel (benchmark)
- **Doet:** generator `tests/gen_big.rs` (of `xtask`) maakt een log van 2 GB met gemengd lijn/JSON en 5% stacktraces. Benchmark meet piek-RSS (via `/proc/self/status` VmHWM op Linux, `getrusage` op macOS) en tijden.
- **Klaar als:** op de 2 GB-fixture in release: **piek-RSS ≤ 150 MB** (index ≈ 8 B × records + cache), eerste scherm **< 200 ms**, volledige index **< 10 s** op CI-hardware, `G` na index **< 50 ms**, filter `--level error` over 2 GB **< 15 s** en blijft responsief (toets binnen 100 ms). Doelen zijn nog niet gemeten; de eerste meting komt in `docs/perf.md`. Een doel dat niet gehaald wordt, gaat terug naar de eigenaar met cijfers, niet stil omlaag.
- **Blijft:** de 2 GB-fixture wordt gegenereerd, niet ingecheckt. In CI draait een 256 MB-variant met een evenredig doel; de 2 GB-test is `#[ignore]` en draait handmatig onder de geheugenwachter.

### T13: Geen TTY en plain-uitvoer
- **Doet:** is stdout geen TTY (pipe, redirect, CI) of is `--plain` gezet, dan geen TUI maar plain tekst: één regel per record na filters, frames ingesprongen, kleur alleen bij TTY en zonder `NO_COLOR`. Is stdin geen TTY maar stdout wel en de bron is `-`, dan TUI met invoer via `/dev/tty` (Unix); lukt dat niet, dan plain. `--follow` werkt ook in plain (streamt). Gevraagd om TUI (`--tui`) zonder TTY: exit 3 met uitleg.
- **Klaar als:** `assert_cmd`-tests: `symlog F1 | cat` geeft golden plain-uitvoer; `--level error --channel request` plain is golden; `--tui` zonder tty geeft exit 3; plain + `--follow` met een groeiend bestand geeft de nieuwe regels.
- **Blijft:** de plain-uitvoer is stabiel genoeg om te grep-pen, maar geen formeel schema (geen `--json` in v1).

### T14: Read-only bewijs
- **Doet:** test draait de binary (plain en TUI via TestBackend-modus) op alle fixtures in een map met `chmod a-w`, en vergelijkt daarna mtimes en hashes (niets veranderd). Op Linux-CI een `strace -f -e trace=openat,socket,connect`-test (overgeslagen als strace ontbreekt) die faalt bij een socket/connect of een `openat` op een logbestand met O_WRONLY/O_RDWR/O_CREAT/O_TRUNC. Uitzondering: de nvim-socket bij de editor-sprong (alleen in T9, niet in deze test).
- **Klaar als:** beide checks staan in CI en zijn groen; een bewust kapotte branch die de log opent met schrijfrechten laat de test falen (één keer handmatig gecontroleerd, genoteerd in de PR).
- **Blijft:** "read-only" wordt in de README alleen geclaimd voor wat deze tests dekken.

### T15: README, help en release-klaar (zonder release)
- **Doet:** README met installatie (`cargo install --path .`, later crates.io), een screenshot als tekstblok (geen gegenereerde beelden met verzonnen cijfers), toetsentabel, flags, config-voorbeeld met `--map`, editor-gedrag, exitcode-tabel, "Wat het NIET doet", "Alternatieven" en de claim→test-tabel. CHANGELOG, `--help` als golden test.
- **Klaar als:** elke README-claim verwijst naar een test of fixture; `cargo package --allow-dirty` (lokaal) slaagt; geen persoonsnamen of handles in de repo (`rg` op een lijst verboden woorden als CI-stap). **Niet** pushen, publiceren of taggen zonder akkoord van de eigenaar.
- **Blijft:** versie 0.1.0, geen badges met ongemeten cijfers.

## Fixtures (tests/fixtures/), elk met een golden output
| # | Bestand | Situatie | Verwacht |
|---|---|---|---|
| F1 | line-basic.log | Monolog 3 lijnformaat, 8 niveaus, 6 kanalen | golden records, filters werken |
| F2 | line-context.log | context/extra JSON aan het eind, message met `{` en `]`, `[] []` | juiste grens message/context |
| F3 | line-stacktrace.log | exception met 40 frames, `[previous exception]`, `[internal function]` | frames, springbaar behalve internal |
| F4 | json-exception.ndjson | JsonFormatter met `context.exception.trace` en `file: "/pad:12"` | frames uit JSON |
| F5 | json-basic.ndjson | zuiver NDJSON, `level` int + `level_name` | golden records |
| F6 | mixed.log | lijn en JSON door elkaar | per record juist formaat |
| F7 | broken.log | afgekapte regels, halve JSON, regel zonder kop aan het begin | Raw, geen crash |
| F8 | bad-utf8.log | ongeldige UTF-8, Latin-1 bytes | U+FFFD, geen crash |
| F9 | timezones.log | +02:00, +01:00, Z, zonder offset, over 25 okt 2026 | golden tijden per `--tz` |
| F10 | wide.log | emoji (incl. ZWJ-reeks), CJK, combining accents, tabs | juiste kolombreedte in snapshots |
| F11 | monolog1.log | Monolog 1 datetime `Y-m-d H:i:s` | geparsed, tijd zonder zone |
| F12 | tricky-heads.log | frame- of messageregels die met `[` beginnen | geen valse recordstart |
| F13 | (gegenereerd) follow/* | groei, rename-rotatie, copytruncate, afkappen midden in record, verwijderen + terug | geen verlies of dubbelen |
| F14 | empty.log | 0 bytes | melding, exit 0 |
| F15 | escapes.log | ANSI-escapes, NUL, CR zonder LF, CRLF | geneutraliseerd, geen terminal-injectie |
| F16 | (gegenereerd) big-256m / big-2g | gemengd, 5% stacktraces | geheugen- en tijddoelen T12 |
| F17 | container-paths.log | frames onder `/var/www/html/src`, `/app/vendor` | juiste mapping met langste prefix |

De fixtures zijn zelf geschreven op basis van de Monolog-documentatie en een eigen Symfony-skeleton, en geanonimiseerd. Er worden geen logs van derden gekopieerd. Geen namen, hostnamen of e-mailadressen van echte personen in fixtures.

## README-claims (alleen deze; elk gekoppeld aan een test)
- Leest Monolog-lijnformaat (Monolog 1–3) en JsonFormatter (NDJSON), ook gemengd in één bestand. → F1, F2, F5, F6, F11; T3, T5
- Toont multiline stacktraces als frames en springt met één toets naar bestand:regel in Neovim (draaiende server of nieuw proces), `$EDITOR` of `vi`. → F3, F4; T4, T9
- Zet containerpaden om naar lokale paden. → F17; T9
- Filtert op kanaal en (gecombineerde) niveaus, met regex-zoeken. → F1; T7
- Toont kapotte regels en ongeldige UTF-8 in plaats van te crashen. → F7, F8, F15; T6
- Correcte kolombreedte voor emoji en brede tekens. → F10; T8
- Behoudt de tijdzone uit de log; omrekenen naar lokale tijd is optioneel. → F9; T7
- Volgt live, ook bij rotatie en afkappen. → F13; T10
- Opent bestanden van gigabytes zonder ze in het geheugen te laden (gemeten piek-RSS in `docs/perf.md`). → F16; T12 (claim pas na meting)
- Werkt zonder TTY als plain-uitvoer voor pipes. → T13
- Read-only op de logs en zonder netwerk. → T14

## Wat het NIET doet (letterlijk in de README)
- Geen logverzameling, geen server, geen netwerk; alleen lokale bestanden of stdin.
- Schrijft, roteert of kapt nooit een logbestand.
- Geen aangepaste LineFormatter-formaatstrings in v1; zulke regels worden als ruwe regels getoond.
- Geen pretty-printed (meerregelige) JSON-records in v1.
- Geen gzip-archieven (`*.log.gz`) in v1 [open punt].
- Geen hersortering op tijd; de volgorde is die van het bestand.
- Opent containers niet zelf (`docker exec`); geef het pad van het gemounte volume of gebruik `docker logs … | symlog -`.

## Alternatieven (eerlijk, in de README; nog te verifiëren)
- **lnav**: krachtige algemene logviewer met SQL; Monolog-formaat via eigen formaatdefinitie [niet geverifieerd].
- **Symfony Profiler / web debug toolbar**: per request in de browser, niet voor bestanden achteraf.
- **`tail -f` + `grep` / `jq`**: altijd beschikbaar, geen frames of editor-sprong.
- **less +F**: volgen en zoeken, geen parsing.
- Andere TUI-viewers (bijvoorbeeld tailspin, hl) [niet geverifieerd voor Monolog en editor-sprong].
Deze lijst wordt vóór publicatie opgehaald en gecontroleerd; niets wordt geclaimd als "uniek" zonder bron.

## Open punten
1. Exitcodes 1/3/4: akkoord, of liever alleen 0/1/2?
2. Nvim-sprong: `--remote-send ':<line><CR>'` of `--remote-expr` met `cursor()`/`execute()`. Het eerste stoort als de gebruiker in insert- of terminal-mode zit; voorkeur is `--remote-expr` [te testen].
3. Tijdcrate: `jiff` (voorkeur, IANA-tz en DST) of `time` + `chrono-tz`.
4. mmap standaard uit (SIGBUS bij afkappen). Akkoord dat pread de standaard is?
5. Gzip (`dev.log.1.gz`) en pretty-printed JSON: in v1 of later?
6. Naam `symlog` op crates.io en GitHub is nog niet gecontroleerd op beschikbaarheid.
7. De geheugen- en tijddoelen in T12 zijn ingeschat, niet gemeten.
8. Windows: wordt niet ondersteund in v1 (alleen Linux en macOS in CI). Akkoord?
9. Push, publicatie en release vragen akkoord van de eigenaar.
