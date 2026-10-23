# Prestatiemeting (task.md T12)

> Eerste meting, zoals task.md T12 "Klaar als" aankondigt: "Doelen zijn nog niet
> gemeten; de eerste meting komt in `docs/perf.md`." Deze meting is op een
> ontwikkelmachine gedaan, **niet** op de uiteindelijke CI-hardware; de cijfers hieronder
> zijn dus een eerste indicatie, niet de definitieve CI-baseline. Een doel dat hier niet
> gehaald wordt, is hieronder met cijfers genoteerd — niet stil verlaagd (task.md T12).

## Methodiek

- Fixture: `tests/gen_big.rs`'s `write_mixed_log`, een gemengd lijn-/JSON-logbestand met
  5% stacktraces (zie de moduledoc van dat bestand voor de exacte generator). De
  fixture wordt gegenereerd in een tempdir en nooit ingecheckt (task.md T12 "Blijft").
- Metingen: `bench_mem_2gib_full_fixture` (`#[ignore]`, handmatig) en
  `bench_mem_256mib_ci_fixture` (draait mee in `cargo test`), beide in
  `tests/gen_big.rs`. Gedraaid als:
  ```
  geheugenwachter --max-gb 16 --naam symlog-t12-2gib -- \
      cargo test --release --test gen_big bench_mem_2gib -- --ignored --nocapture --test-threads=1
  ```
  (en overeenkomstig voor de 256 MiB-variant).
- Piek-RSS: `/proc/self/status`'s `VmHWM` op Linux (deze meting), `getrusage(RUSAGE_SELF)`
  op macOS. De macOS-tak is geschreven volgens de bekende Darwin-`struct
  rusage`-layout maar **nog niet op echte macOS-hardware gedraaid** — door de eigenaar
  te bevestigen zodra de `bench-mem`-CI-job op `macos-latest` draait.
- Machine (deze meting, niet CI-hardware): AMD Ryzen 9 5950X (16C/32T), 125 GiB RAM,
  Linux 6.17 (pop-os), release-build (`cargo build --release`, `rustc 1.99.0`).

## Resultaten: 2 GiB-fixture, release

Fixture: 8.586.652 records (1.073.331 niveau ERROR), 2.147.483.796 bytes, gegenereerd
in 10,47 s (los van de gemeten budgetten hieronder).

| Meting | Doel (blueprint.md) | Gemeten | Resultaat |
|---|---|---|---|
| Piek-RSS | ≤ 150 MB | 83.312.640 bytes (≈ 79,5 MiB) | ✅ ruim binnen budget |
| Eerste scherm | < 200 ms | 2,27 ms | ✅ |
| Volledige index | < 10 s | 3,06 s | ✅ |
| `G` na index | < 50 ms | 9,3 µs | ✅ |
| Filter `--level error` | < 15 s | **16,11 s** | ❌ **~7% over budget** |
| Toetsrespons tijdens filteren | < 100 ms | 44,0 µs | ✅ |

### Niet gehaald: filter `--level error` (16,11 s tegen een budget van 15 s)

`filter::filtered_offsets` parset elk record volledig (`parse::parse_record_lossy`,
dus de hele LineFormatter- of JsonFormatter-parse inclusief de JSON-scan voor
context/extra) om het niveau te kunnen vergelijken; dat is aanzienlijk duurder dan de
indexer, die alleen de recordkop controleert. Op deze meting kost dat bij 8,6 miljoen
records ongeveer 1,9 µs/record, in totaal net over het budget.

Dit is een **gemeten tekort, niet stil verlaagd** (task.md T12 "Klaar als"): de
eigenaar moet beslissen of (a) het doel van 15 s acceptabel wordt bijgesteld met deze
cijfers als onderbouwing, (b) `filtered_offsets` sneller moet (bijvoorbeeld een
goedkopere kop-only check voor een niveaufilter zonder zoekterm, zodat niet elk record
volledig geparset hoeft te worden), of (c) dit zo blijft omdat CI-hardware een ander
resultaat geeft. Dat is bewust geen T12-wijziging aan `filter.rs`: T12 meet, het is aan
de eigenaar om de implementatie (een latere taak) of het doel zelf aan te passen.

## Resultaten: 256 MiB-CI-variant, release (evenredig doel: 1/8 van de 2 GiB-doelen)

Fixture: 1.077.218 records (134.652 niveau ERROR), 268.435.569 bytes.

| Meting | Doel (1/8 van 2 GiB, of gelijk waar grootte-onafhankelijk) | Gemeten | Resultaat |
|---|---|---|---|
| Piek-RSS | ≤ 150 MB (zelfde plafond) | 15.839.232 bytes (≈ 15,1 MiB) | ✅ |
| Eerste scherm | < 200 ms (zelfde) | 2,3 ms | ✅ |
| Volledige index | < 1,25 s | 0,36 s | ✅ |
| `G` na index | < 50 ms (zelfde) | 8,6–10,3 µs | ✅ |
| Filter `--level error` | < 1,875 s | **1,95 s** | ❌ **~4% over budget** |
| Toetsrespons tijdens filteren | < 100 ms (zelfde) | 0,8–16,4 µs | ✅ |

Dezelfde oorzaak als bij de 2 GiB-meting hierboven, consistent over drie herhalingen
(1,94–1,96 s): dit is geen meetruis.

## Resultaten: 256 MiB-variant, debug (`cargo test` zonder `--release`)

Debug-builds hebben geen vergelijkbare optimalisaties, dus `bench_mem_256mib_ci_fixture`
handhaaft de tijd- en RSS-budgetten alleen zonder `debug_assertions` (release); in debug
draait dezelfde meting puur ter informatie en controleert alleen juistheid
(recordaantallen, geen verlies, exact de verwachte ERROR-matches — die controles gelden
altijd, ook in debug). Gemeten (ter informatie, geen budget): piek-RSS 16.416.768 bytes,
eerste scherm 16,1 ms, volledige index 3,84 s, `G` na index 19,9 µs, filter 9,41 s.

## Gevolg voor CI

De `bench-mem`-job in `.github/workflows/ci.yml` draait `cargo test --release
bench_mem` en neemt daarmee `bench_mem_256mib_ci_fixture` mee; die job **faalt dus op
dit moment**, precies op het filter-budget hierboven. Dat is zichtbaar en opzettelijk
(geen stille aanpassing): de job blijft deze meting rapporteren totdat de eigenaar een
van de drie opties hierboven kiest. `bench_mem_2gib_full_fixture` blijft `#[ignore]` en
raakt de gewone CI-run niet.
