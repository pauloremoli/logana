# Field Filters

Field filters let you narrow the log view by the value of a **specific parsed field** rather than matching against the raw line text. This is useful when you want to, for example, show only `error`-level lines without accidentally matching the word "error" in a message body.

## Syntax

```sh
:filter --field <key>=<value>
:exclude --field <key>=<value>
```

The `--field` flag tells logana to treat the pattern as a `key=value` pair. The value is matched as a **substring** of the named field.

```sh
:filter --field level=error         # show only lines where level contains "error"
:filter --field component=auth      # show only lines from the auth component
:exclude --field level=debug        # hide all debug-level lines
```

## Field Name Aliases

The following short aliases are recognised regardless of how the field is named in the raw log:

| Alias(es) | Field |
|---|---|
| `level`, `lvl` | log level |
| `timestamp`, `ts`, `time` | timestamp |
| `target` | logger / target name |
| `message`, `msg` | log message body |
| anything else | looked up by exact key in extra fields |

For example, `:filter --field lvl=warn` and `:filter --field level=warn` are equivalent.

## Combining Field Filters

**Multiple include field filters** — all must match (AND logic):

```sh
:filter --field level=error
:filter --field component=auth
# only lines where level contains "error" AND component contains "auth"
```

**Exclude field filters** — hide any line where the field matches:

```sh
:exclude --field level=debug
```

**Mixed include and exclude** — exclude takes priority. A line that satisfies an include filter but also matches an exclude filter is hidden.

## Pass-Through Behaviour

Lines that cannot be parsed (e.g. plain-text lines in an otherwise structured file) are **always shown** — they are not hidden by field filters. The same applies when the named field is absent from an otherwise parseable line.

This matches the behaviour of [date filters](date-filters.md) for lines without timestamps.

## Sidebar Display

Field filters appear in the filter manager sidebar with a `[field]` tag:

```
[x] In: level=error [field]
[x] Out: level=debug [field]
```

## Requires a Detected Format

Field filters only have an effect when logana has detected a structured log format (JSON, logfmt, syslog, etc.). On plain-text files with no detected format, all lines pass through field filters unchanged.

See [Log Formats](../log-formats.md) for the list of supported formats.
