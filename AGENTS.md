# Commit workflow

Prefix commit messages with the affected area: `[core]` for shared logic,
`[gui]` for the graphical interface, or `[tui]` for the terminal interface.
Use multiple prefixes when a commit affects more than one area.

When committing a user-facing change, update the `Unreleased` section of
`CHANGELOG.md` and include that update in the same commit. Internal changes that
do not affect users do not need a changelog entry.

# Development rules

- Maintain at least 80% test coverage.
- Use RED/GREEN TDD: write a failing test first, confirm it fails, then implement
  the smallest change that makes it pass. Refactor while keeping tests green.
- Always consider SOLID principles when designing code.
- Keep functions small and focused.
- Review your own changes with the rigor of an exceptionally demanding principal
  engineer, checking correctness, edge cases, clarity, maintainability, and tests.
- Keep type definitions at the top of each file, after imports.

# Validation

After finishing changes, always run both commands:

```sh
cargo test
```

```sh
cargo clippy --all-targets --all-features
```
