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

## Schema Validation

logana publishes a JSON Schema for custom schema files. Add a `$schema` line to enable validation and autocomplete in VS Code and other JSON-aware editors:

```json
{
  "$schema": "https://raw.githubusercontent.com/pauloremoli/logana/main/schema/custom-schema.schema.json",
  "name": "my-format",
  "template": "{level} {message}"
}
```

An unrecognized key (e.g. a typo like `"templte"`) is rejected at load time — logana's startup warning names the file and the bad key instead of silently ignoring it.

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
| `template` | one of `template` / `pattern` | A single-line template string, or an array describing a full multi-line record — see [Multiline records](#multiline-records) |
| `pattern` | one of `template` / `pattern` | Raw regex with named capture groups |
| `fields` | no | Overrides the automatic role for named placeholders/groups |
| `multiline` | no | When `true` (and `template` is a plain string), folds continuation lines into the record's `message` field — see [Folding continuation text into `message`](#folding-continuation-text-into-message) |

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

A default filter file can also be configured per schema — see [Default filter files per format](./log-formats.md#default-filter-files-per-format).

## Multiline records

A line that doesn't match the schema's `template`/`pattern` is already treated as a continuation of the previous matched line — it's shown right below its parent and inherits the parent's level and visibility. This covers most multiline formats (stack traces, wrapped messages) automatically, with no schema changes needed.

By default, continuation lines stay unstructured: their content isn't part of any parsed field, so field filters can't see it. Two independent ways to give them structure:

- **`"multiline": true`** — fold all continuation text into the record's `message` field as one blob. Simplest option; use it when you just want the continuation text searchable, not broken into individual fields.
- **An array `template`** — describe the continuation lines' own shape, extracting each into its own field (or a repeating group of sub-records). Use this when continuation lines carry structured `key: value` data you want to filter on individually.

The two can be combined: an array `template` structures what it recognizes, and `multiline` still folds any unrecognized trailing continuation lines into `message`.

### Folding continuation text into `message`

Set `"multiline": true` to fold continuation lines into the record's `message` field:

```json
{
  "name":      "journalctl-verbose",
  "template":  "{weekday} {timestamp} {host} [{cursor}]",
  "multiline": true,
  "fields": {
    "cursor": "extra"
  }
}
```

Given `journalctl --output=verbose` output like:
```
Tue 2024-01-01 10:15:30.123456 UTC myhost [s=abc;i=1;b=def;t=2;x=3]
    MESSAGE=disk usage at 92%
    _PID=1234
    _COMM=diskmond
```

Only the first line matches this schema's template — the two indented `_KEY=VALUE` lines become its continuation. With `multiline` enabled, the record's `message` field becomes the continuation lines' raw text (joined by their original newlines), so `:filter --field message disk usage` matches the record even though the header line itself carries no message text at all. If the schema's template *does* capture a `message` field, continuation text is appended after it instead of replacing it.

This only changes what field filters and the structured fields panel see for the record — each physical line still renders as its own row in the log panel, exactly as before.

## Structured continuation lines

For formats where each continuation line carries its own field — or where the record has an explicit terminator instead of just running until the next header — make `template` an array instead of a single string. Each element describes one line of the record, in order:

- **Element 0** is always the header (the line that starts a new record) — same rules as a plain string `template`.
- **A plain string or object** after the header is a flat continuation line, extracting its own fields.
- **`{"vec": "<name>", "template": ..., "fields": [...], "auto_fields": ...}`** declares a repeating group — see [Repeating groups](#repeating-groups) below.
- **If the last element is a plain line** (not a group), it's the record's terminator: extraction stops once that line is seen, purely by its position as the last array element — no separate marker needed. Without one, the record runs until the next line matching the header.

```json
{
  "name": "transaction",
  "template": [
    "### Start transaction {id}",
    "field1: {field1}",
    "field2: {field2}",
    { "template": "Object {payload}", "json": true },
    "### End transaction"
  ],
  "fields": { "id": "extra" }
}
```

Each flat line accepts the same shorthand as the top-level schema: a bare string (template syntax), or an object with `template`/`pattern`/`fields`/`json` for more control. A continuation field can't be mapped to `timestamp`, `level`, `target`, or `message` — those belong to the header alone; map it to `extra` (the default) or any other field role instead. Set `"json": true` on an entry to treat its single placeholder as an embedded JSON object instead of a plain string — each key in the object becomes its own `extra` field, using the JSON key's own name.

Given:

```
### Start transaction 42
field1: 10
field2: 3
Object { "user": "alice", "amount": 99 }
### End transaction
```

the record is parsed with extra fields `id=42, field1=10, field2=3, user=alice, amount=99`. The terminator (`"### End transaction"`) also bounds where extraction stops — any lines between it and the next `### Start transaction` (stray blank lines, unrelated output, etc.) aren't scanned for fields.

### Repeating groups

Use a `{"vec": ...}` entry when a record contains a variable number of nested sub-records — e.g. a batch job with any number of `workers`. Its `template` matcher opens a new item in the group (and finalizes the previous one, if any); its optional `fields` are matchers tried in order against subsequent lines while that item is open.

```json
{
  "name": "batch-job",
  "template": [
    "### Job {job_id} started",
    "Owner: {owner}",
    { "vec": "workers", "template": "worker: {hostname}" },
    {
      "vec": "checkpoints",
      "template": "checkpoint: {name}, status: {status}",
      "auto_fields": false
    },
    "### Job {job_id} finished"
  ],
  "fields": { "job_id": "extra" }
}
```

A group with only `template` (no `fields`) is a single-line-item group — every matching line closes the previous item and opens a new one from its own captures (`checkpoints` above: each line is a complete record). A group with `fields` opens an item on `template` and keeps extracting fields from subsequent lines into that same item until the next line that matches `template` (a new item), a different group's `template`, or the terminator.

`auto_fields` (default `true`) captures any continuation line inside a group's open item that looks like `key: value` (or `key: "value"`) but wasn't matched by a declared `fields` entry — useful when a nested block's field set varies too much to enumerate. Lines with no `key: value` shape are still ignored. Set it to `false` (as `checkpoints` does above) to only extract explicitly declared fields. `workers` above relies entirely on `auto_fields`: any `key: value` line following a `worker: {hostname}` line (e.g. `cpu: 87%`, `status: running`) is captured automatically into that worker's item.

A group's name becomes the dotted-index column prefix in rendering (`workers.0.hostname`, `workers.1.hostname`, …) and the group-scoped path for field filters (`workers.hostname` — see [Field Filters](filtering/field-filters.md#group-scoped-fields)).

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
