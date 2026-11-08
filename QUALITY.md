# Quality

Gates for this branch, measured with `cargo test --no-fail-fast`,
`cargo clippy --all-targets -- -D warnings` and `cargo fmt --check`:

- Tests: 458 passed, 0 failed, 1 ignored (sum over all test binaries), measured on
  this SIGINT change (parent `f1aa765`) on linux x86_64. `tests/sigint.rs` is built
  only on that target. The 100 MiB row and the line-coverage row stay the earlier
  measurement.

## SIGINT (2026-11-08, W-48, branch prep/w24-fout)

- SIGINT exits 130. Stderr is `onderbroken door SIGINT`. A one-shot `--output` report does not replace the previous file until the new report is complete, and a signal before that leaves no temporary file. `tests/sigint.rs` sends the signal through a pidfd of the child it spawned. Ordinary exit codes stay 0, 1, 2, 3 and 4. The regular-file limit stays 268435456 bytes. The 16 MiB cap per line and per record on stdin stays. A missing parent directory for `--output` stays exit 4.
- `cargo test --no-fail-fast`: 458 passed / 0 failed / 1 ignored.
- `cargo fmt --check`: clean. `cargo clippy --all-targets -- -D warnings`: clean.
- Version stays 1.0.0. `rust-version` stays 1.88.
- Clippy: clean with `-D warnings`.
- Fmt: clean.

README.md and README.nl.md state the same test count; update all three when the suite
changes.

## Input limit (2026-11-08, W-46, branch prep/w24-fout)

- A regular log file may be at most 268435456 bytes (256 MiB). A larger file exits 2 with `boven de limiet van 268435456 bytes` and is not read. The limit is new. The measured 100 MiB input stays under it. The over-limit file is created in `/tmp` by the test and is not committed. The existing cap of 16 MiB per line and per record on stdin stays and was not raised. Exit codes stay 0, 1, 2, 3 and 4. A missing parent directory for `--output` stays exit 4.
- `cargo test --no-fail-fast`: 457 passed / 0 failed / 1 ignored.
- `cargo fmt --check`: clean. `cargo clippy --all-targets -- -D warnings`: clean.
- Version stays 1.0.0. `rust-version` stays 1.88.

## Speed on a 100 MiB input (2026-11-08, W-26)

- Input: a synthetic log of exactly 104857600 bytes (100 MiB), built by repeating `examples/symfony-prod.log` (962 bytes) and cutting at the byte count. It lives in `/tmp` and is not committed. The repeated lines are almost identical (the same block over and over; the last record is cut off mid-line and is shown as a raw line), so this is a best case, not a typical log.
- Command: `/usr/bin/time -f "%e s wall, %M KB" target/release/symlog --plain --no-color big.log > /dev/null` on the release build.
- Measured wall seconds, three runs: 3.47, 3.47, 3.49. Peak memory 115896 to 115928 KB. Output: 980996 lines, 85891636 bytes.
- One machine, no other load recorded; no throughput figure is derived from this.

## Line coverage (2026-11-08, W-28)

- Command: `cargo llvm-cov --ignore-run-fail` (cargo-llvm-cov 0.9.1, unit and integration tests; doctests not included).
- Printed TOTAL line coverage: 93.18% (7416 lines, 506 missed). Function coverage 96.07%, region coverage 93.40%. This is above 90%, so no tests were added for it.
- Weakest files by line coverage: `main.rs` 70.54%, `tui/view.rs` 79.90%, `tui/mod.rs` 80.00%.
- Under llvm-cov one test fails only because of the instrumentation: `strace_shows_no_write_opens_or_network_calls_on_any_fixture` in `tests/read_only_proof.rs`. The instrumented binary opens a `.profraw` file with `O_RDWR|O_CREAT`, and the strace check rejects that write open. This is why `--ignore-run-fail` was needed; it was used for this one failure only (no other test failed). Plain `cargo test --no-fail-fast` has 0 failed.

