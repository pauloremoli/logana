# Multi-Tab

logana supports multiple tabs, each showing an independent log file, directory, stdin stream, or Docker container.

## Tab Keybindings

| Key | Action |
|---|---|
| `Tab` | Switch to next tab |
| `Shift+Tab` | Switch to previous tab |
| `Ctrl+t` | Open a new (empty) tab |
| `Ctrl+w` | Close the current tab |
| `Ctrl+p` | Open a searchable popup to switch between open files |

A tab can also be clicked directly in the tab bar to switch to it.

## Opening Files in Tabs

**From the command line**, a file argument opens in its own tab; a directory argument shows a picker to choose which files to open (not yet supported for multiple positional args):

```sh
logana /var/log/         # shows a picker — pick which files to open, each in its own tab
```

**From within logana**, use the `:open` command:

```sh
:open app.log            # opens in the current tab
:open /var/log/          # shows the same picker (directory)
```

## Tab State

Each tab maintains completely independent state:

- Scroll position and viewport
- Active filters (with their colors and enabled/disabled states)
- Search query
- Marks and annotations
- Detected log format
- Field layout (visible columns and order)
- Display flags (wrap, sidebar, tail mode, show-keys)

## Session Restore

When you close logana and reopen it without arguments, it prompts to restore the previous session — reopening all tabs that were open at exit, with their per-tab state restored. Docker tabs are re-attached by container name.

## Merged View

`:merge` opens a source-selection popup where you can choose any combination of open tabs. Confirming creates a new `merged(N)` tab that interleaves all selected sources sorted by timestamp — no data is copied.

```sh
:merge    # open source-selection popup
```

In the merged tab each line is prefixed with the title of the tab it came from.

The merged tab stays live: as the source tabs receive new lines, the merged index is extended and re-sorted automatically. You can pause or stop updates with the usual commands:

```sh
:pause   # pause live updates for the merged tab
:resume  # resume live updates
:stop    # stop live updates permanently for the merged tab
```

Filters, search, marks, and annotations all work the same as on any other tab.

Files inside a `.zip`/`.tar.gz`/etc archive can be merged directly without opening them as separate tabs first — mark them with `m` in the archive picker instead of `Space`. See [Opening Compressed and Archive Files](quick-start.md#opening-compressed-and-archive-files). Unlike this live `merged(N)` tab, an archive-picker merge is a one-shot snapshot of the extracted files (there's no live source tab to poll for new lines).

## Tail Mode Per Tab

Each tab can independently have tail mode enabled or disabled:

```sh
:tail    # toggle tail mode for the current tab
```

When tail is active for a tab, `[TAIL]` appears in that tab's log panel title.
