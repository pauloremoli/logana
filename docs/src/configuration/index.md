# Configuration

logana is configured via a `config.json` file. The file is entirely optional — all settings have sensible defaults and logana starts normally even if the file is missing. If the file exists but cannot be read or contains invalid JSON or unknown keys, a warning is shown in the notification area on startup.

## How the Config File Works

logana never writes to the config file. Any settings defined there are applied on startup and take precedence over the values stored in the database.

Some settings (UI toggles like `show_mode_bar`, `show_borders`, `show_sidebar`, `show_line_numbers`, and `wrap`) can also be changed at runtime via the UI options menu (`u`). When changed at runtime, the new value is saved to the database and restored on the next session — unless the setting is explicitly defined in the config file, in which case the config file value always wins.

## Config File Location

The path depends on the operating system:

| OS | Path |
|---|---|
| Linux | `~/.config/logana/config.json` |
| macOS | `~/Library/Application Support/logana/config.json` |
| Windows | `%APPDATA%\logana\config.json` |

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
  "restore_session": "always",
  "restore_file_context": "always",
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
| `restore_session` | string | `"always"` | Whether to reopen tabs from the previous session (`"always"`, `"ask"`, `"never"`) |
| `restore_file_context` | string | `"always"` | Whether to restore per-file state (scroll, marks, search) when reopening a file (`"always"`, `"ask"`, `"never"`) |
| `dlt_devices` | array | `[]` | Pre-configured DLT daemon connections; each entry has `name`, `host`, and optional `port` (default `3490`) |

## Sections

- [Keybindings](keybindings.md) — remapping all keyboard shortcuts
- [Themes](themes.md) — built-in themes and creating custom themes
