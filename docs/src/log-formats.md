# Log Formats

logana detects the log format automatically by sampling the first lines of the file. No flags or configuration are required.

## Supported Formats

| Format | Examples |
|---|---|
| OpenTelemetry (OTLP) | OTLP/JSON protobuf-JSON encoding, OTel SDK JSON |
| DLT | AUTOSAR binary (storage, wire, simplified) and `dlt-convert -a` text |
| JSON | tracing-subscriber JSON, bunyan, pino, any structured JSON logger |
| Syslog | RFC 3164 (BSD), RFC 5424 |
| Journalctl | short, short-iso, short-precise, short-full, short-monotonic, short-unix, json-sse, json-seq |
| Common / Combined Log | Apache access, nginx access |
| Logfmt | Go `slog`, Heroku, Grafana Loki |
| Common log family | env_logger, tracing-subscriber fmt (with/without spans), logback, log4j2, Spring Boot, Python logging, loguru, structlog |

## Detection

All registered parsers score a confidence value against the first 200 lines of the file. The parser with the highest score above 0.0 is selected. More specific parsers naturally score higher on their format; the common log parser applies a 0.95× penalty to yield to more specific parsers on ties. The OTLP parser scores up to 1.5 (above the 1.0 maximum for plain JSON) so it wins when OpenTelemetry fields are present.

User-defined schemas (see [Custom Schemas](#custom-schemas) below) are always evaluated first, before any built-in parser.

The detected format name is shown in the status bar. Run `:schema` to show the active one, or `:schema <name>` to force a specific format for the current tab — typing `:schema ` shows every custom and built-in schema in the autocomplete list (custom ones first, alphabetically, each in its own color).

## Format Details

### DLT (AUTOSAR Diagnostic Log and Trace)

Three binary layouts are supported and converted to text at load time:

- **Storage format** — standard AUTOSAR DLT files with 16-byte storage headers (magic bytes `DLT\x01`)
- **Wire format** — concatenated DLT messages without storage headers, as received from a `dlt-daemon` TCP connection
- **Simplified format** — compact `DLT\x01` + ECU + APID + CTID + timestamp + payload

The text output produced by `dlt-convert -a` is also parsed directly.

Fields extracted: `timestamp`, `hw_ts` (hardware timestamp), `mcnt` (message counter), `ecu`, `apid` (application ID), `ctid` (context ID), `type`, `subtype`, `mode` (verbose/non-verbose).

Verbose payloads are decoded (strings, integers, floats, booleans, raw data). Non-verbose payloads are shown as hex.

### OpenTelemetry (OTLP)

Two JSON-based OTel log formats are supported for file-based parsing. logana also accepts **live OTLP streams** over gRPC (`:otel`, port 4317) and HTTP/JSON (`:otel --http`, port 4318) — see [OTel Collector](otel.md).

**OTLP/JSON** (protobuf-JSON encoding — exported by collectors):
```json
{"timeUnixNano":"1700000000000000000","severityNumber":9,"severityText":"INFO","body":{"stringValue":"request received"},"attributes":[{"key":"service.name","value":{"stringValue":"my-svc"}}]}
```
- Timestamp: `timeUnixNano` (nanosecond epoch string)
- Severity: `severityNumber` (1–4=TRACE, 5–8=DEBUG, 9–12=INFO, 13–16=WARN, 17–20=ERROR, 21–24=FATAL) and/or `severityText`
- Body: `body.stringValue` (AnyValue object encoding)
- Attributes: array of `{key, value}` objects

**OTel SDK JSON** (emitted directly by SDKs):
```json
{"timestamp":"2024-01-01T00:00:00.000Z","severity_text":"INFO","severity_number":9,"body":"request received","attributes":{"service.name":"my-svc"}}
```
- Timestamp: `timestamp` (ISO 8601)
- Severity: `severity_text` and/or `severity_number`
- Body: direct string value
- Attributes: flat `{key: value}` dict

Both formats surface `service.name`, `code.namespace`, `logger`, and similar target attributes as the **target** column.

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
- **short**: `Mmm DD HH:MM:SS hostname unit[pid]: message`
- **short-iso**: `YYYY-MM-DDTHH:MM:SS±ZZZZ hostname unit[pid]: message`
- **short-precise**: `Mmm DD HH:MM:SS.FFFFFF hostname unit[pid]: message`
- **short-full**: `Www YYYY-MM-DD HH:MM:SS TZ hostname unit[pid]: message`
- **short-monotonic**: `[SSSSS.FFFFFF] hostname unit[pid]: message`
- **short-unix**: `[EPOCH.FFFFFF] hostname unit[pid]: message`
- **json-sse**: server-sent events wrapping JSON journal entries (`data: {...}`)
- **json-seq**: RFC 7464 JSON sequence (`\x1e{...}\n`)

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

## Custom Schemas

If none of the built-in parsers match your log format, you can define your own schema. Each schema lives in its own JSON file inside a `schema/` directory next to `config.json`:

| OS | Path |
|---|---|
| Linux | `~/.config/logana/schema/<name>.json` |
| macOS | `~/Library/Application Support/logana/schema/<name>.json` |
| Windows | `%APPDATA%\logana\schema\<name>.json` |

On Windows, `%APPDATA%` resolves to `C:\Users\<username>\AppData\Roaming`, e.g. `C:\Users\<username>\AppData\Roaming\logana\schema\<name>.json`.

### Template syntax

The easiest way to describe a format is with a **template** — write the literal shape of a log line with `{field}` placeholders where fields appear:

```
{id} {service} <{timestamp}> {pid} {level}/{component}/{feature}, {message}
```

logana compiles the template to a regex automatically:

- `{name}` — matches non-whitespace characters, stopping at the next literal delimiter or whitespace
- `{name}` when adjacent to a literal character (e.g. `<{timestamp}>`) — stops at that character
- The **last placeholder** always captures the rest of the line

Alternatively, supply a raw `pattern` with named capture groups for formats the template language cannot express.

### Field roles

Placeholder names that match a known semantic are mapped automatically:

| Name | Semantic |
|---|---|
| `timestamp` | Timestamp column |
| `level` | Level column (normalized: `INF`→Info, `ERR`→Error, etc.) |
| `message` | Message column |
| `target` | Target column |
| `component` | Component field |
| `feature` | Feature field |
| `hostname` | Hostname field |
| `pid` | PID field |
| `thread` | Thread field |
| `facility` | Facility field |

Any other placeholder name defaults to an extra field. Use the `fields` map to assign a different role to a non-standard name.

### Example

Acme node log line:
```
04 LINUX-0-syscon <2035-04-04T21:54:53.283856Z> 62A INF/Syscon/StartupMgr, StateChange: dirtyrfservice::instance1 state=CONNECTED
```

Schema file at `~/.config/logana/schema/acme.json`:
```json
{
  "name": "acme",
  "description": "Acme node log format",
  "template": "{id} {service} <{timestamp}> {pid} {level}/{component}/{feature}, {message}",
  "fields": {
    "id":      "extra",
    "service": "target"
  }
}
```

`service` (`LINUX-0-syscon`) is mapped to `target` because it identifies the producing service. `id` is mapped to `extra` since no built-in semantic fits a hex sequence number.

### Critical fields

Three fields unlock core logana features. Map them correctly if your format contains them:

| Field | Features that depend on it |
|---|---|
| `timestamp` | Date & time filters (`:date-filter`, `-t`) — without a timestamp field, date filters are silently skipped |
| `level` | Error/warning navigation (`e`/`w`) and level-based coloring — both are disabled when no level field is present |
| `target` | Field coloring by originating component in the structured view |

### Forcing a schema

```
:schema             — show the active schema
:schema acme     — force the acme schema for this tab
```

### Default filter files per format

```
:default-filters                              — open a popup listing every format and its configured filter file
:default-filters acme ~/logs/filters/acme.json — set acme's default filter file
:default-filters acme                          — clear acme's default filter file mapping
```

When a tab's format becomes `acme` — auto-detected on open, or via `:schema acme` — and the tab has no filters yet, its configured default filter file loads automatically, same effect as `:load-filters`. Setting or clearing a mapping never retroactively affects the tab you're currently on — it only applies the next time a tab's format is assigned.

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

