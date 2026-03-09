# Annotations & Export

Annotations let you attach multiline comments to log lines and export an analysis report. This is useful for incident investigations, code reviews, and sharing findings with your team.

## Visual Selection

Use [Visual Line Mode](visual-line-mode.md) (`V`) to select whole lines, or [Visual Character Mode](visual-char-mode.md) (`v`) to select text within a line. From either mode you can attach a comment, mark lines, copy to clipboard, or build a filter.

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
