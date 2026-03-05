# Multi-Tab

logana supports multiple tabs, each showing an independent log file, directory, stdin stream, or Docker container.

## Tab Keybindings

| Key | Action |
|---|---|
| `Tab` | Switch to next tab |
| `Shift+Tab` | Switch to previous tab |
| `Ctrl+t` | Open a new (empty) tab |
| `Ctrl+w` | Close the current tab |

## Opening Files in Tabs

**From the command line**, each file argument opens in its own tab (not yet supported for multiple positional args, but directory expansion creates one tab per file):

```sh
logana /var/log/         # each file in the directory gets its own tab
```

**From within logana**, use the `:open` command:

```sh
:open app.log            # opens in the current tab
:open /var/log/          # opens each file in a new tab (directory)
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

## Tail Mode Per Tab

Each tab can independently have tail mode enabled or disabled:

```sh
:tail    # toggle tail mode for the current tab
```

When tail is active for a tab, `[TAIL]` appears in that tab's log panel title.
