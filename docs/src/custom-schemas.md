# Custom Schemas

If none of the built-in parsers match your log format, define your own schema. logana loads every `.json` file from the `schema/` directory next to `config.json` and includes them in format detection automatically — no restart required after adding a file.

If a schema file fails to load (malformed JSON) or fails to compile (an invalid `template`/`pattern`), logana shows a startup warning naming the file or schema and the problem, instead of silently dropping it from format detection.

## Schema directory

| OS | Path |
|---|---|
| Linux | `~/.config/logana/schema/` |
| macOS | `~/Library/Application Support/logana/schema/` |
| Windows | `%APPDATA%\logana\schema\` |

On Windows, `%APPDATA%` resolves to `C:\Users\<username>\AppData\Roaming`, e.g. `C:\Users\<username>\AppData\Roaming\logana\schema`.

Each file describes one format. The filename is arbitrary; the `name` field inside the JSON is what identifies the schema at runtime.

## Schema file structure

```json
{
  "name":        "my-format",
  "description": "Optional human-readable description",
  "template":    "{field1} <{timestamp}> {level}/{component}, {message}",
  "fields": {
    "field1": "extra"
  }
}
```

| Key | Required | Description |
|---|---|---|
| `name` | yes | Identifier used by `:schema <name>` and shown in the status bar |
| `description` | no | Free-form description; not used by logana |
| `template` | one of `template` / `pattern` | Template string with `{field}` placeholders |
| `pattern` | one of `template` / `pattern` | Raw regex with named capture groups |
| `fields` | no | Overrides the automatic role for named placeholders/groups |

## Template syntax

A template describes the literal shape of a log line. Write the fixed characters exactly as they appear and mark variable parts with `{name}`:

```
{id} {service} <{timestamp}> {pid} {level}/{component}/{feature}, {message}
```

Compilation rules:

- A placeholder adjacent to a literal delimiter (e.g. `<{ts}>`, `{level}/`) stops at that character — `<{ts}>` becomes `<(?P<ts>[^>]+)>`.
- A placeholder followed by whitespace or another placeholder matches `\S+`.
- The **last placeholder** always captures the rest of the line, regardless of what follows it.

## Raw regex

Use `pattern` instead of `template` when the template language is not expressive enough:

```json
{
  "name": "my-format",
  "pattern": "^(?P<ts>\\S+)\\s+(?P<level>\\w+)\\s+(?P<message>.*)$",
  "fields": { "ts": "timestamp" }
}
```

The regex must use named capture groups (`(?P<name>...)`). Group names are resolved to field roles using the same rules as template placeholders.

## Field roles

Placeholder or capture group names that match a known semantic are mapped automatically:

| Name | Role |
|---|---|
| `timestamp` | Timestamp column |
| `level` | Level column — normalized automatically (`INF`→Info, `ERR`→Error, `WRN`→Warning, etc.) |
| `message` | Message column |
| `target` | Target column |
| `component` | Component field |
| `feature` | Feature field |
| `hostname` | Hostname field |
| `pid` | PID field |
| `thread` | Thread field |
| `facility` | Facility field |

Any name not in the table above defaults to `extra` — it appears in the structured fields sidebar with its original name.

Use the `fields` map to assign a different role to a non-standard name:

```json
"fields": {
  "service": "target",
  "seq":     "extra",
  "thr":     "thread"
}
```

The value can also be `"ignored"` to capture a group in the regex without displaying it.

### Critical fields

Three fields unlock core logana features. If your format contains them, map them correctly:

| Field | Features that depend on it |
|---|---|
| `timestamp` | **Date & time filters** (`:date-filter`, `-t` on the CLI) — without a timestamp field, date filters have nothing to match against and are silently skipped |
| `level` | **Error/warning navigation** (`e`/`w` keys to jump between errors and warnings) and **level-based field coloring** — without a level field both are disabled for that tab. If your level values aren't `error`/`warn`/etc., see [Level values](#level-values) below to map them. |
| `target` | **Field coloring by target** — used to color-code log lines by their originating component in the structured view |

If your format uses non-standard names for these fields, always map them explicitly:

```json
"fields": {
  "sev":     "level",
  "ts":      "timestamp",
  "subsys":  "target"
}
```

### Level values

`level` is normalized automatically against a built-in set of keywords (`error`/`err`, `warn`/`warning`/`wrn`, `info`/`inf`, …). If your format's level values don't match any of them — e.g. severity codes like `SEV1`/`SEV2` — level coloring and `e`/`w` error/warning navigation won't recognize them by default. Declare which raw values (case-insensitive) mean error and warning with `levels`:

```json
{
  "name":     "sev",
  "template": "{level}: {message}",
  "levels": {
    "error":   ["SEV1"],
    "warning": ["SEV2"]
  }
}
```

A value can only be declared for one of `error`/`warning` — declaring the same value in both is a schema error, reported the same way as an invalid `template`/`pattern` (see the startup warning at the top of this page). Values not covered by `levels` still fall back to the built-in keyword matching.

## Rendering

When a `template`-based schema matches a line, the log panel renders it using the template's own field order and literal separators — so `{level}/{component}/{feature}` shows as `INF/Syscon/StartupMgr`, not space-joined columns. Hiding a field with `:select-fields` keeps this: the hidden field's value drops and its adjacent separator collapses with it, so hiding `component` renders `INF/StartupMgr` rather than a dangling `INF//StartupMgr`. Only genuinely *reordering* columns (moving a field with `J`/`K` in `:select-fields`) falls back to the standard space-joined column layout, since an arbitrary new order can't be expressed with the template's fixed separators. This only applies to `template`-defined schemas; a `pattern`-defined schema (raw regex) always uses the column layout, since a regex has no literal skeleton to reconstruct from.

The `:select-fields` popup lists a `template`-defined schema's fields in the template's own order (e.g. `id, target, timestamp, pid, level, component, feature, message` for the acme example below) rather than the generic timestamp/level/target/extras/message grouping used for other formats — matching the order the line actually renders in by default.

## Detection

Custom schemas are evaluated before all built-in parsers. When a schema matches ≥ 50% of the sampled lines, it wins the detection competition and is used for all subsequent parsing.

Run `:schema` to show the current tab's active schema, or `:schema <name>` to force a specific custom schema for the current tab — start typing a name to see every custom and built-in schema in the autocomplete list.

## Full example — Acme node log

Log line:
```
04 LINUX-0-syscon <2035-04-04T21:54:53.283856Z> 62A INF/Syscon/StartupMgr, StateChange: dirtyrfservice::instance1 state=CONNECTED
```

`~/.config/logana/schema/acme.json`:
```json
{
  "name": "acme",
  "description": "Acme node log: id service <timestamp> pid level/component/feature, message",
  "template": "{id} {service} <{timestamp}> {pid} {level}/{component}/{feature}, {message}",
  "fields": {
    "id":      "extra",
    "service": "target"
  }
}
```

Parsed fields:

| Field | Value |
|---|---|
| timestamp | `2035-04-04T21:54:53.283856Z` |
| level | `INF` → Info |
| target | `LINUX-0-syscon` |
| component | `Syscon` |
| feature | `StartupMgr` |
| message | `StateChange: dirtyrfservice::instance1 state=CONNECTED` |
| id (extra) | `04` |
| pid (extra) | `62A` |
