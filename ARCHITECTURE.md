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
    web_error.rs   - Nginx error log + Apache 2.4 error log (WebErrorParser implementing LogFormatParser)
    dmesg.rs       - Linux kernel ring buffer / dmesg output (DmesgParser implementing LogFormatParser)
    kube_cri.rs    - Kubernetes CRI log format (KubeCriParser implementing LogFormatParser)
  date_filter.rs  - Date/time filter: parsing, timestamp normalization, matching (DateFilter)
  file_reader.rs  - Memory-mapped file I/O with SIMD line indexing (FileReader)
  filters.rs      - Filter pipeline: Filter trait, SubstringFilter, RegexFilter, FilterManager, render_line
  log_manager.rs  - Filter/mark state management + SQLite persistence bridge (LogManager)
  db.rs           - SQLite layer via sqlx (FilterStore, FileContextStore traits)
  search.rs       - Regex search with match positions and wrapping navigation
  ui/             - Ratatui TUI module directory
    mod.rs         - Types (KeyResult, LoadContext, TabState, App structs)
    app.rs         - App lifecycle: new(), run(), key dispatch, save/close, execute_command_str()
    commands.rs    - run_command() — 30+ command handler
    loading.rs     - File/stdin/docker loading, file watchers, session restore
    render.rs      - Main render: ui(), render_logs_panel, tab bar, sidebar, command bar, input bar
    render_popups.rs - Popup/modal renders: confirm restore, select fields, value colors, docker, keybindings help, comment editor
    field_layout.rs  - Standalone helpers: get_col, default_cols, apply_field_layout, line_row_count, count_wrapped_lines
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

**FileContext**: Per-source session state (scroll_offset, search_query, wrap, sidebar, marked_lines, file_hash, show_line_numbers, horizontal_scroll, comments).

## Architecture Layers

### File I/O (file_reader.rs)

- **FileReader**: Zero-copy random access backed by either a `memmap2::Mmap` (files) or a `Vec<u8>` (stdin / tests).
- **Line indexing**: `memchr::memchr_iter` scans for `\n` bytes at startup, building a `Vec<usize>` of line start offsets in one pass.
- **`get_line(idx)`**: O(1) slice into the backing storage — no heap allocation per line.
- **`line_count()`**: Skips the phantom empty entry after a trailing newline.
- **`from_bytes(Vec<u8>)`**: Used for stdin input and in-memory test data.
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
- **`detect_format(sample: &[&[u8]]) -> Option<Box<dyn LogFormatParser>>`**: Tries all registered parsers (JsonParser, SyslogParser, JournalctlParser, ClfParser, KubeCriParser, WebErrorParser, LogfmtParser, DmesgParser, CommonLogParser), returns the one with the highest score above 0.0. More specific parsers naturally score higher; CommonLogParser applies a 0.95× penalty to yield to more specific parsers on ties.
- **`format_span_col(&SpanInfo) -> String`**: Formats span as `name: k=v, k=v`.
- **Detection priority** (by score competition): JsonParser (lines must start with `{`), SyslogParser (requires `<PRI>` or BSD timestamp), JournalctlParser (ISO/BSD + valid hostname + unit), ClfParser (strict request/status pattern), KubeCriParser (ISO + stdout/stderr + F/P), WebErrorParser (slash-date + bracketed level or Apache double-bracket), LogfmtParser (≥3 key=value pairs), DmesgParser (bracketed seconds.usecs), CommonLogParser (broadest catch-all, 0.95× penalty).

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
  - `parse_dmesg_timestamp` — `[ seconds.usecs]` (Linux kernel ring buffer)
  - `parse_apache_error_timestamp` — `[Www Mmm DD HH:MM:SS.usecs YYYY]` (Apache 2.4 error log)
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
  - **Generic fallback**: `TIMESTAMP LEVEL rest-as-message` — any recognized timestamp + level keyword, with optional `target::` or `target:` extraction.
- **Key rule**: All sub-strategies require a recognizable level keyword — prevents claiming journalctl/syslog lines that lack level info.
- **`detect_score`**: Proportion of parsed lines × 0.95 (yields to more specific parsers on ties).

### Web Error Parser (parser/web_error.rs)

- **`WebErrorParser`** implements `LogFormatParser`. Handles nginx and Apache 2.4 error log formats.
- **nginx error**: `YYYY/MM/DD HH:MM:SS [level] pid#tid: *connid msg, key: val, ...` — slash-date timestamp, bracketed level, trailing `client`/`server`/`request`/etc. key-value pairs extracted as extra fields.
- **Apache 2.4 error**: `[Www Mmm DD HH:MM:SS.us YYYY] [module:level] [pid N:tid N] msg` — double-bracket pattern, module extracted as extra field.
- **`detect_score`**: Proportion of sample lines that parse successfully.

### Dmesg Parser (parser/dmesg.rs)

- **`DmesgParser`** implements `LogFormatParser`. Parses Linux kernel ring buffer output.
- **Format**: `[ seconds.usecs] message` — bracketed boot-relative timestamp with optional leading spaces.
- **Subsystem extraction**: If the message contains a `subsystem ...:` pattern (e.g., `usb 1-1:`), the subsystem is extracted as the `target` field.
- **No level field**: dmesg text output does not carry priority information.
- **`detect_score`**: Proportion of sample lines with valid `[seconds.usecs]` prefix.

### Kubernetes CRI Parser (parser/kube_cri.rs)

- **`KubeCriParser`** implements `LogFormatParser`. Handles the Kubernetes Container Runtime Interface log format.
- **Format**: `ISO_TIMESTAMP stdout|stderr F|P message` — ISO 8601 timestamp, stream indicator, full/partial flag, then message.
- **Extra fields**: `stream` (stdout/stderr); partial lines with `P` flag add a `partial=true` extra field.
- **Very distinctive**: The `stdout`/`stderr` + `F`/`P` combination makes false positives extremely unlikely.
- **`detect_score`**: Proportion of sample lines that match the strict CRI format.

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

### Export (export.rs)

- **Template-based export** of analysis (comments + marked lines) to formatted documents (Markdown, Jira wiki, or custom).
- **Template syntax**: Section markers `{{#name}}...{{/name}}` with recognized sections: `header` (once), `comment_group` (per comment/mark entry), and optional `footer` (once). Placeholders: `{{filename}}`, `{{date}}`, `{{commentary}}`, `{{lines}}`, `{{line_numbers}}`.
- **`ExportTemplate`**: Parsed template with `header`, `comment_group`, and optional `marked_lines`/`footer` sections.
- **`ExportData`**: Bundles filename, comments, marked indices, and file reader for rendering.
- **`parse_template(raw)`**: Extracts sections from raw template text.
- **`load_template(name)`**: Resolves template files — checks `~/.config/logana/templates/{name}.txt` first, falls back to `./templates/{name}.txt` (same pattern as `Theme::from_file`).
- **`list_templates()`**: Scans both directories for `.txt` files, dedupes, sorts (same pattern as `Theme::list_available_themes`).
- **`render_export(template, data)`**: Renders the full document — header with filename/date, then comments and standalone marked lines interleaved in log order (consecutive standalone marks are grouped). Lines are rendered through the detected format parser (same as the TUI display) when available; falls back to raw bytes for plain text logs.
- **`complete_template(partial)`**: Fuzzy-match completion for template names.
- **Bundled templates**: `templates/markdown.txt` (Markdown with fenced code blocks), `templates/jira.txt` (Jira wiki with `{noformat}` blocks).
- **`:export <path> [-t <template>]`** command: default template is `markdown`. Tab-completes both file paths and template names (`-t`/`--template` flag).

### UI (ui/)

- **`App`** owns a `Vec<TabState>`, the global theme, and an `Arc<Keybindings>` shared across all tabs.
- **`TabState`** owns:
  - `file_reader: FileReader` — the backing log data
  - `log_manager: LogManager` — filter defs and marks
  - `detected_format: Option<Box<dyn LogFormatParser>>` — auto-detected log format parser (sampled on tab creation)
  - `visible_indices: Vec<usize>` — indices of currently visible lines under active filters
  - `scroll_offset: usize` — selected line (index into `visible_indices`)
  - `viewport_offset: usize` — first rendered line (index into `visible_indices`)
  - `visible_height: usize` — content rows available (updated each render frame)
  - `keybindings: Arc<Keybindings>` — shared keybinding config (cloned from `App` on tab creation)
  - `mode: Box<dyn Mode>`, `command_history: Vec<String>`, `search: Search`, plus display flags
- **`Mode` trait**: Each mode owns its key-handling logic via `handle_key(self: Box<Self>, tab, key, modifiers) -> (Box<dyn Mode>, KeyResult)`. Unhandled keys return `KeyResult::Ignored`, falling through to `App::handle_global_key` (quit, Tab switch, Ctrl+w/t). `KeyResult::ExecuteCommand(cmd)` triggers `App::execute_command_str`.
- **Mode structs**: `NormalMode { count }`, `CommandMode` (with tab completion, history), `FilterManagementMode`, `FilterEditMode`, `SearchMode`, `ConfirmRestoreMode`, `ConfirmRestoreSessionMode`, `VisualLineMode { anchor, count }`, `CommentMode`, `KeybindingsHelpMode`, `SelectFieldsMode`, `DockerSelectMode`, `ValueColorsMode`.
- **`ModeRenderState` enum** (ISP-compliant): Each mode implements `render_state() -> ModeRenderState`, returning a typed variant carrying exactly the data its renderer needs. Variants: `Normal`, `Command { input, cursor, completion_index }`, `Search { query, forward }`, `FilterManagement { selected_index }`, `FilterEdit`, `VisualLine { anchor }`, `Comment { lines, cursor_row, cursor_col, line_count }`, `KeybindingsHelp { scroll, search }`, `SelectFields { fields, selected }`, `DockerSelect { containers, selected, error }`, `ValueColors { groups, search, selected }`, `ConfirmRestore`, `ConfirmRestoreSession { files }`. The renderer does a single `match` on the enum instead of calling many optional trait methods.
- **`refresh_visible()`**: Rebuilds `visible_indices` by calling `FilterManager::compute_visible(&file_reader)`, then applies date filters as a post-processing `retain()` step (see Date Filter section).

**Rendering pipeline (per frame)**:
1. Compute `visible_height = logs_area.height - 2` (subtract Block borders).
2. Compute `inner_width` (terminal columns available inside borders, minus line-number prefix).
3. Wrap-aware viewport adjustment: when wrap is ON, sums terminal rows (via `line_row_count`) from `viewport_offset` to `scroll_offset`; scrolls when total exceeds `visible_height`.
4. Wrap-aware `end` computation: walks from `start` accumulating `line_row_count()` until `visible_height` is filled.
5. Build `FilterManager` + 256-slot styles array (filter styles at 0..N, search style at index 255).
6. For each line in `[start..end]`: if `detected_format` is set, parse via the trait (`parser.parse_line(line_bytes)`) to get `DisplayParts`, then format into columns via `apply_field_layout`; otherwise fall back to raw byte rendering. Evaluate filters (`evaluate_line`), overlay search spans at priority 1000, apply level colours and mark styles, compose final `Line` via `render_line`.
7. Apply value-based coloring (`colorize_known_values`) to spans with no `fg` set — HTTP methods, status codes, and IP addresses get per-token colors from `theme.value_colors`. Spans already colored by filters or search are left untouched.
8. `line_row_count(bytes, inner_width)` uses `unicode_width` to compute `ceil(display_width / inner_width)`, keeping wrap-aware viewport math precise.

**Structured field layout**: `apply_field_layout(&DisplayParts, &FieldLayout, &HashSet<String>) -> Vec<String>` — module-level helper that routes through `default_cols` (all columns, default order) or picks specific columns via `get_col`, with name-based hidden-field filtering. Column name resolution: `get_col()` checks all aliases from `TIMESTAMP_KEYS`, `LEVEL_KEYS`, `TARGET_KEYS`, `MESSAGE_KEYS` arrays to map raw JSON key names to `DisplayParts` slots, plus `span` and dotted sub-field names (`span.*`, `fields.*`). Tab completion for the `fields` command completes against the five canonical names plus dynamically discovered field names from the first 200 visible log lines (`TabState::collect_field_names()`, which delegates to the detected format parser's `collect_field_names`).
**Format auto-detection**: On tab creation, the first 200 lines are sampled and passed to `detect_format()`, which tries all registered parsers and stores the best match in `TabState::detected_format`. The rendering pipeline dispatches through the trait: `detected_format.as_ref().and_then(|parser| parser.parse_line(line_bytes))`. If no format is detected, lines fall back to raw byte rendering.
**Select-fields mode** (`:select-fields`): floating popup showing all discovered structured fields with checkboxes. `j`/`k` navigate, `Space` toggle, `J`/`K` reorder, `a`/`n` enable/disable all, `Enter` apply, `Esc` cancel. Implemented by `SelectFieldsMode` in `src/mode/select_fields_mode.rs`.
**Go-to-line** (`:N`): Typing a bare number in command mode (e.g. `:500`) jumps to that 1-based line number. If the target line is hidden by filters, jumps to the closest visible line (binary search on `visible_indices`, picks the nearer neighbour). Line 0 returns an error; numbers beyond the file jump to the last visible line. Implemented via `TabState::goto_line()` with a fast-path check in `run_command` before clap parsing.
**Vim keybindings**: j/k, gg/G, Ctrl+d/u (half page), PageUp/Down, /, ?, n/N, m (mark), V (visual select). **Count prefix**: typing digits before a motion repeats it — `5j` moves down 5, `10k` up 10, `50G` goes to line 50, `3gg` goes to line 3, `2Ctrl+d` scrolls 2 half-pages. Count is accumulated in `NormalMode.count` / `VisualLineMode.count` and capped at 999,999. Digits 1-9 start a count; 0 appends when a count is active. Count is consumed by motions and reset on non-motion keys. The active count is shown in the status bar (e.g. `[NORMAL] 5`).
**Docker logs** (`:docker`): runs `docker ps` to list running containers, opens a `DockerSelectMode` popup (j/k navigate, Enter attach, Esc cancel). On selection, spawns `docker logs -f <id>` via `FileReader::spawn_process_stream()` and opens a new streaming tab. `DockerContainer { id, name, image, status }` in `types.rs`. Docker tabs persist across sessions via `source_file = "docker:name"`; on session restore, the `"docker:"` prefix is detected and `restore_docker_tab()` re-spawns the stream by container name instead of attempting a file load.
**Visual line mode** (`V`): anchor at current line, j/k extend selection, `c` opens comment editor, `y` yanks (copies) selected lines to system clipboard via `arboard`, Esc cancel. All keys are configurable via `VisualLineKeybindings`. Selected range highlighted in the log panel.
**Comment mode**: multiline text editor (Enter = newline, Backspace = delete/merge, Left/Right wrap lines, Up/Down move rows, Ctrl+S = save (configurable), Esc = cancel). Rendered as a floating popup. Commented lines show a `◆` indicator in the line-number margin. In normal mode, `c` on a commented line opens edit mode (pre-filled text); `C` clears all marks and comments. In edit mode, `Ctrl+D` deletes the comment.
**Keybindings help** (`F1`): floating popup listing all configured keybindings grouped by mode. Type to fuzzy-search, j/k scroll, Esc/q/F1 close. The status bar reflects the actual configured keybinding strings.
**Conflict validation**: at startup `Keybindings::validate()` checks for overlapping bindings within each mode scope; conflicts are printed to stderr and logged as warnings.
**Multi-tab**: Tab/Shift+Tab switch, Ctrl+t open, Ctrl+w close
**Command mode** (`:`) with highlight-then-accept tab completion, history, live hints. Tab/BackTab cycle a highlight over completions in the hint area without changing input; Enter accepts the highlighted completion into the input (single match = accept+execute immediately). `CommandMode::compute_completions()` encapsulates the 5-tier completion logic (color → template → file path → theme → command name). `completion_index()` trait method exposes the active highlight to the renderer.
**Commands**: `filter`, `exclude`, `set-color`, `export-marked`, `export`, `save-filters`, `load-filters`, `wrap`, `set-theme`, `level-colors`, `open`, `close-tab`, `hide-field`, `show-field`, `show-all-fields`, `fields [col...]`, `select-fields`, `docker`, `value-colors`, `date-filter`
**Filter management mode** (`f`): navigate, toggle, delete, edit, set color, add include/exclude/date-filter

### Config (config.rs)

- **Config file**: `~/.config/logana/config.json` (loaded at startup; falls back to defaults on parse/IO error — never prevents startup).
- **`Config`**: `{ theme: Option<String>, keybindings: Keybindings }`. `theme` is a theme name without the `.json` extension (e.g. `"dracula"`).
- **`Keybindings`**: groups `NavigationKeybindings` (shared scroll/page keys used across all modes), `NormalKeybindings`, `FilterKeybindings`, `GlobalKeybindings`, `CommentKeybindings`, `VisualLineKeybindings`, `DockerSelectKeybindings`, `ValueColorsKeybindings`, `SelectFieldsKeybindings`, `HelpKeybindings`, `ConfirmKeybindings` — each with `#[serde(default)]` so any absent field uses its built-in default. Navigation keys (scroll_down/up, half_page_down/up, page_down/up) are configured once in the `navigation` group and shared by all modes.
- **`KeyBindings`** (per action): a `Vec<KeyBinding>` — each action supports multiple alternative keys (e.g. `"j"` and `"Down"` for scroll down). Accepts a single JSON string or an array of strings.
- **`KeyBinding`**: parsed from strings like `"j"`, `"Ctrl+d"`, `"Shift+Tab"`, `"F1"`, `"PageDown"`, `"Space"`, `"Esc"`. `"Shift+Tab"` maps to `KeyCode::BackTab`. `matches(key, modifiers)`: for `Char` keys accepts `NONE` or `SHIFT` (terminals vary); for non-`Char` keys (Enter, F-keys, etc.) requires an exact SHIFT match so `"Shift+Enter"` ≠ plain `"Enter"`.
- **`Keybindings::validate() -> Vec<String>`**: checks all (action, keybinding) pairs within each mode scope (normal + global, filter + global) for overlaps and returns human-readable conflict descriptions. Called at startup; conflicts are printed to stderr and logged.
- **Sharing**: `Arc<Keybindings>` is held by `App` and cloned into each `TabState` when tabs are created (including session restores and new tabs opened via commands).
- **Default keybindings** exactly match the previously hardcoded key assignments, so the config file is fully optional. Default for `annotation.save` is `Shift+Enter`; `normal.show_keybindings` is `F1`.

Example `~/.config/logana/config.json`:
```json
{
  "theme": "dracula",
  "keybindings": {
    "navigation": { "scroll_down": ["j", "Down"], "half_page_down": "Ctrl+d" },
    "normal": { "scroll_left": "h", "scroll_right": "l" },
    "global": { "quit": "q" }
  }
}
```

### Theme (theme.rs)

- JSON files from `themes/` or `~/.config/logana/themes/`
- Colors: hex `"#RRGGBB"` or RGB array `[r, g, b]`
- Default: Dracula theme (hardcoded)
- Fields: `root_bg`, `border`, `border_title`, `text`, `text_highlight`, `cursor_fg`, `trace_fg`, `debug_fg`, `notice_fg`, `warning_fg`, `error_fg`, `fatal_fg`, `search_fg`, `visual_select_bg`, `visual_select_fg`, `mark_bg`, `mark_fg`, `value_colors`
- **`ValueColors`**: Per-token color mappings for HTTP methods (`http_get`, `http_post`, `http_put`, `http_delete`, `http_patch`, `http_other`), status codes (`status_2xx`–`status_5xx`), IP addresses (`ip_address`), and UUIDs (`uuid`). All fields have `#[serde(default)]` so existing theme files need no changes. Overridable in theme JSON under `"value_colors": { ... }`.
- **`Theme::list_available_themes() -> Vec<String>`**: Scans both theme directories, returns sorted names (no extension).
- **`fuzzy_match(needle, haystack) -> bool`**: Case-insensitive subsequence check; used for `set-theme` tab completion.

### Value Colors (value_colors.rs)

- **Per-token coloring** for known values: HTTP methods (GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS), HTTP status codes (2xx–5xx), IPv4 and IPv6 addresses, UUIDs.
- **Regex patterns** compiled once via `std::sync::LazyLock`.
- **`colorize_known_values(line, &ValueColors) -> Line`**: Post-processes a rendered `Line`, scanning unstyled spans (no `fg` set) for known patterns. Matched tokens get split into sub-spans with appropriate colors. Spans already colored by filters or search highlights are left untouched.
- **Priority layering** (highest wins): cursor/mark/visual selection → search highlights → filter highlights → value colors → level colors (line-level fallback).
- **`:value-colors` command**: opens `ValueColorsMode` popup with a grouped hierarchy (HTTP methods, Status codes, Network, Identifiers). Groups show tri-state checkboxes (`[x]`/`[ ]`/`[-]`). `j`/`k` navigate, `Space` toggles group or entry, `a` enable all, `n` disable all, `Enter` apply, `Esc` cancel. Typing filters rows via fuzzy search; `Esc` clears search first, then cancels. Disabled categories are stored in `ValueColors.disabled` (runtime-only, not serialized to theme JSON).

## Key Patterns

- **Zero-copy reads**: `FileReader::get_line` returns `&[u8]` slices directly into the mmap — no per-line allocation.
- **Parallel filter evaluation**: `FilterManager::compute_visible` uses `rayon::into_par_iter()` over line indices; order is preserved by rayon's indexed parallel iterator.
- **Dual filter backends**: Aho-Corasick for literals (O(n) multi-pattern), Regex fallback for metacharacter patterns. Selected automatically by `build_filter`.
- **StyleId dispatch**: 256-slot `Vec<Style>` indexed by `u8` avoids per-span HashMap lookups at render time.
- **Wrap-aware viewport**: `line_row_count` (unicode_width) drives both the scroll trigger and the `[start..end]` window, so the selected line is always on-screen regardless of line length.
- **Async DB access**: All `LogManager` methods are `async fn` and `await` DB calls directly. No `block_on` or manual runtime bridging.
- **Repository pattern**: `FilterStore` / `FileContextStore` traits enable in-memory SQLite for tests.
- **Session persistence**: Filters + UI context saved per `source_file`; hash-verified restore prompt on reopen. Docker tabs are stored as `"docker:name"` and restored by detecting the prefix (re-spawns `docker logs -f` by container name).

## App Lifecycle

1. Parse CLI args (optional file path).
2. Init tokio runtime + SQLite DB (`~/.local/share/logana/logana.db`).
3. Load `Config` from `~/.config/logana/config.json` (or defaults on missing/parse error).
4. Build `FileReader` from file path (mmap) or stdin (bytes).
5. Build `LogManager` — loads filters from DB for this source.
6. Enter terminal raw mode, create `App` with theme and `Arc<Keybindings>`.
7. If a file was opened: check for saved `FileContext`, prompt per-file restore (`ConfirmRestoreMode`). If no file and no piped data: check for a saved session (`session_tabs`), prompt session restore (`ConfirmRestoreSessionMode`). On confirm, all session files are opened and their per-file contexts auto-applied without additional prompts.
8. **Event loop** (250ms poll): render frame → wait for key event → handle key → repeat.
9. On exit: save `FileContext` for each tab + save the session (list of open source files), restore terminal.

## Dependencies

anyhow, clap (derive), regex, ratatui 0.26, crossterm 0.27, serde/serde_json, serde_with, sqlx 0.8 (sqlite, tokio), tokio (rt-multi-thread), async-trait, dirs, tempfile, unicode-width, memmap2, memchr, aho-corasick, rayon, time, tracing, tracing-subscriber, tracing-appender, arboard 3 (cross-platform clipboard)

## Testing

- **Unit tests**: db.rs, filters.rs, file_reader.rs, log_manager.rs, search.rs, types.rs, ui/app.rs, ui/commands.rs, ui/field_layout.rs, auto_complete.rs, export.rs, parser/types.rs, parser/mod.rs, parser/json.rs, parser/syslog.rs, parser/journalctl.rs, parser/clf.rs, parser/timestamp.rs, parser/logfmt.rs, parser/common_log.rs, parser/web_error.rs, parser/dmesg.rs, parser/kube_cri.rs, value_colors.rs, date_filter.rs, mode/annotation_mode.rs, mode/visual_mode.rs, mode/app_mode.rs, mode/select_fields_mode.rs, mode/value_colors_mode.rs — 1149 tests total
- **Integration tests** (tests/integration.rs): FileReader line access, filter include/exclude/regex/disabled, marks, search on visible lines, filter CRUD — 15 tests
- **Stdin tests** (tests/stdin.rs): pipe input end-to-end — 14 tests
- **CI**: cargo fmt → clippy → test → tarpaulin coverage (enforces 80%)
