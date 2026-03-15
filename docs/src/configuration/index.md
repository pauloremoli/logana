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
  "show_sidebar": true,
  "show_line_numbers": true,
  "wrap": false,
  "preview_bytes": 16777216,
  "restore_session": "ask",
  "restore_file_context": "ask",
  "dlt_devices": [
    { "name": "my-ecu", "host": "192.168.1.100", "port": 3490 }
  ],
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
| `theme` | string | `"github-dark"` | Active color theme name (without `.json` extension) |
| `show_mode_bar` | bool | `true` | Show the bottom status/mode bar on startup |
| `show_borders` | bool | `true` | Show panel borders on startup |
| `show_sidebar` | bool | `true` | Show the filter sidebar on startup |
| `show_line_numbers` | bool | `true` | Show the line number gutter |
| `wrap` | bool | `false` | Wrap long lines |
| `preview_bytes` | number | `16777216` | Bytes read for the instant preview shown while the full file index is built in the background (16 MiB) |
| `restore_session` | string | `"ask"` | Whether to reopen tabs from the previous session (`"ask"`, `"always"`, `"never"`) |
| `restore_file_context` | string | `"ask"` | Whether to restore per-file state (scroll, marks, search) when reopening a file (`"ask"`, `"always"`, `"never"`) |
| `dlt_devices` | array | `[]` | Pre-configured DLT daemon connections; each entry has `name`, `host`, and optional `port` (default `3490`) |

UI toggles (`show_mode_bar`, `show_borders`, `show_sidebar`, `show_line_numbers`, `wrap`) can also be changed at runtime via the UI options menu (`u`). The runtime state is stored in the database and is not written back to the config file.

## Sections

- [Keybindings](keybindings.md) — remapping all keyboard shortcuts
- [Themes](themes.md) — built-in themes and creating custom themes
