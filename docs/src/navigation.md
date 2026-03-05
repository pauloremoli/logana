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
| `h` | Scroll left |
| `l` | Scroll right |

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

## Line Wrap

Toggle line wrapping with `:wrap` or via the UI menu (`u` → `w`). When wrap is enabled, long lines flow onto multiple terminal rows and all viewport math accounts for the extra rows automatically.
