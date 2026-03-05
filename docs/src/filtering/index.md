# Filtering

Filters are the primary way to narrow the log view. They are layered: include patterns narrow the view, and exclude patterns hide matching lines on top of whatever include filters already selected.

## Quick Keys

| Key | Action |
|---|---|
| `i` | Add include filter (show only matching lines) |
| `o` | Add exclude filter (hide matching lines) |
| `f` | Open filter manager |
| `F` | Toggle all filtering on/off |

## How Filters Work

**Include filters:** If any include filter is enabled, only lines matching at least one include filter are shown.

**Exclude filters:** Any line matching an enabled exclude filter is hidden, regardless of include filters.

**No filters:** All lines are shown.

Both filter types support:
- **Literal strings** — fast multi-pattern matching via Aho-Corasick
- **Regular expressions** — full regex syntax via the `regex` crate (activated automatically when the pattern contains metacharacters)

## Filter Persistence

Filters are saved to SQLite and automatically restored the next time you open the same file. When you reopen a file, logana detects whether the file has changed (via hash) and prompts you to restore the previous session.

## Filter Manager

Press `f` to open the filter manager popup, which lists all active filters.

| Key | Action |
|---|---|
| `j` / `k` | Navigate filters |
| `Space` | Toggle selected filter on/off |
| `e` | Edit selected filter's pattern |
| `d` | Delete selected filter |
| `c` | Set highlight color for selected filter |
| `t` | Add a date/time range filter |
| `J` / `K` | Move filter down / up (order affects priority) |
| `A` | Toggle all filters on/off |
| `C` | Clear all filters |
| `Esc` | Close filter manager |

## Filter Colors

Each filter can have an optional highlight color. When a filter matches part of a line, that part is colored using the filter's configured color. Colors are set per-filter with `c` in the filter manager, or via the `:set-color` command.

```sh
:set-color --fg red
:set-color --fg "#FF5555" --bg "#282A36"
```

Color values accept:
- Named colors: `black`, `red`, `green`, `yellow`, `blue`, `magenta`, `cyan`, `white`, `gray`, `darkgray`, `lightred`, `lightgreen`, `lightyellow`, `lightblue`, `lightmagenta`, `lightcyan`
- Hex: `"#RRGGBB"`

## Save and Load Filters

Export the current filter set to a JSON file, and reload it later:

```sh
:save-filters my-filters.json
:load-filters my-filters.json
```

This is useful for sharing filter sets across machines or between log files with similar structure.

## Sections

- [Text Filters](text-filters.md) — include/exclude patterns, regex syntax
- [Date & Time Filters](date-filters.md) — timestamp-based range and comparison filters
