# Configuration

logana is configured via a `config.json` file. The file is entirely optional — all settings have sensible defaults and logana starts normally even if the file is missing. If the file exists but cannot be read or contains invalid JSON or unknown keys, a warning is shown in the notification area on startup.

## How the Config File Works

logana never writes to the config file. Any settings defined there are applied on startup and take precedence over the values stored in the database.

Many settings can also be changed at runtime — UI toggles via the UI options menu (`u`) and display commands (`:wrap`, `:line-numbers`, `:set-theme`, `:sidebar-position`). When changed at runtime the new value is saved to the database and restored on the next session, unless the setting is also defined in the config file, in which case the config file value always wins.

## Schema Validation

logana publishes a JSON Schema for `config.json`. Add a `$schema` line to enable validation and autocomplete in VS Code and other JSON-aware editors:

```json
{
  "$schema": "https://raw.githubusercontent.com/pauloremoli/logana/main/schema/config.schema.json",
  "theme": "dracula"
}
```

VS Code will highlight unknown fields, suggest valid values for enums such as `restore_session`, and show inline documentation for each option.

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
  "sidebar_side": "right",
  "preview_bytes": 16777216,
  "restore_session": "always",
  "restore_file_context": "always",
  "mcp_port": 9876,
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
      "filter_include": "i",
      "filter_exclude": "o",
      "filter_mode": "f",
      "toggle_filtering": "F",
      "mark_line": "m",
      "toggle_marks_only": "M",
      "visual_mode": "V",
      "enter_ui_mode": "u",
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
| `sidebar_side` | string | `"right"` | Pin the filter sidebar to `"left"` or `"right"` of the log panel |
| `preview_bytes` | number | `16777216` | Bytes read for the instant preview shown while the full file index is built in the background (16 MiB) |
| `restore_session` | string | `"always"` | Whether to reopen tabs from the previous session (`"always"`, `"ask"`, `"never"`) |
| `restore_file_context` | string | `"always"` | Whether to restore per-file state (scroll, marks, search) when reopening a file (`"always"`, `"ask"`, `"never"`) |
| `mcp_port` | number | `9876` | Default port for the embedded MCP server when started via `:enable-mcp` |
| `dlt_devices` | array | `[]` | Pre-configured DLT daemon connections; each entry has `name`, `host`, and optional `port` (default `3490`) |

## Sections

- [Keybindings](keybindings.md) — remapping all keyboard shortcuts
- [Themes](themes.md) — built-in themes and creating custom themes
