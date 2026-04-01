# Changelog

All notable changes to logana will be documented in this file.


## [Unreleased]

## [0.5.1] - 2026-04-01

### Added
- Infer year from file metadata for timestamp formats without year e.g. syslog/journalctl.
- Keep the selected line when filters change.

### Fixed
- Fixed mouse scroll in tail mode: scroll up pauses following, scroll down to the bottom resumes it.
- Filter for timestamp for formats without year.

## [0.5.0] - 2026-03-29
### Added
- Mouse support
- Support for compressed and archive files: `.gz`, `.bz2`, `.xz`, `.zip`, `.tar`, `.tar.gz`/`.tgz`, `.tar.bz2`/`.tbz2`, `.tar.xz`/`.txz`.
- Support for multiline log message, previously each line was treated as separate entry.

### Fixed
- Show an error in the notification bar on startup when the config file exists but cannot be read or parsed
- Selection via mouse when lines are wrapping both on filter and logs.

## [0.4.1] - 2026-03-25
### Changed
- Visual indication in the filter sidebar with different color when the filter mode is active.
- Removed background color from selected line, colors and inherited from line.

### Fixed
- Empty stdin placeholder tab is now removed when files are opened via directory selection or `:open`.
- Fix issue with single file being watched when multiple files were open.
- Scroll to make the active tab name always visible.
- Regex and literal include filters now correctly suppress lines that do not match them even when a date filter is also active. 
- Regex filter match counts are now correct when literal filters are also active. Previously evaluation of regex filters was being skipped for lines already matched by a lower-indexed literal filter, causing regex counts to be underreported.
- Text filters now match against the full display text for formats where level field is not represented as text (syslog `<PRI>` priority codes, DLT subtype). 

## [0.4.0] - 2026-03-21

### Added
- OTLP gRPC receiver: `:otlp` now defaults to gRPC on port 4317, matching the OTel SDK default export protocol. Use `:otlp --http` for the previous HTTP/JSON transport on port 4318. Custom ports still work: `:otlp 4317`, `:otlp --http 4318`.
- OTLP HTTP/JSON receiver: use `:otlp --http [port]` (default 4318) to open a tab that accepts `POST /v1/logs` from any OpenTelemetry SDK. Logs are parsed by the existing OTLP parser and support all filtering features. Session state is persisted and restored across restarts.
- `Left`/`Right` arrow keys now work as alternatives to `h`/`l` for horizontal scrolling in normal mode.
- Embedded MCP (Model Context Protocol) server, controllable via `:enable-mcp [--port N]` and `:disable-mcp` commands. Exposes marked lines, and annotations as MCP resources.
- `--mcp [PORT]` CLI flag to start the MCP server automatically on launch (port defaults to 9876).

### Changed
- `CommonLogParser`, `JournalctlParser`, and `SyslogParser` now use majority voting over the first 50 successfully parsed lines to select the format.
- Format detection now uses exclusivity-weighted scoring: lines matched by multiple parsers contribute proportionally less weight (1/N where N is the number of matching parsers), so format-exclusive lines drive selection. Syslog files containing priority-prefixed lines (`<PRI>...`) are now correctly detected as syslog even when most lines are ambiguous plain BSD-timestamp format.
- `SyslogParser` now recognises rsyslog's default file format (`RSYSLOG_FileFormat`): ISO 8601 timestamp with no `<PRI>` prefix. Files like `/var/log/syslog` written by modern rsyslog are now correctly detected as syslog instead of journalctl.
- Plain BSD-timestamp lines (`Oct 11 22:14:15 hostname tag: msg`) now resolve to syslog over journalctl, as both formats use this timestamp style but it is more common in syslog output.
- `TabState` filtering loops now use `parse_timestamp` instead of `parse_line` when only `@date:` filters are active.
- `TabState` decomposed into focused sub-structs (`ScrollState`, `FilterState`, `SearchState`, `CacheState`, `StreamState`, `DisplayConfig`, `InteractionState`) in `src/ui/tab_state/`.
- All popup UI surfaces extracted into ratatui `Widget` types in `src/ui/widgets/` (`ConfirmRestoreModal`, `ConfirmRestoreSessionModal`, `ConfirmOpenDirModal`, `CommentPopup`, `SelectFieldsPopup`, `DockerSelectPopup`, `DltSelectPopup`, `ValueColorsPopup`, `KeybindingsHelpPopup`).
- Popup widget tests moved to their respective widget modules; `render_popups.rs` deleted.
- `ModeBar` extracted as a ratatui `Widget` type in `src/ui/widgets/`.
- `App::ui()` decomposed into focused helper methods, each under 50 lines.
- MCP server no longer exposes the `logana://filtered` resource (filtered lines can be very large).
- Parser layer refactored
- Field names unified, now it's always the same name regardless of source format.

### Fixed
- Scroll position is now correctly restored after session restore.
- Eliminated a SIGBUS risk due usage of memory mapped files by swiching to pread solution.
- Mechanism to detect log rotation and truncation making the application resilient to non-appending changes to the file.
- OTLP parser now correctly sets the process color for `service.name` when it arrives as a top-level field (as produced by the OTLP HTTP receiver).
- `traceId`, `spanId`, and `telemetry.sdk.*` fields are now hidden by default in the OTLP parser; they can be shown via `:fields`.
- OTLP receiver now supports protobuf-encoded payloads (`application/x-protobuf`), the default format for most OTel SDKs. Previously protobuf requests were rejected with 415.
- OTLP receiver now supports gzip-encoded request bodies (`Content-Encoding: gzip`), which many OTel SDKs send by default. Previously gzip payloads were silently discarded.
- MCP server bind errors are now reported to the user instead of silently failing.
- Normalize all timestamps to UTC format.
- `$` in normal mode now leaves 4 columns of padding between the last character and the right edge of the viewport.

## [0.3.1] - 2026-03-16

### Fixed
- Fix per-filter match counts missing in sidebar when filter is given as CLI parameter.
- Fix flickering tab name due to connection retries.
- Fix issue with style priority between filters and value colors(e.g IP address).
- Fix parsing for Journalctl with short format (timestamp wihtout seconds)
- Filter was being applied twice when a path to a filter file was given as parameter.

### Added
- Extended journalctl output format support with `short`, `short-monotonic`, `short-unix`, `json-sse`, and `json-seq`.
- Added `scripts/release.sh` to automate version bumping, changelog update, commit, and tag creation.

### Changed
- Toggling filters off and back on with no changes in between no longer re-runs the full file scan.
- Default keybinding for saving a comment changed from `Ctrl+Enter` to `Ctrl+s` for compatibility with macOS Terminal.
- Performance improvements on file writing for headless mode and export/save commands, it now uses mmap/rayon to speed up the process.
- All stream sources (stdin, Docker, DLT TCP) now write to a temp file and use mmap with incremental indexing and filtering, keeping low memory usage.
- Headless mode now rejects `--output` paths that point to the same file as the input, preventing data loss.

## [0.3.0] - 2026-03-14

### Added
- DLT (AUTOSAR Diagnostic Log and Trace) format support: supports binary `.dlt` files and `dlt-convert' outuput.
- DLT client implementation, allows connecting to DLT daemon via TCP.
- `:reset` command to restore all settings to defaults and clear all persisted state (filters, marks, comments, hidden fields, session tabs, and app settings)

### Fixed
- Streaming tabs no longer flicker "Filtering…" in the tab name when new data arrives; filters are applied incrementally to new lines only
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
