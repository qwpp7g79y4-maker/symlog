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
| Filter `--level error` | < 15 s | 16,11 s vóór de kop-scan; **niet opnieuw gemeten** | zie de 256 MiB-meting |
| Toetsrespons tijdens filteren | < 100 ms | 44,0 µs | ✅ |

De 2 GiB-rij voor het filter is de meting van vóór de kop-scan hieronder. Die fixture
is in deze ronde niet opnieuw gedraaid (`bench_mem_2gib_full_fixture` blijft
`#[ignore]`). Het budget is niet aangepast.

### Filterpad: waar de tijd zat, en de kop-scan

`filtered_offsets` las elk record met een eigen `pread`, zette het in de LRU-cache
(capaciteit 4096, dus bij een volle scan alleen verdringen) en parste het daarna
volledig (`parse_record_lossy`: context-JSON, frames, `Ts`). Een uitsplitsing op een
64 MiB-fixture uit dezelfde generator (269.717 records, 67.109.000 bytes), release:

| Stap | Tijd | Aandeel |
|---|---|---|
| `record_bytes` (pread + cache-lock, geen parse) | 610 ms | ~2,3 µs/record |
| parse LineFormatter, alleen de parsetijd | 116 ms over 128.116 records | 0,90 µs/record |
| parse JsonFormatter, alleen de parsetijd | 280 ms over 141.601 records | 1,98 µs/record |
| `index.offset` per record | 1,6 ms | verwaarloosbaar |
| `redact_secrets` over dezelfde bytes (niet in het filterpad) | 663 ms, 0 treffers | zou ~2,5 µs/record extra zijn |

Op de 256 MiB-fixture (1.077.218 records) telt dat op tot de gemeten 4,30 s vóór de
wijziging: ongeveer 2,4 s lezen en 1,6 s parsen. `redact::for_display` /
`redact_secrets` zitten in plain en de TUI, niet in `filtered_offsets`; de eerdere
1,95 s in dit document en de 4,30 s van deze ronde zijn dezelfde code, de tweede onder
hogere belasting (load ~32, via de geheugenwachter). Zoeken (`FilterSpec::search`)
blijft de volledige parse doen, omdat de zoekterm in message en context kijkt.

Zonder zoekterm classificeert het filter nu alleen de kop (niveau, en het kanaal als
dat filter aan staat) en leest records in blokken van 4096 per `pread`. Een blok dat
de scan niet zeker kan plaatsen valt terug op de volledige parse. Raw-records blijven
staan bij een niveau- of kanaalfilter (task.md T6). Het budget is niet verlaagd.

## Resultaten: 256 MiB-CI-variant, release (evenredig doel: 1/8 van de 2 GiB-doelen)

Fixture: 1.077.218 records (134.652 niveau ERROR), 268.435.569 bytes.

| Meting | Doel (1/8 van 2 GiB, of gelijk waar grootte-onafhankelijk) | Gemeten | Resultaat |
|---|---|---|---|
| Piek-RSS | ≤ 150 MB (zelfde plafond) | 15.187.968 bytes (≈ 14,5 MiB) | ✅ |
| Eerste scherm | < 200 ms (zelfde) | 6,2 ms | ✅ |
| Volledige index | < 1,25 s | 0,67 s | ✅ |
| `G` na index | < 50 ms (zelfde) | 40 µs | ✅ |
| Filter `--level error` | < 1,875 s | **0,42 s, 0,43 s en 0,45 s** | ✅ |
| Toetsrespons tijdens filteren | < 100 ms (zelfde) | 35 µs | ✅ |

Opnieuw gemeten na de kop-scan, dezelfde machine, release, drie keer
`cargo test --release --test gen_big bench_mem_256mib -- --nocapture --test-threads=1`
onder de geheugenwachter (load ~32). De tabel toont de traagste waarde van die drie
runs; de filtertijden apart zijn 0,421 s, 0,434 s en 0,454 s. Recordtelling
ongewijzigd: 1.077.218 records, 134.652 ERROR. Vóór de kop-scan was het filter op
deze belasting 4,30 s. De eerdere 1,95 s in dit document hoort bij een rustigere run
van het oude, volledige parse-pad.

## Resultaten: 256 MiB-variant, debug (`cargo test` zonder `--release`)

Debug-builds hebben geen vergelijkbare optimalisaties, dus `bench_mem_256mib_ci_fixture`
handhaaft de tijd- en RSS-budgetten alleen zonder `debug_assertions` (release); in debug
draait dezelfde meting puur ter informatie en controleert alleen juistheid
(recordaantallen, geen verlies, exact de verwachte ERROR-matches — die controles gelden
altijd, ook in debug). De cijfers hieronder zijn van vóór de kop-scan en in deze
ronde niet opnieuw uitgelezen (ter informatie, geen budget): piek-RSS 16.416.768
bytes, eerste scherm 16,1 ms, volledige index 3,84 s, `G` na index 19,9 µs, filter
9,41 s.

## Gevolg voor CI

De `bench-mem`-job in `.github/workflows/ci.yml` draait `cargo test --release
bench_mem` en neemt daarmee `bench_mem_256mib_ci_fixture` mee. Na de kop-scan valt
het filter binnen 1,875 s (zie de drie tijden hierboven); het budget zelf is niet
veranderd. `bench_mem_2gib_full_fixture` blijft `#[ignore]` en raakt de gewone CI-run
niet. De 2 GiB-filtertijd van 16,11 s is niet opnieuw gemeten.
