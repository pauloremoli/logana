# Annotations & Export

Annotations let you attach multiline comments to log lines and export an analysis report. This is useful for incident investigations, code reviews, and sharing findings with your team.

## Visual Line Mode

Press `V` in normal mode to enter visual line mode. The current line becomes the anchor.

| Key | Action |
|---|---|
| `j` / `k` | Extend selection down / up |
| `c` | Attach a comment to the selected lines |
| `m` | Mark / unmark all selected lines (toggles group) |
| `y` | Yank (copy) selected lines to system clipboard |
| `i` | Open command bar pre-filled with `filter <first line>` |
| `o` | Open command bar pre-filled with `exclude <first line>` |
| `/` | Open search bar pre-filled with the first selected line |
| `Esc` | Cancel |

Selected lines are highlighted in the log panel.

## Visual Char Mode

Press `v` in normal mode to enter character-level visual mode. The cursor is placed on the current line — at the start of the active search match if one exists, otherwise at column 0. Move the cursor freely with vim motions before anchoring a selection.

### Cursor motions

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

### Anchoring and actions

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

## Adding a Comment

With lines selected in visual mode, press `c` to open the comment editor:

- Type your multiline comment
- `Enter` — insert new line
- `Backspace` — delete character / merge lines
- `Left` / `Right` — move cursor (wraps between lines)
- `Up` / `Down` — move between rows
- `Ctrl+Enter` — save the comment
- `Esc` — cancel without saving

After saving, annotated lines show a `◆` marker in the gutter.

## Editing and Deleting Comments

In normal mode, move to an annotated line and:

| Key | Action |
|---|---|
| `e` | Open the comment editor pre-filled with the existing text |
| `d` | Delete the comment on the current line |

Inside the editor, `Ctrl+D` also deletes the comment.

Press `C` in normal mode to clear **all** marks and comments for the current tab.

## Marks

Press `m` to mark the current line. Marked lines are included in exports even without a comment attached. Press `M` to toggle a marks-only view.

## Exporting

Export all annotations and marked lines to a file:

```sh
:export report.md                    # Markdown (default)
:export report.md -t jira            # Jira wiki markup
:export report.md -t <template>      # custom template
```

The export includes:
- A header with the filename and export date
- Each comment group with the commented log lines and the comment text
- Any standalone marked lines (without a comment) grouped consecutively

## Export Templates

Two templates are bundled: `markdown` and `jira`. Custom templates can be placed in `~/.config/logana/templates/`.

**Template syntax:**

```
{{#header}}
# Analysis: {{filename}}
Date: {{date}}
{{/header}}

{{#comment_group}}
## Lines {{line_numbers}}
```
{{lines}}
```

{{commentary}}
{{/comment_group}}
```

**Available placeholders:**

| Placeholder | Content |
|---|---|
| `{{filename}}` | Source file name |
| `{{date}}` | Export date |
| `{{lines}}` | The raw log lines for this group |
| `{{line_numbers}}` | Comma-separated 1-based line numbers |
| `{{commentary}}` | The comment text |

Template sections: `header` (rendered once), `comment_group` (rendered per annotation/mark group), `footer` (optional, rendered once at the end).

User templates in `~/.config/logana/templates/` shadow bundled ones by name. Tab completion lists all available templates.
