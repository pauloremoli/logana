# Filtering

Filters are the primary way to narrow the log view. They are layered: include patterns narrow the view, and exclude patterns hide matching lines on top of whatever include filters already selected.

## Quick Keys

| Key | Action |
|---|---|
| `i` | Add include filter (show only matching lines) |
| `o` | Add exclude filter (hide matching lines) |
| `f` | Open filter manager |
| `F` | Toggle all filtering on/off |
| `H` | Toggle highlight mode (see below) |

## How Filters Work

**Include filters:** If any include filter is enabled, only lines matching at least one include filter are shown.

**Exclude filters:** Any line matching an enabled exclude filter is hidden, regardless of include filters.

**Highlight filters:** Apply their color styling to matching lines but never affect visibility — every line stays shown. See [Text Filters](text-filters.md#highlight-filters) for details.

**No filters:** All lines are shown.

All filter types support:
- **Text search** — fast multi-pattern matching
- **Regular expressions** — full regex syntax, opt-in with `--regex` / `-r`

## Highlight Mode

Press `H` in normal mode to put **all** active filters — include, exclude, and highlight — into highlight mode: every line in the file stays visible, but filter colors still render on their matches. This is for reading the full context around the lines you actually care about — an include/exclude filter narrows the log down to just the matches, but the surrounding lines that explain *why* something happened are often not in the filter at all. Highlight mode gives you the whole log back, with your filters still marking what matters, so you can scroll through real context without losing track of what you were looking for. The sidebar title shows `[HIGHLIGHT]` while it's active. Press `H` again to return to normal filtering.

## Filter Persistence

Filters are saved to SQLite and automatically restored the next time you open the same file. When you reopen a file, logana detects whether the file has changed (via hash) and prompts you to restore the previous session.

## Filter Manager

Press `f` to open the filter manager popup, which lists all active filters. Navigation matches the log panel: count-prefixed motions, page scrolling, jump-to-top/bottom, and search.

| Key | Action |
|---|---|
| `j` / `k` | Move selection down / up — accepts a count prefix, e.g. `4j` moves down 4 |
| `Ctrl+d` / `Ctrl+u` | Half page down / up |
| `PageDown` / `PageUp` | Full page down / up |
| `gg` / `G` | Jump to the first / last filter — `{count}gg` or `{count}G` jumps to filter N |
| `/` | Search the filter list (see below) |
| `Space` | Toggle selected filter on/off |
| `e` | Edit selected filter's pattern |
| `d` | Delete selected filter |
| `c` | Set highlight color for selected filter |
| `t` | Add a date/time range filter |
| `h` | Add a highlight filter |
| `J` / `K` | Move filter down / up (order affects priority) |
| `A` | Toggle all filters on/off |
| `C` | Clear all filters |
| `Esc` | Close filter manager |

### Searching the Filter List

Press `/` to start typing a search query — the sidebar title immediately shows a `type to search...` placeholder so it's clear you're now typing a query, even before you've entered any characters. The list narrows live to filters whose type, pattern, or group match what you've typed so far (the same convention used by the Value Colors, Level Colors, and Keybindings Help popups); once you've typed something, the title shows `/query` instead of the placeholder. `Backspace` edits the query; `j`/`k` move between the narrowed matches. `Enter` confirms your selection and shows the full list again; `Esc` cancels and restores whatever was selected before you started searching. While searching, every key is captured as query text — including letters that are normally shortcuts (`e`, `d`, `i`, …) — so none of the usual filter-manager actions fire until you confirm or cancel.

## Filter Colors

Each filter can have an optional highlight color. When a filter matches part of a line, that part is colored using the filter's configured color. Colors are set per-filter with `c` in the filter manager, or via the `:set-color` command.

```sh
:set-color --fg red
:set-color --fg "#FF5555" --bg "#282A36"
```

Color values accept:
- Named colors: `black`, `red`, `green`, `yellow`, `blue`, `magenta`, `cyan`, `white`, `gray`, `darkgray`, `lightred`, `lightgreen`, `lightyellow`, `lightblue`, `lightmagenta`, `lightcyan`
- Hex: `"#RRGGBB"`

### Style composition

When multiple filters overlap on the same text segment, their `fg` and `bg` attributes are composed independently — the highest-priority filter that has `fg` set contributes the foreground color, and the highest-priority filter that has `bg` set contributes the background color. So a level filter that sets `--fg yellow` and a text filter that sets `--bg darkgray` on the same word will both apply without one canceling the other.

### Color priority

Filter colors take priority over automatic value colors (HTTP methods, status codes, IPs, UUIDs) and log-level colors. Value colors are applied only to spans that are not already covered by a filter — they can still appear alongside filter colors on the same line, just not on the same character span. Log-level colors are the lowest-priority fallback and apply only to text that carries no explicit color from any other source.

## Save and Load Filters

Export the current filter set to a JSON file, and reload it later:

```sh
:save-filters my-filters.json
:load-filters my-filters.json
```

This is useful for sharing filter sets across machines or between log files with similar structure.

## Inline Filters at Startup

Add filters directly on the command line without creating a JSON file first:

| Flag | Short | Purpose |
|---|---|---|
| `--include <args>` | `-i` | Add include filter |
| `--exclude <args>` | `-o` | Add exclude filter |
| `--timestamp <args>` | `-t` | Add date/time range filter |

The argument string passed to each flag accepts exactly the same options as the corresponding TUI command (`:filter`, `:exclude`, `:date-filter`):

```sh
# Simple pattern
logana app.log -i error -o debug

# Field-scoped filter
logana app.log -i "--field level=ERROR"

# Include filter with highlight color (flags before pattern)
logana app.log -i "--bg red error"

# Date range filter
logana app.log -t "> 2024-02-21"

# Combined
logana app.log -i error -o debug -t "01:00 .. 02:00"
```

All flags can be repeated. Inline filters are applied after any `--filters` file. Invalid argument strings are rejected before the TUI opens.

## Preloading Filters at Startup

Pass `--filters` (or `-f`) on the command line to apply a saved filter set before the TUI opens:

```sh
logana app.log --filters my-filters.json
```

The filters are evaluated in a single pass during file indexing, so the filtered view is ready as soon as loading completes — no separate computation step. The same filters remain active for interactive use once the TUI is open (you can add, remove, or edit them normally).

Combined with `--tail`, the last matching line is shown immediately after loading:

```sh
logana app.log --filters errors.json --tail
```

> **Tip:** Save your most-used filter sets with `:save-filters` once, then reuse them from the command line.

## Sections

- [Text Filters](text-filters.md) — include/exclude patterns, regex syntax
- [Date & Time Filters](date-filters.md) — timestamp-based range and comparison filters
- [Field Filters](field-filters.md) — match against specific parsed fields (level, message, component, …)
