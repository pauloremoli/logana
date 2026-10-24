# Quick Start

## Opening Logs

```sh
# Open a file
logana app.log

# Open multiple files — each opens in its own tab
logana app.log system.log

# Open every file matching a glob pattern, each in its own tab
logana '/var/log/system.log*'

# Open a directory — every file inside it opens, each in its own tab
logana /var/log/

# Open a directory with a picker instead, to choose which files to open
logana            # then type :file-picker /var/log/

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

### GUI file and folder selection

Open files or folders from the in-app selection window. Browse folders by clicking
their names; use the checkboxes to select files and entire folders together.
Selected paths stay in the right pane as you browse. You can enter a path directly,
filter the current folder, and show hidden entries.

By default, each file opens in its own tab, including files inside selected folders.
Enable **Auto merge rotated logs** to combine numbered rotations of each log,
oldest first (for example, `runtime.log2`, `runtime.log1`, and `runtime.log` become
`runtime-merged.log`). Unrelated logs and identically named logs in different
folders open separately. Compressed rotations are supported. Folders are scanned
recursively, and overlapping folder and file selections are counted once. Confirm to start opening;
Cancel leaves the current tabs unchanged.

### GUI archive selection

The GUI opens a separate, resizable archive selection window and lists files
from every readable nested archive automatically. The split pane keeps the archive tree beside your selected files
and saved glob patterns.
Add patterns such as `*.log*` or `logs/**` to select matching names or archive
paths. Patterns are saved and reused for every archive, including after restarting.
Adding or removing a pattern reapplies the saved patterns; you can then adjust
individual files manually. Filename search only narrows the list on screen.

Enable **Auto merge rotated logs** to combine selected rotations from the same
folder and archive. For example, `runtime.log2`, `runtime.log1`, and `runtime.log`
produce `runtime-merged.log`, while `startup.log*` produces `startup-merged.log`.
The selected-files pane previews each merged output with its original files indented beneath it; remove a file there to exclude it.
Unrelated files open separately; files from different folders or nested archives stay separate.
Numbered rotations with or without a dot, including compressed rotations, are
joined from the highest number down to the current log. Only selected files are
included. The combined logs are saved under the archive's basename (for example, `bundle.zip` or `bundle.tar.gz`
uses a `bundle` folder) in the output
folder shown in the picker and opened; individual rotation
pieces remain temporary. If a file already occupies the destination folder's
path, extracted logs are placed under `.logana-extracted` instead. Existing combined
logs are preserved; a name such as `runtime-merged (2).log` is used when needed.

Use **Choose folder…** to change the output folder before opening files. It
defaults to the input archive's folder (or `--out`, if provided). Individual files
and combined logs are saved in the chosen folder. Progress appears only after
confirming the selection and once extraction is running.
If saving fails, the log still opens as a temporary file and an error notification
identifies the destination that could not be saved.

Unreadable nested archives remain visible with an error; other files can still
be selected and opened.

### TUI archive selection

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

