# Text Filters

Text filters match against the raw content of each log line.

## Adding Filters

**From normal mode:**
- Press `i` to add an include filter (opens command mode pre-filled with `filter `)
- Press `o` to add an exclude filter (opens command mode pre-filled with `exclude `)

**From command mode:**
```sh
:filter <pattern>       # show only lines matching pattern
:exclude <pattern>      # hide lines matching pattern
```

## Text Search

The default mode. Fast multi-pattern scanning. Case-sensitive. Multi-word patterns work without quotes.

```sh
:filter ERROR
:filter connection refused
:filter 500 Internal Server Error
:filter database connection pool exhausted
```

## Regex Filters

Opt in with `--regex` / `-r`. Supports full regex syntax. Words after `-r` are joined, so spaces in the pattern do not need quoting.

```sh
:filter -r (ERROR|WARN)                      # errors and warnings together
:filter -r (timeout|connection refused)      # any connectivity failure
:filter -r authentication failed.*user       # auth failures with user context
:filter -r response time: [5-9]\d{3}ms      # slow responses over 5 seconds
```

> **Flag ordering:** Options (`-r`, `--fg`, `--bg`, `-l`, `--field`) must appear **before** the pattern. Everything after the first pattern word is part of the pattern.
>
> ```sh
> :filter --fg red -r timeout.*retry   # correct
> :filter timeout.*retry --fg red      # wrong — "--fg" becomes part of the pattern
> ```

## Multiple Filters

You can add as many filters as you like. They combine as follows:

1. **Include filters** — a line must match at least one enabled include filter to be shown (if any exist).
2. **Exclude filters** — a line matching any enabled exclude filter is hidden.

Exclude takes priority: a line that satisfies an include filter but also matches an exclude filter is hidden.

## Toggling Filters

- In the filter manager (`f`), press `Space` to enable/disable individual filters.
- Press `F` in normal mode to toggle **all** filtering on/off instantly (useful for comparing filtered vs. unfiltered view).
- Press `A` in the filter manager to enable/disable all filters at once.

## Filter Groups

Assign a filter to a named group with `--group <name>` when adding it:

```sh
:filter --group errors ERROR
:filter --group errors FATAL
:exclude --group noise health.?check --regex
```

Toggle every filter in a group on/off together:

```sh
:toggle-group errors
```

If any filter in the group is enabled, this disables the whole group; otherwise it enables the whole group. Group names autocomplete from existing filters.

Grouped filters show their group name in brackets in the filter sidebar, e.g. `[x] In: [errors] ERROR (12)`.

## Highlight Colors

Each include filter highlights its matching byte spans in the log line. The color is configurable per filter. When no color is set, logana uses a default highlight style from the active theme.

To set a color for the currently selected filter in the filter manager, press `c`, then use `:set-color`:

```sh
:set-color --fg yellow
:set-color --fg "#FF5555" --bg "#44475A"
```

By default, only the matched portion of the line is colored. To highlight the entire line instead, pass `-l` when adding the filter:

```sh
:filter -l ERROR            # highlight the full line for every ERROR match
:filter --fg red -l ERROR   # full-line red highlight
```

The `-l` flag can also be applied later with `:set-color -l` from the filter manager.

When multiple filters overlap on the same span, their `fg` and `bg` are composed: one filter can contribute the foreground color while another contributes the background. Automatic value colors (HTTP methods, status codes, IPs, UUIDs) apply only to spans not already colored by a filter, and log-level colors are the lowest-priority fallback.

## Editing Filters

Editing a filter's pattern or color from the filter manager (`e` to edit pattern, `c` to change color) updates it in-place. The filter keeps its current position in the list — order is never changed by an edit.
