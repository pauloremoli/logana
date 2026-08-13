# Commands

## CLI Flags

These flags are passed when launching logana from the shell:

| Flag | Description |
|---|---|
| `<file>` | File or directory to open. Omit to read from stdin. |
| `-f`, `--filters <path>` | Preload a saved filter set (JSON). Filters are applied in a single pass during indexing and remain active for interactive use. |
| `-i`, `--include <args>` | Add an include filter. Accepts the same arguments as `:filter`. May be repeated. Examples: `-i "error"`, `-i "--field level=ERROR"` |
| `-o`, `--exclude <args>` | Add an exclude filter. Accepts the same arguments as `:exclude`. May be repeated. Examples: `-o "debug"`, `-o "--field level=debug"` |
| `-t`, `--timestamp <args>` | Add a date/time range filter. Accepts the same arguments as `:date-filter`. May be repeated. |
| `--tail` | Start at the end of the file and enable tail mode. Combined with `--filters`, the last matching line is available immediately after loading. |
| `--mcp [PORT]` | Start the embedded MCP server on launch. Port defaults to 9876. See [MCP Server](mcp.md). |
| `--headless` | Run without TUI — apply filters and write matching lines to stdout or `--output`. |
| `--output <path>` | Write headless output to a file instead of stdout. Requires `--headless`. |

## In-App Commands

Press `:` in normal mode to open command mode. Tab completes commands, flags, colors, themes, and file paths. Command history is navigable with `Up` / `Down`.

## Filtering

| Command | Description |
|---|---|
| `:filter [--regex\|-r] [--ignore-case\|-i] [-l] [--fg COLOR] [--bg COLOR] <pattern>` | Add an include filter (show only matching lines) |
| `:filter --field <key>=<value>` | Add a field-scoped include filter (e.g. `level=error`); repeat to require several fields at once |
| `:filter --group\|-g <name> <pattern>` | Assign the filter to a named group, toggleable together via `:toggle-group` |
| `:filter --auto\|-a <pattern>` | Add an include filter with a randomly generated, readable fg/bg color pair instead of specifying `--fg`/`--bg` |
| `:exclude [--regex\|-r] [--ignore-case\|-i] <pattern>` | Add an exclude filter (hide matching lines) |
| `:exclude --field <key>=<value>` | Add a field-scoped exclude filter (e.g. `level=debug`) |
| `:exclude --group\|-g <name> <pattern>` | Assign the exclude filter to a named group |
| `:highlight [--regex\|-r] [--ignore-case\|-i] [-l] [--fg COLOR] [--bg COLOR] <pattern>` (alias `:h`) | Add a highlight filter — colors matches without affecting visibility |
| `:highlight --auto\|-a <pattern>` | Add a highlight filter with a randomly generated, readable fg/bg color pair |
| `:date-filter <expr>` | Add a date/time range filter |
| `:set-color [--fg COLOR] [--bg COLOR]` | Set highlight color for the selected filter |
| `:toggle-group <name>` | Toggle every filter in a named group on/off together |
| `:group <name> [--fg COLOR] [--bg COLOR] [-l] [--auto] [--clear]` | Set, update, or clear a group's predefined color style, used by filters in the group with no color of their own |
| `:filtering` | Toggle all filtering on/off (bypass every filter) |
| `:clear-filters` | Remove all filter definitions |
| `:disable-filters` | Disable all filters without removing them |
| `:enable-filters` | Enable all disabled filters |
| `:save-filters <file>` | Save current filters to a JSON file |
| `:load-filters <file>` | Load filters from a JSON file |

> **Flag ordering:** All options (`--regex`, `--fg`, `--bg`, `-l`, `--field`, `--group`/`-g`, `--ignore-case`/`-i`, `--auto`) must appear **before** the pattern. Everything after the first pattern word is treated as part of the pattern text. `--auto` cannot be combined with `--fg`/`--bg`.

See [Filter Groups](filtering/index.md#filter-groups) for the Groups sidebar and group management mode (`Ctrl+g`).

See [Filtering](filtering/index.md), [Date & Time Filters](filtering/date-filters.md), and [Field Filters](filtering/field-filters.md) for full details.

## Navigation

| Command | Description |
|---|---|
| `:<N>` | Jump to line N (e.g. `:500`) |

## Files and Tabs

| Command | Description |
|---|---|
| `:open <path>` | Open a file, directory, or compressed/archive file. Directories and archives both show the same contents picker first — see [Quick Start](quick-start.md#opening-compressed-and-archive-files) |
| `:close-tab` | Close the current tab (quits if it's the last tab) |
| `:save <path>` | Save the currently visible (filtered) lines to a file in raw format |
| `:export-marked <path>` | Export marked lines to a file |
| `:run <program> [args...]` | Execute a command and stream its output to a new tab; stderr lines show as errors |
| `:schema [name]` | Show the active schema, or switch this tab to a named custom or built-in schema. Use `:schema none` to treat the file as plain text |
| `:default-filters [format] [path]` | Configure a filter file to auto-load whenever a format is assigned to a tab with no filters yet. No args opens a popup listing every format |

## Display

| Command | Description |
|---|---|
| `:wrap` | Toggle line wrap on/off (persisted across sessions) |
| `:line-numbers` | Toggle the line number gutter on/off (persisted across sessions) |
| `:relative-line-numbers` | Toggle relative line numbers — other rows show their distance from the selected row (persisted across sessions) |
| `:tail` | Toggle tail mode (auto-scroll on new content) |
| `:raw` | Toggle raw mode — bypass the format parser and show unformatted log lines; title shows `[RAW]` when active |
| `:collapse` | Hide continuation lines file-wide, showing only each entry's first line |
| `:expand` | Reveal continuation lines previously hidden by `:collapse` |
| `:level-colors` | Open the level colors dialog — toggle coloring per level (TRACE, DEBUG, INFO, NOTICE, WARNING, ERROR, FATAL); INFO/TRACE/DEBUG/NOTICE are off by default |
| `:value-colors` | Open the value colors dialog — toggle coloring for HTTP methods, status codes, IPs, UUIDs, and process/logger names |
| `:set-theme <name>` | Switch the color theme (persisted across sessions) |
| `:theme` | Open a searchable picker to browse themes with live preview; Enter applies and persists, Esc restores the previous theme |
| `:sidebar-position left\|right` | Move the filter sidebar to the left or right of the log panel (persisted across sessions) |

## OTel Collector

| Command | Description |
|---|---|
| `:otel [port]` | Open an OTLP gRPC receiver tab (default port 4317) |
| `:otel --http [port]` | Open an OTLP HTTP/JSON receiver tab (default port 4318) |

See [OTel Collector](otel.md) for full details.

## MCP Server

| Command | Description |
|---|---|
| `:enable-mcp [--port N]` | Start the embedded MCP server (default port 9876) |
| `:disable-mcp` | Stop the MCP server |

See [MCP Server](mcp.md) for full details.

## Live Data

These commands control how the current tab handles incoming data from a file watcher or stream (stdin, Docker).

| Command | Description |
|---|---|
| `:stop` | Permanently stop all incoming data for the current tab — drops the file watcher and/or stream |
| `:pause` | Freeze the view; the background watcher/stream keeps running. Title shows `[PAUSED]` |
| `:resume` | Resume applying incoming data; the latest snapshot is applied immediately |

> **Note:** `:pause` / `:resume` are non-destructive — no data is lost while paused. `:stop` is permanent; to resume watching a file after stopping, reopen it with `:open`.

## Structured Fields

| Command | Description |
|---|---|
| `:hide-field <col>` | Hide a single column |
| `:show-field <col>` | Show a previously hidden column |
| `:show-all-fields` | Reset to default column display |
| `:select-fields` | Open an interactive column picker |
| `:show-keys` | Show field keys alongside values (e.g. `method=GET`) |
| `:hide-keys` | Show only values, hiding field keys (default) |

## Merged View

| Command | Description |
|---|---|
| `:merge` | Open a source-selection popup, then create a new tab interleaving the selected tabs sorted by timestamp |

See [Multi-Tab](multi-tab.md#merged-view) for full details.

## Export and Streaming

| Command | Description |
|---|---|
| `:export <file> [-t <template>]` | Export annotations to a file (default template: markdown) |
| `:docker` | Pick and stream a running Docker container |
| `:dlt` | Pick and stream from a DLT daemon over TCP |

## Session

| Command | Description |
|---|---|
| `:reset` | Restore all settings to defaults and clear all persisted state |

## Tab Completion

Command mode supports multi-tier tab completion:

1. **Color names** — after `--fg` or `--bg` flags
2. **Template names** — after `-t` / `--template` flags in `:export`
3. **File paths** — for `:open`, `:save-filters`, `:load-filters`, `:export`
4. **Theme names** — for `:set-theme`
5. **Command names** — for everything else

Press `Tab` / `Shift+Tab` to cycle through completions. A highlighted suggestion appears in the hint area; `Space` accepts it.
