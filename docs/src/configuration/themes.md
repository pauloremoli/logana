# Themes

logana ships with 19 bundled themes and supports fully custom themes via JSON files.

## Switching Themes

```sh
:set-theme catppuccin-mocha
```

Tab completes theme names. To set the default, add it to `~/.config/logana/config.json`:

```json
{ "theme": "catppuccin-mocha" }
```

## Bundled Themes

### Dark

| Name | Description |
|---|---|
| `atomic` | Vibrant, high-saturation |
| `catppuccin-macchiato` | Pastel purple, slightly lighter than mocha |
| `catppuccin-mocha` | Pastel purple, the most popular Catppuccin variant |
| `dracula` | Purple, default theme |
| `everforest-dark` | Earthy green, easy on the eyes |
| `gruvbox-dark` | Warm retro browns and yellows |
| `jandedobbeleer` | Colorful, high contrast |
| `kanagawa` | Japanese ink — deep blues and warm golds |
| `monokai` | Classic dark with vivid accents |
| `nord` | Cool blue-grey Arctic palette |
| `onedark` | Atom-inspired, muted cool colors |
| `paradox` | High contrast |
| `rose-pine` | Muted roses and purples |
| `solarized` | Classic muted palette |
| `tokyonight` | Deep blue, inspired by Tokyo at night |

### Light

| Name | Description |
|---|---|
| `catppuccin-latte` | Pastel, warm cream background |
| `everforest-light` | Earthy green, warm paper background |
| `onelight` | Atom-inspired, clean white background |
| `rose-pine-dawn` | Warm rose tones on a parchment background |

## Custom Themes

Place `.json` files in `~/.config/logana/themes/`. A user theme with the same name as a bundled one takes priority.

### Minimal example

Only five fields are required — everything else falls back to built-in defaults:

```json
{
  "root_bg":    "#1e1e2e",
  "border":     "#6272a4",
  "border_title": "#f8f8f2",
  "text":       "#f8f8f2",
  "error_fg":   "#ff5555",
  "warning_fg": "#f1fa8c",
  "process_colors": ["#ff5555", "#50fa7b", "#ffb86c", "#bd93f9", "#ff79c6", "#8be9fd"]
}
```

### Full example (Dracula)

```json
{
  "root_bg":          "#282a36",
  "border":           "#6272a4",
  "cursor_bg":        "#6272a4",
  "border_title":     "#f8f8f2",
  "text":             "#f8f8f2",
  "text_highlight_fg": "#ffb86c",
  "text_highlight_bg": "#7a4a10",
  "cursor_fg":        "#1c1c1c",
  "trace_fg":         "#6272a4",
  "debug_fg":         "#8be9fd",
  "notice_fg":        "#f8f8f2",
  "warning_fg":       "#f1fa8c",
  "error_fg":         "#ff5555",
  "fatal_fg":         "#ff5555",
  "search_fg":        "#1c1c1c",
  "visual_select_bg": "#44475a",
  "visual_select_fg": "#f8f8f2",
  "mark_bg":          "#463c0f",
  "mark_fg":          "#f8f8f2",
  "process_colors":   ["#ff5555", "#50fa7b", "#ffb86c", "#bd93f9", "#ff79c6", "#8be9fd"],
  "value_colors": {
    "http_get":    "#50fa7b",
    "http_post":   "#8be9fd",
    "http_put":    "#ffb86c",
    "http_delete": "#ff5555",
    "http_patch":  "#bd93f9",
    "http_other":  "#6272a4",
    "status_2xx":  "#50fa7b",
    "status_3xx":  "#8be9fd",
    "status_4xx":  "#ffb86c",
    "status_5xx":  "#ff5555",
    "ip_address":  "#bd93f9",
    "uuid":        "#6c71c4"
  }
}
```

## Color Formats

All color values accept:
- Hex string: `"#RRGGBB"`
- RGB array: `[r, g, b]` (each 0–255)

## Fields Reference

### Required

| Field | Used for |
|---|---|
| `root_bg` | Main background |
| `border` | Panel border lines and dimmed decorator text |
| `border_title` | Panel title text |
| `text` | Default log line text |
| `error_fg` | ERROR level lines |
| `warning_fg` | WARN/WARNING level lines |
| `process_colors` | Array of colors cycled across process/logger name columns (can be toggled via `:value-colors`) |

### Optional (with defaults)

| Field | Default | Used for |
|---|---|---|
| `cursor_bg` | = `border` | Background of the cursor line, command bar, and search bar |
| `text_highlight_fg` | `#ffb86c` | Search match background; also the cursor for the current match |
| `text_highlight_bg` | `#7a4a10` | Background behind search highlight |
| `cursor_fg` | `#1c1c1c` | Text color on the cursor line (sits on `cursor_bg`) |
| `trace_fg` | `#6272a4` | TRACE level lines |
| `debug_fg` | `#8be9fd` | DEBUG level lines |
| `info_fg` | = `text` | INFO level lines (disabled by default; enable via `:level-colors`) |
| `notice_fg` | `#f8f8f2` | NOTICE level lines |
| `fatal_fg` | `#ff5555` | FATAL/CRITICAL level lines |
| `search_fg` | `#1c1c1c` | Foreground of search match highlights |
| `visual_select_bg` | `#44475a` | Visual line selection background |
| `visual_select_fg` | `#f8f8f2` | Visual line selection foreground |
| `mark_bg` | `#463c0f` | Marked line background |
| `mark_fg` | `#f8f8f2` | Marked line foreground |
| `value_colors` | see below | Per-token HTTP/IP/UUID colors |

### value_colors sub-object

All fields are optional and fall back to Dracula-palette defaults.

| Field | Default | Token type |
|---|---|---|
| `http_get` | `#50fa7b` | GET |
| `http_post` | `#8be9fd` | POST |
| `http_put` | `#ffb86c` | PUT |
| `http_delete` | `#ff5555` | DELETE |
| `http_patch` | `#bd93f9` | PATCH |
| `http_other` | `#6272a4` | HEAD, OPTIONS, and others |
| `status_2xx` | `#50fa7b` | 2xx success codes |
| `status_3xx` | `#8be9fd` | 3xx redirect codes |
| `status_4xx` | `#ffb86c` | 4xx client error codes |
| `status_5xx` | `#ff5555` | 5xx server error codes |
| `ip_address` | `#bd93f9` | IPv4 and IPv6 addresses |
| `uuid` | `#6c71c4` | UUID strings |

### Toggling token and level colors at runtime

Use `:value-colors` to open an interactive dialog where you can enable or disable individual token types — including **Process / logger colors** — without editing the theme file.

Use `:level-colors` to open a similar dialog for log levels. Each level (TRACE, DEBUG, NOTICE, WARNING, ERROR, FATAL) can be toggled independently. The choices are saved per-file across sessions.

## Tips for Light Themes

Set `cursor_bg` to a color that is noticeably darker than `root_bg` so the cursor line and command bar are clearly visible. Keep `border` as a subtle separator — it can be close to `root_bg` if you prefer minimal panel borders.

Set `cursor_fg` and `search_fg` to a dark color — they appear as text on the `cursor_bg` background and must contrast against it.

```json
{
  "root_bg":   "#fafafa",
  "border":    "#d0d0d0",
  "cursor_bg": "#aaaaaa",
  "cursor_fg": "#383a42",
  "search_fg": "#383a42"
}
```
