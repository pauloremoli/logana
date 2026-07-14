# Quick Start

## Opening Logs

```sh
# Open a file
logana app.log

# Open a directory — each file opens in its own tab
logana /var/log/

# Pipe from stdin
journalctl -f | logana
tail -f app.log | logana

# Stream a Docker container
logana            # then type :docker

# Preload a saved filter set — filters are applied in a single pass during indexing
logana app.log --filters my-filters.json

# Add inline filters directly on the command line
logana app.log -i error -o debug
logana app.log -i "--field level=ERROR" -t "> 2024-02-21"

# Start at the end of the file with tail mode enabled
logana app.log --tail

# Combined: preload filters and jump to the last matching line immediately
logana app.log --filters my-filters.json --tail
```

## Opening Compressed and Archive Files

```sh
logana app.log.gz
logana logs.tar.gz
logana logs.zip
```

Opening a `.gz`/`.bz2`/`.xz`/`.zip`/`.tar`/`.tar.gz`/`.tar.bz2`/`.tar.xz` file — whether from the command line or with `:open` inside the TUI — shows a popup listing everything inside it as a tree, without extracting anything yet. If an entry is itself an archive (a `.zip` inside a `.tar.gz`, for example), one nested level is expanded automatically so its contents show as nested rows too; anything nested deeper than that shows as a collapsed row you can expand yourself.

- `Space` toggles the file under the cursor. Toggling a nested archive's own row selects or deselects everything inside it at once.
- `m` marks the file under the cursor to be merged instead — independently of `Space`, and toggled the same way for a nested archive's whole subtree.
- `Right` reads and reveals a not-yet-expanded nested archive's contents (or just reveals them again, with no re-read, if they were already fetched and merely folded shut). `Left` folds an expanded archive's contents back out of view.
- `a` / `n` select or deselect every file.
- `Enter` extracts the confirmed selection: `Space`-toggled files each open as their own tab, and `m`-marked files are extracted and combined into a single timestamp-sorted tab. Both happen together in one press. If any `m`-marked file's format can't be recognized, only the merge is skipped (with an error naming the file) — toggled files still open normally.
- `Esc` cancels without extracting anything.

## First Steps

Once logana opens, you'll see the log content with the detected format shown in the title bar.

**Basic navigation:**
- `j` / `k` — scroll down / up one line
- `gg` / `G` — jump to first / last line
- `Ctrl+d` / `Ctrl+u` — half page down / up
- `q` — quit

**Add your first filter:**
- Press `i` and type a pattern to show only matching lines
- Press `o` and type a pattern to hide matching lines
- Press `f` to open the filter manager and see all active filters

**Search:**
- Press `/` and type a query to search forward
- Press `n` / `N` to jump between matches

**Commands:**
- Press `:` to open command mode
- Type a command and press `Enter` (Tab completes commands, flags, and paths)

