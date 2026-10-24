# Testing

- Minimum 80% coverage; use RED/GREEN TDD (failing test first).
- After every change, run **both**:
  ```
  cargo test
  cargo clippy --all-targets --all-features
  ```
  `cargo build` passing is not enough — test code is compiled separately
  under `--all-targets`, and type-signature changes (e.g. `u16` →
  `Option<u16>`) can break test assertions that only surface there.

## Do not assume default state is empty

`TabState::new()` and `DisplayConfig::default()` pre-populate certain
fields by design (e.g. `level_colors_disabled` starts with
`["trace", "debug", "info", "notice"]` already disabled, since
warning/error/fatal are highlighted by default and the rest are not).
Before asserting that a `HashSet`/`bool` field changed from an assumed
baseline, inspect the actual `Default`/constructor implementation rather
than assuming a default state is empty or fully enabled.

Prefer asserting that one *specific, named entry* flipped
(`assert_ne!(after.contains(x), before.contains(x))`) rather than asserting
that the whole set went from empty to non-empty. This is robust regardless
of what the default composition turns out to be.

Note also: `GuiState::new()` calls `Config::load()`, which reads the real
`~/.config/logana/config.json` on the machine running the test — not a
hermetic default. This is a latent source of environment-dependent GUI
test flakiness if that file ever carries more than a harmless override.

## Use generic test data

Never copy real-looking, domain-specific example data (e.g. hardware-specific
terms such as `"operations"`, `"object_name"`, `"txCarrier1"`) from
existing fixtures into new tests, even if that data already exists
elsewhere in this codebase's tests. Use clearly generic placeholder names
instead (`"mygroup"`, `"key1"`, `"value1"`).

## Command autocomplete: test the actual Tab path

See `commands.md`. A test that only calls `resolve_completions` gives
false confidence. Drive `KeyCode::Tab` through `CommandMode::handle_key`
and assert that the input cycles through at least two distinct candidates.
