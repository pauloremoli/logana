# Changelog

All notable changes to logana will be documented in this file.


## [Unreleased]

### Added
- `:reset` command to restore all settings to defaults and clear all persisted state (filters, marks, comments, hidden fields, session tabs, and app settings)

### Fixed
- The global filtering toggle state (on/off) is now persisted across sessions via `file_context` in the database (schema v9)

### Changed
- Whole-buffer AC scan: when only text filters are active and a combined Aho-Corasick automaton is available, the filter scan now runs a single AC pass over the contiguous data buffer per rayon sub-chunk instead of calling `ac.find_iter()` per line.
- Selected (cursor) line is now rendered with bold and underline modifiers, making it visually distinct even when surrounded by highlighted lines (log-level colors, search matches, filters)
- Filter results now stream in chunks: the first matching lines appear immediately while the remainder of the file is scanned in the background
- Filter pipeline performance: the Aho-Corasick automaton is now scanned once per line instead of twice (previously `count_line_matches` and `evaluate_text` each triggered a full scan).
- Log lines are also parsed at most once when date or field filters are active (previously parsed separately for counting and for visibility) 
- `parse_line` is now skipped entirely for hidden lines
- Replace all atomics writes from the hot path with thread local counter
- Headless mode now runs the filter scan in parallel and writes results sequentially
- Refactored UI module into smaller functions.


## [0.2.1] - 2026-03-13

### Added
- Incremental search delivery — results are streamed in chunks of 5,000 lines so the first matches appear almost immediately on large files instead of waiting for the full scan to complete
- Aho-Corasick acceleration for literal (non-regex) search patterns, matching the fast path already used by filters; regex patterns continue to use the regex engine

### Fixed
- Changing a filter's color (`set-color`) no longer triggers a full file rescan; only the render cache is invalidated so visible lines update instantly
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
