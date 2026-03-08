# Date & Time Filters

Date filters narrow the visible lines by their parsed timestamp. They work as a post-processing step after text filters — only lines already passing text filters are checked against date filters.

## Adding a Date Filter

**From the filter manager** (`f` → `t`): opens command mode pre-filled with `date-filter `.

**From command mode:**
```sh
:date-filter <expression>
```

---

## Expression Syntax

### Equals (no operator)

Omitting an operator matches the full period implied by the input's granularity.

| Input | Matches |
|-------|---------|
| `09:00` | the whole minute 09:00:00 – 09:00:59 |
| `09:00:30` | the exact second 09:00:30 |
| `Feb 21` | all of Feb 21 (00:00:00 – 23:59:59) |
| `Feb/21` | same — `/` is accepted as month/day separator |
| `02/21` | same — numeric month/day |
| `02-21` | same — numeric month-day |
| `02/21/2024` | all of Feb 21 2024 |
| `02-21-2024` | same with dash separators |
| `2024-02-21` | all of Feb 21 2024 |
| `2024-02-21 10:15` | the whole minute 10:15:00 – 10:15:59 |
| `2024-02-21 10:15:30` | the exact second |

```sh
:date-filter Feb/21
:date-filter 02/21
:date-filter 02-21
:date-filter 09:00
```

---

### Range (`..`)

Both bounds are **inclusive**. Spaces around `..` are optional.

The **upper bound is expanded to the end of its granularity period**: a day-level upper bound covers up to `23:59:59.999999`, a minute-level upper bound covers up to `:59.999999`, and a second-level upper bound is exact.

```sh
# time-only (compares seconds since midnight)
:date-filter 09:00 .. 17:00       # 09:00:00 – 17:00:59
:date-filter 09:00..17:00         # same, no spaces required
:date-filter 09:00:00 .. 17:00:00 # exact seconds

# BSD month names
:date-filter Feb 21 .. Feb 22     # Feb 21 00:00:00 – Feb 22 23:59:59
:date-filter Feb/21 .. Feb/22

# numeric month/day
:date-filter 02/21 .. 02/22       # Feb 21 00:00:00 – Feb 22 23:59:59
:date-filter 02-21 .. 02-22
:date-filter 03-21..03-25         # no spaces

# ISO dates
:date-filter 2024-02-21 .. 2024-02-22

# full datetimes (second-exact bounds, no expansion)
:date-filter 2024-02-21T10:00:00 .. 2024-02-21T11:30:00
:date-filter 2024-02-21 10:00:00 .. 2024-02-21 11:30:00
```

---

### Comparison operators

```sh
:date-filter > 2024-02-21T10:00:00    # after
:date-filter >= Feb 21 10:00:00       # from (inclusive)
:date-filter < 02/22                  # before Feb 22
:date-filter <= Feb 22                # up to and including
```

Supported operators: `>`, `>=`, `<`, `<=`

---

## Accepted Date/Time Formats

### Date bounds

| Format | Example | Year |
|--------|---------|------|
| BSD month name + day | `Feb 21`, `Feb/21` | none (month/day only) |
| Numeric MM/DD | `02/21` | none |
| Numeric MM-DD | `02-21` | none |
| Numeric MM/DD/YYYY | `02/21/2024` | included |
| Numeric MM-DD-YYYY | `02-21-2024` | included |
| ISO date | `2024-02-21` | included |

### Time bounds

| Format | Example | Granularity |
|--------|---------|-------------|
| `HH:MM` | `09:00` | minute |
| `HH:MM:SS` | `09:00:30` | second |

### Combined datetime bounds

Any date format above followed by a space and a time:

```
Feb/21 09:00
02/21 09:00:30
02-21-2024 10:15
2024-02-21T10:15:30
2024-02-21 10:15:30
```

ISO 8601 `T` separator and a plain space are both accepted.

---

## Rules and Limitations

- **Inclusive bounds**: `..` ranges include both endpoints (`>=` lower AND `<=` upper).
- **No midnight wraparound**: `23:00 .. 01:00` is invalid. Use two comparison filters instead.
- **Mixed-mode ranges are rejected**: both sides of a `..` must use the same format (both time-only or both date).
- **Multiple date filters are OR-ed**: a line passes if it satisfies *any* enabled date filter.
- **Lines without a timestamp pass through**: continuation lines, stack traces, and multi-line messages are never hidden by date filters.
- **Requires a detected format parser**: if logana cannot detect the log format, date filters return an error. This means plain-text logs without timestamps cannot be date-filtered.

---

## Display

Date filters appear in the filter manager and sidebar as `Date: <expression>`, not as raw `@date:` patterns.

---

## How It Works

Date filters are stored as regular `FilterDef` entries in the database with an `@date:` prefix in the pattern field (e.g. `@date:01:00:00 .. 02:00:00`). They are excluded from the text-filter pipeline and applied separately in `refresh_visible()` after text filters run, via `retain()` on `visible_indices`.

Timestamps are normalized to a canonical `YYYY-MM-DD HH:MM:SS.ffffff` string before comparison, so all supported log format timestamps (ISO 8601, BSD, logback datetime, CLF, journalctl, Apache error, etc.) are comparable regardless of their original format.
