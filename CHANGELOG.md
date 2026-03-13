# Changelog

All notable changes to logana will be documented in this file.


## [For next release]

### Added
- Incremental search delivery — results are streamed in chunks of 5,000 lines so the first matches appear almost immediately on large files instead of waiting for the full scan to complete
- Aho-Corasick acceleration for literal (non-regex) search patterns, matching the fast path already used by filters; regex patterns continue to use the regex engine

### Fixed
- Headless mode (`--headless`) no longer touches the real database before dispatching; it now exits early (before `LogManager` construction) so no saved session state — filters, marks, scroll position — from previous TUI runs can be inadvertently applied. Output is determined solely by the parameters given (`-f`, `-i`, `-o`, `-t`).
- Headless mode now rejects directory arguments with a clear error message instead of failing with a raw I/O error.

## [0.2.0] - 2025-03-13

### Added
- Headless mode (`--headless`) — run the full filter pipeline without a TUI and write matching lines to stdout or a file via `--output`
- Keybinding conflict warnings are now shown in the status bar on startup instead of being printed to stderr; the bar grows up to 10 lines and is dismissed on the first keypress
- Tab-completion for `:hide-field` suggests all known field names; `:show-field` suggests currently hidden fields (falls back to all fields when none are hidden)

### Fixed
- Filtering with multiple literal include filters now performs a single Aho-Corasick scan per line instead of one scan per filter, eliminating O(N) slowdown on large files with N include patterns
- Search highlighting in raw mode now computes match offsets against raw bytes instead of parsed text, fixing incorrect highlight positions
- Viewport size is now reduced when the search or command bar is visible, preventing matches from being hidden behind the bar
- Cursor no longer disappears in visual character mode
- `:hide-field <N>` now correctly resolves the index against the currently **visible** (non-hidden) fields instead of all fields, so index 0 always refers to the first field shown on screen
- `:show-field` now accepts field names only; numeric arguments are no longer misinterpreted as indices

### Changed
- Marked lines now have a different visual than warning lines.
- Return to normal mode after UI commands.

---

## [0.1.0] - 2025-03-11

### Added
- Auto-detected log formats: JSON (bunyan, pino, tracing-subscriber), syslog RFC 3164/5424, journalctl, logfmt, Common/Combined log, OTel, env_logger, logback, log4j2, Spring Boot, Python logging, loguru, structlog
- Real-time filtering with include/exclude patterns (literal or regex), date-range filters, and field-scoped filters
- CLI filter flags: `-i` (include), `-o` (exclude), `-t` (timestamp), `-f` (load filter file), `--tail`
- Persistent sessions via SQLite: filters, scroll position, marks, comments, and field layout restored across runs
- Configurable restore policy (`ask` / `always` / `never`) for sessions and per-file context
- Structured field view: parsed timestamps, levels, targets, and extra fields displayed in columns; show/hide/reorder via `:select-fields`
- Vim-style navigation: `j`/`k`, `gg`/`G`, `Ctrl+d`/`u`, count prefixes (`5j`, `10G`), `/`/`?` search, `e`/`w` error/warning jumps
- Visual line mode (`V`) and visual character mode (`v`) with yank support
- Multiline annotations (`c`) attached to single lines or visual selections; export to Markdown or Jira via `:export`
- Docker container streaming via `:docker`
- Multi-tab support: `Ctrl+t` / `Ctrl+w` / `Tab` / `Shift+Tab`
- Raw mode (`:raw`) — bypass format parser and display unformatted bytes
- Value coloring for HTTP methods, status codes, IP addresses, and UUIDs
- Fully configurable keybindings via `~/.config/logana/config.json`
- 22 bundled themes (17 dark, 5 light)
- Custom theme support via `~/.config/logana/themes/`
- Tab completion for commands, flags, field names, field values, colors, themes, and file paths
- Autocomplete for filter command parameters
- Background filtering with parallel Rayon workers and live progress bar
- Memory-mapped I/O with SIMD-accelerated line indexing
- Single-pass optimization when combining `--filters` with file loading
- Cached rendering pipeline for high-performance redraws
- Line number gutter
- Horizontal scrolling
- Line wrap toggle (`:wrap` / `w` in UI mode)
- Sidebar resize (`>` / `<` in filter manager)
- Filter match counters per filter
- Save and load filter sets (`:save-filters` / `:load-filters`)
- Tail mode (`--tail` / `:tail`) — auto-scroll on new content
- File watcher for live file updates
- Directory argument — opens each file in its own tab
- Session restore for Docker tabs
- Keybindings help overlay (`F1`)
- UI toggles menu (`u`): sidebar, mode bar, borders, line wrap
