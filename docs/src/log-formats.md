# Log Formats

logana detects the log format automatically by sampling the first lines of the file. No flags or configuration are required.

## Supported Formats

| Format | Examples |
|---|---|
| JSON | tracing-subscriber JSON, bunyan, pino, any structured JSON logger |
| Syslog | RFC 3164 (BSD), RFC 5424 |
| Journalctl | short-iso, short-precise, short-full |
| Common / Combined Log | Apache access, nginx access |
| Logfmt | Go `slog`, Heroku, Grafana Loki |
| Common log family | env_logger, tracing-subscriber fmt (with/without spans), logback, log4j2, Spring Boot, Python logging, loguru, structlog |
| nginx / Apache error | Error log format |
| dmesg | Linux kernel ring buffer |
| Kubernetes CRI | Container runtime interface log format |

## Detection

All registered parsers score a confidence value (0.0–1.0) against the first 200 lines of the file. The parser with the highest score above 0.0 is selected. More specific parsers naturally score higher on their format; the common log parser applies a 0.95× penalty to yield to more specific parsers on ties.

The detected format name is shown in the status bar.

## Format Details

### JSON

Structured JSON logs, one JSON object per line. Supports:
- **tracing-subscriber JSON** — `{"timestamp":...,"level":...,"target":...,"span":{...},"fields":{"message":...}}`
- **bunyan** — `{"time":...,"level":...,"name":...,"msg":...}`
- **pino** — `{"time":...,"level":...,"msg":...}`
- Any structured JSON log with recognizable timestamp/level/message keys

Span sub-fields (e.g. `span.name`, `span.id`, `fields.request_id`) are discoverable and selectable as columns.

### Syslog

- **RFC 3164 (BSD)**: `<PRI>Mmm DD HH:MM:SS hostname app[pid]: message`
- **RFC 5424**: `<PRI>VER TIMESTAMP HOSTNAME APP PROCID MSGID [SD] MSG`

Priority is decoded to a log level; facility is exposed as an extra field.

### Journalctl

Text output from `journalctl` in several formats:
- **short-iso**: `YYYY-MM-DDTHH:MM:SS±ZZZZ hostname unit[pid]: message`
- **short-precise**: `Mmm DD HH:MM:SS.FFFFFF hostname unit[pid]: message`
- **short-full**: `Www YYYY-MM-DD HH:MM:SS TZ hostname unit[pid]: message`

Header/footer lines (`-- Journal begins...`, `-- No entries --`) are silently skipped.

### Common / Combined Log Format

Apache and nginx access logs:
- **CLF**: `host ident authuser [dd/Mmm/yyyy:HH:MM:SS ±ZZZZ] "request" status bytes`
- **Combined**: CLF + `"referer" "user-agent"`

Fields with value `-` are omitted.

### Logfmt

Space-separated `key=value` pairs. Used by Go `slog`, Heroku, Grafana Loki, and many 12-factor apps. Quoted values (`key="value with spaces"`) are supported.

Requires at least 3 key=value pairs per line to distinguish from plain text.

### Common Log Family

A broad family sharing the `TIMESTAMP LEVEL TARGET MESSAGE` structure, with several sub-strategies:

- **env_logger**: `[ISO LEVEL  target] msg` or `[LEVEL target] msg`
- **logback / log4j2**: `DATETIME [thread] LEVEL target - msg`
- **Spring Boot**: `DATETIME  LEVEL PID --- [thread] target : msg`
- **Python basic**: `LEVEL:target:msg`
- **Python prod**: `DATETIME - target - LEVEL - msg`
- **loguru**: `DATETIME | LEVEL | location - msg`
- **structlog**: `DATETIME [level] msg key=val...`
- **tracing-subscriber fmt with spans**: `TIMESTAMP LEVEL  span_name{k=v ...}: target: msg` — span context is parsed and available as the `span` column
- **Generic fallback**: `TIMESTAMP LEVEL rest-as-message` — any timestamp + level keyword combination

### tracing-subscriber fmt (Rust / Axum)

Rust applications using `tracing-subscriber`'s default `fmt` output produce lines like:

**Startup (no span):**
```
2024-02-21T10:00:00.123456Z  INFO app::server: listening on 0.0.0.0:3000
```

**Runtime (with span):**
```
2024-02-21T10:00:01.234Z  INFO request{method=GET uri=/api/users id="0.5"}: app::handler: processing request
```

Both forms are handled: span lines are parsed into a `span` column with `name` and `fields`; non-span lines fall through to the generic fallback.

### nginx / Apache Error

Error log formats distinct from access logs:
- nginx: `YYYY/MM/DD HH:MM:SS [level] pid#tid: *cid message, ...`
- Apache 2.4: `[Www Mon DD HH:MM:SS.usec YYYY] [module:level] [pid tid] message`

### dmesg

Linux kernel ring buffer output: `[seconds.usecs] message`

The bracketed timestamp is normalized; the rest is the message.

### Kubernetes CRI

Container runtime interface format: `ISO STDOUT/STDERR F/P message`

`F` = full line, `P` = partial line. Partial lines are concatenated.
