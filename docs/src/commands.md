# Commands

## CLI Flags

These flags are passed when launching logana from the shell:

| Flag | Description |
|---|---|
| `<file>` | File or directory to open. Omit to read from stdin. |
| `-f`, `--filters <path>` | Preload a saved filter set (JSON). Filters are applied in a single pass during indexing and remain active for interactive use. |
| `-t`, `--tail` | Start at the end of the file and enable tail mode. Combined with `--filters`, the last matching line is available immediately after loading. |

## In-App Commands

Press `:` in normal mode to open command mode. Tab completes commands, flags, colors, themes, and file paths. Command history is navigable with `Up` / `Down`.

## Filtering

| Command | Description |
|---|---|
| `:filter <pattern>` | Add an include filter (show only matching lines) |
| `:exclude <pattern>` | Add an exclude filter (hide matching lines) |
| `:date-filter <expr>` | Add a date/time range filter |
| `:set-color [--fg COLOR] [--bg COLOR]` | Set highlight color for the selected filter |
| `:save-filters <file>` | Save current filters to a JSON file |
| `:load-filters <file>` | Load filters from a JSON file |

See [Filtering](filtering/index.md) and [Date & Time Filters](filtering/date-filters.md) for full details.

## Navigation

| Command | Description |
|---|---|
| `:<N>` | Jump to line N (e.g. `:500`) |

## Files and Tabs

| Command | Description |
|---|---|
| `:open <path>` | Open a file or directory |
| `:close-tab` | Close the current tab |

## Display

| Command | Description |
|---|---|
| `:wrap` | Toggle line wrap on/off |
| `:tail` | Toggle tail mode (auto-scroll on new content) |
| `:raw` | Toggle raw mode — bypass the format parser and show unformatted log lines; title shows `[RAW]` when active |
| `:level-colors` | Open the level colors dialog — toggle coloring per level (TRACE, DEBUG, INFO, NOTICE, WARNING, ERROR, FATAL); INFO/TRACE/DEBUG/NOTICE are off by default |
| `:value-colors` | Open the value colors dialog — toggle coloring for HTTP methods, status codes, IPs, UUIDs, and process/logger names |
| `:set-theme <name>` | Switch the color theme |

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
| `:fields [col ...]` | Set visible columns (e.g. `:fields timestamp level message`) |
| `:hide-field <col>` | Hide a single column |
| `:show-field <col>` | Show a previously hidden column |
| `:show-all-fields` | Reset to default column display |
| `:select-fields` | Open an interactive column picker |
| `:show-keys` | Show field keys alongside values (e.g. `method=GET`) |
| `:hide-keys` | Show only values, hiding field keys (default) |

## Export and Docker

| Command | Description |
|---|---|
| `:export <file> [-t <template>]` | Export annotations to a file (default template: markdown) |
| `:docker` | Pick and stream a running Docker container |

## Tab Completion

Command mode supports multi-tier tab completion:

1. **Color names** — after `--fg` or `--bg` flags
2. **Template names** — after `-t` / `--template` flags in `:export`
3. **File paths** — for `:open`, `:save-filters`, `:load-filters`, `:export`
4. **Theme names** — for `:set-theme`
5. **Command names** — for everything else

Press `Tab` / `Shift+Tab` to cycle through completions. A highlighted suggestion appears in the hint area; `Space` accepts it.
