# Navigation

logana uses Vim-style keybindings for all navigation. All bindings are configurable — see [Keybindings](configuration/keybindings.md).

## Scrolling

| Key | Action |
|---|---|
| `j` / `Down` | Scroll down one line |
| `k` / `Up` | Scroll up one line |
| `Ctrl+d` | Half page down |
| `Ctrl+u` | Half page up |
| `PageDown` | Full page down |
| `PageUp` | Full page up |
| `gg` | Jump to first line |
| `G` | Jump to last line |

## Horizontal Scroll

When line wrap is off, long lines can be scrolled horizontally:

| Key | Action |
|---|---|
| `h` / `Left` | Scroll left |
| `l` / `Right` | Scroll right |
| `0` | Jump to start of line (reset horizontal scroll) |
| `$` | Jump to end of line |

## GUI navigation and shared line selection

The GUI reuses the scrolling and horizontal navigation bindings above. Its
additional defaults are `Home` / `End` for the first / last row and
`Shift+Up` / `Shift+Down` to extend row selection. `Shift+Home` / `Shift+End`
extend selection to the first / last row. These shortcuts are configurable under
`keybindings.gui` as `first_row`, `last_row`, `extend_up`, `extend_down`,
`extend_first`, and `extend_last`. The four `extend_*` bindings also work in the
TUI: they enter visual line mode from the current row and keep extending the same
selection on subsequent presses. See [Visual Line Mode](visual-line-mode.md).

Use `m` to bookmark selected rows and `M` to switch between Logs and Bookmarks.
The toggle reuses `normal.toggle_marks_only`; menus and the view selector provide
the same view choices. See [GUI shortcuts](configuration/keybindings.md#gui-shortcuts)
for the configuration reference. The count-prefix and terminal-mode instructions
below describe the TUI.

## Count Prefix

Prepend a number to most motion keys to repeat them:

```
5j      — scroll down 5 lines
10k     — scroll up 10 lines
3Ctrl+d — scroll down 3 half-pages
50G     — jump to line 50
3gg     — jump to line 3
```

The active count is shown in the status bar (e.g. `[NORMAL] 5`). Counts are capped at 999,999.

## Go to Line

From command mode, type a bare line number to jump there:

```
:500    — jump to line 500
:1      — jump to the first line
```

If the target line is hidden by an active filter, logana jumps to the nearest visible line instead.

## Marks

Mark important lines to jump back to them or include them in an export.

| Key | Action |
|---|---|
| `m` | Mark / unmark the current line |
| `M` | Toggle marks-only view (show only marked lines) |

Marked lines show a highlighted indicator in the gutter. Marks are per-session and not persisted across runs.

## Visual Selection

| Key | Action |
|---|---|
| `V` | Enter visual **line** mode — select whole lines for bulk mark / comment / yank / filter |
| `v` | Enter visual **char** mode — move a cursor within the current line and select a text range |

See [Visual Line Mode](visual-line-mode.md) and [Visual Character Mode](visual-char-mode.md) for the full key reference.

## Log Level Navigation

Jump directly between error and warning lines without scrolling:

| Key | Action |
|---|---|
| `e` | Jump to next ERROR / FATAL line |
| `E` | Jump to previous ERROR / FATAL line |
| `w` | Jump to next WARN line |
| `W` | Jump to previous WARN line |

Navigation wraps to the nearest visible line that matches the level. Positions are pre-indexed whenever the visible set changes, so each jump is O(log n) regardless of file size.

## Continuation Lines

Multiline entries (stack traces, wrapped messages, or a custom schema's structured continuation lines) can be collapsed to just their first line:

| Key | Action |
|---|---|
| `>` | Collapse the entry under the cursor |
| `<` | Expand the entry under the cursor |

`:collapse` and `:expand` apply the same fold/reveal file-wide instead of to a single entry. `>`/`<` still work afterward to flip an individual entry against whatever the file-wide default currently is.

## Line Wrap

Toggle line wrapping with `:wrap` or via the UI menu (`u` → `w`). When wrap is enabled, long lines flow onto multiple terminal rows and all viewport math accounts for the extra rows automatically.
