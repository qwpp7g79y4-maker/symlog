# symlog: blueprint (22 okt 2026)

> Hoort bij `task.md` in deze map (T-nummers verwijzen daarheen). Publiek bestand: geen persoonsnamen of handles.

## Doel in één zin
Een Symfony/Monolog-log van elke grootte openen, filteren en vanuit een stacktrace met één toets in de eigen editor op de juiste regel landen, zonder de log ooit te wijzigen.

## Modules
```
src/
  main.rs          CLI (clap), config laden, modus kiezen (TUI / plain), exitcodes
  config.rs        config.toml (serde + toml), samenvoegen met flags
  reader/
    mod.rs         Source-trait: File, Stdin
    index.rs       offset-index (Vec<u64>, groeit), achtergrondindexer, blokscan met memchr
    cache.rs       LRU van geparste records (lru crate), sleutel = offset
  parse/
    mod.rs         detect(bytes) -> Format; parse_record(bytes) -> Record
    line.rs        LineFormatter (Monolog 1–3), context/extra JSON van achteren
    json.rs        JsonFormatter / NDJSON
    trace.rs       frames uit "#0 /pad(12): fn" en uit JSON exception.trace
    raw.rs         terugval: lossy UTF-8, neutraliseren van escapes
  model.rs         Record, Level, Channel (geïnterneerd), Frame, Ts
  filter.rs        FilterSpec (kanalen, niveaus, regex), incrementele gefilterde index
  time.rs          tz-weergave (log / local / IANA)
  follow/
    mod.rs         Watcher: notify + polling-terugval
    rotate.rs      detectie groei / rotatie / afkappen / verdwijnen (dev+inode+size)
  editor/
    mod.rs         EditorPlan bepalen en uitvoeren
    nvim.rs        server-detectie ($NVIM, --server, config), remote-aanroepen
    fallback.rs    $VISUAL/$EDITOR (+line-ondersteuning), vi
    pathmap.rs     container- naar lokaal pad (langste prefix)
  tui/
    app.rs         state (selectie, scroll, modus, statusbalk)
    keys.rs        vim-keymap, multi-key (gg), zoekprompt
    view.rs        lijst, detailpaneel, kiezers, help
    width.rs       unicode-width + grafeem-afkappen
  plain.rs         niet-TTY uitvoer
```

## Crate-keuzes (elk eerst read-only gecontroleerd, zie task.md T1)
| Crate | Waarvoor | Waarom |
|---|---|---|
| `ratatui` + `crossterm` | TUI | de gangbare combinatie; crossterm werkt op Linux en macOS zonder ncurses; `TestBackend` voor snapshot-tests |
| `clap` (derive) | CLI | standaard, goede help-uitvoer voor golden test |
| `memchr` | newline-scan in de indexer | SIMD, veel sneller dan byte-voor-byte |
| `serde`, `serde_json` | JSON-records, context | standaard; borrowed parsing om allocaties te sparen |
| `regex` | zoeken en filteren | lineaire tijd, geen catastrofale backtracking op kwaadaardige patronen |
| `jiff` (voorkeur) of `time`+`chrono-tz` | tijden en tz | behoud van offset, IANA-tz voor `--tz`; keuze open (task.md open punt 3) |
| `unicode-width`, `unicode-segmentation` | kolombreedte | emoji/CJK = 2, geen afkappen midden in een grafeem |
| `notify` | follow | inotify/kqueue/FSEvents; polling-terugval zit er ook in |
| `lru` | recordcache | begrensd geheugen |
| `shell-words` | `$EDITOR="code --wait"` splitsen | zonder een shell te starten |
| `toml`, `dirs` | config | XDG-pad |
| `anyhow` (bin) / `thiserror` (lib) | fouten | getypte fouten in de lib, context in de bin |
| `memmap2` (optioneel, feature `mmap`) | snelle scan | standaard uit vanwege SIGBUS bij afkappen |
| dev: `insta`, `assert_cmd`, `predicates`, `tempfile`, `proptest` | tests | golden/snapshot, CLI, fuzz-achtig |

Geen async runtime: threads + kanalen (`std::sync::mpsc` of `crossbeam-channel`) zijn genoeg en houden de binary klein.

## Datastroom
```
bestand ──open read-only──► Indexer-thread ──offsets──► Index (Vec<u64>)
                                │                          │
   Watcher (notify/poll) ───────┘ (groei/rotatie/afkap)    │
                                                           ▼
   TUI-loop: venster [top..top+h] ──offsets──► pread + parse ──► LRU-cache ──► view
                    ▲                                         │
   FilterSpec ──► Filter-thread: loopt index af, parst, ──────┘
                  levert gefilterde offset-lijst incrementeel
   'o' op frame ──► pathmap ──► EditorPlan ──► nvim remote / kindproces
```
- De indexer publiceert offsets in batches; de UI leest een snapshot (`Arc<RwLock<Vec<u64>>>` of append-only chunks), zodat lezen nooit blokkeert op indexeren.
- Een record = bytes van offset[i] tot offset[i+1]. Recordstart = regel die begint met een geldige kop (`[YYYY-` of `{"`). Om valse starts te vermijden checkt de indexer de kop strikt (datetime-patroon), niet alleen `[`.
- Filteren loopt in een eigen thread en annuleert bij een nieuwe FilterSpec (generatienummer).

## Follow en rotatie (T10)
Bij elke wake (notify-event of poll-tick 500 ms):
1. `stat(pad)`: ontbreekt → status "wacht", blijf pollen.
2. `(dev, ino)` ≠ open handle → **rotatie**: lees de oude handle uit tot EOF, markeer scheiding, open het nieuwe pad, index vanaf 0.
3. Zelfde inode en `size < indexed_end` → **afkappen**: index en cache leeg, scheiding, index vanaf 0.
4. `size > indexed_end` → **groei**: index vanaf `indexed_end`; onvolledige laatste regel wachten tot newline (max 2 s, dan als Raw-staart tonen en later vervangen).
Ook bij gewoon lezen: een `pread` die minder bytes geeft dan verwacht leidt tot stap 3, niet tot een panic. Daarom is mmap standaard uit: afkappen onder een mmap geeft SIGBUS.

## Editor-sprong (T9)
```
frame(path,line) → pathmap → bestaat lokaal? nee → statusmelding, stop
  server = --server ∥ config.server ∥ $NVIM
  server en socket bereikbaar → nvim --server S --remote FILE ; nvim --server S --remote-expr "execute(':LINE')" (of cursor)
  anders nvim op PATH        → TUI pauzeren, spawn nvim +LINE FILE, wachten, TUI herstellen
  anders $VISUAL/$EDITOR     → +LINE als editor bekend is (vi/vim/nvim/nano/emacs/micro/kak), hx → FILE:LINE, anders FILE
  anders                     → vi +LINE FILE
```
- Altijd argv, nooit `sh -c`; paden met spaties of quotes kunnen geen commando injecteren.
- Pauzeren: `disable_raw_mode`, `LeaveAlternateScreen`, cursor tonen; na afloop terug en volledig hertekenen.

## Foutafhandeling
- **Lib** geeft `thiserror`-fouten; **parsefouten zijn geen fouten** maar Raw-records met reden.
- **I/O tijdens lezen**: herstelbaar (afkappen, rotatie, verdwijnen) → statusbalk; niet herstelbaar (EIO, geen rechten na heropenen) → TUI netjes afsluiten, melding, exit 4.
- **Panic-hook** herstelt altijd de terminal vóór de panic-melding.
- **Editor faalt** (exitcode ≠ 0, niet gevonden) → statusbalk, symlog blijft draaien.
- Geen `unwrap()` buiten tests; clippy `unwrap_used` op warn in de lib.
- Logtekst wordt vóór weergave ontdaan van control- en escape-tekens (geen terminal-injectie via een log).

## Prestatiedoel (te meten in T12; nog niet gemeten)
| Meting | Doel |
|---|---|
| Piek-RSS bij 2 GB-log | ≤ 150 MB |
| Eerste scherm | < 200 ms, ongeacht grootte |
| Volledige index 2 GB | < 10 s (CI-hardware) |
| `G` na index | < 50 ms |
| Toetsrespons tijdens indexeren/filteren | < 100 ms |
| Follow-latentie (notify) | < 250 ms; polling < 750 ms |
Geheugenbudget: index ≈ 8 B/record (bij ~10 M records ≈ 80 MB) + LRU-cache (≈ 4096 records) + gefilterde lijst. Overschrijdt een echte log dit (veel korte regels), dan wordt de index gecomprimeerd (u32-delta's per blok) en dat wordt dan een extra taak, niet stil.

## Bewust buiten v1
Aangepaste formaatstrings, pretty-printed JSON, gzip, Windows, hersortering op tijd, schrijven van welke aard ook.
