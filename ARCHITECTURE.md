# logana Architecture

Terminal-based log analysis tool built in Rust with a Ratatui TUI. Logs are read via memory-mapped files; filters and UI context are persisted in SQLite.

## Project Structure

```
src/
  main.rs         - Entry point, CLI args, runtime setup, app lifecycle
  lib.rs          - Library re-exports
  types.rs        - Shared data types (LogLevel, FilterType, FilterDef, ColorConfig, SearchResult, FieldLayout)
  parser/          - Log format parsing module directory
    mod.rs         - Re-exports, detect_format()
    types.rs       - LogFormatParser trait, DisplayParts, SpanInfo, format_span_col()
    timestamp.rs   - Shared timestamp parsing utilities (ISO, BSD, datetime, slash-date, dmesg, Apache) + level normalization
    json.rs        - JSON log parser (JsonParser implementing LogFormatParser)
    syslog.rs      - Syslog parser: RFC 3164 + RFC 5424 (SyslogParser implementing LogFormatParser)
    journalctl.rs  - Journalctl text parser: short-iso, short-precise, short-full (JournalctlParser implementing LogFormatParser)
    clf.rs         - Common Log Format + Combined Log Format parser (ClfParser implementing LogFormatParser)
    logfmt.rs      - Logfmt key=value parser: Go slog, Heroku, Grafana Loki (LogfmtParser implementing LogFormatParser)
    common_log.rs  - Timestamp+LEVEL+target family: env_logger, tracing fmt, logback, Spring Boot, Python, loguru, structlog (CommonLogParser implementing LogFormatParser)
  date_filter.rs  - Date/time filter: parsing, timestamp normalization, matching (DateFilter)
  file_reader.rs  - Memory-mapped file I/O with SIMD line indexing (FileReader)
  filters.rs      - Filter pipeline: Filter trait, SubstringFilter, RegexFilter, FilterManager, render_line
  log_manager.rs  - Filter/mark state management + SQLite persistence bridge (LogManager)
  db.rs           - SQLite layer via sqlx (FilterStore, FileContextStore traits)
  search.rs       - Regex search with match positions and wrapping navigation
  ui/             - Ratatui TUI module directory
    mod.rs         - Types (KeyResult, LoadContext, TabState, App structs, VisibleLines enum)
    app.rs         - App lifecycle: new(), run(), key dispatch, save/close, execute_command_str()
    commands.rs    - run_command() — 30+ command handler
    loading.rs     - File/stdin/docker loading, file watchers, session restore
    render.rs      - Main render: ui(), render_logs_panel, tab bar, sidebar, command bar, input bar
    render_popups.rs - Popup/modal renders: confirm restore, select fields, value colors, docker, keybindings help, comment editor
    field_layout.rs  - Standalone helpers: get_col, default_cols, apply_field_layout, line_row_count, count_wrapped_lines, effective_row_count
  export.rs       - Template-based export of analysis (comments + marked lines) to Markdown, Jira, etc.
  theme.rs        - JSON-based theme loading and color management (Theme, ValueColors)
  value_colors.rs - Per-token color coding for HTTP methods, status codes, IP addresses
  config.rs       - JSON config file loading (Config, Keybindings, KeyBinding)
  auto_complete.rs - Tab completion for commands, colors, and file paths
  commands.rs     - Clap-based command definitions for the command bar
templates/
  markdown.txt    - Bundled Markdown export template
  jira.txt        - Bundled Jira wiki export template
tests/
  integration.rs  - End-to-end flows (file reading, filtering, marks, search, filter CRUD)
  stdin.rs        - Stdin reading tests
.github/workflows/rust.yml - CI: fmt, clippy, test, coverage (80% threshold via tarpaulin)
```

## Core Data Models

**LogLevel**: `Trace | Debug | Info | Notice | Warning | Error | Fatal | Unknown`
Detected by scanning raw bytes with a fast case-insensitive byte-window scan (`detect_from_bytes`). `Fatal` covers FATAL, CRITICAL, CRIT, EMERG, and ALERT.

**FilterType**: `Include | Exclude`

**ColorConfig**: `{ fg: Option<Color>, bg: Option<Color>, match_only: bool }`
When `match_only` is true (default), only matched byte spans are highlighted; `-l` flag highlights the whole line.

**FilterDef**: `{ id, pattern, filter_type, enabled, color_config: Option<ColorConfig> }`
Persisted to SQLite. Renamed from `Filter` to avoid conflict with the `Filter` trait.

**SearchResult**: `{ line_idx: usize, matches: Vec<(start, end)> }`
Byte positions within the line.

**Comment**: `{ text: String, line_indices: Vec<usize> }`
A multiline comment attached to a group of log lines. Multiple comments can exist per session. `text` is newline-joined and `line_indices` are raw file-line indices (same space as `visible_indices`).

**FieldLayout**: `{ columns: Option<Vec<String>>, columns_order: Option<Vec<String>> }` — when `columns` is `Some`, only the listed column names are shown in that order; `None` restores default display order. `columns_order` stores the full ordered list (enabled + disabled) for modal reopening. Held by `TabState`; not persisted.

**FileContext**: Per-source session state (scroll_offset, search_query, wrap, sidebar, marked_lines, file_hash, show_line_numbers, horizontal_scroll, comments, show_mode_bar, show_borders, show_keys).

## Architecture Layers

### File I/O (file_reader.rs)

- **FileReader**: Zero-copy random access backed by either a `memmap2::Mmap` (files) or a `Vec<u8>` (stdin / tests).
- **Line indexing**: `memchr::memchr3_iter` scans simultaneously for `\n`, `\x1b`, and `\r` in a single pass. On the first ESC/CR byte ANSI is detected — the raw bytes are stripped and re-indexed; otherwise line starts are collected in the same loop with no extra scan.
- **`strip_ansi_and_index(input) -> (Vec<u8>, Vec<usize>)`**: Single-pass ANSI stripping + line indexing. Handles CSI sequences (`ESC [` … final byte), OSC sequences (terminated by `BEL` or `ST`), two-byte ESC sequences, and `\r` stripping. Emits `line_starts` inline when each `\n` is written — eliminates the second O(N) scan over stripped data that a separate `compute_line_starts` call would require.
- **`get_line(idx)`**: O(1) slice into the backing storage — no heap allocation per line.
- **`line_count()`**: Skips the phantom empty entry after a trailing newline.
- **`from_bytes(Vec<u8>)`**: Used for stdin input and in-memory test data.
- **`from_file_tail(path, preview_bytes) -> io::Result<Self>`**: Reads only the last `preview_bytes` of a file synchronously (without mmap), drops the first partial line, and returns a `FileReader::from_bytes`. Used by `begin_file_load` to provide immediate tail preview before the background indexing job completes — makes `--tail` on large files feel instant.
- **`load(path, predicate, tail) -> FileLoadHandle`**: Starts background indexing via `spawn_blocking`. `predicate: Option<VisibilityPredicate>` — when `Some`, each line is tested after indexing and the matching indices are stored in `FileLoadResult::precomputed_visible`, avoiding a separate `compute_visible` call after load. `tail=true` evaluates the predicate in reverse (last line first) so the tail is confirmed earliest; result is returned in ascending order. When `predicate` is `None`, `precomputed_visible` is `None` and `refresh_visible` runs after load.
- **`VisibilityPredicate`**: `Box<dyn Fn(&[u8]) -> bool + Send + Sync>` — passed from `main.rs` as a closure over a `FilterManager`, keeping `file_reader.rs` free of filter dependencies.
- **Scan mmap / access mmap split**: Both `new()` and `index_chunked()` use two separate mmaps. The *scan mmap* is created for the indexing phase (`MADV_SEQUENTIAL` set for prefetch throughput), then explicitly `drop`ped when scanning is done — `munmap()` is guaranteed to remove all its pages from process RSS immediately. A fresh *access mmap* is then created with zero RSS; `get_line` faults in only the specific 4 KiB page(s) it needs. This is more reliable than `MADV_DONTNEED`, which is advisory and ignored by the Linux kernel for file-backed shared mappings. `UncheckedAdvice::DontNeed` is still applied after the predicate phase-2 (when a filter predicate re-faults every page during `index_chunked`). All hints are `#[cfg(unix)]` no-ops on other platforms.
- **`spawn_process_stream(program, args)`**: Spawns a child process, merges stdout+stderr via mpsc, strips ANSI, and delivers complete lines every 500 ms through a `watch::Receiver<Vec<u8>>`. Used by the `docker` command.

### Format Abstraction (parser/)

Trait-based log format parsing. New parsers are added by implementing a single trait.

- **`DisplayParts<'a>`**: Format-agnostic structured representation of a parsed log line. All fields (`timestamp`, `level`, `target`, `span`, `extra_fields`, `message`) borrow `&'a str` slices from the original line bytes — zero-copy during parsing. Owned `String`s are only created at the column-formatting step in the UI.
- **`SpanInfo<'a>`**: Span context (name + key-value fields) extracted from structured logs (e.g. tracing JSON).
- **`LogFormatParser` trait** (object-safe):
  - `parse_line(&self, line: &'a [u8]) -> Option<DisplayParts<'a>>` — zero-copy parse
  - `collect_field_names(&self, lines: &[&[u8]]) -> Vec<String>` — discover field names by sampling
  - `detect_score(&self, sample: &[&[u8]]) -> f64` — confidence score (0.0–1.0) for format detection
  - `name(&self) -> &str` — human-readable format name
- **`detect_format(sample: &[&[u8]]) -> Option<Box<dyn LogFormatParser>>`**: Tries all registered parsers (OtlpParser, JsonParser, SyslogParser, JournalctlParser, ClfParser, LogfmtParser, CommonLogParser), returns the one with the highest score above 0.0. More specific parsers naturally score higher; CommonLogParser applies a 0.95× penalty to yield to more specific parsers on ties.
- **`format_span_col(&SpanInfo, show_keys: bool) -> String`**: Formats span as `name: v1 v2` (values only, default) or `name: k1=v1 k2=v2` (key=value pairs when `show_keys=true`).
- **Detection priority** (by score competition): OtlpParser (scores up to 1.5 to beat JSON), JsonParser (lines must start with `{`), SyslogParser (requires `<PRI>` or BSD timestamp), JournalctlParser (ISO/BSD + valid hostname + unit), ClfParser (strict request/status pattern), LogfmtParser (≥3 key=value pairs), CommonLogParser (broadest catch-all, 0.95× penalty).

### JSON Parser (parser/json.rs)

- **`JsonParser`** implements `LogFormatParser`. Parses JSON log lines (tracing, bunyan, etc.).
- **`parse_json_line(line: &[u8]) -> Option<Vec<JsonField<'a>>>`**: Zero-copy JSON field extraction.
- **`classify_json_fields(fields, hidden_fields) -> DisplayParts<'a>`**: Maps JSON fields to canonical names (timestamp/level/target/span/message) with hidden-field filtering.
- **`classify_json_fields_all(fields) -> DisplayParts<'a>`**: Same without hidden-field filtering (used by trait impl).
- **`detect_score`**: Proportion of sample lines starting with `{` that parse successfully.
- **`collect_field_names`**: Returns raw JSON key names (not canonical names) ordered by canonical slot (timestamp-group → level-group → target-group → sorted extras → message-group last), plus dotted sub-fields like `span.name`, `fields.count`.

### Syslog Parser (parser/syslog.rs)

- **`SyslogParser`** implements `LogFormatParser`. Supports RFC 3164 (BSD) and RFC 5424 formats.
- **RFC 3164**: `<PRI>Mmm DD HH:MM:SS hostname app[pid]: message` — optional priority prefix, BSD timestamp, hostname, app name with optional PID.
- **RFC 5424**: `<PRI>VER TIMESTAMP HOSTNAME APP PROCID MSGID [SD] MSG` — ISO 8601 timestamp, structured data sections with `key="value"` params.
- **Priority decoding**: `priority = facility * 8 + severity`. Severity → level: 0–3=ERROR, 4=WARN, 5–6=INFO, 7=DEBUG. Facility → name from 24-entry lookup table.
- **Zero-copy**: Converts `&[u8]` to `&str` once, then extracts `&str` slices by byte offsets. Severity levels are `&'static str` constants.
- **`detect_score`**: Proportion of sample lines that parse successfully.
- **`collect_field_names`**: Static canonical names + dynamically discovered extras (hostname, pid, facility, msgid, SD param names).

### Journalctl Parser (parser/journalctl.rs)

- **`JournalctlParser`** implements `LogFormatParser`. Handles `journalctl` text output variants that SyslogParser does not cover (ISO and precise timestamps).
- **short-iso**: `YYYY-MM-DDTHH:MM:SS±ZZZZ hostname unit[pid]: message` — ISO 8601 timestamp.
- **short-precise**: `Mmm DD HH:MM:SS.FFFFFF hostname unit[pid]: message` — BSD timestamp with microseconds (the `.` suffix distinguishes from plain BSD handled by SyslogParser).
- **short-full**: `Www YYYY-MM-DD HH:MM:SS TZ hostname unit[pid]: message` — weekday prefix + date + time + timezone name.
- **Header/footer lines** (`-- Journal begins...`, `-- No entries --`) are skipped (return `None`).
- **Hostname validation**: `is_likely_hostname()` rejects tokens that are recognized level keywords (via `normalize_level`), contain `::` (Rust module paths), or are short all-uppercase tokens — this prevents false positives where lines like `2024-07-24T10:00:00Z INFO myapp::server: msg` would incorrectly match as journalctl format.
- **Zero-copy**: Converts `&[u8]` to `&str` once, then extracts `&str` slices by byte offsets for all fields.
- **Fields**: `timestamp`, `target` (unit name), extras (`hostname`, `pid`), `message`. No `level` — journalctl text output does not carry priority.
- **`detect_score`**: Proportion of sample lines that parse successfully.
- **`collect_field_names`**: Static canonical names (`timestamp`, `target`) + dynamically discovered extras + `message` last.

### CLF Parser (parser/clf.rs)

- **`ClfParser`** implements `LogFormatParser`. Supports both Common Log Format (CLF) and Combined Log Format.
- **CLF**: `host ident authuser [dd/Mmm/yyyy:HH:MM:SS ±ZZZZ] "request" status bytes`.
- **Combined**: CLF + `"referer" "user-agent"`.
- **Date validation**: Strict `dd/Mmm/yyyy:HH:MM:SS ±ZZZZ` format check (month abbreviation + digit positions).
- **Status validation**: Must be a 3-digit number or `"-"`.
- **Dash handling**: Fields with value `"-"` are omitted from `extra_fields` (ident, authuser, bytes, referer, user_agent).
- **Zero-copy**: All fields are `&str` slices into the original line.
- **Field mapping**: `timestamp` = date, `target` = host, `message` = request line. Extras: `ident`, `authuser`, `status`, `bytes`, `referer`, `user_agent`.
- **`detect_score`**: Proportion of sample lines that parse successfully.
- **`collect_field_names`**: Static canonical names (`timestamp`, `target`) + discovered extras in order + `message` last.

### Shared Timestamp Utilities (parser/timestamp.rs)

- `pub(crate)` module — internal utility, not part of the public parser API.
- **Timestamp parsers** (all return `Option<&str>` slices from the input):
  - `parse_iso_timestamp` — `YYYY-MM-DDTHH:MM:SS...` (ISO 8601, with optional fractional seconds and timezone)
  - `parse_bsd_precise_timestamp` — `Mmm DD HH:MM:SS.FFFFFF` (BSD with microseconds)
  - `parse_full_timestamp` — `Www YYYY-MM-DD HH:MM:SS TZ` (weekday-prefixed full timestamp)
  - `parse_datetime_timestamp` — `YYYY-MM-DD HH:MM:SS[.mmm][,mmm]` (logback, Python, Spring Boot; supports `.` and `,` as fractional separator, optional timezone)
  - `parse_slash_datetime` — `YYYY/MM/DD HH:MM:SS[.frac]` (nginx error, Go standard log)
- **`normalize_level(token) -> Option<&'static str>`**: Maps level keywords (case-insensitive) to canonical strings: TRACE, DEBUG, INFO, NOTICE, WARN, ERROR, FATAL. Recognizes abbreviations (TRC, DBG, INF, WRN, ERR, FTL, CRIT, EMERG, ALERT).
- **`is_level_keyword(token) -> bool`**: Returns true if the token is a recognized log level keyword.
- **Constants**: `WEEKDAYS`, `BSD_MONTHS` — used across parsers for date validation.

### Logfmt Parser (parser/logfmt.rs)

- **`LogfmtParser`** implements `LogFormatParser`. Parses space-separated `key=value` pairs, commonly used by Go slog, Heroku, Grafana Loki, and 12-factor apps.
- **Key mapping**: `time`/`timestamp`/`ts`/`datetime` → timestamp; `level`/`lvl`/`severity` → level (normalized); `msg`/`message` → message; `source`/`caller`/`logger`/`component`/`module` → target. All other keys → extra_fields.
- **Quoted values**: Values wrapped in `"..."` are parsed with backslash-escape support.
- **Minimum threshold**: Requires ≥3 key=value pairs to match. Rejects lines starting with `{` (not JSON).
- **`detect_score`**: Lines with known semantic keys get full weight (1.0); lines with only unknown keys get 0.5× weight.

### Common Log Parser (parser/common_log.rs)

- **`CommonLogParser`** implements `LogFormatParser`. Broadest catch-all for the `TIMESTAMP + LEVEL + TARGET + MESSAGE` family, with internal sub-strategies tried in order:
  - **env_logger**: `[ISO LEVEL  target] msg` or `[LEVEL target] msg` — bracketed, level inside brackets.
  - **logback/log4j2**: `DATETIME [thread] LEVEL target - msg` — `[thread]` after timestamp, ` - ` separator.
  - **Spring Boot**: `DATETIME  LEVEL PID --- [thread] target : msg` — `---` triple dash marker.
  - **Python basic**: `LEVEL:target:msg` — no timestamp, colon-separated.
  - **Python prod**: `DATETIME - target - LEVEL - msg` — dash-delimited, level in 4th position.
  - **loguru**: `DATETIME | LEVEL | location - msg` — pipe `|` separators.
  - **structlog**: `DATETIME [level] msg key=val...` — `[level]` in brackets, trailing key-value pairs.
  - **tracing-subscriber fmt (with spans)**: `TIMESTAMP LEVEL  span_name{k=v ...}: target: message` — detects the `name{...}: ` span prefix; bails early (falls through to generic) if no span present. Handles unquoted and quoted (`"..."`) values. Used at runtime when span context is active; startup lines (no span) are handled by the generic fallback.
  - **Generic fallback**: `TIMESTAMP LEVEL rest-as-message` — any recognized timestamp + level keyword, with optional `target::` or `target:` extraction.
- **Key rule**: All sub-strategies require a recognizable level keyword — prevents claiming journalctl/syslog lines that lack level info.
- **`try_parse_span_prefix(s)`**: Detects `identifier{k=v ...}: ` prefix; returns `(Some(SpanInfo), remaining)` if found, else `(None, s)`.
- **`parse_tracing_span_fields(s)`**: Parses space-separated `key=value` pairs inside the braces; handles quoted (`"value"`) and unquoted values.
- **`detect_score`**: Proportion of parsed lines × 0.95 (yields to more specific parsers on ties).
- **Field discovery**: `collect_field_names` emits `"span"` (the formatted column) plus dotted names like `"span.method"`, `"span.uri"` for each discovered span sub-field, enabling `:fields` and `:select-fields` to expose span sub-fields.


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
- **`StyleId`** (`u8`): Index into the 256-slot styles array. `SEARCH_STYLE_ID = u8::MAX = 255` is reserved for search highlights; `CURRENT_SEARCH_STYLE_ID = u8::MAX - 1 = 254` is reserved for the currently-selected search occurrence (rendered with a distinct style from other matches).
- **`render_line(&MatchCollector, &[Style]) -> Line`**: Flattens overlapping spans into a ratatui `Line` using a boundary-sweep algorithm. Spans are sorted by priority (desc). All start/end byte positions are collected as boundary points, sorted and deduplicated. For each interval `[seg_s, seg_e)`, the first (highest-priority) covering span determines the style. Adjacent intervals with the same style are merged. This is O(S log S) in the number of spans, with no per-span Vec allocation.

### Date Filter (date_filter.rs)

- **Date/time filter for log lines**: Filters visible lines by timestamp using range or comparison expressions. Operates as a post-processing step in `refresh_visible()` — after the text-based `FilterManager` runs, date filters further narrow `visible_indices` via `retain()`.
- **Persistence**: Stored as regular `FilterDef` entries with `FilterType::Include` and an `@date:` prefix in the pattern field (e.g., `@date:01:00:00 .. 02:00:00`). This avoids DB schema changes (the `CHECK(filter_type IN ('Include','Exclude'))` constraint remains). `build_filter_manager()` skips `@date:` patterns so they don't compile as text filters.
- **User syntax** (after `date-filter` command or `@date:` prefix):
  - Range: `01:00:00 .. 02:00:00`, `Feb 21 .. Feb 22`, `2024-02-21 .. 2024-02-22`
  - Comparison: `> Feb 21 01:00:00`, `>= 2024-02-22`, `< 2024-02-22T10:15:30`, `<= Feb 22`
  - Time-only (`HH:MM:SS` or `HH:MM`): compares seconds since midnight
  - Full datetime: canonical `YYYY-MM-DD HH:MM:SS.ffffff` string comparison
- **Types** (all `pub(crate)`):
  - `ComparisonOp`: `Gt | Ge | Lt | Le`
  - `ComparisonMode`: `TimeOnly | FullDatetime`
  - `DateBound`: holds either `time_val: u32` (seconds since midnight) or `datetime_val: String` (canonical form)
  - `DateFilter`: `Range { mode, lower, upper }` or `Comparison { mode, op, bound }`
- **Key functions**:
  - `parse_date_filter(input) -> Result<DateFilter, String>` — parses expression after `@date:` prefix
  - `normalize_log_timestamp(ts) -> Option<NormalizedTimestamp>` — normalizes any parser-produced timestamp to canonical `YYYY-MM-DD HH:MM:SS.ffffff` form
  - `DateFilter::matches(timestamp) -> bool` — checks if a raw timestamp string passes the filter
  - `extract_date_filters(filter_defs) -> Vec<DateFilter>` — collects all enabled `@date:` filters, parsed and ready
- **Integration in `refresh_visible()`**: After text filters compute `visible_indices`, date filters are extracted via `extract_date_filters()`. If any exist and a format parser is detected, each visible line is parsed to extract its timestamp and tested against all date filters (AND logic). Lines without a parseable timestamp pass through (continuation/stack-trace lines).
- **Access**: `t` key in filter management mode (opens command mode with `"date-filter "` prefix) or `:date-filter <expr>` command directly. Date filters display as `Date: <expr>` in the sidebar (not `In: @date:<expr>`).
- **Design decisions**: Range bounds are inclusive (`..` = `>=` lower AND `<=` upper). Midnight wraparound is not supported (`23:00 .. 01:00` is an error). Multiple date filters are AND-ed. No color support (date filters only affect visibility). Requires a detected format parser (error if none).

### Log Manager (log_manager.rs)

- **`LogManager`**: Owns `filter_defs: Vec<FilterDef>` (in-memory, DB-backed), `marks: HashSet<usize>` (in-memory only), and `comments: Vec<Comment>` (in-memory only). Does **not** own the `FileReader`.
- **Filter CRUD**: `add_filter_with_color`, `remove_filter`, `toggle_filter`, `edit_filter`, `move_filter_up/down`, `set_color_config`, `clear_filters`, `save_filters` (JSON), `load_filters` (JSON).
- **`build_filter_manager() -> (FilterManager, Vec<Style>)`**: Converts enabled `FilterDef`s into a renderable `FilterManager` + parallel style palette (one `Style` per enabled filter, indexed by `StyleId`). Skips `@date:` prefixed patterns (date filters are applied separately in `refresh_visible()`).
- **Marks**: `toggle_mark`, `is_marked`, `get_marked_indices`, `get_marked_lines(&FileReader)`.
- **Comments**: `add_comment(text, line_indices)`, `get_comments() -> &[Comment]`, `has_comment(line_idx) -> bool`, `set_comments(Vec<Comment>)`, `remove_comment(index)`, `clear_all_marks_and_comments()`. Multiple comment groups can share the same log lines.
- **File hash**: `compute_file_hash(path)` hashes file size + mtime for change detection.

### Database (db.rs)

- **Three trait abstractions**: `FilterStore`, `FileContextStore`, `SessionStore`.
- **Tables**: `filters` (per `source_file`), `file_context` (PK: `source_file`), `session_tabs` (ordered list of last-open source files).
- **`FilterStore`**: `get_filters`, `get_filters_for_source`, `clear_filters_for_source`, `replace_all_filters`, `insert_filter`, `delete_filter`, `toggle_filter`, `update_filter_pattern`, `update_filter_color`, `swap_filter_order`.
- **`FileContextStore`**: `save_file_context`, `load_file_context`.
- **`SessionStore`**: `save_session(&[String])`, `load_session() -> Vec<String>` — persists the ordered list of open tabs across runs.
- In-memory mode (`Database::in_memory()`) for tests; runs the same migration path.
- **Schema versioning**: `PRAGMA user_version` tracks the applied schema version. `run_migrations()` reads the current version and calls `migrate_to_vN()` only for versions not yet applied. Each migration runs exactly once. To add a new migration: add `migrate_to_vN` with the required SQL and an `if version < N` block in `run_migrations`.
  - **v1**: Initial schema (`filters`, `file_context`, `session_tabs` tables).
  - **v2**: `ALTER TABLE file_context ADD COLUMN show_keys INTEGER NOT NULL DEFAULT 0` — persists the show/hide-keys display preference per file.
- Shared via `Arc<Database>`; callers use `.await` directly within the tokio runtime.

### Search (search.rs)

- Regex-based search over `visible_indices` only (respects active filters).
- `Search::search(pattern, impl Iterator<Item=usize>, &FileReader)` — accepts an iterator so both `VisibleLines::All` and `VisibleLines::Filtered` can be passed without materialising a `Vec`. Builds `Vec<SearchResult>` with byte-position match spans.
- `set_pattern(regex, forward)` — pre-sets the active regex without replacing results, used during background search so highlights appear immediately as the user types.
- `set_results(results, regex)` — called when the background task delivers its results; replaces the result set and updates the pattern atomically.
- Wrapping `next_match()` / `previous_match()` navigation.
- Case sensitivity toggle (`set_case_sensitive`).
- `results` is always sorted by `line_idx`; render uses binary search (`binary_search_by_key`) for O(log N) per-line lookup with zero allocation per frame.

### Background Filter Computation (ui/mod.rs + ui/loading.rs)

Filter changes triggered by user input (toggle, add, delete, clear) call `begin_filter_refresh()` instead of the synchronous `refresh_visible()`. This keeps the TUI responsive on large files.

- **`FilterHandle`**: Stored on `TabState` while a filter computation is in flight.
  - `result_rx: oneshot::Receiver<Vec<usize>>` — resolves with the new visible-line indices.
  - `cancel: Arc<AtomicBool>` — set to `true` to abort early; checked every 10 000 lines in the rayon loop.
  - `progress_rx: watch::Receiver<f64>` — [0.0, 1.0] progress fraction shown as "Filtering…" in the tab bar.
- **Fast paths** (synchronous, O(1) or O(marks)):
  1. No active filters → `VisibleLines::All(n)`, no task spawned.
  2. `show_marks_only = true` → apply marks directly.
  3. Leaving marks-only → restore `saved_filter_view` (saved on marks-only entry).
  4. `filtering_enabled = false` → `VisibleLines::All(n)`.
- **Slow path**: active text/date filters require scanning the full file. `begin_filter_refresh()` updates `filter_manager_arc` immediately (so highlights render with the new filter), then spawns a `spawn_blocking` task: phase 1 (text filters via `FilterManager::is_visible`) → phase 2 (date filters via parser + `matches_any`) → sends `Vec<usize>` on the oneshot.
- **`App::advance_filter_computation()`**: Called each frame; calls `try_recv()` on each tab's `filter_handle`. On success, applies `visible_indices` and clamps `scroll_offset`.
- **Tab bar indicator**: When a tab has a `filter_handle`, its tab-bar label shows "Filtering…".
- **Cancellation**: A new `begin_filter_refresh()` call cancels the previous `FilterHandle` before spawning. `App::close_tab()` also cancels the in-flight filter handle on the closing tab.

### Background File Load Cancellation (file_reader.rs + ui/loading.rs)

- **`FileLoadState::cancel: Arc<AtomicBool>`**: Added to `FileLoadState`. Set to `true` when a tab is closed via `App::close_tab()` before the load completes.
- **`FileReader::load(cancel: Arc<AtomicBool>)`**: Passes cancel through to `index_chunked()`.
- **`index_chunked(cancel)`**: Checks `cancel.load(Relaxed)` at the start of each 4 MiB chunk. On cancellation returns `Err(ErrorKind::Interrupted)`, which causes `advance_file_load` to call `skip_or_fail_load` (removes the placeholder tab gracefully).

### Background Search (ui/mod.rs)

Search is performed asynchronously to keep the TUI responsive on large files.

- **`SearchHandle`**: Stored on `TabState` while a search is in flight.
  - `result_rx: oneshot::Receiver<(Vec<SearchResult>, Regex)>` — resolves when the task finishes.
  - `cancel: Arc<AtomicBool>` — set to `true` to abort the task early (checked every 10 000 lines).
  - `progress_rx: watch::Receiver<f64>` — [0.0, 1.0] progress fraction polled each frame for the status bar.
  - `pattern: String`, `forward: bool`, `navigate: bool` — remembered so `confirm` can flip `navigate=true` without re-issuing a search.
- **`TabState::begin_search(pattern, forward, navigate)`**:
  1. Cancels any in-flight search via `cancel.store(true, Relaxed)`.
  2. Validates the regex upfront (returns early on invalid pattern).
  3. Pre-sets the pattern via `search.set_pattern(re.clone(), forward)` — highlights render immediately.
  4. Clones `file_reader` (O(1) Arc clone of mmap + line index).
  5. Materialises `visible: Vec<usize>` from `visible_indices`.
  6. Spawns a `tokio::task::spawn_blocking` closure that iterates visible lines, checks the cancel flag every 10 000 lines, sends progress via the watch channel, and resolves the oneshot with `(Vec<SearchResult>, Regex)`.
  7. Stores a `SearchHandle` on `tab.search_handle`.
- **`App::advance_search()`**: Called each frame in the event loop (after `advance_file_watches`). Calls `result_rx.try_recv()` non-blockingly; on success calls `search.set_results(...)` and, when `navigate=true`, scrolls to the first/previous match.
- **Enter fast-paths** (in `SearchMode::handle_key`):
  1. In-flight search with same pattern → set `handle.navigate = true`, exit to `NormalMode` without spawning a new task.
  2. Search already complete for same pattern → navigate directly, no new task.
  3. Otherwise → `begin_search(..., navigate=true)`.
- **`TabState::scroll_to_current_search_match()`**: Scrolls vertically to the current match via `scroll_to_line_idx` and, when wrap is off, also adjusts `horizontal_scroll` to center the matched byte span in the viewport. Centering uses `visible_width` (terminal columns of the log area, stored on `TabState` and updated each render frame) to compute `match_center.saturating_sub(visible_width / 2)`. Called by `n`/`N` and search confirm instead of the old `scroll_to_line_idx`-only path.

### Export (export.rs)

- **Template-based export** of analysis (comments + marked lines) to formatted documents (Markdown, Jira wiki, or custom).
- **Template syntax**: Section markers `{{#name}}...{{/name}}` with recognized sections: `header` (once), `comment_group` (per comment/mark entry), and optional `footer` (once). Placeholders: `{{filename}}`, `{{date}}`, `{{commentary}}`, `{{lines}}`, `{{line_numbers}}`.
- **`ExportTemplate`**: Parsed template with `header`, `comment_group`, and optional `marked_lines`/`footer` sections.
- **`ExportData`**: Bundles filename, comments, marked indices, file reader, and `show_keys: bool` for rendering (respects per-tab key display setting).
- **`parse_template(raw)`**: Extracts sections from raw template text.
- **`load_template(name)`**: Resolves template files — checks `~/.config/logana/templates/{name}.txt` first, falls back to `./templates/{name}.txt` (dev), then to `BUNDLED_TEMPLATES` embedded in the binary (same pattern as `Theme::from_file`).
- **`list_templates()`**: Seeds from `BUNDLED_TEMPLATES`, then overlays names from local `templates/` and `~/.config/logana/templates/`. User-config and local names shadow bundled ones (same pattern as `Theme::list_available_themes`).
- **`render_export(template, data)`**: Renders the full document — header with filename/date, then comments and standalone marked lines interleaved in log order (consecutive standalone marks are grouped). Lines are rendered through the detected format parser (same as the TUI display) when available; falls back to raw bytes for plain text logs.
- **`complete_template(partial)`**: Fuzzy-match completion for template names.
- **Bundled templates**: `markdown` and `jira` are embedded in the binary via `include_str!` — available without any installation step.
- **`:export <path> [-t <template>]`** command: default template is `markdown`. Tab-completes both file paths and template names (`-t`/`--template` flag).

### UI (ui/)

- **`App`** owns a `Vec<TabState>`, the global theme, and an `Arc<Keybindings>` shared across all tabs.
- **`TabState`** owns:
  - `file_reader: FileReader` — the backing log data
  - `log_manager: LogManager` — filter defs and marks
  - `detected_format: Option<Arc<dyn LogFormatParser>>` — auto-detected log format parser (sampled on tab creation); stored behind `Arc` so background filter tasks can clone it in O(1)
  - `visible_indices: VisibleLines` — virtual representation of visible line indices: `All(n)` when no filters are active (O(1), zero allocation), `Filtered(Vec<usize>)` when filters or marks narrow the set
  - `scroll_offset: usize` — selected line (index into `visible_indices`)
  - `viewport_offset: usize` — first rendered line (index into `visible_indices`)
  - `visible_height: usize` — content rows available (updated each render frame)
  - `visible_width: usize` — content columns available (updated each render frame; used by `scroll_to_current_search_match` to center matches horizontally)
  - `keybindings: Arc<Keybindings>` — shared keybinding config (cloned from `App` on tab creation)
  - `show_mode_bar: bool` — whether the bottom status/mode bar is visible (default `true`; toggled with `b`)
  - `show_borders: bool` — whether all panel borders (logs, sidebar, status bar) are visible (default `true`; toggled with `B`)
  - `show_keys: bool` — whether structured field keys are shown alongside values in parsed log columns (default `false`; toggled via `:show-keys` / `:hide-keys` commands). Persisted in `FileContext`.
  - `filter_manager_arc: Arc<FilterManager>` — cached filter manager, rebuilt by `refresh_visible()`, cloned O(1) per render frame (atomic ref-count increment)
  - `filter_styles: Vec<Style>` / `filter_date_styles: Vec<DateFilterStyle>` — cached style palettes matching the filter manager
  - `parse_cache_gen: u64` — monotonically increasing generation counter; incremented whenever filters, field layout, display mode, or raw mode changes
  - `parse_cache: HashMap<usize, (u64, CachedParsedLine)>` — per-line parse cache keyed by `line_idx`; entry is valid only when the stored generation equals `parse_cache_gen`. `CachedParsedLine` holds `rendered: String`, `level`, `timestamp`, `target`, `pid`, `all_cols_hidden: bool`.
  - `mode: Box<dyn Mode>`, `command_history: Vec<String>`, `search: Search`, plus display flags
- **`Mode` trait**: Each mode owns its key-handling logic via `handle_key(self: Box<Self>, tab, key, modifiers) -> (Box<dyn Mode>, KeyResult)`. Unhandled keys return `KeyResult::Ignored`, falling through to `App::handle_global_key` (quit, Tab switch, Ctrl+w/t). `KeyResult::ExecuteCommand(cmd)` triggers `App::execute_command_str`.
- **Mode structs**: `NormalMode { count }`, `CommandMode` (with tab completion, history), `FilterManagementMode`, `FilterEditMode`, `SearchMode`, `ConfirmRestoreMode`, `ConfirmRestoreSessionMode`, `ConfirmOpenDirMode`, `VisualLineMode { anchor, count }`, `CommentMode`, `KeybindingsHelpMode`, `SelectFieldsMode`, `DockerSelectMode`, `ValueColorsMode`, `UiMode { sidebar, status_bar, borders, wrap }`.
- **`ModeRenderState` enum** (ISP-compliant): Each mode implements `render_state() -> ModeRenderState`, returning a typed variant carrying exactly the data its renderer needs. Variants: `Normal`, `Command { input, cursor, completion_index }`, `Search { query, forward }`, `FilterManagement { selected_index }`, `FilterEdit`, `VisualLine { anchor }`, `Comment { lines, cursor_row, cursor_col, line_count }`, `KeybindingsHelp { scroll, search }`, `SelectFields { fields, selected }`, `DockerSelect { containers, selected, error }`, `ValueColors { groups, search, selected }`, `ConfirmRestore`, `ConfirmRestoreSession { files }`, `ConfirmOpenDir { dir, files }`. The renderer does a single `match` on the enum instead of calling many optional trait methods.
- **`refresh_visible()`**: Synchronous rebuild of `visible_indices`. With no active filters or marks, sets `VisibleLines::All(n)` — a zero-allocation O(1) operation. With filters, calls `FilterManager::compute_visible(&file_reader)` and wraps the result in `VisibleLines::Filtered`. Date filters are then applied as a post-processing `retain()` step (see Date Filter section). Always rebuilds `filter_manager_arc`, `filter_styles`, and `filter_date_styles`, then bumps `parse_cache_gen` and clears `parse_cache`. Used internally (file load completion, initial preview, stdin updates).
- **`begin_filter_refresh()`**: Non-blocking replacement for `refresh_visible()`, used in response to user filter/mark changes. Takes fast paths synchronously (no filters → `All(n)`, marks-only, leaving marks-only, filtering disabled). For the slow path (active text/date filters over the full file), updates `filter_manager_arc` immediately for render highlights, then spawns a `tokio::task::spawn_blocking` task that calls `FilterManager::is_visible()` per line (with progress updates every 10 000 lines) and delivers results via a `FilterHandle`. `advance_filter_computation()` picks up the result each frame.
- **`apply_incremental_exclude(pattern)`**: Additive fast-path for new exclude filters — compiles the pattern and calls `VisibleLines::retain()` to remove matching lines from the current visible set, then updates the filter manager cache. Avoids scanning the full file when only lines need to be removed. Falls back to `refresh_visible()` when editing an existing filter.
- **`invalidate_parse_cache()`**: Bumps `parse_cache_gen` and clears `parse_cache`. Called whenever field layout, display mode, raw mode, or show-keys toggles change.

**Rendering pipeline (per frame)**:
1. Compute `visible_height = logs_area.height - border_size` where `border_size` is 2 when `show_borders` is `true`, 0 otherwise.
2. Compute `inner_width` (terminal columns available inside borders, minus line-number prefix).
3. Wrap-aware viewport adjustment: when wrap is ON, checks if `scroll_offset - viewport_offset > visible_height` first (O(1) fast-path for large jumps, e.g. `G` on a large file). Only if the gap is small enough does it sum terminal rows via `effective_row_count` from `viewport_offset` to `scroll_offset`; scrolls when total exceeds `visible_height`. Without this fast-path, `G` on a large file required summing row counts for every visible line.
4. Wrap-aware `end` computation: walks from `start` accumulating `effective_row_count()` until `visible_height` is filled.
5. Clone `tab.filter_manager_arc` (O(1) atomic increment) and `tab.filter_styles`/`tab.filter_date_styles`. No `build_filter_manager()` call per frame.
6. **Parse cache pre-population**: for each line in `[start..end)`, if not cached at the current `parse_cache_gen`, call `parser.parse_line()` + `apply_field_layout()`, join columns into a pre-sized `String::with_capacity` buffer (sum of column lengths + separators), and store a `CachedParsedLine` with `rendered`, `level`, `timestamp`, `target`, `pid`, `all_cols_hidden`. This block runs before search results borrow `tab.search` to satisfy the borrow checker.
7. For each line in `[start..end]`: use the cached `rendered` string (or raw bytes in raw mode). Evaluate filters (`evaluate_line`), overlay search spans at priority 1000, apply level colours (from `cached.level` for structured lines — avoids `detect_from_bytes` rescan) and mark styles, compose final `Line` via `render_line`.
8. Apply value-based coloring (`colorize_known_values`) to spans with no `fg` set — HTTP methods, status codes, and IP addresses get per-token colors from `theme.value_colors`. Spans already colored by filters or search are left untouched.
9. `effective_row_count(bytes, inner_width, parser, layout, hidden, show_keys)` uses the structured-rendering width when a format parser is active (raw JSON/tracing bytes can be 3–5× wider than rendered columns, causing `line_row_count` on raw bytes to underestimate how many lines fit). Falls back to `line_row_count(bytes, inner_width)` — `unicode_width`-based word-wrap simulation — when no parser is active or parsing fails.

**Structured field layout**: `apply_field_layout(&DisplayParts, &FieldLayout, &HashSet<String>, show_keys: bool) -> Vec<String>` — module-level helper that routes through `default_cols` (all columns, default order) or picks specific columns via `get_col`, with name-based hidden-field filtering. `show_keys` controls whether extra field and span values are rendered as `key=value` pairs or values-only. Column name resolution: `get_col()` checks all aliases from `TIMESTAMP_KEYS`, `LEVEL_KEYS`, `TARGET_KEYS`, `MESSAGE_KEYS` arrays to map raw JSON key names to `DisplayParts` slots, plus `span` and dotted sub-field names (`span.*`, `fields.*`). Tab completion for the `fields` command completes against the five canonical names plus dynamically discovered field names from the first 200 visible log lines (`TabState::collect_field_names()`, which delegates to the detected format parser's `collect_field_names`).
**Format auto-detection**: On tab creation, the first 200 lines are sampled and passed to `detect_format()`, which tries all registered parsers and stores the best match in `TabState::detected_format`. The rendering pipeline dispatches through the trait: `detected_format.as_ref().and_then(|parser| parser.parse_line(line_bytes))`. If no format is detected, lines fall back to raw byte rendering.
**Select-fields mode** (`:select-fields`): floating popup showing all discovered structured fields with checkboxes. `j`/`k` navigate, `Space` toggle, `J`/`K` reorder, `a`/`n` enable/disable all, `Enter` apply, `Esc` cancel. Implemented by `SelectFieldsMode` in `src/mode/select_fields_mode.rs`.
**Go-to-line** (`:N`): Typing a bare number in command mode (e.g. `:500`) jumps to that 1-based line number. If the target line is hidden by filters, jumps to the closest visible line (binary search on `visible_indices`, picks the nearer neighbour). Line 0 returns an error; numbers beyond the file jump to the last visible line. Implemented via `TabState::goto_line()` with a fast-path check in `run_command` before clap parsing.
**Vim keybindings**: j/k, gg/G, Ctrl+d/u (half page), PageUp/Down, /, ?, n/N, m (mark), V (visual select). **Count prefix**: typing digits before a motion repeats it — `5j` moves down 5, `10k` up 10, `50G` goes to line 50, `3gg` goes to line 3, `2Ctrl+d` scrolls 2 half-pages. Count is accumulated in `NormalMode.count` / `VisualLineMode.count` and capped at 999,999. Digits 1-9 start a count; 0 appends when a count is active. Count is consumed by motions and reset on non-motion keys. The active count is shown in the status bar (e.g. `[NORMAL] 5`).
**Docker logs** (`:docker`): runs `docker ps` to list running containers, opens a `DockerSelectMode` popup (j/k navigate, Enter attach, Esc cancel). On selection, spawns `docker logs -f <id>` via `FileReader::spawn_process_stream()` and opens a new streaming tab. `DockerContainer { id, name, image, status }` in `types.rs`. Docker tabs persist across sessions via `source_file = "docker:name"`; on session restore, the `"docker:"` prefix is detected and `restore_docker_tab()` re-spawns the stream by container name instead of attempting a file load.
**Visual line mode** (`V`): anchor at current line, j/k extend selection, `c` opens comment editor, `m` marks/unmarks all selected lines (group toggle: all marked → unmark all, else mark all), `y` yanks (copies) selected lines to system clipboard via `arboard`, Esc cancel. All keys are configurable via `VisualLineKeybindings`. Selected range highlighted in the log panel.
**Comment mode**: multiline text editor (Enter = newline, Backspace = delete/merge, Left/Right wrap lines, Up/Down move rows, Ctrl+Enter = save (configurable), Esc = cancel). Rendered as a floating popup. Commented lines show a `◆` indicator in the line-number margin. In normal mode, `e` on a commented line opens edit mode (pre-filled text); `d` deletes the comment on the current line; `C` clears all marks and comments. In edit mode, `Ctrl+D` also deletes the comment.
**Keybindings help** (`F1`): floating popup listing all configured keybindings grouped by mode. Type to fuzzy-search, j/k scroll, Esc/q/F1 close. The status bar reflects the actual configured keybinding strings.
**Conflict validation**: at startup `Keybindings::validate()` checks for overlapping bindings within each mode scope; conflicts are printed to stderr and logged as warnings.
**Multi-tab**: Tab/Shift+Tab switch, Ctrl+t open, Ctrl+w close
**Command mode** (`:`) with highlight-then-accept tab completion, history, live hints. Tab/BackTab cycle a highlight over completions in the hint area without changing input; Enter accepts the highlighted completion into the input (single match = accept+execute immediately). `CommandMode::compute_completions()` encapsulates the 5-tier completion logic (color → template → file path → theme → command name). `completion_index()` trait method exposes the active highlight to the renderer.
**Commands**: `filter`, `exclude`, `set-color`, `export-marked`, `export`, `save-filters`, `load-filters`, `wrap`, `set-theme`, `level-colors`, `open`, `close-tab`, `hide-field`, `show-field`, `show-all-fields`, `fields [col...]`, `select-fields`, `docker`, `value-colors`, `date-filter`, `tail`, `show-keys` (show field keys alongside values, e.g. `method=GET`), `hide-keys` (show values only, e.g. `GET`; default)
**UI mode** (`u`): display-only toggles — sidebar (`s`), status bar (`b`), borders (`B`), wrap (`w`). Stays open until `Esc`. Status bar shows current ON/OFF state for each toggle. `UiMode` stores a snapshot of tab display flags to render state without access to `TabState` in `dynamic_status_line`.
**Quick filter shortcuts** (Normal mode): `i` → opens CommandMode prefilled with `"filter "` (include), `o` → opens CommandMode prefilled with `"exclude "` (exclude). These allow adding filters without entering Filter Management mode.
**Open directory**: `logana <dir>` or `:open <dir>` lists flat (non-recursive), non-hidden regular files in the directory and shows a `ConfirmOpenDirMode` popup. Confirming opens each file in its own tab. Empty directories are rejected with an error before the TUI starts (for CLI) or as a command error (for `:open`). `list_dir_files(path) -> Vec<String>` in `ui/mod.rs` implements the listing logic.
**Tail mode** (`:tail`): per-tab flag `tail_mode: bool` on `TabState`. When enabled, every call to `advance_file_watches()` or `update_stdin_tab()` moves `scroll_offset` to the last visible line after new content arrives. When disabled, the view is not auto-scrolled (stays wherever the user left it). Enabling tail immediately jumps to the last line. The logs panel title shows `[TAIL]` when active.
**Filter management mode** (`f`): navigate, toggle, delete, edit, set color, add include/exclude/date-filter

### Config (config.rs)

- **Config file**: `~/.config/logana/config.json` (loaded at startup; falls back to defaults on parse/IO error — never prevents startup).
- **`Config`**: `{ theme: Option<String>, keybindings: Keybindings, show_mode_bar: bool, show_borders: bool }`. `theme` is a theme name without the `.json` extension (e.g. `"dracula"`). `show_mode_bar` (default `true`) hides/shows the bottom mode/status bar at startup; `show_borders` (default `true`) hides/shows all panel borders at startup. Both can be toggled at runtime via UI mode (`u` → `b` / `B`).
- **`Keybindings`**: groups `NavigationKeybindings` (shared scroll/page keys used across all modes), `NormalKeybindings`, `FilterKeybindings`, `GlobalKeybindings`, `CommentKeybindings`, `VisualLineKeybindings`, `DockerSelectKeybindings`, `ValueColorsKeybindings`, `SelectFieldsKeybindings`, `HelpKeybindings`, `ConfirmKeybindings`, `UiKeybindings` — each with `#[serde(default)]` so any absent field uses its built-in default. Navigation keys (scroll_down/up, half_page_down/up, page_down/up) are configured once in the `navigation` group and shared by all modes.
- **`KeyBindings`** (per action): a `Vec<KeyBinding>` — each action supports multiple alternative keys (e.g. `"j"` and `"Down"` for scroll down). Accepts a single JSON string or an array of strings.
- **`KeyBinding`**: parsed from strings like `"j"`, `"Ctrl+d"`, `"Shift+Tab"`, `"F1"`, `"PageDown"`, `"Space"`, `"Esc"`. `"Shift+Tab"` maps to `KeyCode::BackTab`. `matches(key, modifiers)`: for `Char` keys accepts `NONE` or `SHIFT` (terminals vary); for non-`Char` keys (Enter, F-keys, etc.) requires an exact SHIFT match so `"Shift+Enter"` ≠ plain `"Enter"`.
- **`Keybindings::validate() -> Vec<String>`**: checks all (action, keybinding) pairs within each mode scope (normal + global, filter + global) for overlaps and returns human-readable conflict descriptions. Called at startup; conflicts are printed to stderr and logged.
- **Sharing**: `Arc<Keybindings>` is held by `App` and cloned into each `TabState` when tabs are created (including session restores and new tabs opened via commands).
- **Default keybindings** exactly match the previously hardcoded key assignments, so the config file is fully optional. Default for `comment.save` is `Ctrl+Enter`; `comment.newline` is `Enter`; `normal.show_keybindings` is `F1`.

Example `~/.config/logana/config.json`:
```json
{
  "theme": "dracula",
  "show_mode_bar": false,
  "show_borders": false,
  "keybindings": {
    "navigation": { "scroll_down": ["j", "Down"], "half_page_down": "Ctrl+d" },
    "normal": { "scroll_left": "h", "scroll_right": "l", "toggle_status_bar": "b", "toggle_borders": "B" },
    "global": { "quit": "q" }
  }
}
```

### Theme (theme.rs)

- **Lookup order**: `~/.config/logana/themes/` (user override) → `themes/` relative to CWD (dev) → bundled themes embedded in the binary at compile time via `include_str!`. User-config always wins; bundled themes are always available regardless of install location.
- **Bundled themes** (`BUNDLED_THEMES` static, 19 total): dark: catppuccin-mocha, catppuccin-macchiato, dracula, everforest-dark, gruvbox-dark, jandedobbeleer, kanagawa, monokai, nord, onedark, paradox, rose-pine, solarized, tokyonight, atomic. Light: catppuccin-latte, everforest-light, onelight, rose-pine-dawn.
- Colors: hex `"#RRGGBB"` or RGB array `[r, g, b]`
- Default: Dracula theme (loaded from bundled data; hardcoded fallback if parse fails)
- Fields: `root_bg`, `border`, `border_title`, `text`, `text_highlight`, `cursor_fg`, `trace_fg`, `debug_fg`, `notice_fg`, `warning_fg`, `error_fg`, `fatal_fg`, `search_fg`, `visual_select_bg`, `visual_select_fg`, `mark_bg`, `mark_fg`, `value_colors`
- **`ValueColors`**: Per-token color mappings for HTTP methods (`http_get`, `http_post`, `http_put`, `http_delete`, `http_patch`, `http_other`), status codes (`status_2xx`–`status_5xx`), IP addresses (`ip_address`), and UUIDs (`uuid`). All fields have `#[serde(default)]` so existing theme files need no changes. Overridable in theme JSON under `"value_colors": { ... }`.
- **`Theme::list_available_themes() -> Vec<String>`**: Seeds from bundled names, then overlays names from `themes/` and `~/.config/logana/themes/`. Returns sorted deduplicated list.
- **`fuzzy_match(needle, haystack) -> bool`**: Case-insensitive subsequence check; used for `set-theme` tab completion.

### Value Colors (value_colors.rs)

- **Per-token coloring** for known values: HTTP methods (GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS), HTTP status codes (2xx–5xx), IPv4 and IPv6 addresses, UUIDs.
- **Regex patterns** compiled once via `std::sync::LazyLock`.
- **`colorize_known_values(line, &ValueColors) -> Line`**: Post-processes a rendered `Line`, scanning unstyled spans (no `fg` set) for known patterns. Matched tokens get split into sub-spans with appropriate colors. Spans already colored by filters or search highlights are left untouched.
- **Priority layering** (highest wins): cursor/mark/visual selection → search highlights → filter highlights → value colors → level colors (line-level fallback).
- **`:value-colors` command**: opens `ValueColorsMode` popup with a grouped hierarchy (HTTP methods, Status codes, Network, Identifiers). Groups show tri-state checkboxes (`[x]`/`[ ]`/`[-]`). `j`/`k` navigate, `Space` toggles group or entry, `a` enable all, `n` disable all, `Enter` apply, `Esc` cancel. Typing filters rows via fuzzy search; `Esc` clears search first, then cancels. Disabled categories are stored in `ValueColors.disabled` (runtime-only, not serialized to theme JSON).

## Key Patterns

- **Zero-copy reads**: `FileReader::get_line` returns `&[u8]` slices directly into the mmap — no per-line allocation.
- **Virtual visible lines**: `VisibleLines::All(n)` stores only a count, so the no-filter case (the common case for large files) allocates nothing and provides O(1) random access via arithmetic. `Filtered(Vec<usize>)` is only materialised when filters or marks are active.
- **Parallel filter evaluation**: `FilterManager::compute_visible` uses `rayon::into_par_iter()` over line indices; order is preserved by rayon's indexed parallel iterator.
- **Dual filter backends**: Aho-Corasick for literals (O(n) multi-pattern), Regex fallback for metacharacter patterns. Selected automatically by `build_filter`.
- **StyleId dispatch**: 256-slot `Vec<Style>` indexed by `u8` avoids per-span HashMap lookups at render time.
- **Wrap-aware viewport**: `effective_row_count` drives both the scroll trigger and the `[start..end]` window. For structured log formats it uses the rendered column width (not raw bytes) so the viewport is accurate when JSON lines are 3–5× wider than their parsed representation. Falls back to `line_row_count` (unicode_width word-wrap simulation) for plain text.
- **Async DB access**: All `LogManager` methods are `async fn` and `await` DB calls directly. No `block_on` or manual runtime bridging.
- **Repository pattern**: `FilterStore` / `FileContextStore` traits enable in-memory SQLite for tests.
- **Session persistence**: Filters + UI context saved per `source_file`; hash-verified restore prompt on reopen. Docker tabs are stored as `"docker:name"` and restored by detecting the prefix (re-spawns `docker logs -f` by container name).
- **Filter manager cache** (`filter_manager_arc`): `Arc<FilterManager>` rebuilt only in `refresh_visible()` (filter changes); cloned O(1) on every render frame instead of rebuilding from scratch.
- **Parse cache** (`parse_cache`): `HashMap<usize, (gen, CachedParsedLine)>` caches `parse_line` + `apply_field_layout` + column join per visible line. Entries are validated by generation counter and pre-populated in a batch before the render loop. Invalidated on filter, field layout, display mode, or raw mode changes.
- **Incremental exclude**: Adding a new exclude filter calls `apply_incremental_exclude` which runs `VisibleLines::retain()` on the current visible set — O(visible) instead of O(total file lines). Only falls back to full `refresh_visible()` when editing an existing filter.
- **Pre-sized join buffer**: Column strings are joined into a `String::with_capacity(sum_of_col_lens + separators)` to avoid intermediate allocations from `Vec::join`.
- **Tail preview**: `begin_file_load` with `tail=true` calls `FileReader::from_file_tail` synchronously before spawning the background indexing job. The last ~64 KiB of the file is read, parsed, and set as the initial `file_reader` with `scroll_offset` at the last line. The background job replaces this preview with the full index when complete.
- **Scan/access mmap split**: After scanning, the scan mmap is `drop`ped — `munmap()` is guaranteed to evict all its pages from process RSS (unlike `MADV_DONTNEED`, which is advisory and ignored by the Linux kernel for file-backed shared mappings). A fresh mmap is then created for `get_line` access with zero initial RSS. `MADV_SEQUENTIAL` on the scan mmap and `MADV_RANDOM` on the access mmap are applied as hints; `MADV_DONTNEED` is still used after predicate phase-2 in `index_chunked` since that re-faults all pages.
- **Background search / `FileReader` clone**: `FileReader` is `Clone` via `Arc`-wrapped storage (`Arc<Mmap>` or `Arc<Vec<u8>>`) and `Arc<Vec<usize>>` line index — clone is O(1) atomic reference increments. `begin_search` clones the reader into a `spawn_blocking` task; the task and the UI share memory with zero copy.
- **Binary-search render lookup**: `search_results` is sorted by `line_idx`. Each render frame uses a `binary_search_by_key` closure (O(log N)) rather than a `HashMap` built per frame (O(N) + allocation). Zero allocation per frame for the common case.

## CLI Flags

- **`<file>`** (positional, optional) — file or directory to open. Omit to read from stdin.
- **`-f` / `--filters <path>`** — path to a JSON filter file (saved via `:save-filters`). Filters are loaded before the TUI starts and used for the single-pass visible-line computation during indexing. The loaded filters are also active for interactive use (add/remove/edit) once the TUI is open.
- **`-t` / `--tail`** — start at the end of the file and enable tail mode. When combined with `--filters`, the predicate is evaluated backward (last line first) so the tail view is ready immediately after loading; the result is returned in ascending order. Without `--filters`, the file is indexed normally and the scroll position is jumped to the last visible line after load.

## App Lifecycle

1. Parse CLI args (optional file path or directory path, optional `--filters`, optional `--tail`).
2. Validate file path and filter file path (if provided) before entering the TUI.
3. Init tokio runtime + SQLite DB (`~/.local/share/logana/logana.db`).
4. Load `Config` from `~/.config/logana/config.json` (or defaults on missing/parse error).
5. Enter terminal raw mode, create `App` with empty placeholder `FileReader`, theme, and `Arc<Keybindings>`.
6. If `--filters` was given: call `log_manager.load_filters(path)`, then extract a `VisibilityPredicate` from `build_filter_manager()`. Set `app.startup_tail = args.tail` and `app.startup_filters = true`.
7. Kick off `begin_file_load(path, context, predicate, tail)` — indexing runs in a background thread via `spawn_blocking`.
8. If a directory was given: set `ConfirmOpenDirMode`. If stdin is piped: begin stdin streaming.
9. **Event loop** (250ms poll): render frame → wait for key event → handle key → `advance_file_load` polls the background result channel each frame.
10. On load complete (`on_load_success`): if `precomputed_visible` is `Some`, set `VisibleLines::Filtered` directly (skips `refresh_visible`). If `startup_tail`, set `tail_mode = true` and jump `scroll_offset` to the last visible line. If `startup_filters` is false, check for a saved `FileContext` and prompt restore if found (restore is suppressed when `--filters` was provided, since the user's explicit filter set takes precedence).
11. On exit: save `FileContext` for each tab + save the session (list of open source files), restore terminal.

## Dependencies

anyhow, clap (derive), regex, ratatui 0.30, crossterm 0.29, serde/serde_json, serde_with, sqlx 0.8 (sqlite, tokio), tokio (rt-multi-thread), async-trait, dirs, unicode-width, memmap2, memchr, aho-corasick, rayon, time, tracing, tracing-subscriber, tracing-appender, arboard 3 (cross-platform clipboard)

## Testing

- **Unit tests**: co-located with each module (`#[cfg(test)]`). Each module tests its own logic in isolation — parsers are tested against representative log line samples, modes are tested by constructing a `TabState` and asserting on the returned `KeyResult` and state mutations, DB traits are tested against an in-memory SQLite instance.
- **Integration tests** (`tests/integration.rs`): end-to-end flows exercising `FileReader` → `FilterManager` → `Search` together, without the TUI layer.
- **Stdin tests** (`tests/stdin.rs`): pipe input end-to-end.
- **Benchmarks** (`benches/`): Criterion benchmarks for `FileReader` (plain and ANSI paths, file and byte-slice variants) and `VisibleLines` (collect-all cost and `FilterManager::compute_visible` under various filter configurations). Used to measure performance of the file reading and visible-line pipeline.
- **CI**: cargo fmt → clippy → test → tarpaulin coverage (enforces 80% threshold).
