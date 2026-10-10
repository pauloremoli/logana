# Adding a new `:command`

Two locations must be updated; omitting either leaves the feature only
partially implemented:

1. **`src/commands/parser.rs`** — add the `Commands::Foo` variant.
2. **`src/commands/info.rs`** — add a matching `CommandInfo { name, usage,
   description }` entry to the `COMMANDS` slice. This entry drives both
   auto-completion and the help/command list in the UI. A command added
   only to the parser is not visible to users.

## Commands with argument autocomplete

This codebase maintains two separate, duplicated autocomplete
implementations. Both must be updated when a command's argument supports
autocomplete:

1. `resolve_completions` in `src/ui/widgets/command_bar.rs` feeds only the
   completion hint list shown under the command bar. Updating this alone
   makes the feature appear complete, since the hint list shows valid
   candidates, but it does not drive the actual completion behavior.
2. `CommandMode::completions_for` in `src/mode/command_mode.rs` is what
   the Tab key actually invokes (`compute_completions` →
   `completions_for`, from the `KeyCode::Tab` handler). If a command has
   no branch here, Tab falls through to `find_command_completions`, which
   fuzzy-matches command names instead — so `:toggle-group ` followed by
   Tab silently re-inserts `toggle-group` rather than cycling through
   argument values. This fails silently; no error is raised.

Add a matching branch to both functions, following the existing
`hide-field`/`show-field`/`toggle-group`/`sidebar-position` cases. Then add
a test that drives `KeyCode::Tab` through `CommandMode::handle_key` — not
only `compute_completions`, and not `resolve_completions` alone — and
asserts that the input cycles through at least two distinct candidates.
