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
| `ratatui` | tui | gepland, nog geen dependency | MIT, geen netwerk, komt in T8 |
| `crossterm` | tui | gepland, nog geen dependency | MIT, geen netwerk, komt in T8 |
| `clap` (derive) | main | gepland, nog geen dependency | MIT OR Apache-2.0, komt met de echte CLI-parser |
| `memchr` | reader::index | toegevoegd in T2 | MIT OR Apache-2.0 |
| `serde_json` | model, parse::line, parse::json | toegevoegd in T3 (v1.0.154), hergebruikt in T5 | MIT OR Apache-2.0; alleen `serde_json::Value` (geen `serde`-dependency, geen `derive`-feature: die zou `proc-macro2`/`quote`/`syn`/`unicode-ident` meebrengen, met een `Unicode-3.0`-licentieclausule die nog niet in de allowlist stond). `context`/`extra` in LineFormatter-regels zijn JSON (task.md T3), dus eerder nodig dan gepland. **Bijgesteld in T5**: de T3-notitie hierboven verwachtte dat `serde` zelf (met `derive`) in T5 bij zou komen voor de JsonFormatter-structs; de bouwcontext van T5 draagt echter op `deny.toml` ongewijzigd te laten t.o.v. T1 en geen crate buiten de gescreende lijst zonder melding toe te voegen. `derive` zou de `Unicode-3.0`-toevoeging aan de allowlist vereisen die er niet is, dus `parse::json` blijft bij hetzelfde patroon als `parse::line`: velden met `Value::get`/`as_str`/`as_i64` uit een eenmalig geparste `Value` halen, pas naar een eigen `String` kopiëren zodra het record geldig blijkt ("borrowed strings waar mogelijk" uit task.md T5) |
| `regex` | filter | gepland, nog geen dependency | MIT OR Apache-2.0, lineaire tijd (geen backtracking-DoS), komt in T7 |
| `jiff` | time | gepland, nog geen dependency | MIT OR Apache-2.0; voorkeur boven `time`+`chrono-tz` is vastgelegd (task.md open punt 3), komt in T7 |
| `unicode-width`, `unicode-segmentation` | tui::width | gepland, nog geen dependency | MIT OR Apache-2.0, komt in T8 |
| `notify` | follow | gepland, nog geen dependency | MIT OR Apache-2.0/CC0-1.0 (meerdere sub-crates), komt in T10; alleen lokale FS-events, geen netwerk |
| `lru` | reader::cache | toegevoegd in T2 (v0.19.0) | MIT; ondergrens `>=0.18.2` in `deny.toml`. Die ondergrens vereist rustc 1.85.0, dus `rust-version` in `Cargo.toml` ging in T2 van 1.82 naar 1.85 |
| `shell-words` | editor::fallback | gepland, nog geen dependency | MIT OR Apache-2.0, komt in T9 |
| `toml` | config | gepland, nog geen dependency | MIT OR Apache-2.0, komt met config-laden |
| `anyhow` (bin) | main | gepland, nog geen dependency | MIT OR Apache-2.0; ondergrens `>=1.0.103` in `deny.toml` |
| `thiserror` (lib) | reader/parse/editor | gepland, nog geen dependency | MIT OR Apache-2.0 |
| `memmap2` (feature `mmap`, standaard uit) | reader | toegevoegd in T2 als optionele dependency (v0.9.12); niet standaard aan | MIT OR Apache-2.0; ondergrens `>=0.9.11` in `deny.toml`; feature `mmap` is gedefinieerd maar nog niet in code gebruikt (pread/seek is het standaardpad, zie blueprint "mmap standaard uit") |
| dev: `insta`, `assert_cmd`, `predicates`, `proptest` | tests | gepland, nog geen dependency | alle MIT OR Apache-2.0, dev-only, niet in het release-pad |
| dev: `tempfile` | tests | toegevoegd in T2 (v3.27.0) | MIT OR Apache-2.0, dev-only; gebruikt voor tijdelijke logbestanden in de reader-tests |

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
