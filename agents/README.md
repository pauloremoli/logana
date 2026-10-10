# Agent rules for logana

This folder defines the rules an AI coding agent must follow when working
in this repository. It is sourced from `.claude/CLAUDE.md` and from
engineering feedback accumulated across prior sessions on this codebase.
It is agent-facing documentation; see `CONTRIBUTING.md` for
human-contributor documentation.

- [coding-standards.md](coding-standards.md) — TDD, SOLID, function size, and code review bar
- [testing.md](testing.md) — coverage requirements, verification process, and test-data hygiene
- [comments-and-docs.md](comments-and-docs.md) — comment policy and documentation/changelog style
- [architecture.md](architecture.md) — TUI/GUI separation rules and their rationale
- [commands.md](commands.md) — the two locations a new `:command` must be registered in
- [git-workflow.md](git-workflow.md) — commit style, backlog tracking, and prioritization
- [tooling.md](tooling.md) — CodeGraph usage
- [domain-notes.md](domain-notes.md) — product-behavior notes that inform feature design

## Validation

Run both commands before considering any change complete:

```
cargo test
cargo clippy --all-targets --all-features
```
