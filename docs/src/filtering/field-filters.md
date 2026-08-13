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

`--field` can be repeated within a single command to require several fields at once, and combined with trailing free text that must also match — all AND'd together in one filter:

```sh
:filter --field level=INFO --field component=Draco Power measurements:
# shows only lines where level contains "INFO" AND component contains "Draco"
# AND the line contains "Power measurements:"
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

There are two distinct ways to combine field conditions, with different logic:

**Multiple `--field` flags in one command** — AND logic. Every condition (and any trailing text) must match:

```sh
:filter --field level=error --field component=auth
# only lines where level contains "error" AND component contains "auth"
```

**Multiple separate `:filter` commands** — OR logic, same as any other include filters. Each broadens what's visible:

```sh
:filter --field level=error
:filter --field level=warn
# shows lines where level contains "error" OR level contains "warn"
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

Field filters appear in the filter manager sidebar with a `[field]` tag. A filter with multiple `--field` conditions and/or trailing text shows all of them, comma-separated:

```
[x] In: level=error [field]
[x] Out: level=debug [field]
[x] In: level=INFO, component=Draco, Power measurements: [field]
```

## Group-Scoped Fields

A [custom schema](../custom-schemas.md#repeating-groups) can declare a repeating group of sub-records (e.g. a batch job's `workers`). Filter on a field inside any item of the group with `<group>.<field>=<value>` — it matches if **any** item in the group has that field:

```sh
:filter --field workers.hostname=worker-3
# shows records where any worker's hostname contains "worker-3"
```

This is "any item matches" semantics, independent from plain field lookup — an indexed path like `workers.0.hostname` (as shown in the structured fields columns) is display-only and can't be used as a filter path.

## Requires a Detected Format

Field filters only have an effect when logana has detected a structured log format (JSON, logfmt, syslog, etc.). On plain-text files with no detected format, all lines pass through field filters unchanged.

See [Log Formats](../log-formats.md) for the list of supported formats.
