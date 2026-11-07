# Quality

Gates for this branch, measured with `cargo test --no-fail-fast`,
`cargo clippy --all-targets -- -D warnings` and `cargo fmt --check`:

- Tests: 427 passed, 0 failed, 1 ignored (sum over all test binaries), measured on
  commit `2c4e4c2`.
- Clippy: clean with `-D warnings`.
- Fmt: clean.

README.md and README.nl.md state the same test count; update all three when the suite
changes.
