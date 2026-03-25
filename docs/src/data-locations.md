# Data Locations

## Runtime Files

Paths depend on the operating system:

| Location | Linux | macOS | Windows |
|---|---|---|---|
| Database | `~/.local/share/logana/logana.db` | `~/Library/Application Support/logana/logana.db` | `%APPDATA%\logana\logana.db` |
| Config file | `~/.config/logana/config.json` | `~/Library/Application Support/logana/config.json` | `%APPDATA%\logana\config.json` |
| Themes dir | `~/.config/logana/themes/` | `~/Library/Application Support/logana/themes/` | `%APPDATA%\logana\themes\` |
| Templates dir | `~/.config/logana/templates/` | `~/Library/Application Support/logana/templates/` | `%APPDATA%\logana\templates\` |

## Database

The SQLite database stores:

- **Filters** — include/exclude patterns and date filters, per source file
- **File context** — per-file session state: scroll position, search query, wrap, sidebar visibility, marked lines, field layout, show-keys preference, and more
- **Session tabs** — the ordered list of files/Docker streams open when logana last exited (used for session restore)

The database is created automatically on first run. Schema migrations run on startup — no manual setup needed.

## Config File

The config file is optional. If it is absent, logana starts with all defaults. If the file exists but cannot be read, contains invalid JSON, or has unknown keys, a warning is shown in the notification area on startup. Partial configs are valid — only specified keys override defaults.

See [Configuration](configuration/index.md) for the full schema.

## Custom Themes

Place `.json` files in `~/.config/logana/themes/`. Files here shadow bundled themes of the same name. See [Themes](configuration/themes.md) for the theme JSON format.

## Custom Export Templates

Place `.txt` template files in `~/.config/logana/templates/`. Files here shadow bundled templates (`markdown`, `jira`) of the same name. See [Annotations & Export](annotations.md) for the template format.
