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

