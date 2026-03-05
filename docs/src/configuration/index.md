# Configuration

logana is configured via `~/.config/logana/config.json`. The file is entirely optional — all settings have sensible defaults and logana starts normally even if the file is missing or contains invalid JSON.

## Config File Location

```
~/.config/logana/config.json
```

## Full Example

```json
{
  "theme": "dracula",
  "show_mode_bar": true,
  "show_borders": true,
  "keybindings": {
    "navigation": {
      "scroll_down": ["j", "Down"],
      "scroll_up": ["k", "Up"],
      "half_page_down": "Ctrl+d",
      "half_page_up": "Ctrl+u",
      "page_down": "PageDown",
      "page_up": "PageUp"
    },
    "normal": {
      "add_include_filter": "i",
      "add_exclude_filter": "o",
      "open_filter_manager": "f",
      "toggle_filters": "F",
      "mark_line": "m",
      "toggle_marks_view": "M",
      "enter_visual_mode": "V",
      "open_ui_options": "u",
      "show_keybindings": "F1",
      "scroll_left": "h",
      "scroll_right": "l"
    },
    "global": {
      "quit": "q"
    }
  }
}
```

## Top-level Options

| Key | Type | Default | Description |
|---|---|---|---|
| `theme` | string | `"dracula"` | Active color theme name (without `.json` extension) |
| `show_mode_bar` | bool | `true` | Show the bottom status/mode bar on startup |
| `show_borders` | bool | `true` | Show panel borders on startup |

Both `show_mode_bar` and `show_borders` can be toggled at runtime via the UI options menu (`u` → `b` / `B`). The runtime state is not written back to the config file.

## Sections

- [Keybindings](keybindings.md) — remapping all keyboard shortcuts
- [Themes](themes.md) — built-in themes and creating custom themes
