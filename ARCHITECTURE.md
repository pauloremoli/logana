# logana Architecture

Terminal-based log analysis tool built in Rust with a Ratatui TUI. Filters and UI context are persisted in SQLite.

## High-Level Design

logana is structured around a strict separation between domain logic and the UI layer, divided into five broad concerns:

**File I/O & Ingestion** (`ingestion/`) — `FileReader` reads files and streaming sources (stdin, Docker, DLT TCP, file tailing, OTLP HTTP). Compressed and archive files are extracted in the background; only the extracted content is opened as tabs.

**Log Parsing** (`parser/`) — A format-detection registry inspects incoming bytes and selects the best `LogFormatParser` (JSON, syslog, journalctl, logfmt, CLF, DLT, user-defined custom schemas, etc.). Parsers produce a normalised `DisplayParts` struct consumed uniformly by the rest of the system. Extra fields carry a `FieldSemantic` tag enabling format-agnostic field filtering.

**Filter Pipeline** (`filters/`) — `FilterManager` compiles filter definitions into Aho-Corasick automata or regexes and evaluates them against every line to produce a visibility bitmap. Each filter has a `FilterType` of `Include`, `Exclude`, or `Highlight`; `Highlight` filters apply their styling like `Include` but never contribute to the visibility decision. `FilterState::highlight_mode` (toggled with `H` in normal mode) is a separate, temporary override that makes every active filter — regardless of its own type — apply styling only, bypassing visibility for the whole tab. The pipeline runs in a background thread. Filter definitions are persisted to SQLite and reloaded on startup.

**Mode System** (`mode/`) — Modal UI where each mode owns keyboard input and returns a `KeyResult` for effects that cross mode boundaries. Example of modes: Normal, Command, Search, Filter, Visual, Comment. Each window is also treated as a mode.

**UI & Rendering** (`ui/`) — The renderer reads tab state and produces widgets each frame; it never mutates state. The event loop dispatches key events to the active mode. Session state is persisted to SQLite and restored on reopen.

**Persistence** (`db/`) — `Database` owns the SQLite connection and schema migrations. Storage is accessed through four traits: `FilterStore` (filter definitions), `FileContextStore` (per-file scroll/search/display context), `SessionStore` (open file list), and `AppSettingsStore` (runtime toggles keyed by `SettingsKey`). `LogManager` builds `FilterManager` instances from persisted filter definitions. `MarkManager` and `CommentManager` are in-memory managers owned by `TabState`; their state is flushed to SQLite via `FileContext` on tab close/switch. Session save/restore is coordinated by `SessionManager` in `ui/session.rs`.

## Custom Schema Loading

User-defined schemas are loaded once at startup into a process-level `OnceLock` via `config::init_schemas()`. Every call to `detect_format` reads them from `config::custom_schemas()` without any parameter threading — schemas are automatically available to all code paths (startup, `:open`, streaming sources, Docker, DLT).

`config::load_schemas` returns both the successfully loaded schemas and a list of per-file errors (malformed JSON); `parser::validate_custom_schemas` additionally compiles each loaded schema's template/pattern and collects compilation errors. Both error lists feed into `app.session.startup_warnings`, surfaced as a notification once the TUI opens, instead of silently dropping the offending schema from format detection.

```mermaid
graph LR
    Dir["~/.config/logana/schema/*.json"] -->|load_schemas| OnceLock["static CUSTOM_SCHEMAS"]
    OnceLock -->|custom_schemas| detect_format
    detect_format -->|prepend before built-ins| Parsers["CustomParser ×N + OtlpParser + ..."]
    Parsers -->|exclusivity-weighted scoring| Winner[LogFormatParser]
```

`CustomParser` is built from a `CustomSchemaConfig`. The config supports either a **template** (`{field}` placeholders compiled to named-capture regex) or a raw **regex pattern**. Placeholder names that match a canonical `FieldSemantic` name are resolved implicitly; others are mapped via the `fields` override map. The two new semantics added for this feature are `Component` and `Feature`.

## Component Diagram

```mermaid
graph TD
    CLI[CLI / main.rs] -->|creates| App[App]
    App -->|owns| Tab[TabState ×N]
    App -->|owns| Session[SessionManager]
    App -->|renders via| Renderer[Renderer]
    Tab -->|owns| Scroll[ScrollState]
    Tab -->|owns| Filter[FilterState]
    Tab -->|owns| Search[SearchState]
    Tab -->|owns| Cache[CacheState]
    Tab -->|owns| Stream[StreamState]
    Tab -->|owns| Display[DisplayConfig]
    Tab -->|owns| Interaction[InteractionState]
    Tab -->|owns| Marks[MarkManager]
    Tab -->|owns| Comments[CommentManager]
    Tab -->|owns| FileReader[FileReader]
    Tab -->|owns| LogManager[LogManager]
    Filter -->|holds| FM[FilterManager]
    Display -->|holds| Parser[LogFormatParser]
    LogManager -->|FilterStore| DB[(SQLite DB)]
    Session -->|FileContextStore / SessionStore / AppSettingsStore| DB
    FileReader -->|reads| Files[(Log files / stdin / streams)]
    Renderer -->|reads| Tab
    App -->|snapshot| MCP[MCP Server]
    MCP -->|McpCommand| App
```

`TabState` is decomposed into focused sub-structs (`ScrollState`, `FilterState`, `SearchState`, `CacheState`, `StreamState`, `DisplayConfig`, `InteractionState`). All `impl TabState` methods remain on `TabState` to avoid cross-cutting borrow complexity.

## Merged View

The `:merge` command opens a source-selection popup, then creates a new tab interleaving lines from the selected source tabs sorted by timestamp. No data is copied — the merged tab holds `Arc` references to the source `FileReader` instances.

```mermaid
graph TD
    Cmd[:merge] --> Popup[MergeSelectMode popup]
    Popup -->|source_tab_indices| Open[open_merge_tab]
    Open -->|build_merged_index| Index[Vec&lt;MergedEntry&gt; sorted by CanonicalTs]
    Index -->|FileReader::from_merged| MergedReader[FileReader - Merged storage]
    MergedReader -->|positional idx → entries[idx] → sources[source_idx].get_line| Lines[Source line bytes]
    Open --> MergedState[MergedState on TabState]
    MergedState -->|source_tab_indices| Advance[advance_merged_tabs each frame]
    Advance -->|extend_merged_index + begin_filter_refresh| MergedReader
```

`MergedEntry` holds a 23-byte `CanonicalTs` sort key, a `source_idx`, and a `line_idx`. The merged `FileReader` uses `Storage::Merged { entries, sources }` — `get_line(pos)` decodes `entries[pos]` and delegates to `sources[source_idx].get_line(line_idx)`. This makes the entire rendering pipeline work without modification.

Live updates are driven by `advance_merged_tabs`, which compares per-source line counts stored in `MergedState` against the current source tab `FileReader` sizes on every tick. When a source grows, new entries are appended to the sorted index and `begin_filter_refresh` is called so any active filter is re-evaluated. Updates stop when `MergedState::stopped` is `true` (set by `:stop`) or `StreamState::paused` is `true` (set by `:pause`).

## Commands

`src/commands/` contains clap-derived command definitions shared across layers. `src/ui/commands/` contains the handlers that execute `:` commands.

## MCP Server

An optional embedded MCP server (`mcp/`) exposes marks and annotations as resources and accepts tool calls that mutate TUI state. Tool calls are sent over an mpsc channel and applied by the event loop, keeping all mutable state on the TUI thread.

```mermaid
graph LR
    Tab[Active Tab] -->|marks + annotations| Snapshot[McpSnapshot\nArc-RwLock]
    Snapshot -->|read_resource| Client[MCP Client]
    Client -->|tool call| Server[LoganaServer]
    Server -->|McpCommand| Ch[mpsc channel]
    Ch -->|poll each frame| App[App event loop]
    App -->|mutate| Tab
```

## Headless Mode

```mermaid
graph LR
    CLI[CLI --headless] --> FR[FileReader]
    CLI -->|--include / --exclude| FM[FilterManager]
    FR -->|lines| FM
    FM -->|visible lines| Out[file / stdout]
    CLI -->|archive path| Decomp[extract_with_progress]
    Decomp -->|ExtractedFile ×N| FR
```

## File-Based Ingestion

```mermaid
graph LR
    File[(Log file)] --> FR[FileReader]
    FR -->|build line index| Lines[Line offsets]
    FR -->|sample lines| Detect{Format detection}
    Detect -->|select| Parser[LogFormatParser]
    Parser -->|parse_line| Fields[timestamp, level, message, ...]
```

## Stream-Based Ingestion

```mermaid
graph LR
    Source[DLT daemon / Docker / file tail / OTLP HTTP] -->|TCP / process / poll / HTTP POST| BG[Background task]
    BG -->|chunks| WatchCh[watch channel]
    WatchCh -->|each frame| Append[FileReader.append]
    Append -->|incremental| Filter[Filter new lines]

    Fail{Connection lost?} -->|yes| Retry[StreamRetryState]
    Retry -->|backoff delay| Reconnect[ConnectFn]
    Reconnect -->|success| WatchCh
    Reconnect -->|failure| Retry
```

## Archive Decompression

Opening a compressed or archive file first lists its contents in a picker popup instead of extracting immediately; only the confirmed selection is extracted. No tab is created for the archive itself — only tabs for the extracted files the user selected.

```mermaid
graph TD
    CLI[logana archive.zip] --> BAL[begin_archive_listing]
    Cmd[:open archive.zip] --> BAL
    BAL -->|spawn_blocking| List["list_archive_tree\n(archive_tree.rs)"]
    List -->|result_tx oneshot| PollList[poll_archive_listing\ncalled each frame]
    PollList -->|ArchiveTree| Picker[ArchivePickerMode\ncontents popup]
    Picker -->|Space/a/n toggle selection| Picker
    Picker -->|Enter → KeyResult::ExtractSelectedArchiveFiles| BAS[begin_archive_extraction_selected]
    BAS -->|spawn_blocking| Extract["extract_selected\n(confirmed files only)"]
    Extract -->|progress_tx watch| Poll[poll_archive_extraction\ncalled each frame]
    Poll -->|decompression_message| Notif[App-level notification bar]
    Extract -->|result_tx oneshot| Poll
    Poll -->|ExtractedFile ×N| Tabs[Push content tabs]
    Tabs -->|begin_file_load ×N| Load[Background file load]
    Load -->|ReplaceTab| Tab[TabState with content]
```

`ArchiveTree` (`ingestion::archive_tree`) is a flat arena — `Vec<ArchiveNode>` with parent/children indices — rather than a nested recursive enum, so toggling a subtree's selection and resolving a selected leaf's bytes back through its ancestor chain are both cheap index walks. `list_archive_tree` recurses into archives found nested inside other archives (a `.zip` inside a `.tar.gz`, for example) up to a depth/entry-count cap; an entry named like an archive whose content doesn't actually parse as one falls back to a plain selectable `File` node rather than a dead-end row. Entries read from a streaming source (`TarGz`/`TarBz2`/`TarXz`, which must be decompressed sequentially to enumerate) have their bytes cached during listing, up to a byte budget, so `extract_selected` can reuse them instead of decompressing the stream a second time; `Zip`/`Tar` support cheap re-reads and are never cached.

A nested entry that is itself a lone compressed file (Gz/Bz2/Xz — the shape the listing leaves as a `File` leaf rather than expanding into a `Container`, since it wraps exactly one file) is decompressed by `decompress_if_lone_compressed` inside `resolve_node_bytes`/`resolve_root_entry_bytes`, so extraction yields the file's actual content rather than raw compressed bytes; `display_name_for_extraction` strips the compression suffix from its extracted tab name to match. The picker's file tree also supports `/` to narrow by a live regex query (`ArchivePickerMode`), reusing the same `regex_search_match` helper as the filter sidebar search below — an invalid/incomplete regex falls back to a plain substring match.

## Filter Pipeline

```mermaid
graph TD
    Defs[Filter definitions from SQLite] --> Build{Build filter sets}
    Build --> Text[Text filters: Aho-Corasick / Regex]
    Build --> Date[Date filters]
    Build --> Field[Field filters]

    Line[Each line] --> Text
    Text -->|Include / Exclude / Neutral| Decision{Text decision}
    Decision -->|Exclude| Hidden[Line hidden]
    Decision -->|Include or Neutral| DateCheck{Date filters active?}
    DateCheck -->|yes| Date
    Date -->|no match| Hidden
    Date -->|match| FieldCheck
    DateCheck -->|no| FieldCheck{Field filters active?}
    FieldCheck -->|yes| Field
    Field -->|exclude match| Hidden
    Field -->|include match| Visible[Line visible]
    FieldCheck -->|no| Resolve{Has text includes?}
    Resolve -->|yes + Neutral| Hidden
    Resolve -->|no or Include| Visible
```

A `Highlight`-type filter's decision flows through this diagram the same way `Neutral` does — it never decides visibility on its own — but unlike `Neutral`, it still applies its color styling to the match. `FilterState::highlight_mode` short-circuits the whole diagram: every line is `Visible`, styling still applies.

A single `FieldFilter` (the `Field` box above) can hold several `(field, pattern)` conditions plus an optional free-text condition, stored as one `@field:`-prefixed `FilterDef` pattern with conditions joined by `\x1F` and a `\x02`-marked text segment; `field_filter_matches` requires *all* of a filter's own conditions (and its text, matched against the raw line) to hold — an AND within that one filter. The `Field` box's `.any()` loop across the filter *list* is unchanged: separate `FieldFilter`s (from separate `:filter`/`:exclude` commands) still OR together, same as plain text filters.

The filter sidebar (`FilterManagementMode`) supports the same count-prefixed `j`/`k`, half/full page, and `gg`/`G` motions as the log panel, plus `/` to narrow the filter list by a live regex query via `regex_search_match` (`commands::auto_complete`) — falling back to a plain substring match on an invalid/incomplete regex.

## Dependencies

| Crate | Role |
|---|---|
| **ratatui** | TUI rendering |
| **crossterm** | Terminal I/O, key events |
| **tokio** | Async runtime |
| **async-trait** | Async trait methods for the mode system |
| **memchr** | SIMD byte scanning for line indexing |
| **aho-corasick** | Literal substring filter matching |
| **regex** | Regex filter matching |
| **rayon** | Parallel line indexing and visibility scan |
| **sqlx** | SQLite async driver |
| **clap** | CLI argument parsing |
| **serde / serde_json / serde_with** | Config and theme serialisation |
| **schemars** | JSON Schema generation for config |
| **strum** | Enum-to-string and string-to-enum derives |
| **time** | Timestamp parsing for date-range filters |
| **unicode-width** | Unicode display width for cursor/truncation |
| **arboard** | Clipboard |
| **dirs** | XDG config and data directories |
| **anyhow** | Error handling |
| **libc** | Low-level OS interfaces |
| **tempfile** | Temporary files for archive extraction and stdin streaming |
| **flate2** | Gzip / deflate decompression |
| **zip** | ZIP archive parsing |
| **bzip2** | Bzip2 decompression |
| **xz2** | XZ/LZMA decompression |
| **tar** | Tar archive iteration |
| **rmcp** | MCP server implementation |
| **axum** | HTTP transport for MCP |
| **tonic** | gRPC transport for OTLP receiver |
| **opentelemetry-proto** | OTLP protobuf types |
| **prost** | Protobuf decoding |
