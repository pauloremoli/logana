# Contributing to logana

Thanks for your interest in contributing. This guide covers bug reports, feature requests, documentation, and code contributions.

## Quick start

You'll need a stable Rust toolchain. Install one via [rustup](https://rustup.rs) if you don't have it.

```sh
git clone https://github.com/pauloremoli/logana
cd logana
cargo build
cargo test
```

Before making changes, read [ARCHITECTURE.md](ARCHITECTURE.md) to get your bearings. It explains the module structure, the mode system, the filter pipeline, and the key data flows.

## Reporting bugs

Search [existing issues](https://github.com/pauloremoli/logana/issues) before opening a new one.

A good bug report includes:

- **logana version** — run `logana --version`
- **Operating system and terminal**
- **Steps to reproduce** — the shortest sequence that triggers the bug
- **Expected behaviour** — what you expected to happen
- **Actual behaviour** — what happened instead
- **Stack trace** — if logana panicked, the stack trace is printed to the terminal; please include it

## Requesting features

Open a [GitHub issue](https://github.com/pauloremoli/logana/issues) describing:

- The use case — what you're trying to do
- Why existing features don't cover it

It's worth opening an issue and getting some feedback before diving into an implementation — that way we can make sure we're aligned before you put in the effort.

## Contributing code

1. Fork the repository and create a branch for your change.
2. Make your changes.
3. Run `cargo fmt` and `cargo clippy -- -D warnings` — both must pass cleanly.
4. Add tests for any new behaviour. Unit tests go in the same file as the code they cover, in a `#[cfg(test)]` module.
5. Open a pull request with a description of what changed and why.

CI runs `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo build`, and `cargo test` on every pull request. All checks must pass before merging.

## Contributing documentation

Docs live in `docs/src/` and are built with [mdBook](https://rust-lang.github.io/mdBook/).

To preview locally:

```sh
cargo install mdbook
mdbook serve docs
```

When writing user-facing documentation, describe what things do — not how they work internally. Doc-only pull requests are welcome.
