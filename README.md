# logana

A TUI log analyzer/viewer built for speed - handles files with millions of lines with instant filtering and VIM like navigation.

Open a file, pipe stdin, or stream Docker containers — logana auto-detects the format and lets you filter, search, annotate, and export without ever leaving the terminal.

---

## Features

- **Auto-detected log formats** — JSON, syslog, journalctl, logfmt, OTel, and more
- **Real-time filtering** — include/exclude patterns (literal or regex), date-range filters, field-scoped filters; add filters from the command line with `-i`/`-o`/`-t`
- **Persistent sessions** — filters, scroll position, marks, and annotations survive across runs; configurable restore policy (ask / always / never)
- **Structured field view** — parsed timestamps, levels, targets, and extra fields displayed in columns; show/hide/reorder per session
- **Vim-style navigation** — `j`/`k`, `gg`/`G`, `Ctrl+d`/`u`, count prefixes (`5j`, `10G`), `/` search, `e`/`w` error/warning jumps
- **Annotations** — attach multiline comments to log lines; export analaysis to a Markdown or Jira
- **Value coloring** — HTTP methods, status codes, IP addresses, and UUIDs colored automatically
- **Fully configurable** — all keybindings remappable via `~/.config/logana/config.json`; 19 bundled themes

---

## Performance

- **Zero-copy reads** — memory-mapped files let the OS page in only what's accessed, keeping RAM usage flat regardless of file size.
- **SIMD-accelerated scanning** — line indexing uses CPU vector instructions to find new lines.
- **Background filtering** — filter scans run across all CPU cores without blocking the UI.

For a deeper look at design decisions, see [ARCHITECTURE.md](ARCHITECTURE.md).

---

## Supported Log Formats

Detected automatically on open — no flags or config required:

| Format | Examples |
|---|---|
| JSON | tracing-subscriber JSON, bunyan, pino, any structured JSON logger |
| Syslog | RFC 3164 (BSD), RFC 5424 |
| Journalctl | short-iso, short-precise, short-full |
| Common / Combined Log | Apache access, nginx access |
| Logfmt | Go `slog`, Heroku, Grafana Loki |
| Common log family | env_logger, tracing-subscriber fmt (with/without spans), logback, log4j2, Spring Boot, Python logging, loguru, structlog |

---

## Installation

### Pre-built binaries (recommended)

Download from the [Releases page](https://github.com/pauloremoli/logana/releases), or use the install script:

**Linux / macOS**
```sh
curl -fsSL https://github.com/pauloremoli/logana/releases/latest/download/install.sh | sh
```

**Windows (PowerShell)**
```powershell
irm https://github.com/pauloremoli/logana/releases/latest/download/install.ps1 | iex
```

### Homebrew (macOS / Linux)

```sh
brew tap pauloremoli/logana
brew install logana
```

### Cargo (crates.io)

```sh
cargo install logana
```

### Cargo (from source)

```sh
cargo install --git https://github.com/pauloremoli/logana
```

### AUR (Arch Linux)

```sh
paru -S logana
# or
yay -S logana
```

---

## Quick Start

```sh
# Open a file
logana app.log

# Open a directory (each file opens in its own tab)
logana /var/log/

# Pipe from stdin
journalctl -f | logana
tail -f app.log | logana

# Stream a Docker container
logana            # then type :docker

# Add inline filters on the command line
logana app.log -i error -o debug
logana app.log -i "--field level=ERROR" -t "> 2024-02-21"
```

---

## Filtering

Filters are layered — include patterns narrow the view, exclude patterns hide matching lines. Both support literal strings (fast Aho-Corasick) and regular expressions.

| Action | Key / Command |
|---|---|
| Add include filter | `i` or `:filter <pattern>` |
| Add exclude filter | `o` or `:exclude <pattern>` |
| Open filter manager | `f` |
| Toggle filtering on/off | `F` |
| Add date range filter | `t` in filter manager, or `:date-filter <expr>` |
| Add field-scoped filter | `:filter --field <key>=<value>` |
| Add field-scoped exclude | `:exclude --field <key>=<value>` |

**Date filter syntax:**
```
:date-filter 01:00:00 .. 02:00:00
:date-filter > 2024-02-21T10:00:00
:date-filter Feb 21 .. Feb 22
```

**Field filter syntax:**
```
:filter --field level=error
:filter --field component=auth
:exclude --field level=debug
```

Field filters match against parsed structured fields rather than raw line text. Aliases: `level`/`lvl`, `timestamp`/`ts`/`time`, `target`, `message`/`msg`. Any other key is looked up in the line's extra fields. Lines that are unparseable or missing the named field pass through unchanged.

Filters are persisted to SQLite and restored the next time you open the same file.

**CLI flags:** Add filters before the TUI opens using `-i` (include), `-o` (exclude), and `-t` (timestamp). Each flag accepts the same argument string as the corresponding TUI command and can be repeated:

```sh
logana app.log -i error -o debug
logana app.log -i "--field level=ERROR"
logana app.log -i "--bg Red error" -t "> 2024-02-21"
```

---

## Search

| Action | Key |
|---|---|
| Search forward | `/` |
| Search backward | `?` |
| Next match | `n` |
| Previous match | `N` |

Search operates on visible lines only (respects active filters). Matches are highlighted inline.

---

## Structured Field View

When a structured format is detected (JSON, logfmt, syslog, etc.), logana parses each line into columns: timestamp, level, target, and extra fields. Use `:select-fields` to show, hide, and reorder columns interactively.

```sh
:fields timestamp level message     # show specific fields
:hide-field span                    # hide a field
:show-all-fields                    # reset to defaults
:select-fields                      # interactive picker
```

---

## Annotations

Select lines with `V` (visual mode), then press `c` to attach a multiline comment. You can also press `c` in normal mode to comment the current line directly. Annotated lines show a `◆` marker in the gutter. Press `r` on an annotated line to edit its comment. Export everything to a report:

```sh
:export report.md                   # Markdown (default)
:export report.md -t jira           # Jira wiki markup
:export report.md -t <template>     # custom template
```

---

## Docker

Stream any running container without leaving the terminal:

```
:docker
```

A picker lists running containers (`j`/`k` to navigate, `Enter` to attach). The stream opens in a new tab. Docker tabs are persisted across sessions — logana re-attaches automatically on next launch.

---

## Key Reference

### Navigation

| Key | Action |
|---|---|
| `j` / `k` | Scroll down / up |
| `gg` / `G` | First / last line |
| `Ctrl+d` / `Ctrl+u` | Half page down / up |
| `PageDown` / `PageUp` | Full page down / up |
| `h` / `l` | Scroll left / right |
| `5j`, `10G` | Count prefix — repeat motion N times |
| `e` / `E` | Next / previous ERROR or FATAL line |
| `w` / `W` | Next / previous WARN line |

### Normal Mode

| Key | Action |
|---|---|
| `i` | Add include filter |
| `o` | Add exclude filter |
| `f` | Open filter manager |
| `F` | Toggle filtering on/off |
| `/` / `?` | Search forward / backward |
| `n` / `N` | Next / previous match |
| `e` / `E` | Next / previous ERROR or FATAL line |
| `w` / `W` | Next / previous WARN line |
| `m` | Mark / unmark current line |
| `M` | Toggle marks-only view |
| `c` | Comment current line |
| `r` | Edit existing comment on current line |
| `d` | Delete comment on current line |
| `v` | Enter visual character mode |
| `V` | Enter visual line mode |
| `u` | UI options |
| `F1` | Keybindings help |
| `:` | Open command mode |
| `q` | Quit |

### Visual Line Mode

| Key | Action |
|---|---|
| `j` / `k` | Extend selection down / up |
| `c` | Attach comment to selection |
| `y` | Yank (copy) lines to clipboard |
| `Esc` | Cancel |

### Visual Character Mode

| Key | Action |
|---|---|
| `h` / `l` | Extend selection left / right |
| `y` | Yank (copy) selection to clipboard |
| `Esc` | Cancel |

### Filter Manager (`f`)

| Key | Action |
|---|---|
| `Space` | Toggle filter on/off |
| `e` | Edit pattern |
| `d` | Delete filter |
| `c` | Set highlight color |
| `t` | Add date filter |
| `J` / `K` | Move filter down / up |
| `A` | Toggle all on/off |
| `C` | Clear all filters |
| `Esc` | Exit |

### Multi-tab

| Key | Action |
|---|---|
| `Tab` / `Shift+Tab` | Next / previous tab |
| `Ctrl+t` | Open new tab |
| `Ctrl+w` | Close current tab |

### UI Toggles (`u`)

| Key | Action |
|---|---|
| `s` | Sidebar |
| `b` | Mode bar |
| `B` | Borders |

---

## Commands

Type `:` to enter command mode. Tab completes commands, flags, colors, themes, and file paths.

| Command | Description |
|---|---|
| `:filter <pattern>` | Add include filter |
| `:filter --field <key>=<value>` | Add field-scoped include filter |
| `:exclude <pattern>` | Add exclude filter |
| `:exclude --field <key>=<value>` | Add field-scoped exclude filter |
| `:date-filter <expr>` | Add date/time range filter |
| `:set-color [--fg COLOR] [--bg COLOR]` | Set highlight color for selected filter |
| `:open <path>` | Open file or directory |
| `:close-tab` | Close current tab |
| `:docker` | Pick and stream a Docker container |
| `:tail` | Toggle tail mode (auto-scroll on new content) |
| `:wrap` | Toggle line wrap |
| `:level-colors` | Toggle log-level coloring |
| `:value-colors` | Configure HTTP/IP/UUID token coloring |
| `:fields [col ...]` | Set visible columns |
| `:select-fields` | Interactive column picker |
| `:set-theme <name>` | Switch color theme |
| `:save-filters <file>` | Save current filters to JSON |
| `:load-filters <file>` | Load filters from JSON |
| `:export <file> [-t template]` | Export annotations to file |
| `:<N>` | Jump to line N |

---

## Configuration

Config file: `~/.config/logana/config.json`

```json
{
  "theme": "dracula",
  "show_mode_bar": true,
  "show_borders": true,
  "restore_session": "ask",
  "keybindings": {
    "navigation": {
      "scroll_down": ["j", "Down"],
      "scroll_up": ["k", "Up"],
      "half_page_down": "Ctrl+d",
      "half_page_up": "Ctrl+u"
    },
    "global": {
      "quit": "q"
    }
  }
}
```

**`restore_session`** controls the behaviour when reopening a file that has a saved session:
- `"ask"` — prompt on every open (default)
- `"always"` — restore silently without asking
- `"never"` — always start fresh without asking

You can also set this preference interactively: when the restore prompt appears, press `Y` (always) or `N` (never) instead of `y`/`n` to save the choice permanently.

All keybindings have sensible defaults — the config file is entirely optional. Each action supports a single key or an array of alternatives.

---

## Themes

19 themes are bundled:

**Dark:** `catppuccin-mocha`, `catppuccin-macchiato`, `dracula`, `everforest-dark`, `gruvbox-dark`, `jandedobbeleer`, `kanagawa`, `monokai`, `nord`, `onedark`, `paradox`, `rose-pine`, `solarized`, `tokyonight`, `atomic`

**Light:** `catppuccin-latte`, `everforest-light`, `onelight`, `rose-pine-dawn`

```sh
:set-theme nord
```

Place custom themes (JSON) in `~/.config/logana/themes/`. Colors accept hex (`"#RRGGBB"`) or RGB arrays (`[r, g, b]`).

---

## Data Locations

| Path | Contents |
|---|---|
| `~/.local/share/logana/logana.db` | Filters, session state, file contexts |
| `~/.config/logana/config.json` | Keybindings, theme, UI defaults, restore policy |
| `~/.config/logana/themes/` | Custom themes |
| `~/.config/logana/templates/` | Custom export templates |

---
