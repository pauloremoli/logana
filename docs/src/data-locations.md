# Data Locations

## Runtime Files

| Path | Contents |
|---|---|
| `~/.local/share/logana/logana.db` | SQLite database: filters, session state, file contexts |
| `~/.config/logana/config.json` | Keybindings, theme, UI defaults |
| `~/.config/logana/themes/` | Custom theme JSON files |
| `~/.config/logana/templates/` | Custom export template files |

## Database

The SQLite database stores:

- **Filters** — include/exclude patterns and date filters, per source file
- **File context** — per-file session state: scroll position, search query, wrap, sidebar visibility, marked lines, field layout, show-keys preference, and more
- **Session tabs** — the ordered list of files/Docker streams open when logana last exited (used for session restore)

The database is created automatically on first run. Schema migrations run on startup — no manual setup needed.

## Config File

The config file is optional. If it is absent or contains invalid JSON, logana starts with all defaults. Partial configs are valid — only specified keys override defaults.

See [Configuration](configuration/index.md) for the full schema.

## Custom Themes

Place `.json` files in `~/.config/logana/themes/`. Files here shadow bundled themes of the same name. See [Themes](configuration/themes.md) for the theme JSON format.

## Custom Export Templates

Place `.txt` template files in `~/.config/logana/templates/`. Files here shadow bundled templates (`markdown`, `jira`) of the same name. See [Annotations & Export](annotations.md) for the template format.
