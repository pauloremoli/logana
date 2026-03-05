# Structured Fields

When logana detects a structured log format (JSON, logfmt, syslog, tracing-subscriber, etc.), it parses each line into named columns: **timestamp**, **level**, **target**, **span**, and **message**, plus any extra fields specific to the format.

## Columns

| Column | Description |
|---|---|
| `timestamp` | Parsed log timestamp |
| `level` | Normalized log level (TRACE, DEBUG, INFO, WARN, ERROR, FATAL) |
| `target` | Logger name, module path, or source identifier |
| `span` | Tracing span context (name + fields), if present |
| `message` | The log message body |
| extra fields | Format-specific extras (e.g. `pid`, `thread`, `hostname`, `request_id`) |

## Showing and Hiding Columns

Use `:select-fields` to open an interactive column picker:

- `j` / `k` — navigate
- `Space` — toggle column on/off
- `J` / `K` — reorder columns
- `a` — enable all
- `n` — disable all
- `Enter` — apply
- `Esc` — cancel

Or use commands directly:

```sh
:fields timestamp level message       # show only these columns, in this order
:hide-field span                      # hide a single column
:show-field span                      # show a previously hidden column
:show-all-fields                      # reset to default display
```

## Field Key Display

Extra fields and span fields carry both a key and a value. By default logana shows only the values to keep lines compact. Use `:show-keys` to include the key names:

```sh
:show-keys     # request_id=abc123  status=200  request: method=GET uri=/api/users
:hide-keys     # abc123  200  request: GET /api/users  (default)
```

This applies to all structured formats — JSON extra fields, logfmt pairs, syslog structured data, span fields, and any other key-value extras that don't map to a canonical column (timestamp, level, target, message). This setting is persisted per file in the session database.

## Span Fields

Span context is parsed from formats that carry it (tracing-subscriber JSON, tracing-subscriber fmt text, and others). The `span` column shows the span name followed by its fields:

```
request: GET /api/users        # hide-keys (default)
request: method=GET uri=/api/users   # show-keys
```

Span sub-fields can also be selected as individual columns:

```sh
:fields timestamp level span.method span.uri message
```

## Value Coloring

Even within structured columns, known value patterns are colored automatically:

- **HTTP methods** — GET (green), POST (yellow), PUT (blue), DELETE (red), PATCH (magenta)
- **HTTP status codes** — 2xx (green), 3xx (cyan), 4xx (yellow), 5xx (red)
- **IP addresses** — IPv4 and IPv6
- **UUIDs**

Configure which categories are colored via `:value-colors`.

## Tab Completion for Field Names

The `:fields` command and `:hide-field` / `:show-field` commands complete against the field names discovered from the first 200 visible log lines, so you don't need to remember exact field names.
