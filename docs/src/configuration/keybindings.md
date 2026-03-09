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
  "scroll_left": "h",
  "scroll_right": "l",
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
  "move_down": "J",
  "move_up": "K",
  "toggle_all": "A",
  "clear_all": "C"
}
```

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
  "save": "Ctrl+Enter",
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
  "enable_all": "a",
  "disable_all": "n",
  "apply": "Enter"
}
```

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
