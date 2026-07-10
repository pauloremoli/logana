# logana Architecture

Terminal-based log analysis tool built in Rust with a Ratatui TUI. Filters and UI context are persisted in SQLite.

## High-Level Design

logana is structured around a strict separation between domain logic and the UI layer, divided into five broad concerns:

**File I/O & Ingestion** (`ingestion/`) — `FileReader` reads files and streaming sources (stdin, Docker, DLT TCP, file tailing, OTLP HTTP). Compressed and archive files are extracted in the background; only the extracted content is opened as tabs.

**Log Parsing** (`parser/`) — A format-detection registry inspects incoming bytes and selects the best `LogFormatParser` (JSON, syslog, journalctl, logfmt, CLF, DLT, user-defined custom schemas, etc.). Parsers produce a normalised `DisplayParts` struct consumed uniformly by the rest of the system. Extra fields carry a `FieldSemantic` tag enabling format-agnostic field filtering.

**Filter Pipeline** (`filters/`) — `FilterManager` compiles filter definitions into Aho-Corasick automata or regexes and evaluates them against every line to produce a visibility bitmap. The pipeline runs in a background thread. Filter definitions are persisted to SQLite and reloaded on startup.

**Mode System** (`mode/`) — Modal UI where each mode owns keyboard input and returns a `KeyResult` for effects that cross mode boundaries. Example of modes: Normal, Command, Search, Filter, Visual, Comment. Each window is also treated as a mode.

**UI & Rendering** (`ui/`) — The renderer reads tab state and produces widgets each frame; it never mutates state. The event loop dispatches key events to the active mode. Session state is persisted to SQLite and restored on reopen.

**Persistence** (`db/`) — `Database` owns the SQLite connection and schema migrations. Storage is accessed through four traits: `FilterStore` (filter definitions), `FileContextStore` (per-file scroll/search/display context), `SessionStore` (open file list), and `AppSettingsStore` (runtime toggles keyed by `SettingsKey`). `LogManager` builds `FilterManager` instances from persisted filter definitions. `MarkManager` and `CommentManager` are in-memory managers owned by `TabState`; their state is flushed to SQLite via `FileContext` on tab close/switch. Session save/restore is coordinated by `SessionManager` in `ui/session.rs`.

## Custom Schema Loading

User-defined schemas are loaded once at startup into a process-level `OnceLock` via `config::init_schemas()`. Every call to `detect_format` reads them from `config::custom_schemas()` without any parameter threading — schemas are automatically available to all code paths (startup, `:open`, streaming sources, Docker, DLT).

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

Decompression is an app-level operation. No tab is created for the archive itself — only tabs for its extracted contents.

```mermaid
graph TD
    CLI[logana archive.zip] --> BAE[begin_archive_extraction]
    Cmd[:open archive.zip] --> BAE
    BAE -->|spawn_blocking| Extract[extract_with_progress]
    Extract -->|progress_tx watch| Poll[poll_archive_extraction\ncalled each frame]
    Poll -->|decompression_message| Notif[App-level notification bar]
    Extract -->|result_tx oneshot| Poll
    Poll -->|ExtractedFile ×N| Tabs[Push content tabs]
    Tabs -->|begin_file_load ×N| Load[Background file load]
    Load -->|ReplaceTab| Tab[TabState with content]
```

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
