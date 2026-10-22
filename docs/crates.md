# Crates: read-only check

> Bij task.md T1. Bijgehouden per taak; een crate komt pas in `Cargo.toml` na deze check.

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

| Crate | Module | Status in T1 | Opmerking |
|---|---|---|---|
| `ratatui` | tui | gepland, nog geen dependency | MIT, geen netwerk, komt in T8 |
| `crossterm` | tui | gepland, nog geen dependency | MIT, geen netwerk, komt in T8 |
| `clap` (derive) | main | gepland, nog geen dependency | MIT OR Apache-2.0, komt met de echte CLI-parser |
| `memchr` | reader::index | gepland, nog geen dependency | MIT OR Apache-2.0, komt in T2 |
| `serde`, `serde_json` | parse::json, model | gepland, nog geen dependency | MIT OR Apache-2.0, komt in T5 |
| `regex` | filter | gepland, nog geen dependency | MIT OR Apache-2.0, lineaire tijd (geen backtracking-DoS), komt in T7 |
| `jiff` | time | gepland, nog geen dependency | MIT OR Apache-2.0; voorkeur boven `time`+`chrono-tz` is vastgelegd (task.md open punt 3), komt in T7 |
| `unicode-width`, `unicode-segmentation` | tui::width | gepland, nog geen dependency | MIT OR Apache-2.0, komt in T8 |
| `notify` | follow | gepland, nog geen dependency | MIT OR Apache-2.0/CC0-1.0 (meerdere sub-crates), komt in T10; alleen lokale FS-events, geen netwerk |
| `lru` | reader::cache | gepland, nog geen dependency | MIT OR Apache-2.0; ondergrens `>=0.18.2` in `deny.toml`, komt in T2 |
| `shell-words` | editor::fallback | gepland, nog geen dependency | MIT OR Apache-2.0, komt in T9 |
| `toml` | config | gepland, nog geen dependency | MIT OR Apache-2.0, komt met config-laden |
| `anyhow` (bin) | main | gepland, nog geen dependency | MIT OR Apache-2.0; ondergrens `>=1.0.103` in `deny.toml` |
| `thiserror` (lib) | reader/parse/editor | gepland, nog geen dependency | MIT OR Apache-2.0 |
| `memmap2` (feature `mmap`, standaard uit) | reader | gepland, nog geen dependency | MIT OR Apache-2.0; ondergrens `>=0.9.11` in `deny.toml`; blijft achter een feature-vlag, zie T2 en blueprint ("mmap standaard uit") |
| dev: `insta`, `assert_cmd`, `predicates`, `tempfile`, `proptest` | tests | gepland, nog geen dependency | alle MIT OR Apache-2.0, dev-only, niet in het release-pad |

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
