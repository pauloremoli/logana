# logana

<p align="center">
  <a href="https://github.com/pauloremoli/logana/actions?query=workflow%3ARust"><img src="https://img.shields.io/github/actions/workflow/status/pauloremoli/logana/rust.yml?style=flat-square" /></a>
  <a href="https://codecov.io/gh/pauloremoli/logana"><img src="https://codecov.io/gh/pauloremoli/logana/branch/main/graph/badge.svg?style=flat-square" /></a>
  <a href="https://crates.io/crates/logana"><img src="https://img.shields.io/crates/v/logana.svg?style=flat-square" /></a>
  <a href="https://crates.io/crates/logana"><img src="https://img.shields.io/crates/d/logana.svg?style=flat-square" /></a>
  <a href="https://github.com/pauloremoli/logana/blob/main/LICENSE"><img src="https://img.shields.io/crates/l/logana.svg?style=flat-square" /></a>
</p>

A fast terminal log viewer for files of any size — including multi-GB logs. Built on SIMD-accelerated line indexing, search, and filtering. Auto-detects log formats, filters by pattern, regex, field value, or date range — bookmark lines, add annotations, and export your analysis.

<p align="center">
  <img src="docs/src/demo.gif" alt="logana demo" />
</p>

---

## Performance


### Headless mode

Benchmarked against [lnav](https://lnav.org/) with headless mode filtering a [**3.3 GB web server access log with 10 million+ lines**](https://www.kaggle.com/datasets/eliasdabbas/web-server-access-logs), with a cold disk cache, measured on 10 runs.

**logana**
```
$ hyperfine --prepare 'rm filtered.log;sync; echo 3 | sudo tee /proc/sys/vm/drop_caches' \
  'logana ~/logs/access.log -i food --headless > filtered.log' --runs 10

Benchmark 1: logana ~/logs/access.log -i food --headless > filtered.log
  Time (mean ± σ):     999.9 ms ±   2.9 ms    [User: 2277.7 ms, System: 3057.0 ms]
  Range (min … max):   995.2 ms … 1003.6 ms    10 runs
```

**lnav**
```
$ hyperfine --prepare 'rm filtered.log;sync; echo 3 | sudo tee /proc/sys/vm/drop_caches' \
  'lnav ~/logs/access.log -c ":filter-in food" -n > filtered.log' --runs 10

Benchmark 1: lnav ~/logs/access.log -c ":filter-in food" -n > filtered.log
  Time (mean ± σ):     11.197 s ±  0.140 s    [User: 14.177 s, System: 1.580 s]
  Range (min … max):   10.980 s … 11.483 s    10 runs
```

**logana is ~11× faster than lnav.**

<p align="center">
  <img src="docs/src/performance.gif" alt="logana performance comparison with lnav" />
</p>

The headless mode was used for more reliable measurements.


### TUI 

Same 3.3GB file with the TUI (filter passed as command argument, quit once data was fully loaded and filter applied):

```
$ time logana ~/logs/access.log -i food
logana ~/logs/access.log -i food  5.21s user 3.61s system 197% cpu 4.475 total

$ time lnav ~/logs/access.log -c ":filter-in food"
lnav ~/logs/access.log -c ":filter-in food"  12.14s user 1.37s system 114% cpu 11.819 total
```

**~2.6× faster end-to-end with the UI.**

Hardware: AMD Ryzen 9 8945HS · 32 GB DDR5 5600 MHz · PCIe NVMe 4.0 x4

**Note:** lnav is a mature tool with a significantly broader feature set.

---

## Features

- **Auto-detected log formats** — JSON, syslog, journalctl, logfmt, OpenTelemetry, DLT (AUTOSAR), and more
- **Filtering** — include/exclude patterns (literal or regex), date-range filters, field-scoped filters; add filters from the command line with `-i`/`-o`/`-t`
- **Persistent sessions** — filters, scroll position, marks, and annotations survive across runs; configurable restore policy (ask / always / never)
- **Structured field view** — parsed timestamps, levels, targets, and extra fields displayed in columns; select which columns you want visible
- **Vim-style navigation** — `j`/`k`, `gg`/`G`, `Ctrl+d`/`u`, count prefixes (`5j`, `10G`), `/` search, `e`/`w` error/warning jumps
- **Annotations** — attach comments to log lines; export analysis to Markdown or Jira
- **Value coloring** — HTTP methods, status codes, IP addresses, and UUIDs colored automatically
- **OTel collector** — receive OpenTelemetry logs in real time over gRPC or HTTP/JSON; compatible with any OTel SDK
- **Multi-tab** — open multiple files, Docker streams, DLT daemon connections, or OTel collector tabs; each tab has independent filters and session state
- **MCP server** — embedded Model Context Protocol server; expose marks and annotations to AI assistants
- **Headless mode** — run the full filter pipeline without a TUI to preprocess huge logs
- **Fully configurable** — all keybindings remappable via a config file

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
logana            # then type :otel

# Add inline filters on the command line
logana app.log -i error -o debug
logana app.log -i "--field level=ERROR" -t "> 2024-02-21"

# Headless — filter without the TUI, output to stdout or a file
logana app.log --headless -i error -o debug
logana app.log --headless -i error --output filtered.log
```

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
