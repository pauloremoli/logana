# logana

A fast, keyboard-driven terminal log viewer and analyzer built in Rust.

Open a file, pipe stdin, or stream Docker containers — logana auto-detects the log format and lets you filter, search, annotate, and export without ever leaving the terminal.

## Feature Overview

| Feature | Description |
|---|---|
| **Auto-detected formats** | JSON, syslog, journalctl, logfmt, tracing-subscriber, logback, and more |
| **Real-time filtering** | Include/exclude patterns (literal or regex), date-range filters, instant preview |
| **Persistent sessions** | Filters, scroll position, marks, and annotations survive across runs |
| **Structured field view** | Parsed columns (timestamp, level, target, span, extras); show/hide/reorder per session |
| **Multi-tab** | Open multiple files or Docker streams side-by-side |
| **Vim-style navigation** | `j`/`k`, `gg`/`G`, `Ctrl+d`/`u`, count prefixes (`5j`, `10G`), `/` search |
| **Visual line selection** | Select a range, yank to clipboard, or attach a comment |
| **Annotations** | Attach multiline comments to log lines; export to Markdown or Jira |
| **Value coloring** | HTTP methods, status codes, IP addresses, and UUIDs colored automatically |
| **Fully configurable** | All keybindings remappable via `~/.config/logana/config.json`; 9 bundled themes |
