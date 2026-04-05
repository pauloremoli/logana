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
- `Ctrl+s` — save the comment
- `Esc` — cancel without saving

After saving, annotated lines show a `◆` marker in the gutter.

## Editing and Deleting Comments

In normal mode, move to an annotated line and:

| Key | Action |
|---|---|
| `r` | Open the comment editor pre-filled with the existing text |
| `d` | Delete the comment on the current line |

Inside the editor, `Ctrl+D` also deletes the comment.

In normal mode, `c` opens the comment editor for the current line directly (without entering visual mode first).

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

### Export Window

When the selected template's `footer` section contains any `{{placeholder}}` variables, `:export` opens a window before writing the file so you can fill in those sections interactively. Both bundled templates (`markdown` and `jira`) include `{{conclusion}}` and `{{next_steps}}` by default, and any custom placeholder name works the same way.

| Key | Action |
|---|---|
| `Tab` / `Shift+Tab` | Switch between Conclusion and Next Steps |
| `Enter` | Insert a new line |
| `Backspace` | Delete character before cursor / merge lines |
| `Delete` | Delete character at cursor / merge next line |
| `Left` / `Right` | Move cursor (wraps between lines) |
| `Up` / `Down` | Move between rows |
| `Ctrl+S` | Write the file |
| `Esc` | Cancel without writing |

The active field scrolls to keep the cursor visible when content exceeds the window height.

## Export Templates

Two templates are bundled: `markdown` and `jira`. Custom templates can be placed in `~/.config/logana/templates/`.

**Template syntax:**

```
{{#header}}
# Analysis: {{filename}}
Date: {{date}}
{{/header}}

{{#comment_group}}
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
| `{{lines}}` | The annotated log lines, each prefixed with its 1-based line number |
| `{{commentary}}` | The comment text |
| `{{conclusion}}` | Conclusion text (footer) |
| `{{next_steps}}` | Next steps text (footer) |

Any `{{custom_name}}` placeholder you add to the `footer` section becomes an editable field in the export window. Use underscores for multi-word names (`{{root_cause}}` → "Root Cause").

Template sections: `header` (rendered once), `comment_group` (rendered per annotation/mark group), `footer` (optional, rendered once at the end).

User templates in `~/.config/logana/templates/` shadow bundled ones by name. Tab completion lists all available templates.
