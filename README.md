# logana

<p align="center">
  <a href="https://github.com/pauloremoli/logana/actions?query=workflow%3ARust"><img src="https://img.shields.io/github/actions/workflow/status/pauloremoli/logana/rust.yml?style=flat-square" /></a>
  <a href="https://codecov.io/gh/pauloremoli/logana"><img src="https://codecov.io/gh/pauloremoli/logana/branch/main/graph/badge.svg?style=flat-square" /></a>
  <a href="https://crates.io/crates/logana"><img src="https://img.shields.io/crates/v/logana.svg?style=flat-square" /></a>
  <a href="https://crates.io/crates/logana"><img src="https://img.shields.io/crates/d/logana.svg?style=flat-square" /></a>
  <a href="https://github.com/pauloremoli/logana/blob/main/LICENSE"><img src="https://img.shields.io/crates/l/logana.svg?style=flat-square" /></a>
</p>

A fast terminal log viewer for files of any size — including multi-GB logs. Built on memory-mapped I/O and SIMD line indexing. Auto-detects log formats, filters by pattern, regex, field value, or date range — bookmark lines, add annotations, and export your analysis.

<p align="center">
  <img src="docs/src/demo.gif" alt="logana demo" />
</p>

---

## Features

- **Auto-detected log formats** — JSON, syslog, journalctl, logfmt, OpenTelemetry, DLT (AUTOSAR), and more
- **Filtering** — include/exclude patterns (literal or regex), date-range filters, field-scoped filters; add filters from the command line with `-i`/`-o`/`-t`
- **Persistent sessions** — filters, scroll position, marks, and annotations survive across runs; configurable restore policy (ask / always / never)
- **Structured field view** — parsed timestamps, levels, targets, and extra fields displayed in columns; select which columns you want visible
- **Vim-style navigation** — `j`/`k`, `gg`/`G`, `Ctrl+d`/`u`, count prefixes (`5j`, `10G`), `/` search, `e`/`w` error/warning jumps
- **Annotations** — attach comments to log lines; export analysis to Markdown or Jira
- **Value coloring** — HTTP methods, status codes, IP addresses, and UUIDs colored automatically
- **Live OTLP receiver** — receive OpenTelemetry logs in real time over gRPC or HTTP/JSON; compatible with any OTel SDK
- **Multi-tab** — open multiple files, Docker streams, DLT daemon connections, or OTLP receivers; each tab has independent filters and session state
- **MCP server** — embedded Model Context Protocol server; expose marks and annotations to AI assistants
- **Headless mode** — run the full filter pipeline without a TUI to preprocess huge logs
- **Fully configurable** — all keybindings remappable via `~/.config/logana/config.json`; 22 bundled themes

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

### Cargo

```sh
cargo install logana
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

# Start at the end and follow new lines
logana app.log --tail

# Stream a Docker container
logana            # then type :docker

# Receive OpenTelemetry logs over gRPC (port 4317)
logana            # then type :otlp

# Add inline filters on the command line
logana app.log -i error -o debug
logana app.log -i "--field level=ERROR" -t "> 2024-02-21"

# Headless — filter without the TUI, output to stdout or a file
logana app.log --headless -i error -o debug
logana app.log --headless -i error --output filtered.log
```

---

## Documentation

Full documentation is available in the [`docs/`](docs/) directory:

- [Keybindings](docs/src/configuration/keybindings.md)
- [Configuration](docs/src/configuration/index.md)
- [Filtering](docs/src/filtering/index.md)
- [Annotations](docs/src/annotations.md)
- [MCP Server](docs/src/mcp.md)
- [OTLP Streaming](docs/src/otlp.md)
- [Log Formats](docs/src/log-formats.md)
- [Architecture](ARCHITECTURE.md)
