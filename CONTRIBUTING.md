# Contributing

Thanks for your interest in symlog.

## Building and testing

```sh
cargo build
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

These checks must pass before a change is merged. Test fixtures must be synthetic, or anonymised copies of a public post. Never add a real secret, a real token, or a real log.

## Commit messages

Commit messages are short English sentences that say what changed, for example `Mask a Bearer value after a line break`.
