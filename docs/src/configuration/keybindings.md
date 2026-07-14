# Keybindings

All keybindings are configurable via `~/.config/logana/config.json`. Only the keys you want to change need to be specified — all others retain their defaults.

## Key Syntax

Each binding is a string (or array of strings for multiple alternatives):

| Syntax | Example | Description |
|---|---|---|
| Single character | `"j"` | A printable key |
| Modified | `"Ctrl+d"`, `"Shift+Tab"` | Modifier + key |
| Special keys | `"Enter"`, `"Esc"`, `"Space"`, `"Backspace"` | Named keys |
| Function keys | `"F1"`, `"F12"` | Function row keys |
| Navigation keys | `"Up"`, `"Down"`, `"Left"`, `"Right"`, `"PageUp"`, `"PageDown"`, `"Home"`, `"End"` | Arrow/navigation keys |

Multiple alternatives:
```json
"scroll_down": ["j", "Down"]
```

## Navigation (shared across all modes)

```json
"navigation": {
  "scroll_down": ["j", "Down"],
  "scroll_up": ["k", "Up"],
  "half_page_down": "Ctrl+d",
  "half_page_up": "Ctrl+u",
  "page_down": "PageDown",
  "page_up": "PageUp"
}
```

## Normal Mode

```json
"normal": {
  "filter_include": "i",
  "filter_exclude": "o",
  "open_filter_manager": "f",
  "toggle_filters": "F",
  "toggle_highlight_mode": "H",
  "search_forward": "/",
  "search_backward": "?",
  "next_match": "n",
  "prev_match": "N",
  "mark_line": "m",
  "toggle_marks_view": "M",
  "enter_visual_mode": "V",
  "visual_char": "v",
  "yank_marked": "Y",
  "open_ui_options": "u",
  "show_keybindings": "F1",
  "open_command_mode": ":",
  "scroll_left": ["h", "Left"],
  "scroll_right": ["l", "Right"],
  "start_of_line": "0",
  "end_of_line": "$",
  "goto_first_line": "g",
  "goto_last_line": "G",
  "toggle_status_bar": "b",
  "toggle_borders": "B",
  "edit_comment": "r",
  "delete_comment": "d",
  "comment_line": "c",
  "next_error": "e",
  "prev_error": "E",
  "next_warning": "w",
  "prev_warning": "W",
  "clear_all": "C"
}
```

## Global (always active)

```json
"global": {
  "quit": "q",
  "next_tab": "Tab",
  "prev_tab": "Shift+Tab",
  "new_tab": "Ctrl+t",
  "close_tab": "Ctrl+w"
}
```

## Filter Manager

```json
"filter": {
  "toggle": "Space",
  "edit": "e",
  "delete": "d",
  "set_color": "c",
  "add_date_filter": "t",
  "add_highlight_filter": "h",
  "search": "/",
  "move_down": "J",
  "move_up": "K",
  "toggle_all": "A",
  "clear_all": "C"
}
```

The filter manager also reuses the shared `navigation` group above (`scroll_down`/`scroll_up`/`half_page_down`/`half_page_up`/`page_down`/`page_up`) and, for jump-to-top/bottom, the `normal.go_to_top_chord`/`normal.go_to_bottom` bindings — no separate fields needed for those.

## Search Confirm/Cancel

```json
"search": {
  "confirm": "Enter",
  "cancel": "Esc"
}
```

Shared by both the log panel's `/`/`?` search and the filter manager's `/` search — confirms or cancels whichever one is currently active.

## Visual Line Mode

```json
"visual_line": {
  "comment": "c",
  "mark": "m",
  "yank": "y",
  "filter_include": "i",
  "filter_exclude": "o",
  "search": "/"
}
```

## Visual Char Mode

```json
"visual": {
  "move_left": ["h", "Left"],
  "move_right": ["l", "Right"],
  "word_forward": "w",
  "word_backward": "b",
  "word_end": "e",
  "word_forward_big": "W",
  "word_backward_big": "B",
  "word_end_big": "E",
  "start_of_line": "0",
  "first_nonblank": "^",
  "end_of_line": "$",
  "find_forward": "f",
  "find_backward": "F",
  "till_forward": "t",
  "till_backward": "T",
  "repeat_motion": ";",
  "repeat_motion_rev": ",",
  "start_selection": "v",
  "filter_include": "i",
  "filter_exclude": "o",
  "search": "/",
  "yank": "y",
  "exit": "Esc"
}
```

## Comment (Annotation) Mode

```json
"comment": {
  "newline": "Enter",
  "save": "Ctrl+s",
  "cancel": "Esc",
  "delete": "Ctrl+d"
}
```

## Confirm Dialogs

```json
"confirm": {
  "yes": "y",
  "no": "n"
}
```

## UI Options Mode

```json
"ui": {
  "toggle_sidebar": "s",
  "toggle_status_bar": "b",
  "toggle_borders": "B"
}
```

## Select Fields Mode

```json
"select_fields": {
  "toggle": "Space",
  "move_down": "J",
  "move_up": "K",
  "all": "a",
  "none": "n",
  "reset": "r",
  "apply": "Enter",
  "cancel": "Esc",
  "search": "/"
}
```

`reset` restores the popup's staged fields to the format's default order with everything visible — clearing both any `J`/`K` reorder and any hidden fields in one step. Like `all`/`none`, it only changes what's staged; `apply` still commits it.

## Archive Picker Mode

```json
"archive_picker": {
  "toggle": "Space",
  "merge_toggle": "m",
  "expand": "Right",
  "collapse": "Left",
  "all": "a",
  "none": "n",
  "apply": "Enter",
  "cancel": "Esc",
  "search": "/"
}
```

`toggle` marks a file (or a container's whole subtree) for extraction — each
toggled file opens as its own tab on `apply`. `merge_toggle` marks a file
independently for merging instead: every merge-marked file is extracted and
combined into one timestamp-sorted tab on `apply`, rather than opening
separately. A file can be `toggle`d, `merge_toggle`d, both, or neither, and
`apply` performs both actions together in one press. If a merge-marked
file's format can't be recognized, only the merge is skipped (with an error
naming the file) — toggled files still extract and open normally.

Archive listing only auto-decompresses one nested level; a nested archive
found any deeper shows as a collapsed row instead of being read upfront.
`expand` reads and reveals it on demand (or, on an already-fetched row
that's merely folded shut, just reveals its children again with no
re-fetch); `collapse` folds an expanded container's children back out of
view without discarding the already-fetched data.

`search` opens a live regex query that narrows the file tree to matching
files (keeping their containing archive visible for context) — `Enter`
confirms and un-narrows the list, `Esc` cancels back to the pre-search
selection. An invalid/incomplete regex (e.g. while still typing) falls back
to a plain substring match rather than matching nothing.

Row navigation reuses the same keys as the filter sidebar and log panel
(from the `navigation` group) rather than duplicating them here: `j`/`k`
(optionally count-prefixed, e.g. `12j`), `Ctrl+d`/`Ctrl+u` for a half page,
`PageDown`/`PageUp` for a full page, and `gg`/`G` (optionally
count-prefixed, e.g. `25G`) to jump to the first/last or a specific row.

## Docker Select Mode

```json
"docker_select": {
  "confirm": "Enter"
}
```

## Keybindings Help

```json
"help": {
  "close": ["Esc", "q", "F1"]
}
```

## Conflict Validation

At startup, logana validates all configured keybindings for conflicts within each mode scope. Conflicts are printed to stderr with a description of the overlapping bindings, but do not prevent startup.
