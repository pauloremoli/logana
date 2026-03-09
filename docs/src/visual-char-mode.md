# Visual Character Mode

Press `v` in normal mode to enter character-level visual mode. The cursor is placed on the current line — at the start of the active search match if one exists, otherwise at column 0. Move the cursor freely with vim motions before anchoring a selection.

## Cursor motions

| Key | Action |
|---|---|
| `h` / `l` | Move left / right one character |
| `w` / `b` / `e` | Word start forward / backward / word end |
| `W` / `B` / `E` | WORD (whitespace-delimited) variants |
| `0` | Move to start of line |
| `^` | Move to first non-blank character |
| `$` | Move to end of line |
| `f<c>` | Find next occurrence of character `c` |
| `F<c>` | Find previous occurrence of character `c` |
| `t<c>` | Move to one before next `c` |
| `T<c>` | Move to one after previous `c` |
| `;` | Repeat last `f`/`F`/`t`/`T` motion |
| `,` | Repeat last motion in reverse |

## Anchoring and actions

Press `v` again to anchor the selection at the current cursor position. Any subsequent cursor motion extends the selection. Without an anchor, actions operate on the single character under the cursor.

| Key | Action |
|---|---|
| `v` | Anchor selection at cursor |
| `i` | Open command bar pre-filled with `filter <selected>` |
| `o` | Open command bar pre-filled with `exclude <selected>` |
| `/` | Open search bar pre-filled with selected text |
| `y` | Yank (copy) selection to system clipboard |
| `Esc` | Cancel |

The selected character range is highlighted with a reversed colour in the log panel. When a `f`/`F`/`t`/`T` motion is pending (waiting for the target character), the mode bar shows `pending — type a character`.
