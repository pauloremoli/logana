# Date & Time Filters

Date filters narrow the visible lines by their parsed timestamp. They work as a post-processing step after text filters — only lines already passing text filters are checked against date filters.

## Adding a Date Filter

**From the filter manager** (`f` → `t`): opens command mode pre-filled with `date-filter `.

**From command mode:**
```sh
:date-filter <expression>
```

## Expression Syntax

### Time-only range (same day)
```sh
:date-filter 01:00:00 .. 02:00:00
:date-filter 09:30 .. 17:00
```
Compares seconds since midnight. Both bounds are inclusive.

### Date range
```sh
:date-filter Feb 21 .. Feb 22
:date-filter 2024-02-21 .. 2024-02-22
```

### Full datetime range
```sh
:date-filter 2024-02-21T10:00:00 .. 2024-02-21T11:30:00
:date-filter 2024-02-21 10:00:00 .. 2024-02-21 11:30:00
```

### Comparison operators
```sh
:date-filter > 2024-02-21T10:00:00    # after
:date-filter >= Feb 21 10:00:00       # from
:date-filter < 2024-02-22             # before
:date-filter <= Feb 22                # up to and including
```

Supported operators: `>`, `>=`, `<`, `<=`

## Display

Date filters appear in the filter manager and sidebar as `Date: <expression>`, not as raw `@date:` patterns.

## Rules and Limitations

- **Inclusive bounds**: `..` ranges include both endpoints (`>=` lower AND `<=` upper).
- **No midnight wraparound**: `23:00 .. 01:00` is invalid. Use two comparison filters instead.
- **Multiple date filters are AND-ed**: a line must satisfy every enabled date filter.
- **Lines without a timestamp pass through**: continuation lines, stack traces, and multi-line messages are never hidden by date filters.
- **Requires a detected format parser**: if logana cannot detect the log format, date filters return an error. This means plain-text logs without timestamps cannot be date-filtered.

## How It Works

Date filters are stored as regular `FilterDef` entries in the database with an `@date:` prefix in the pattern field (e.g. `@date:01:00:00 .. 02:00:00`). They are excluded from the text-filter pipeline and applied separately in `refresh_visible()` after text filters run, via `retain()` on `visible_indices`.

Timestamps are normalized to a canonical `YYYY-MM-DD HH:MM:SS.ffffff` string before comparison, so all supported log format timestamps (ISO 8601, BSD, logback datetime, etc.) are comparable.
