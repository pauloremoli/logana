# logana

<p align="center">
  <a href="https://github.com/pauloremoli/logana/actions?query=workflow%3ARust"><img src="https://img.shields.io/github/actions/workflow/status/pauloremoli/logana/rust.yml?style=flat-square" /></a>
  <a href="https://codecov.io/gh/pauloremoli/logana"><img src="https://codecov.io/gh/pauloremoli/logana/branch/main/graph/badge.svg?style=flat-square" /></a>
  <a href="https://crates.io/crates/logana"><img src="https://img.shields.io/crates/v/logana.svg?style=flat-square" /></a>
  <a href="https://crates.io/crates/logana"><img src="https://img.shields.io/crates/d/logana.svg?style=flat-square" /></a>
  <a href="https://github.com/pauloremoli/logana/blob/main/LICENSE"><img src="https://img.shields.io/crates/l/logana.svg?style=flat-square" /></a>
</p>

<p align="center">
  logana turns any log source — files, compressed archives, Docker containers, or OTel streams — into structured, filterable, annotatable data. Filter by pattern, field, or date range; jump between errors and warnings; annotate key lines; bookmark findings; and export to Markdown, Jira, or AI assistants via the built-in MCP server — all persistent across sessions.
</p>

<p align="center">
  <img src="docs/src/demo.gif" alt="logana demo" />
</p>

---

## Features

- **Any log format** — JSON, syslog, journalctl, logfmt, OpenTelemetry, DLT, or your own custom `{field}` schema
- **Any source** — files, directories, compressed/archives, Docker containers, OTel (gRPC/HTTP), stdin
- **Filtering** — include/exclude, regex, field-scoped, date-range, and highlight-only filters, all remappable and scriptable from the CLI
- **Vim-style navigation** — `j`/`k`, `gg`/`G`, count prefixes, `/` search, jump straight to the next error or warning
- **Annotations** — comment on lines and export the analysis to Markdown or Jira
- **Persistent sessions** — filters, marks, and scroll position are restored automatically
- **MCP server** — expose marks and annotations to AI assistants
- **Headless mode** — run the full filter pipeline without a TUI, for scripting and huge logs
- **Fully configurable** — every keybinding is remappable

---

## Installation

### Pre-built binaries (recommended)

Download from the [Releases page](https://github.com/pauloremoli/logana/releases), or use the install script:

**Linux / macOS**
```sh
curl -fsSL https://github.com/pauloremoli/logana/releases/latest/download/logana-installer.sh | sh
```

**Windows (PowerShell)**
```powershell
irm https://github.com/pauloremoli/logana/releases/latest/download/logana-installer.ps1 | iex
```

### Homebrew (macOS / Linux)

```sh
brew tap pauloremoli/logana && brew install logana
```

### Cargo

```sh
cargo install logana
# or install the latest binary directly
cargo binstall logana
```

---

## Performance

Filtering a [3.3 GB access log with 10M+ lines](https://www.kaggle.com/datasets/eliasdabbas/web-server-access-logs) against [lnav](https://lnav.org/), cold disk cache:

| | logana | lnav |
|---|---|---|
| Headless (10-run avg) | 0.99 s | 11.2 s |
| TUI, open → filter → quit | 1.8 s | 11.8 s |

<p align="center">
  <img src="docs/src/performance.gif" alt="logana performance comparison with lnav" />
</p>

> lnav offers features beyond filtering that may account for part of the difference — this compares filtering performance only. Hardware: AMD Ryzen 9 8945HS · 32 GB DDR5 5600 MHz · NVMe 4.0 x4.

---

## Documentation

Full documentation is at **[pauloremoli.github.io/logana](https://pauloremoli.github.io/logana/)**.

- [Quick Start](https://pauloremoli.github.io/logana/quick-start.html)
- [Commands](https://pauloremoli.github.io/logana/commands.html)
- [Filtering](https://pauloremoli.github.io/logana/filtering/)
- [Configuration](https://pauloremoli.github.io/logana/configuration/)
- [Keybindings](https://pauloremoli.github.io/logana/configuration/keybindings.html)
- [Annotations](https://pauloremoli.github.io/logana/annotations.html)
- [OTel Collector](https://pauloremoli.github.io/logana/otel.html)
- [MCP Server](https://pauloremoli.github.io/logana/mcp.html)
- [Log Formats](https://pauloremoli.github.io/logana/log-formats.html)
- [Custom Schemas](https://pauloremoli.github.io/logana/custom-schemas.html)
