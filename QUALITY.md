# Quality

Gates for this branch, measured with `cargo test --no-fail-fast`,
`cargo clippy --all-targets -- -D warnings` and `cargo fmt --check`:

- Tests: 437 passed, 0 failed, 1 ignored (sum over all test binaries), measured on
  commit `f5ab1a4` plus the quickstart tests.
- Clippy: clean with `-D warnings`.
- Fmt: clean.

README.md and README.nl.md state the same test count; update all three when the suite
changes.

## Speed on a 100 MiB input (2026-11-08, W-26)

- Input: a synthetic log of exactly 104857600 bytes (100 MiB), built by repeating `examples/symfony-prod.log` (962 bytes) and cutting at the byte count. It lives in `/tmp` and is not committed. The repeated lines are almost identical (the same block over and over; the last record is cut off mid-line and is shown as a raw line), so this is a best case, not a typical log.
- Command: `/usr/bin/time -f "%e s wall, %M KB" target/release/symlog --plain --no-color big.log > /dev/null` on the release build.
- Measured wall seconds, three runs: 3.47, 3.47, 3.49. Peak memory 115896 to 115928 KB. Output: 980996 lines, 85891636 bytes.
- One machine, no other load recorded; no throughput figure is derived from this.
