# Changelog

All notable changes to logana will be documented in this file.


## [Unreleased]

### Added
- Add relative line numbers, toggleable via `:relative-line-numbers` and the UI options menu.
- Add `:collapse`/`:expand` to hide or reveal continuation lines file-wide, with `>`/`<` to expand or collapse the entry under the cursor at any time.
- Add a Groups section to the bottom of the filter sidebar (at least 8 rows even when empty), showing each group's filter count and style under a Groups label, clickable to toggle, recolor, or clear its style; toggleable via the UI options menu.
- Allow `:group <name>` with no flags to create a group with no style.
- Add `Ctrl+g` in normal mode to enter group management, and `a`/`e` inside it to add or edit a group.

### Fixed
- Fix filter groups being shared across tabs/files instead of scoped per file like filters, including `:toggle-group` affecting other tabs and `:save-filters`/`:load-filters` leaking groups between them.
- Fix a group's color not reapplying to lines already highlighted via that group's style after editing it.
- Fix a multiline schema's `continuation.end_pattern` line becoming its own entry instead of collapsing with the block it terminates.
- Fix `:collapse`/`:expand` not applying to lines appended to a live/watched file after the command ran.
- Fix collapsed continuation lines not staying collapsed when reopening a file across sessions.
- Fix `e`/`w` error/warning navigation in a merged tab misclassifying lines by ignoring each line's own source format.
- Fix a crash when opening multiple files at once from the directory or archive picker.

## [0.7.6] - 2026-08-06

### Added
- Custom schema files now support a `$schema` line and reject unknown keys, matching `config.json`'s validation.
- Custom schemas can set `"multiline": true` to fold continuation lines into the record's `message` field.
- Custom schemas can declare a `continuation` block to extract structured fields (including embedded JSON) from continuation lines, optionally bounded by an `end_pattern`.
- `:schema none` clears a tab's schema, treating its content as plain text.
- `:group` sets a predefined color style for a filter group, used as a fallback by filters in the group that have no color of their own.
- Archive/directory picker: `Ctrl+e`/`Alt+m` toggle extraction/merge marks while searching, without leaving search.
- Archive/directory picker: `Ctrl+a`/`Ctrl+Alt+m` mark every row matching the search for extraction/merging in one press.
- `Ctrl+p` opens a searchable popup to switch between open files.
- `:theme` opens a searchable popup to browse and live-preview color themes.
- Click a tab in the tab bar to switch to it.

## [0.7.5] - 2026-07-18

### Added
- `a` keybinding (Normal mode and filter manager) to add an include filter with an automatically generated filter style (bg/fg).
- `:default-filters` configures a filter file to auto-load whenever a format is assigned to a tab with no filters yet, per format name.

### Changed
- Command-mode help (typing `:filter`, `:highlight`, etc.) now shows usage, description, and examples as separate, distinctly styled lines.
- `:schema `'s autocomplete now lists built-in log formats alongside custom schemas — custom schemas first (alphabetical), then built-in formats (alphabetical).

### Fixed
- A schema with an invalid regex/template no longer leaks its error to stderr, corrupting the TUI — it now shows in the startup-warning notification.

## [0.7.4] - 2026-07-15

### Added
- Archive picker: mark files with `m` to extract and merge them into one timestamp-sorted tab (instead of opening each as its own tab).
- Archive picker now supports the same navigation as the filter sidebar and log panel: count-prefixed `j`/`k`, `Ctrl+d`/`Ctrl+u` , `PageDown`/`PageUp` (full page), and `gg`/`G`. 
- Archive picker: nested archives can be expanded/collapsed with the Right/Left arrow keys.
- `:filter`/`:highlight` now accept `--auto`/`-a` to generate a random fg/bg color pair with guaranteed readable contrast, instead of picking `--fg`/`--bg` yourself.
- Custom keybindings in `config.json` can now bind a key to a fixed command line (`keybindings.custom`), e.g. binding a key to `load-filters ~/logs/filters/myfilter.json`.

### Changed
- Archive listing now only auto-decompresses one nested level, with option to expand compressed files on demand.
- Opening a directory now reuses the archive picker's tree/checkbox/merge-mark UI instead of a plain "open all files?".
- `:save` on a temporary tabs (an extracted archive file, or a picker-triggered merge) now switches that tab to the file you just saved — dropping the temp copy.
- Improve rendering performance on archive picker navigation and search on large archives.

### Fixed
- Merged tab source-file labels no longer leak into the actual line content.
- A merged tab's title bar no longer shows the misleading "[unknown format]" when every source shares the same detected format (e.g. merging two journalctl files).
- Merge from the archive/directory picker for big files no longer freezes the UI.
- A merge from the archive/directory picker is now fully self-contained: each merge-marked source and the final merged result are saved to temp files.
- Fixed a crash (`index out of bounds`) that could happen after a picker-triggered merge failed (e.g. an unrecognized format)
- Fixed autocomplete for `:toggle-group` and and `:sidebar-position` commands.

## [0.7.3] - 2026-07-13

### Fixed
- Visual Char Mode's word motions (`w`/`e`/`b`), `/` search match offsets, `y`/`Y`/visual-line yank, and `:export` no longer diverge from what's actually rendered on a custom schema's `template` line.

## [0.7.2] - 2026-07-13

### Added
- Filter manager now supports the same navigation as the log panel: count-prefixed `j`/`k`, `Ctrl+d`/`Ctrl+u`, `PageDown`/`PageUp`, `gg`/`G`, and `/`.
- Archive file picker now supports `/` to search the file tree using a regex query (same fallback behavior as the filter list search).
- A startup warning now appears if a user-provided schema in `~/.config/logana/schema/` fails to load (malformed JSON) or fails to compile (invalid template/pattern).
- `:filter`/`:exclude`/`:highlight --field` can now be repeated to require several parsed fields at once.
- Log lines parsed by a custom schema's `template` now render using that schema's own field order and literal separators (e.g. `{level}/{component}/{feature}`) instead of a generic space-joined column layout. 
- Custom schemas can now declare which raw `level` values count as error/warning via a `levels` config key so that `e`/`w` navigation and level coloring works for values that don't match the built-in keywords.
- `:filter`/`:exclude`/`:highlight` now accept `--ignore-case`/`-i` to match regardless of case (text or `--regex` patterns). Case-insensitive filters show an `[i]` tag in the sidebar.
- `:select-fields` now has an option to reset the fields to the default order with everything visible.

### Fixed
- Extracting a selected archive file that is itself a compressed file (e.g. a `.gz` nested inside a `.zip`) now decompresses it, instead of opening the raw compressed bytes.
- `:load-filters`, `:save-filters`, and `:export` now expand a leading `~` in the given path.
- `:show-all-fields` now also clears any custom column order from a prior `:select-fields` reorder.

## [0.7.1] - 2026-07-11

### Added
- Introduce highligh mode that uses filter styling but without excluding any line.
- New filter type for highlight for applying custom styling but without affecting log visibility.

### Changed
- Improve archive handling with pop up to select files to extract.

## [0.7.0] - 2026-07-11

### Added
- Filters can be assigned to a named group with `--group <name>` on `:filter`/`:exclude`. 
- Command to toggle every filter in a group on/off together with `:toggle-group <name>`. 
- User-defined log formats can be provided in schema folder.

### Fixed
- Make the order of fields in select fields pop-up consistent with the order rendered in log panel.

### Changed
- Scroll with mouse wheel on filters sidebar. 
- Filters visual on sidebar is applied only to text.

## [0.6.0] - 2026-04-26

### Added
- Export windows allows filling the placeholders from the template (e.g. Context, next steps and conclusion).
- Merge view: merge different source into a single view sorted by timestamp.
- Generate JSON schema for config file validation.

### Fixed
- Fixed crash when applying a regex filter after the log file grew past the size it had when the continuation map was built.

## [0.5.1] - 2026-04-02

### Added
- `:sidebar-position left|right` command to move the filter sidebar to either side of the log panel. Persisted across sessions.
- `:line-numbers` command to toggle the line number gutter.
- Infer year from file metadata for timestamp formats without year e.g. syslog/journalctl.
- Keep the selected line when filters change.

### Fixed
- Fixed mouse scroll in tail mode: scroll up pauses following, scroll down to the bottom resumes it.
- Filter for timestamp for formats without year.
- Parsing of unix epoch timestamp for syslog/journalctl.

### Changed
- Performance improvements on CLF parser
- Theme, sidebar visibility, borders, wrap, and line numbers are now persisted to the database when changed at runtime via commands or the UI options menu (`u`). Previously only `show_mode_bar` was persisted.
- Regex filters now require an explicit `--regex` / `-r` flag
- Literal filters no longer require quoting or escaping — multi-word patterns are accepted as-is (e.g. `:filter connection refused`).
- All filter options (`--regex`/`-r`, `--fg`, `--bg`, `-l`, `--field`) must appear before the pattern
- Regex patterns with spaces work without quoting — words following `-r` are joined (e.g. `:filter -r \d{3} \d+`).
- Editing a filter from the filter manager no longer wraps the pattern in quotes.

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
