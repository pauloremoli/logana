# Visual Line Mode

Press `V` in normal mode to enter visual line mode. The current line becomes the anchor.
You can also start a selection directly with `Shift+Up` / `Shift+Down` or
`Shift+Home` / `Shift+End`. These shortcuts keep the original row as the anchor
while moving the other end of the selection through visible rows.

| Key | Action |
|---|---|
| `j` / `k` | Extend selection down / up |
| `Shift+Down` / `Shift+Up` | Start or extend selection down / up |
| `Shift+Home` / `Shift+End` | Start or extend selection to the first / last visible row |
| `c` | Attach a comment to the selected lines |
| `m` | Mark / unmark all selected lines (toggles group) |
| `y` | Yank (copy) selected lines to system clipboard |
| `/` | Open search bar pre-filled with the first selected line |
| `Esc` | Cancel |

Selected lines are highlighted in the log panel.

The extension shortcuts reuse `keybindings.gui.extend_up`, `extend_down`,
`extend_first`, and `extend_last` in both interfaces. Rebinding one changes it in
the TUI and GUI; `[]` disables it. In the TUI, a count such as `3Shift+Down`
extends by three visible rows. Selection stops at the first or last visible row,
respects filters, and can be shortened or reversed by moving back across the
anchor. `Esc` leaves visual line mode using `visual_line.exit`.
