# logana

<p align="center">
  <a href="https://github.com/pauloremoli/logana/actions?query=workflow%3ARust"><img src="https://img.shields.io/github/actions/workflow/status/pauloremoli/logana/rust.yml?style=flat-square" /></a>
  <a href="https://codecov.io/gh/pauloremoli/logana"><img src="https://codecov.io/gh/pauloremoli/logana/branch/main/graph/badge.svg?style=flat-square" /></a>
  <a href="https://crates.io/crates/logana"><img src="https://img.shields.io/crates/v/logana.svg?style=flat-square" /></a>
  <a href="https://crates.io/crates/logana"><img src="https://img.shields.io/crates/d/logana.svg?style=flat-square" /></a>
  <a href="https://github.com/pauloremoli/logana/blob/main/LICENSE"><img src="https://img.shields.io/crates/l/logana.svg?style=flat-square" /></a>
</p>

<p align="center">
  A fast terminal log viewer. Filter logs of any size and format, mark and annotate what matters, then export your findings.
</p>

<p align="center">
  <img src="docs/src/filters-marks-comments.png" alt="logana TUI showing colored, grouped filters alongside marked lines and inline comments" />
</p>

- Any log format — JSON, syslog, logfmt, OpenTelemetry, DLT, or a custom schema
- Include/exclude/highlight filters — by text, regex, field, or date range
- Bookmark and annotate lines, then export to Markdown or Jira
- Vim-style navigation, fully remappable keybindings
- MCP server to expose your findings to AI assistants

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

These install the TUI (`logana`). For the GUI only, swap in
`logana-gui-installer.sh` / `.ps1`; for both frontends, use
`logana-full-installer.sh` / `.ps1`.

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

Source builds enable `tui` by default.
Build only the GUI with `cargo build -p logana-gui`, or both frontends with
`cargo build -p logana-full`. See
[frontend build configurations](CONTRIBUTING.md#frontend-features).

The GUI accepts the same arguments as the TUI, including filters, tail mode,
MCP, subprocess streams, archive extraction, merging, and headless output:

```sh
logana-gui app.log "logs/another file.log" bundle.zip
logana-gui --tail -i "error" app.log
logana-gui --merge a.log b.log --output merged.log
logana-gui --headless -o "debug" app.log
```

Explicit paths take precedence over restoring the previous session. Use `--`
before a path that starts with a dash, for example `logana-gui -- -app.log`.

The desktop GUI loads and indexes logs in the background. A persistent progress
panel shows the current phase and lets you cancel active or queued operations.
Opening an archive displays an entry picker, including nested archives; select
files and choose **Extract and open**. Files opened through the GUI picker
stream to temporary files; command-line archives extract permanently into
`--out` (the current directory by default). Cancellation keeps completed tabs
while discarding unfinished output. Temporary files remain available until
their tabs close.


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
