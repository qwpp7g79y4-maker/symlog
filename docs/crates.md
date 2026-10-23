# Crates: read-only check

> Methodiek vastgelegd bij task.md T1. Bijgehouden per taak; een crate komt pas in
> `Cargo.toml` na deze check.

## Methodiek

Voordat een crate wordt toegevoegd:

1. **Licentie** past in de allowlist van `deny.toml` (`cargo deny check licenses`).
2. **Onderhoud**: geen RUSTSEC-advisory voor "unmaintained" zonder gemotiveerde keuze
   (vervangen, of een genoteerde `ignore` in `deny.toml` met reden).
3. **`cargo tree -e normal`**: geen netwerk-crate (`reqwest`, `hyper`, `ureq`, `tokio`
   met `net`-feature) in de boom; zie het verbod in `deny.toml`.
4. **Build-script**: geen build-script dat het netwerk raakt (gecontroleerd door de
   broncode van de crate te lezen, niet alleen te vertrouwen op de beschrijving).
5. **`cargo deny check`** is groen (licenties, bans, sources, advisories).

Dit document legt per crate uit de geplande lijst (`blueprint.md`, sectie
"Crate-keuzes") de status vast. Geen crate buiten deze lijst zonder nieuwe melding.

## Status per crate

| Crate | Module | Status | Opmerking |
|---|---|---|---|
| `ratatui` | tui | toegevoegd in T8 (v0.30.2) | MIT, geen netwerk. `default-features = false`, alleen de features `crossterm`, `layout-cache`, `underline-color`: dat laat `all-widgets` (en daarmee de Calendar-widget en zijn `time`-dependency, niet gebruikt) en `macros` (de losse `ratatui-macros`-crate, niet gebruikt) weg. **MSRV-opmerking**: `ratatui` 0.29.0 (de nieuwste versie die `rust-version = "1.85"` nog toeliet) bracht intern een eigen, niet-optionele `lru 0.12`-dependency mee voor zijn layoutcache — dat botst met de ondergrens `lru >= 0.18.2` uit `deny.toml` (T1), die geldt voor *elke* `lru` in de boom, ook transitief. `ratatui` 0.30.x (via de nieuwe `ratatui-core`-subcrate) gebruikt zelf `lru 0.18` en vereist rustc 1.88.0; `rust-version` in `Cargo.toml` ging daarom in T8 van 1.85 naar 1.88 — precies het scenario dat de T1-notitie bij `rust-version` al aankondigde. Geen build-script. `cargo tree -e normal` bevestigt: geen netwerk-crate, en nog maar één `lru`-versie per gebruiker (`0.19.0` voor `reader::cache`, `0.18.5` intern in `ratatui-core`, geen van beide onder de ondergrens) |
| `crossterm` | tui | **geen eigen dependency**: gebruikt via `ratatui::crossterm` (`pub use crossterm;`, aan bij de `crossterm`-feature van `ratatui`) | Een eigen `crossterm`-dependency naast `ratatui`'s eigen, intern gepinde `crossterm`-versie (0.28.x in 0.29, 0.29.x in 0.30) zou twee incompatibele `crossterm`-versies in de boom zetten (`cargo tree` bevestigde dit tijdens het screenen); `ratatui` bestaat specifiek om dat te voorkomen door de crate zelf te re-exporteren. Daarom gebruikt `src/tui` overal `ratatui::crossterm::...` in plaats van een los `use crossterm::...`. MIT, geen netwerk |
| `clap` (derive) | main | toegevoegd in T11 (v4.6.7) | MIT OR Apache-2.0. `default-features = false`, features `derive`, `std`, `help`, `usage`, `error-context`: dat laat `color` (geen kleurbehoefte voor de `--help`/foutmeldingen die T11 nodig heeft; `--no-color`/`NO_COLOR` is pas T8/T13) en `suggestions` (`did you mean`, `strsim`-dependency, niet nodig voor de twee argumenten die T11 kent) weg. Geen build-script. `cargo tree -e normal` bevestigt: geen netwerk-crate; brengt `clap_builder`, `clap_derive` (proc-macro), `clap_lex`, `anstyle` en (transitief via `clap_derive`) `heck` mee, allemaal MIT OR Apache-2.0 |
| `memchr` | reader::index | toegevoegd in T2 | MIT OR Apache-2.0 |
| `serde_json` | model, parse::line, parse::json | toegevoegd in T3 (v1.0.154), hergebruikt in T5 | MIT OR Apache-2.0; alleen `serde_json::Value` (geen `serde`-dependency, geen `derive`-feature: die zou `proc-macro2`/`quote`/`syn`/`unicode-ident` meebrengen, met een `Unicode-3.0`-licentieclausule die nog niet in de allowlist stond). `context`/`extra` in LineFormatter-regels zijn JSON (task.md T3), dus eerder nodig dan gepland. **Bijgesteld in T5**: de T3-notitie hierboven verwachtte dat `serde` zelf (met `derive`) in T5 bij zou komen voor de JsonFormatter-structs; de bouwcontext van T5 draagt echter op `deny.toml` ongewijzigd te laten t.o.v. T1 en geen crate buiten de gescreende lijst zonder melding toe te voegen. `derive` zou de `Unicode-3.0`-toevoeging aan de allowlist vereisen die er niet is, dus `parse::json` blijft bij hetzelfde patroon als `parse::line`: velden met `Value::get`/`as_str`/`as_i64` uit een eenmalig geparste `Value` halen, pas naar een eigen `String` kopiëren zodra het record geldig blijkt ("borrowed strings waar mogelijk" uit task.md T5) |
| `regex` | filter | toegevoegd in T7 (v1.13.1) | MIT OR Apache-2.0, lineaire tijd (geen backtracking-DoS); alleen `regex::Regex` over message/context, geen `regex::bytes` |
| `jiff` | model (`Ts`), time | toegevoegd in T7 (v0.2.38) | MIT OR Apache-2.0; voorkeur boven `time`+`chrono-tz` lag al vast (task.md open punt 3). Standaardfeatures (geen `serde`, geen `static`); `tz-system` leest de systeemtijdzone (`$TZ`/`/etc/localtime`), `tzdb-zoneinfo` de IANA-database onder `/usr/share/zoneinfo` voor `--tz <IANA>`. `tzdb-bundle-platform`/`tzdb-bundle-windows` staan standaard aan maar zijn op Linux/macOS leeg (alleen relevant op Windows, dat niet in CI zit, task.md open punt 8): `cargo tree -e normal` bevestigt dat er op dit platform geen aparte `jiff-tzdb*`-crate meegebouwd wordt. Geen build-script dat het netwerk raakt (leest alleen lokale tzdata-bestanden tijdens runtime, niet tijdens build) |
| `unicode-width` | tui::width | toegevoegd in T8 (v0.2.0, gepind: `ratatui` 0.29/0.30 vereisen zelf exact `=0.2.0`) | MIT OR Apache-2.0; standaardfeature `cjk` staat aan (CJK/emoji = 2 kolommen, task.md T8 "Doet") |
| `unicode-segmentation` | tui::width | toegevoegd in T8 (v1.13.3) | MIT OR Apache-2.0; grafeemclusters (`graphemes(true)`) zodat afkappen nooit een cluster middendoor snijdt |
| `notify` | follow | toegevoegd in T10 (v8.2.0) | CC0-1.0, al in de allowlist sinds T1. Standaardfeatures (`macos_fsevent` aan, `macos_kqueue` uit): op Linux brengt dat `inotify`/`inotify-sys`/`mio` mee, op macOS `fsevent-sys`; beide paden ook `libc`, `log`, `notify-types`, `walkdir`+`same-file` en (alleen op Windows, niet in CI, task.md open punt 8) `windows-sys`. Alle MIT OR Apache-2.0 (op zichzelf en samen, `cargo deny check licenses` is groen over de hele boom), geen van deze crates heeft een build-script dat het netwerk raakt (gecontroleerd in de broncode: alleen lokale OS-syscalls/headers). `cargo tree -e normal` bevestigt: geen netwerk-crate. `notify` zelf doet geen bestandsschrijven, alleen `inotify_add_watch`/kqueue/FSEvents-registraties lezen events; de polling-terugval uit task.md T10 ("ook voor NFS/Docker-volumes") is een eigen, aparte timer-thread in `follow::Watcher`, niet `notify::PollWatcher` |
| `lru` | reader::cache | toegevoegd in T2 (v0.19.0) | MIT; ondergrens `>=0.18.2` in `deny.toml`. Die ondergrens vereist rustc 1.85.0, dus `rust-version` in `Cargo.toml` ging in T2 van 1.82 naar 1.85 |
| `shell-words` | editor::fallback | toegevoegd in T9 (v1.1.1) | MIT OR Apache-2.0, geen netwerk, geen build-script. Splitst `$VISUAL`/`$EDITOR`-waarden zoals `"code --wait"` in losse argv-delen zonder een shell te starten (task.md T9 "Doet": "nooit via een shell") |
| `toml` | config | gepland, nog geen dependency | MIT OR Apache-2.0, komt met config-laden |
| `anyhow` (bin) | main | gepland, nog geen dependency | MIT OR Apache-2.0; ondergrens `>=1.0.103` in `deny.toml` |
| `thiserror` (lib) | reader/parse/editor | gepland, nog geen dependency | MIT OR Apache-2.0 |
| `memmap2` (feature `mmap`, standaard uit) | reader | toegevoegd in T2 als optionele dependency (v0.9.12); niet standaard aan | MIT OR Apache-2.0; ondergrens `>=0.9.11` in `deny.toml`; feature `mmap` is gedefinieerd maar nog niet in code gebruikt (pread/seek is het standaardpad, zie blueprint "mmap standaard uit") |
| dev: `assert_cmd`, `predicates` | tests | toegevoegd in T11 (v2.2.2 resp. v3.1.4) | beide MIT OR Apache-2.0, dev-only, niet in het release-pad; draaien de echte `symlog`-binary als subproces voor `tests/startup_cli.rs` (task.md T11 "Klaar als"). `insta` (snapshot-tests) is nog geen dependency: T8's TUI-snapshots gebruiken `ratatui::backend::TestBackend` met handgeschreven golden-bestanden, niet `insta` |
| dev: `proptest` | parse::raw | toegevoegd in T6 (v1.11.0, de hoogste versie die `rust-version = "1.85"` nog toestaat) | MIT OR Apache-2.0, dev-only; fuzz/property-test over `parse_record_lossy` (task.md T6 "Klaar als": 10 000 cases, geen paniek) |
| dev: `tempfile` | tests | toegevoegd in T2 (v3.27.0) | MIT OR Apache-2.0, dev-only; gebruikt voor tijdelijke logbestanden in de reader-tests |

## Beslissing: `Unicode-3.0` op de licentie-allowlist (T8)

`ratatui` brengt, via zijn `instability`- en `strum_macros`-proc-macro's (`darling`,
`syn`, `proc-macro2`), onvermijdelijk `unicode-ident` mee als transitieve
dependency — `cargo tree -e normal` bevestigt dat dit niet via features te vermijden
is, het zit al in `ratatui-core`/`ratatui-crossterm`/`ratatui-widgets` zelf.
`unicode-ident` staat onder de **`Unicode-3.0`**-licentie (Unicode License v3,
OSI-goedgekeurd, permissief), die nog niet in de `[licenses] allow`-lijst van
`deny.toml` stond; `cargo deny check licenses` faalde daarop vóór deze toevoeging.

Besluit (door de eigenaar bevestigd bij de T8-taakbrief): `Unicode-3.0` toevoegen aan
de allowlist, als enige wijziging aan `deny.toml` voor T8. Geen andere licentie, crate
of regel in `deny.toml` is in T8 aangepast.

## Beslissing: XDG-configpad (`dirs` vs. alternatief)

Blueprint noemt `dirs` voor het XDG-configpad. `dirs` zelf heeft geen openstaande
RUSTSEC-advisory op dit moment; de advisory **RUSTSEC-2020-0053** (unmaintained)
geldt voor de oudere, losse crate-lijn rond `dirs` die destijds als opvolger
`dirs-next` kreeg — en `dirs-next` is op zijn beurt zelf stil gevallen.

Keuze: **`directories`** (niet `dirs`, niet `dirs-next`, niet `etcetera`) voor het
XDG-pad, zodra de configmodule wordt gebouwd (nog niet in T1):
- actief onderhouden, geen openstaande advisory;
- dezelfde MIT OR Apache-2.0-licentie, past in de bestaande allowlist;
- functioneel gelijk aan `dirs` voor ons enige gebruik (`$XDG_CONFIG_HOME/symlog/`),
  dus geen extra risico-oppervlak t.o.v. `etcetera`, dat breder maar minder gangbaar is.

Omdat geen van beide crates vandaag een dependency is, staat er geen `ignore`-regel
voor RUSTSEC-2020-0053 in `deny.toml`: die advisory raakt de huidige lockfile niet.
Komt er later toch een reden om `dirs` te gebruiken, dan hoort een gemotiveerde
`ignore`-regel in `deny.toml` bij die keuze, niet een stille uitzondering.
