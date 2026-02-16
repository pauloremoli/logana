# logsmith-rs Architecture

Terminal-based log analysis tool built in Rust with SQLite persistence and a Ratatui TUI.

## Project Structure

```
src/
  main.rs         - Entry point, CLI args, runtime setup, app lifecycle
  lib.rs          - Library re-exports
  analyzer.rs     - Log parsing, ingestion, filtering (LogAnalyzer, LogParser, LogEntry)
  db.rs           - SQLite layer via sqlx (LogStore, FilterStore, FileContextStore traits)
  ui.rs           - Ratatui TUI (App, TabState, AppMode, rendering, keybindings)
  search.rs       - Regex search with match positions and navigation
  theme.rs        - JSON-based theme loading and color management
  commands.rs     - Commands that can be used to control the application
tests/
  integration.rs  - End-to-end flows (ingestion, filtering, marks, persistence)
  stdin.rs        - Stdin reading tests
.github/workflows/rust.yml - CI: fmt, clippy, test, coverage (80% threshold via tarpaulin)
```

## Core Data Models

**LogEntry**: `{ id, timestamp, hostname, process_name, pid, level: LogLevel, message, marked, source_file }`
**LogLevel**: Info | Warning | Error | Debug | Unknown
**Filter**: `{ id, pattern (regex), filter_type: Include|Exclude, enabled, color_config: Option<ColorConfig> }`
**ColorConfig**: `{ fg: Option<Color>, bg: Option<Color> }`
**FileContext**: Per-file session state (scroll, search query, wrap, sidebar, marked lines)

## Architecture Layers

### Database (db.rs)
- **Three trait abstractions**: `LogStore`, `FilterStore`, `FileContextStore`
- **Tables**: `log_entries` (indexed on level, process_name, source_file), `filters` (per source_file), `file_context` (PK: source_file)
- Batched inserts (500/tx), in-memory mode for tests, migration support
- Shared via `Arc<Database>` with `Arc<tokio::runtime::Runtime>` for async-sync bridging

### Log Parsing (analyzer.rs)
- **LogParser**: 4 regex patterns for syslog/journalctl variants (with/without PID, with/without level)
- **Level detection**: Structured field first, then content scanning fallback
- **LogAnalyzer**: Main API wrapping parser + DB + filters
  - `ingest_file()` / `ingest_file_chunk()` / `start_file_stream()` / `ingest_reader()`
  - `apply_filters()`: Include filters (OR logic), then exclude filters (AND-NOT logic)
  - Filter CRUD, mark/toggle, save/load filters as JSON

### Search (search.rs)
- Regex-based with case sensitivity toggle
- Returns `SearchResult { log_id, matches: Vec<(start, end)> }` with byte positions
- Wrapping next/previous navigation with current match tracking

### UI (ui.rs)
- **App** > **TabState** (each tab has its own LogAnalyzer, mode, scroll, caches)
- **AppMode**: Normal | Command | FilterManagement | FilterEdit | Search | ConfirmRestore
- **Multi-tab**: Tab/Shift+Tab switch, Ctrl+t open, Ctrl+w close
- **Vim keybindings**: j/k, gg/G, Ctrl+d/u, /, ?, n/N, m (mark)
- **Command mode** (:) with tab completion, history, live hints
- **Commands**: `filter`, `exclude`, `set-color`, `export-marked`, `save-filters`, `load-filters`, `wrap`, `set-theme`, `level-colors`, `open`, `close-tab`
- **Filter management mode** (f): navigate, toggle, delete, edit, set color, add include/exclude
- **Rendering**: Viewport-based, merges search highlights + process colors + filter colors + level colors at character level
- **Performance**: Dirty flags (`logs_dirty`, `filters_dirty`), async cache refresh, optimistic UI updates via `rt.spawn()`

### Theme (theme.rs)
- JSON files from `themes/` or `~/.config/logsmith-rs/themes/`
- Colors: hex `"#RRGGBB"` or RGB array `[r, g, b]`
- Default: Dracula theme (hardcoded)
- Fields: root_bg, border, text, text_highlight, error_fg, warning_fg, process_colors

## Key Patterns

- **Repository pattern**: DB traits abstract storage (enables in-memory testing)
- **Async/sync bridge**: `rt.block_on()` for sync callers, `rt.spawn()` for fire-and-forget writes
- **Background streaming**: `mpsc::channel` for chunked file loading (5000 lines/chunk, initial 200 for fast display)
- **Session persistence**: Filters + UI context saved per source_file, prompt to restore on reopen
- **Optimistic updates**: UI cache updated immediately, DB write spawned async

## App Lifecycle

1. Parse CLI args (optional file path)
2. Init tokio runtime + SQLite DB (`~/.local/share/logsmith-rs/logsmith.db`)
3. Clear previous logs (filters persist per file)
4. If file: ingest initial chunk (200 lines), start background stream for rest
5. If stdin: ingest all from reader
6. Enter terminal raw mode, create App with theme
7. Check for saved FileContext, prompt restore
8. **Event loop** (16ms poll if async pending, 250ms otherwise): handle keys → poll background loads → poll cache refresh → render
9. On exit: save context, restore terminal

## Dependencies

anyhow, clap (derive), regex, ratatui 0.26, crossterm 0.27, serde/serde_json, sqlx 0.8 (sqlite, tokio), tokio (rt-multi-thread), async-trait, dirs, tempfile, unicode-width

## Testing

- **Unit tests**: db.rs (25+), analyzer.rs (50+), search.rs (10+), ui.rs (70+)
- **Integration tests**: Full ingestion/query, filter combos, mark/export, search, JSON roundtrip, DB persistence, background loading
- **CI**: cargo fmt → clippy → test → tarpaulin coverage (enforces 80%)
