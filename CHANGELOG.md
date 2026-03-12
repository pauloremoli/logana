# Changelog

All notable changes to logana will be documented in this file.

## [Unreleased]

### Added
- Headless mode (`--headless`) — run the full filter pipeline without a TUI and write matching lines to stdout or a file via `--output`
- Keybinding conflict warnings are now shown in the status bar on startup instead of being printed to stderr; the bar grows up to 10 lines and is dismissed on the first keypress

### Fixed
- Search highlighting in raw mode now computes match offsets against raw bytes instead of parsed text, fixing incorrect highlight positions
- Viewport size is now reduced when the search or command bar is visible, preventing matches from being hidden behind the bar
- Cursor no longer disappears in visual character mode

### Changed
- Warning and error log lines use a distinct background color; marked lines use a blue background across all themes to avoid visual confusion with warning lines

---

## [0.1.0] - 2025-02-06

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
