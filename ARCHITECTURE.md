# logsmith-rs Architecture

Terminal-based log analysis tool built in Rust with a Ratatui TUI. Logs are read via memory-mapped files; filters and UI context are persisted in SQLite.

## Project Structure

```
src/
  main.rs         - Entry point, CLI args, runtime setup, app lifecycle
  lib.rs          - Library re-exports
  types.rs        - Shared data types (LogLevel, FilterType, FilterDef, ColorConfig, SearchResult)
  file_reader.rs  - Memory-mapped file I/O with SIMD line indexing (FileReader)
  filters.rs      - Filter pipeline: Filter trait, SubstringFilter, RegexFilter, FilterManager, render_line
  log_manager.rs  - Filter/mark state management + SQLite persistence bridge (LogManager)
  db.rs           - SQLite layer via sqlx (FilterStore, FileContextStore traits)
  search.rs       - Regex search with match positions and wrapping navigation
  ui.rs           - Ratatui TUI (App, TabState, AppMode, rendering, keybindings)
  theme.rs        - JSON-based theme loading and color management
  config.rs       - JSON config file loading (Config, Keybindings, KeyBinding)
  log_line.rs     - Syslog/journalctl line parser (LogLine)
  auto_complete.rs - Tab completion for commands, colors, and file paths
  commands.rs     - Clap-based command definitions for the command bar
tests/
  integration.rs  - End-to-end flows (file reading, filtering, marks, search, filter CRUD)
  stdin.rs        - Stdin reading tests
.github/workflows/rust.yml - CI: fmt, clippy, test, coverage (80% threshold via tarpaulin)
```

## Core Data Models

**LogLevel**: `Info | Warning | Error | Debug | Unknown`
Detected by scanning raw bytes with a fast case-insensitive byte-window scan (`detect_from_bytes`).

**FilterType**: `Include | Exclude`

**ColorConfig**: `{ fg: Option<Color>, bg: Option<Color>, match_only: bool }`
When `match_only` is true only matched byte spans are highlighted; otherwise the whole line is coloured.

**FilterDef**: `{ id, pattern, filter_type, enabled, color_config: Option<ColorConfig> }`
Persisted to SQLite. Renamed from `Filter` to avoid conflict with the `Filter` trait.

**SearchResult**: `{ line_idx: usize, matches: Vec<(start, end)> }`
Byte positions within the line.

**Annotation**: `{ text: String, line_indices: Vec<usize> }`
A multiline comment attached to a group of log lines. Multiple annotations can exist per session. `text` is newline-joined and `line_indices` are raw file-line indices (same space as `visible_indices`).

**FieldLayout**: `{ json_columns: Option<Vec<String>> }` — when `Some`, only the listed column names are shown in that order; `None` restores default display order. Held by `TabState`; not persisted.

**FileContext**: Per-source session state (scroll_offset, search_query, wrap, sidebar, marked_lines, file_hash, show_line_numbers, horizontal_scroll, annotations).

## Architecture Layers

### File I/O (file_reader.rs)

- **FileReader**: Zero-copy random access backed by either a `memmap2::Mmap` (files) or a `Vec<u8>` (stdin / tests).
- **Line indexing**: `memchr::memchr_iter` scans for `\n` bytes at startup, building a `Vec<usize>` of line start offsets in one pass.
- **`get_line(idx)`**: O(1) slice into the backing storage — no heap allocation per line.
- **`line_count()`**: Skips the phantom empty entry after a trailing newline.
- **`from_bytes(Vec<u8>)`**: Used for stdin input and in-memory test data.

### Filter Pipeline (filters.rs)

- **`Filter` trait**: `fn evaluate(&self, line: &[u8], collector: &mut MatchCollector) -> FilterDecision`
- **`FilterDecision`**: `Include | Exclude | Neutral`
- **`SubstringFilter`**: Aho-Corasick (`aho-corasick` crate) for literal patterns — selected automatically when the pattern contains no regex metacharacters.
- **`RegexFilter`**: `regex` crate fallback for patterns with metacharacters.
- **`build_filter(pattern, decision, match_only, style_id)`**: Dispatches to the correct implementation.
- **`FilterManager`**:
  - `compute_visible(&FileReader) -> Vec<usize>` — parallel evaluation via `rayon::into_par_iter()`, returns ascending sorted indices.
  - `evaluate_line(&[u8]) -> MatchCollector` — collects styled byte spans for rendering.
  - `is_visible(&[u8]) -> bool` — logic: if any enabled Include filter exists, line must match one; any Exclude match hides the line regardless.
- **`MatchCollector`**: Accumulates `MatchSpan { start, end, style: StyleId, priority }` for a single line.
- **`StyleId`** (`u8`): Index into the 256-slot styles array. `SEARCH_STYLE_ID = u8::MAX = 255` is reserved for search highlights.
- **`render_line(&MatchCollector, &[Style]) -> Line`**: Flattens overlapping spans by priority order into a ratatui `Line` of styled `Span`s.

### Log Manager (log_manager.rs)

- **`LogManager`**: Owns `filter_defs: Vec<FilterDef>` (in-memory, DB-backed), `marks: HashSet<usize>` (in-memory only), and `annotations: Vec<Annotation>` (in-memory only). Does **not** own the `FileReader`.
- **Filter CRUD**: `add_filter_with_color`, `remove_filter`, `toggle_filter`, `edit_filter`, `move_filter_up/down`, `set_color_config`, `clear_filters`, `save_filters` (JSON), `load_filters` (JSON).
- **`build_filter_manager() -> (FilterManager, Vec<Style>)`**: Converts enabled `FilterDef`s into a renderable `FilterManager` + parallel style palette (one `Style` per enabled filter, indexed by `StyleId`).
- **Marks**: `toggle_mark`, `is_marked`, `get_marked_indices`, `get_marked_lines(&FileReader)`.
- **Annotations**: `add_annotation(text, line_indices)`, `get_annotations() -> &[Annotation]`, `has_annotation(line_idx) -> bool`, `set_annotations(Vec<Annotation>)`. Multiple annotation groups can share the same log lines.
- **DB bridge**: Filter mutations are fully `async` — all methods on `LogManager` are `async fn` and `await` their DB calls directly. On construction, filters are loaded from DB via `reload_filters_from_db().await`.
- **File hash**: `compute_file_hash(path)` hashes file size + mtime for change detection.

### Database (db.rs)

- **Three trait abstractions**: `FilterStore`, `FileContextStore`, `SessionStore`.
- **Tables**: `filters` (per `source_file`), `file_context` (PK: `source_file`), `session_tabs` (ordered list of last-open source files).
- **`FilterStore`**: `get_filters`, `get_filters_for_source`, `clear_filters_for_source`, `replace_all_filters`, `insert_filter`, `delete_filter`, `toggle_filter`, `update_filter_pattern`, `update_filter_color`, `swap_filter_order`.
- **`FileContextStore`**: `save_file_context`, `load_file_context`.
- **`SessionStore`**: `save_session(&[String])`, `load_session() -> Vec<String>` — persists the ordered list of open tabs across runs.
- In-memory mode (`Database::in_memory()`) for tests; migration support.
- Shared via `Arc<Database>`; callers use `.await` directly within the tokio runtime.

### Search (search.rs)

- Regex-based search over `visible_indices` only (respects active filters).
- `Search::search(pattern, &[usize], &FileReader)` — builds `Vec<SearchResult>` with byte-position match spans.
- Wrapping `next_match()` / `previous_match()` navigation.
- Case sensitivity toggle (`set_case_sensitive`).

### UI (ui.rs)

- **`App`** owns a `Vec<TabState>`, the global theme, and an `Arc<Keybindings>` shared across all tabs.
- **`TabState`** owns:
  - `file_reader: FileReader` — the backing log data
  - `log_manager: LogManager` — filter defs and marks
  - `visible_indices: Vec<usize>` — indices of currently visible lines under active filters
  - `scroll_offset: usize` — selected line (index into `visible_indices`)
  - `viewport_offset: usize` — first rendered line (index into `visible_indices`)
  - `visible_height: usize` — content rows available (updated each render frame)
  - `keybindings: Arc<Keybindings>` — shared keybinding config (cloned from `App` on tab creation)
  - `mode: Box<dyn Mode>`, `command_history: Vec<String>`, `search: Search`, plus display flags
- **`Mode` trait**: Each mode owns its key-handling logic via `handle_key(self: Box<Self>, tab, key, modifiers) -> (Box<dyn Mode>, KeyResult)`. Unhandled keys return `KeyResult::Ignored`, falling through to `App::handle_global_key` (quit, Tab switch, Ctrl+w/t). `KeyResult::ExecuteCommand(cmd)` triggers `App::execute_command_str`.
- **Mode structs**: `NormalMode`, `CommandMode` (with tab completion, history), `FilterManagementMode`, `FilterEditMode`, `SearchMode`, `ConfirmRestoreMode`, `ConfirmRestoreSessionMode`, `VisualLineMode`, `AnnotationMode`, `KeybindingsHelpMode`, `SelectFieldsMode`. Rendering data is exposed through trait methods: `status_line()`, `dynamic_status_line(&Keybindings)`, `selected_filter_index()`, `command_state()`, `search_state()`, `needs_input_bar()`, `confirm_restore_context()`, `confirm_restore_session_files()`, `visual_selection_anchor()`, `annotation_popup()`, `keybindings_help_scroll()`, `keybindings_help_search()`, `select_fields_state()`.
- **`refresh_visible()`**: Rebuilds `visible_indices` by calling `FilterManager::compute_visible(&file_reader)`.

**Rendering pipeline (per frame)**:
1. Compute `visible_height = logs_area.height - 2` (subtract Block borders).
2. Compute `inner_width` (terminal columns available inside borders, minus line-number prefix).
3. Wrap-aware viewport adjustment: when wrap is ON, sums terminal rows (via `line_row_count`) from `viewport_offset` to `scroll_offset`; scrolls when total exceeds `visible_height`.
4. Wrap-aware `end` computation: walks from `start` accumulating `line_row_count()` until `visible_height` is filled.
5. Build `FilterManager` + 256-slot styles array (filter styles at 0..N, search style at index 255).
6. For each line in `[start..end]`: evaluate filters (`evaluate_line`), overlay search spans at priority 1000, apply level colours and mark styles, compose final `Line` via `render_line`.
7. `line_row_count(bytes, inner_width)` uses `unicode_width` to compute `ceil(display_width / inner_width)`, keeping wrap-aware viewport math precise.

**JSON field layout**: `apply_json_field_layout(&JsonDisplayParts, &FieldLayout) -> Vec<String>` — module-level helper that routes through `default_json_cols` (all columns, default order) or picks specific columns via `get_json_col`. Column name aliases: `timestamp|ts|time`, `level|lvl`, `target`, `span`, `message|msg`, plus any extra-field key. Tab completion for the `fields` command completes against the five canonical names plus dynamically discovered field names from the first 200 visible log lines (`TabState::collect_json_field_names()`).
**Select-fields mode** (`:select-fields`): floating popup showing all discovered JSON fields with checkboxes. `j`/`k` navigate, `Space` toggle, `a`/`n` enable/disable all, `Enter` apply, `Esc` cancel. Implemented by `SelectFieldsMode` in `src/mode/select_fields_mode.rs`.
**Vim keybindings**: j/k, gg/G, Ctrl+d/u (half page), PageUp/Down, /, ?, n/N, m (mark), V (visual select)
**Visual line mode** (`V`): anchor at current line, j/k extend selection, `c` opens annotation editor, Esc cancel. Selected range highlighted in the log panel.
**Annotation mode**: multiline text editor (Enter = newline, Backspace = delete/merge, Left/Right wrap lines, Up/Down move rows, Shift+Enter = save (configurable), Esc = cancel). Rendered as a floating popup. Annotated lines show a `◆` indicator in the line-number margin.
**Keybindings help** (`F1`): floating popup listing all configured keybindings grouped by mode. Type to fuzzy-search, j/k scroll, Esc/q/F1 close. The status bar reflects the actual configured keybinding strings.
**Conflict validation**: at startup `Keybindings::validate()` checks for overlapping bindings within each mode scope; conflicts are printed to stderr and logged as warnings.
**Multi-tab**: Tab/Shift+Tab switch, Ctrl+t open, Ctrl+w close
**Command mode** (`:`) with tab completion, history, live hints
**Commands**: `filter`, `exclude`, `set-color`, `export-marked`, `save-filters`, `load-filters`, `wrap`, `set-theme`, `level-colors`, `open`, `close-tab`, `hide-field`, `show-field`, `show-all-fields`, `fields [col...]`, `select-fields`
**Filter management mode** (`f`): navigate, toggle, delete, edit, set color, add include/exclude

### Config (config.rs)

- **Config file**: `~/.config/logsmith-rs/config.json` (loaded at startup; falls back to defaults on parse/IO error — never prevents startup).
- **`Config`**: `{ theme: Option<String>, keybindings: Keybindings }`. `theme` is a theme name without the `.json` extension (e.g. `"dracula"`).
- **`Keybindings`**: groups `NormalKeybindings`, `FilterKeybindings`, `GlobalKeybindings`, `AnnotationKeybindings` — each with `#[serde(default)]` so any absent field uses its built-in default.
- **`KeyBindings`** (per action): a `Vec<KeyBinding>` — each action supports multiple alternative keys (e.g. `"j"` and `"Down"` for scroll down). Accepts a single JSON string or an array of strings.
- **`KeyBinding`**: parsed from strings like `"j"`, `"Ctrl+d"`, `"Shift+Tab"`, `"F1"`, `"PageDown"`, `"Space"`, `"Esc"`. `"Shift+Tab"` maps to `KeyCode::BackTab`. `matches(key, modifiers)`: for `Char` keys accepts `NONE` or `SHIFT` (terminals vary); for non-`Char` keys (Enter, F-keys, etc.) requires an exact SHIFT match so `"Shift+Enter"` ≠ plain `"Enter"`.
- **`Keybindings::validate() -> Vec<String>`**: checks all (action, keybinding) pairs within each mode scope (normal + global, filter + global) for overlaps and returns human-readable conflict descriptions. Called at startup; conflicts are printed to stderr and logged.
- **Sharing**: `Arc<Keybindings>` is held by `App` and cloned into each `TabState` when tabs are created (including session restores and new tabs opened via commands).
- **Default keybindings** exactly match the previously hardcoded key assignments, so the config file is fully optional. Default for `annotation.save` is `Shift+Enter`; `normal.show_keybindings` is `F1`.

Example `~/.config/logsmith-rs/config.json`:
```json
{
  "theme": "dracula",
  "keybindings": {
    "normal": { "scroll_down": ["j", "Down"], "half_page_down": "Ctrl+d" },
    "global": { "quit": "q" }
  }
}
```

### Theme (theme.rs)

- JSON files from `themes/` or `~/.config/logsmith-rs/themes/`
- Colors: hex `"#RRGGBB"` or RGB array `[r, g, b]`
- Default: Dracula theme (hardcoded)
- Fields: `root_bg`, `border`, `border_title`, `text`, `text_highlight`, `error_fg`, `warning_fg`
- **`Theme::list_available_themes() -> Vec<String>`**: Scans both theme directories, returns sorted names (no extension).
- **`fuzzy_match(needle, haystack) -> bool`**: Case-insensitive subsequence check; used for `set-theme` tab completion.

## Key Patterns

- **Zero-copy reads**: `FileReader::get_line` returns `&[u8]` slices directly into the mmap — no per-line allocation.
- **Parallel filter evaluation**: `FilterManager::compute_visible` uses `rayon::into_par_iter()` over line indices; order is preserved by rayon's indexed parallel iterator.
- **Dual filter backends**: Aho-Corasick for literals (O(n) multi-pattern), Regex fallback for metacharacter patterns. Selected automatically by `build_filter`.
- **StyleId dispatch**: 256-slot `Vec<Style>` indexed by `u8` avoids per-span HashMap lookups at render time.
- **Wrap-aware viewport**: `line_row_count` (unicode_width) drives both the scroll trigger and the `[start..end]` window, so the selected line is always on-screen regardless of line length.
- **Async DB access**: All `LogManager` methods are `async fn` and `await` DB calls directly. No `block_on` or manual runtime bridging.
- **Repository pattern**: `FilterStore` / `FileContextStore` traits enable in-memory SQLite for tests.
- **Session persistence**: Filters + UI context saved per `source_file`; hash-verified restore prompt on reopen.

## App Lifecycle

1. Parse CLI args (optional file path).
2. Init tokio runtime + SQLite DB (`~/.local/share/logsmith-rs/logsmith.db`).
3. Load `Config` from `~/.config/logsmith-rs/config.json` (or defaults on missing/parse error).
4. Build `FileReader` from file path (mmap) or stdin (bytes).
5. Build `LogManager` — loads filters from DB for this source.
6. Enter terminal raw mode, create `App` with theme and `Arc<Keybindings>`.
7. If a file was opened: check for saved `FileContext`, prompt per-file restore (`ConfirmRestoreMode`). If no file and no piped data: check for a saved session (`session_tabs`), prompt session restore (`ConfirmRestoreSessionMode`). On confirm, all session files are opened and their per-file contexts auto-applied without additional prompts.
8. **Event loop** (250ms poll): render frame → wait for key event → handle key → repeat.
9. On exit: save `FileContext` for each tab + save the session (list of open source files), restore terminal.

## Dependencies

anyhow, clap (derive), regex, ratatui 0.26, crossterm 0.27, serde/serde_json, serde_with, sqlx 0.8 (sqlite, tokio), tokio (rt-multi-thread), async-trait, dirs, tempfile, unicode-width, memmap2, memchr, aho-corasick, rayon, tracing, tracing-subscriber, tracing-appender

## Testing

- **Unit tests**: db.rs, filters.rs, file_reader.rs, log_manager.rs, search.rs, types.rs, ui.rs, auto_complete.rs, mode/annotation_mode.rs, mode/visual_mode.rs, mode/app_mode.rs, mode/select_fields_mode.rs — 374 tests total
- **Integration tests** (tests/integration.rs): FileReader line access, filter include/exclude/regex/disabled, marks, search on visible lines, filter CRUD — 15 tests
- **Stdin test** (tests/stdin.rs): pipe input end-to-end — 1 test
- **CI**: cargo fmt → clippy → test → tarpaulin coverage (enforces 80%)
