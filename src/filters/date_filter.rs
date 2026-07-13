use crate::filters::FilterDef;
use crate::filters::StyleId;
use crate::parser::timestamp::MONTHS;

pub const DATE_PREFIX: &str = "@date:";

/// Fixed-size canonical timestamp buffer: `"YYYY-MM-DD HH:MM:SS.fff"` (23 bytes, ASCII).
pub type CanonicalTs = [u8; 23];

#[derive(Debug, Clone)]
pub struct DateFilterStyle {
    pub filter: DateFilter,
    pub style_id: StyleId,
    /// When `false`, the style is applied to the whole line instead of just the timestamp.
    pub match_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonOp {
    Gt,
    Ge,
    Lt,
    Le,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonMode {
    TimeOnly,
    FullDatetime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Granularity {
    Day,    // date with no time component
    Minute, // HH:MM with no seconds
    Second, // HH:MM:SS or full datetime
}

#[derive(Debug, Clone)]
pub struct DateBound {
    /// Seconds since midnight (only meaningful when mode == TimeOnly).
    time_val: Option<u32>,
    /// Canonical `"YYYY-MM-DD HH:MM:SS.fff"` buffer (mode == FullDatetime).
    datetime_val: Option<CanonicalTs>,
}

#[derive(Debug, Clone)]
pub enum DateFilter {
    Range {
        mode: ComparisonMode,
        lower: DateBound,
        upper: DateBound,
    },
    Comparison {
        mode: ComparisonMode,
        op: ComparisonOp,
        bound: DateBound,
    },
}

#[derive(Debug, Clone)]
struct NormalizedTimestamp {
    time_of_day: u32,
    canonical: CanonicalTs,
}

/// Write 7 integers into a `CanonicalTs` buffer without any allocation.
/// Produces `"YYYY-MM-DD HH:MM:SS.fff"`.
fn write_canonical(y: u32, mo: u32, d: u32, h: u32, m: u32, s: u32, ms: u16) -> CanonicalTs {
    let mut buf = [b'0'; 23];
    buf[0] = b'0' + (y / 1000) as u8;
    buf[1] = b'0' + (y / 100 % 10) as u8;
    buf[2] = b'0' + (y / 10 % 10) as u8;
    buf[3] = b'0' + (y % 10) as u8;
    buf[4] = b'-';
    buf[5] = b'0' + (mo / 10) as u8;
    buf[6] = b'0' + (mo % 10) as u8;
    buf[7] = b'-';
    buf[8] = b'0' + (d / 10) as u8;
    buf[9] = b'0' + (d % 10) as u8;
    buf[10] = b' ';
    buf[11] = b'0' + (h / 10) as u8;
    buf[12] = b'0' + (h % 10) as u8;
    buf[13] = b':';
    buf[14] = b'0' + (m / 10) as u8;
    buf[15] = b'0' + (m % 10) as u8;
    buf[16] = b':';
    buf[17] = b'0' + (s / 10) as u8;
    buf[18] = b'0' + (s % 10) as u8;
    buf[19] = b'.';
    buf[20] = b'0' + (ms / 100) as u8;
    buf[21] = b'0' + (ms / 10 % 10) as u8;
    buf[22] = b'0' + (ms % 10) as u8;
    buf
}

/// Parse a user-provided date/time token into a `(DateBound, ComparisonMode, Granularity)`.
///
/// Accepted forms:
///   HH:MM:SS             → TimeOnly,     Second
///   HH:MM                → TimeOnly,     Minute
///   Mmm DD HH:MM:SS      → FullDatetime, Second  (year = 0000)
///   Mmm/DD HH:MM:SS      → FullDatetime, Second  (slash separator accepted)
///   Mmm DD               → FullDatetime, Day     (year = 0000, time = 00:00:00)
///   Mmm/DD               → FullDatetime, Day
///   MM/DD                → FullDatetime, Day     (numeric month, year = 0000)
///   MM/DD/YYYY           → FullDatetime, Day
///   MM/DD HH:MM[:SS]     → FullDatetime, Minute/Second
///   MM/DD/YYYY HH:MM[:SS]→ FullDatetime, Minute/Second
///   YYYY-MM-DD           → FullDatetime, Day
///   YYYY-MM-DD HH:MM     → FullDatetime, Minute
///   YYYY-MM-DD HH:MM:SS  → FullDatetime, Second
///   YYYY-MM-DDTHH:MM:SS  → FullDatetime, Second
fn parse_bound(input: &str) -> Result<(DateBound, ComparisonMode, Granularity), String> {
    let s = input.trim();
    if s.is_empty() {
        return Err("Empty date/time value".to_string());
    }

    // Try HH:MM:SS or HH:MM
    if let Some((secs, gran)) = try_parse_time_only(s) {
        return Ok((
            DateBound {
                time_val: Some(secs),
                datetime_val: None,
            },
            ComparisonMode::TimeOnly,
            gran,
        ));
    }

    // Try BSD month prefix: "Feb 21 HH:MM:SS", "Feb/21", "Feb 21", etc.
    if s.len() >= 3 {
        let month_abbr = &s[..3];
        if let Some(month_num) = bsd_month_number(month_abbr) {
            return parse_bsd_bound(s, month_num);
        }
    }

    // Try numeric MM/DD[/YYYY] [HH:MM[:SS]] or MM-DD[-YYYY] [HH:MM[:SS]].
    // b[2] being '/' or '-' disambiguates from YYYY-MM-DD where b[2] is a
    // year digit.
    let b = s.as_bytes();
    if s.len() >= 5
        && b[0].is_ascii_digit()
        && b[1].is_ascii_digit()
        && (b[2] == b'/' || b[2] == b'-')
        && b[3].is_ascii_digit()
        && b[4].is_ascii_digit()
    {
        return parse_slash_month_day_bound(s, b[2]);
    }

    // Try YYYY-MM-DD[T| ]HH:MM:SS or YYYY-MM-DD
    if s.len() >= 10 && b[4] == b'-' && b[7] == b'-' && b[0].is_ascii_digit() {
        return parse_iso_bound(s);
    }

    Err(format!("Unrecognized date/time format: '{}'", s))
}

/// Returns `(seconds_since_midnight, granularity)` for `HH:MM[:SS]`.
fn try_parse_time_only(s: &str) -> Option<(u32, Granularity)> {
    let b = s.as_bytes();
    if !(b.len() == 5 || b.len() == 8) {
        return None;
    }
    if !b[0].is_ascii_digit() || !b[1].is_ascii_digit() || b[2] != b':' {
        return None;
    }
    if !b[3].is_ascii_digit() || !b[4].is_ascii_digit() {
        return None;
    }
    let h: u32 = s[..2].parse().ok()?;
    let m: u32 = s[3..5].parse().ok()?;
    if h > 23 || m > 59 {
        return None;
    }
    if b.len() == 8 {
        if b[5] != b':' || !b[6].is_ascii_digit() || !b[7].is_ascii_digit() {
            return None;
        }
        let sec_val: u32 = s[6..8].parse().ok()?;
        if sec_val > 59 {
            return None;
        }
        Some((h * 3600 + m * 60 + sec_val, Granularity::Second))
    } else {
        Some((h * 3600 + m * 60, Granularity::Minute))
    }
}

fn bsd_month_number(abbr: &str) -> Option<u32> {
    MONTHS
        .iter()
        .position(|&m| m.eq_ignore_ascii_case(abbr))
        .map(|i| i as u32 + 1)
}

/// Parse a BSD-style bound: `Mmm DD [HH:MM[:SS]]` or `Mmm/DD [HH:MM[:SS]]`.
fn parse_bsd_bound(
    s: &str,
    month_num: u32,
) -> Result<(DateBound, ComparisonMode, Granularity), String> {
    // Accept space or slash between month abbreviation and day number.
    let rest = s[3..].trim_start_matches([' ', '/']);
    // Parse day
    let day_end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    if day_end == 0 {
        return Err(format!("Expected day number after month in '{}'", s));
    }
    let day: u32 = rest[..day_end]
        .parse()
        .map_err(|_| format!("Invalid day in '{}'", s))?;
    if !(1..=31).contains(&day) {
        return Err(format!("Day out of range in '{}'", s));
    }

    let after_day = rest[day_end..].trim_start();
    let (h, m, sec, gran) = if after_day.is_empty() {
        (0, 0, 0, Granularity::Day)
    } else if let Some((secs, g)) = try_parse_time_only(after_day) {
        (secs / 3600, (secs % 3600) / 60, secs % 60, g)
    } else {
        return Err(format!("Invalid time in '{}'", s));
    };

    Ok((
        DateBound {
            time_val: None,
            datetime_val: Some(write_canonical(0, month_num, day, h, m, sec, 0)),
        },
        ComparisonMode::FullDatetime,
        gran,
    ))
}

/// Parse a numeric `MM/DD[/YYYY]` or `MM-DD[-YYYY]` bound, optionally followed
/// by `HH:MM[:SS]`. The `sep` byte is either `b'/'` or `b'-'`.
fn parse_slash_month_day_bound(
    s: &str,
    sep: u8,
) -> Result<(DateBound, ComparisonMode, Granularity), String> {
    let month: u32 = s[..2]
        .parse()
        .map_err(|_| format!("Invalid month in '{}'", s))?;
    let day: u32 = s[3..5]
        .parse()
        .map_err(|_| format!("Invalid day in '{}'", s))?;
    if !(1..=12).contains(&month) {
        return Err(format!("Month out of range in '{}'", s));
    }
    if !(1..=31).contains(&day) {
        return Err(format!("Day out of range in '{}'", s));
    }

    let after_day = &s[5..];
    // Optional /YYYY or -YYYY (same separator as MM/DD or MM-DD)
    let (year, after_year) = if after_day.as_bytes().first().copied() == Some(sep) {
        let rest = &after_day[1..];
        if rest.len() >= 4 && rest[..4].bytes().all(|c| c.is_ascii_digit()) {
            let y: u32 = rest[..4]
                .parse()
                .map_err(|_| format!("Invalid year in '{}'", s))?;
            (y, &rest[4..])
        } else {
            return Err(format!("Invalid year after separator in '{}'", s));
        }
    } else {
        (0u32, after_day)
    };

    let after = after_year.trim_start();
    let (h, m, sec, gran) = if after.is_empty() {
        (0u32, 0u32, 0u32, Granularity::Day)
    } else if let Some((secs, g)) = try_parse_time_only(after) {
        (secs / 3600, (secs % 3600) / 60, secs % 60, g)
    } else {
        return Err(format!("Invalid time in '{}'", s));
    };

    Ok((
        DateBound {
            time_val: None,
            datetime_val: Some(write_canonical(year, month, day, h, m, sec, 0)),
        },
        ComparisonMode::FullDatetime,
        gran,
    ))
}

fn parse_iso_bound(s: &str) -> Result<(DateBound, ComparisonMode, Granularity), String> {
    let date_part = &s[..10]; // YYYY-MM-DD
    let year: u32 = date_part[..4]
        .parse()
        .map_err(|_| format!("Invalid year in '{}'", s))?;
    let month: u32 = date_part[5..7]
        .parse()
        .map_err(|_| format!("Invalid month in '{}'", s))?;
    let day: u32 = date_part[8..10]
        .parse()
        .map_err(|_| format!("Invalid day in '{}'", s))?;
    if !(1..=12).contains(&month) {
        return Err(format!("Month out of range in '{}'", s));
    }
    if !(1..=31).contains(&day) {
        return Err(format!("Day out of range in '{}'", s));
    }

    let after_date = &s[10..];
    let (h, m, sec, gran) = if after_date.is_empty() {
        (0u32, 0u32, 0u32, Granularity::Day)
    } else {
        let sep = after_date.as_bytes()[0];
        if sep == b'T' || sep == b' ' {
            let time_str = &after_date[1..];
            if let Some((secs, g)) = try_parse_time_only(time_str) {
                (secs / 3600, (secs % 3600) / 60, secs % 60, g)
            } else {
                return Err(format!("Invalid time in '{}'", s));
            }
        } else {
            return Err(format!("Unexpected character after date in '{}'", s));
        }
    };

    Ok((
        DateBound {
            time_val: None,
            datetime_val: Some(write_canonical(year, month, day, h, m, sec, 0)),
        },
        ComparisonMode::FullDatetime,
        gran,
    ))
}

/// Find the byte position of `..` in `s`, skipping any surrounding whitespace
/// so that both `"09:00..10:00"` and `"09:00 .. 10:00"` are detected.
fn find_range_separator(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'.' && bytes[i + 1] == b'.' {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Expand a bound to the **end** of the period implied by its granularity:
/// - `Day`    → 23:59:59.999 of that day
/// - `Minute` → :59.999 of that minute
/// - `Second` → unchanged
fn expand_upper_bound(
    bound: DateBound,
    mode: ComparisonMode,
    granularity: Granularity,
) -> DateBound {
    match mode {
        ComparisonMode::TimeOnly => {
            let t = bound.time_val.unwrap();
            let upper_t = match granularity {
                Granularity::Minute => t + 59,
                Granularity::Second | Granularity::Day => t,
            };
            DateBound {
                time_val: Some(upper_t),
                datetime_val: None,
            }
        }
        ComparisonMode::FullDatetime => {
            let src = bound.datetime_val.as_ref().unwrap();
            let mut buf = *src;
            match granularity {
                Granularity::Day => {
                    buf[10..23].copy_from_slice(b" 23:59:59.999");
                }
                Granularity::Minute => {
                    buf[16..23].copy_from_slice(b":59.999");
                }
                Granularity::Second => {}
            }
            DateBound {
                time_val: None,
                datetime_val: Some(buf),
            }
        }
    }
}

/// Expand a single bound into an inclusive `(lower, upper)` pair based on
/// granularity so that "equals" semantics match the full period implied by
/// the input (e.g. a day-only bound covers 00:00:00 – 23:59:59.999).
fn make_equals_range(
    bound: &DateBound,
    mode: ComparisonMode,
    granularity: Granularity,
) -> (DateBound, DateBound) {
    let lower = bound.clone();
    let upper = expand_upper_bound(bound.clone(), mode, granularity);
    (lower, upper)
}

/// Parse the expression that follows `@date:`.
///
/// Syntax:
///   `<bound> .. <bound>`               — inclusive range (spaces around `..` are optional)
///   `> <bound>` / `>= <bound>`         — after
///   `< <bound>` / `<= <bound>`         — before
///   `<bound>`                          — equals (expands to an inclusive range)
pub fn parse_date_filter(input: &str) -> Result<DateFilter, String> {
    let s = input.trim();
    if s.is_empty() {
        return Err("Empty date filter expression".to_string());
    }

    // Range: contains ".." (spaces around ".." are optional).
    if let Some(dot_pos) = find_range_separator(s) {
        let left = s[..dot_pos].trim();
        let right = s[dot_pos + 2..].trim();
        let (lower, l_mode, _) = parse_bound(left)?;
        let (upper, u_mode, u_gran) = parse_bound(right)?;
        if l_mode != u_mode {
            return Err(
                "Both sides of a range must use the same format (both time-only or both date)"
                    .to_string(),
            );
        }
        // Validate lower <= upper before expanding the upper bound.
        match l_mode {
            ComparisonMode::TimeOnly => {
                if lower.time_val.unwrap() > upper.time_val.unwrap() {
                    return Err(
                        "Range lower bound is greater than upper bound (midnight wraparound is not supported)"
                            .to_string(),
                    );
                }
            }
            ComparisonMode::FullDatetime => {
                if lower.datetime_val.as_ref().unwrap() > upper.datetime_val.as_ref().unwrap() {
                    return Err("Range lower bound is greater than upper bound".to_string());
                }
            }
        }
        // Expand the upper bound to end-of-period so that e.g. `03-21..03-25`
        // includes all of Mar 25, not just 00:00:00.
        let upper = expand_upper_bound(upper, u_mode, u_gran);
        return Ok(DateFilter::Range {
            mode: l_mode,
            lower,
            upper,
        });
    }

    // Comparison: starts with >=, >, <=, <
    if let Some(rest) = s.strip_prefix(">=") {
        let (bound, mode, _) = parse_bound(rest)?;
        return Ok(DateFilter::Comparison {
            mode,
            op: ComparisonOp::Ge,
            bound,
        });
    }
    if let Some(rest) = s.strip_prefix('>') {
        let (bound, mode, _) = parse_bound(rest)?;
        return Ok(DateFilter::Comparison {
            mode,
            op: ComparisonOp::Gt,
            bound,
        });
    }
    if let Some(rest) = s.strip_prefix("<=") {
        let (bound, mode, _) = parse_bound(rest)?;
        return Ok(DateFilter::Comparison {
            mode,
            op: ComparisonOp::Le,
            bound,
        });
    }
    if let Some(rest) = s.strip_prefix('<') {
        let (bound, mode, _) = parse_bound(rest)?;
        return Ok(DateFilter::Comparison {
            mode,
            op: ComparisonOp::Lt,
            bound,
        });
    }

    // No operator: treat as equals (expand to an inclusive range).
    let (bound, mode, gran) = parse_bound(s)?;
    let (lower, upper) = make_equals_range(&bound, mode, gran);
    Ok(DateFilter::Range { mode, lower, upper })
}

/// Convert an optional `SystemTime` to a `time::Date` (UTC).
pub(crate) fn system_time_to_date(st: Option<std::time::SystemTime>) -> Option<time::Date> {
    let secs = st?
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .ok()?
        .as_secs() as i64;
    time::OffsetDateTime::from_unix_timestamp(secs)
        .ok()
        .map(|dt| dt.date())
}

/// Extract the BSD month number (1–12) from a raw timestamp string.
/// Returns `None` if the timestamp is not BSD-format.
pub(crate) fn bsd_month_from_timestamp(ts: &str) -> Option<u32> {
    let s = ts.trim_ascii();
    if s.len() >= 3 && bsd_month_number(&s[..3]).is_some() {
        let n = normalize_bsd_ts(s)?;
        let c = &n.canonical;
        Some((c[5] - b'0') as u32 * 10 + (c[6] - b'0') as u32)
    } else {
        None
    }
}

/// Normalize a raw timestamp string (as produced by a log format parser's
/// `DisplayParts.timestamp`) into a canonical form for comparison.
///
/// If `year_override` is `Some(y)`, it is used in place of the current year
/// for BSD-format timestamps (those stored with year `0000`).
/// Pass `None` to fall back to the current year (streaming/stdin behaviour).
///
/// Returns `None` for dmesg-style boot-relative timestamps or unparseable input.
/// Return the raw `CanonicalTs` bytes for `ts`, filling in the year when
/// the source format omits it (BSD syslog, etc.).
pub fn timestamp_to_canonical(ts: &str, year_override: Option<i32>) -> Option<CanonicalTs> {
    normalize_log_timestamp(ts).map(|n| {
        let mut c = n.canonical;
        if &c[..5] == b"0000-" {
            fill_year(
                &mut c,
                year_override.unwrap_or_else(|| time::OffsetDateTime::now_utc().year()),
            );
        }
        c
    })
}

pub fn canonical_timestamp(ts: &str, year_override: Option<i32>) -> Option<String> {
    normalize_log_timestamp(ts).map(|n| {
        let mut c = n.canonical;
        if &c[..5] == b"0000-" {
            let year =
                year_override.unwrap_or_else(|| time::OffsetDateTime::now_utc().year()) as u32;
            c[0] = b'0' + (year / 1000) as u8;
            c[1] = b'0' + (year / 100 % 10) as u8;
            c[2] = b'0' + (year / 10 % 10) as u8;
            c[3] = b'0' + (year % 10) as u8;
        }
        std::str::from_utf8(&c).unwrap_or("").to_string()
    })
}

fn normalize_log_timestamp(ts: &str) -> Option<NormalizedTimestamp> {
    let s = ts.trim_ascii();
    if s.is_empty() {
        return None;
    }

    // Nanosecond Unix epoch: 19 all-digit chars (e.g. OTLP timeUnixNano)
    if s.len() == 19 && s.bytes().all(|b| b.is_ascii_digit()) {
        return normalize_nanos_ts(s);
    }

    // Unix epoch: 10-digit seconds, 13-digit milliseconds, 16-digit microseconds
    // (journalctl JSON __REALTIME_TIMESTAMP), or decimal seconds (journalctl short-unix)
    if s.as_bytes()
        .first()
        .map(|b| b.is_ascii_digit())
        .unwrap_or(false)
        && let Some(n) = normalize_epoch_ts(s)
    {
        return Some(n);
    }

    // Handle bracket-prefixed timestamps
    if s.starts_with('[') {
        // dmesg: [  seconds.usecs] — digits/spaces/dots only inside brackets
        if s.ends_with(']') && is_dmesg_content(&s[1..s.len() - 1]) {
            return None;
        }
        // Apache error: [Www Mmm DD HH:MM:SS... YYYY]
        return normalize_apache_error_ts(s);
    }

    // Try ISO 8601: 2024-02-22T10:15:30...
    if s.len() >= 19 && s.as_bytes()[4] == b'-' && s.as_bytes().get(10) == Some(&b'T') {
        return normalize_iso_ts(s);
    }

    // Try weekday prefix: "Mon 2024-02-22 10:15:30 UTC" (journalctl short-full)
    if s.len() >= 4 && s.as_bytes()[3] == b' ' {
        let weekday = &s[..3];
        if crate::parser::timestamp::WEEKDAYS.contains(&weekday) {
            return normalize_full_ts(s);
        }
    }

    // Try YYYY-MM-DD HH:MM:SS (datetime)
    if s.len() >= 19 && s.as_bytes()[4] == b'-' && s.as_bytes()[10] == b' ' {
        return normalize_datetime_ts(s);
    }

    // Try YYYY/MM/DD HH:MM:SS (slash datetime)
    if s.len() >= 19 && s.as_bytes()[4] == b'/' && s.as_bytes()[10] == b' ' {
        return normalize_slash_ts(s);
    }

    // Try CLF: dd/Mmm/yyyy:HH:MM:SS ±ZZZZ
    if s.len() >= 20 && s.as_bytes()[2] == b'/' && s.as_bytes()[6] == b'/' {
        return normalize_clf_ts(s);
    }

    // Try BSD: "Feb 22 10:15:30" or "Feb 22 10:15:30.123456"
    if s.len() >= 3 && bsd_month_number(&s[..3]).is_some() {
        return normalize_bsd_ts(s);
    }

    None
}

/// Returns true if the content between `[` and `]` looks like dmesg:
/// only digits, spaces, and exactly one dot (e.g. `    0.000000`).
fn is_dmesg_content(inner: &str) -> bool {
    let mut has_dot = false;
    for b in inner.as_bytes() {
        match b {
            b' ' | b'0'..=b'9' => {}
            b'.' if !has_dot => has_dot = true,
            _ => return false,
        }
    }
    has_dot
}

fn normalize_iso_ts(s: &str) -> Option<NormalizedTimestamp> {
    // "2024-02-22T10:15:30..."
    let year: u32 = s[..4].parse().ok()?;
    let month: u32 = s[5..7].parse().ok()?;
    let day: u32 = s[8..10].parse().ok()?;
    let (h, m, sec, ms) = parse_hms_frac(&s[11..])?;
    Some(NormalizedTimestamp {
        time_of_day: h * 3600 + m * 60 + sec,
        canonical: write_canonical(year, month, day, h, m, sec, ms),
    })
}

fn normalize_full_ts(s: &str) -> Option<NormalizedTimestamp> {
    // "Mon 2024-02-22 10:15:30 UTC"
    if s.len() < 23 {
        return None;
    }
    let year: u32 = s[4..8].parse().ok()?;
    let month: u32 = s[9..11].parse().ok()?;
    let day: u32 = s[12..14].parse().ok()?;
    let (h, m, sec, ms) = parse_hms_frac(&s[15..])?;
    Some(NormalizedTimestamp {
        time_of_day: h * 3600 + m * 60 + sec,
        canonical: write_canonical(year, month, day, h, m, sec, ms),
    })
}

fn normalize_datetime_ts(s: &str) -> Option<NormalizedTimestamp> {
    // "2024-01-15 10:30:00[.mmm]"
    let year: u32 = s[..4].parse().ok()?;
    let month: u32 = s[5..7].parse().ok()?;
    let day: u32 = s[8..10].parse().ok()?;
    let (h, m, sec, ms) = parse_hms_frac(&s[11..])?;
    Some(NormalizedTimestamp {
        time_of_day: h * 3600 + m * 60 + sec,
        canonical: write_canonical(year, month, day, h, m, sec, ms),
    })
}

fn normalize_slash_ts(s: &str) -> Option<NormalizedTimestamp> {
    // "2024/01/15 10:30:00"
    let year: u32 = s[..4].parse().ok()?;
    let month: u32 = s[5..7].parse().ok()?;
    let day: u32 = s[8..10].parse().ok()?;
    let (h, m, sec, ms) = parse_hms_frac(&s[11..])?;
    Some(NormalizedTimestamp {
        time_of_day: h * 3600 + m * 60 + sec,
        canonical: write_canonical(year, month, day, h, m, sec, ms),
    })
}

fn normalize_clf_ts(s: &str) -> Option<NormalizedTimestamp> {
    // "10/Oct/2000:13:55:36 -0700"
    if s.len() < 20 {
        return None;
    }
    let b = s.as_bytes();
    if !b[0].is_ascii_digit() || !b[1].is_ascii_digit() {
        return None;
    }
    let day = (b[0] - b'0') as u32 * 10 + (b[1] - b'0') as u32;
    let month_abbr = &s[3..6];
    let month_num = bsd_month_number(month_abbr)?;
    if !b[7].is_ascii_digit()
        || !b[8].is_ascii_digit()
        || !b[9].is_ascii_digit()
        || !b[10].is_ascii_digit()
    {
        return None;
    }
    let year = (b[7] - b'0') as u32 * 1000
        + (b[8] - b'0') as u32 * 100
        + (b[9] - b'0') as u32 * 10
        + (b[10] - b'0') as u32;
    if b[11] != b':' {
        return None;
    }
    let (h, m, sec, ms) = parse_hms_frac(&s[12..])?;
    Some(NormalizedTimestamp {
        time_of_day: h * 3600 + m * 60 + sec,
        canonical: write_canonical(year, month_num, day, h, m, sec, ms),
    })
}

fn normalize_bsd_ts(s: &str) -> Option<NormalizedTimestamp> {
    // "Feb 22 10:15:30" or "Feb 22 10:15:30.123456"
    let month_num = bsd_month_number(&s[..3])?;
    let rest = s[3..].trim_start();
    let day_end = rest.find(|c: char| !c.is_ascii_digit())?;
    if day_end == 0 {
        return None;
    }
    let day: u32 = rest[..day_end].parse().ok()?;
    let after_day = rest[day_end..].trim_start();
    if after_day.is_empty() {
        return None;
    }
    let (h, m, sec, ms) = parse_hms_frac(after_day)?;
    Some(NormalizedTimestamp {
        time_of_day: h * 3600 + m * 60 + sec,
        canonical: write_canonical(0, month_num, day, h, m, sec, ms),
    })
}

fn nanos_to_normalized(nanos: i128) -> Option<NormalizedTimestamp> {
    let dt = time::OffsetDateTime::from_unix_timestamp_nanos(nanos).ok()?;
    let ms = ((nanos.unsigned_abs() % 1_000_000_000) / 1_000_000) as u16;
    let year = dt.year() as u32;
    let month = dt.month() as u32;
    let day = dt.day() as u32;
    let h = dt.hour() as u32;
    let m = dt.minute() as u32;
    let sec = dt.second() as u32;
    Some(NormalizedTimestamp {
        time_of_day: h * 3600 + m * 60 + sec,
        canonical: write_canonical(year, month, day, h, m, sec, ms),
    })
}

fn normalize_nanos_ts(s: &str) -> Option<NormalizedTimestamp> {
    let nanos: i128 = s.parse().ok()?;
    nanos_to_normalized(nanos)
}

/// Handle Unix epoch timestamps in integer and decimal forms:
/// - 10 digits: seconds          (e.g. `1436735381`)
/// - 13 digits: milliseconds     (e.g. `1436735381000`)
/// - 16 digits: microseconds     (e.g. `1699999999000000`, journalctl JSON)
/// - decimal:   seconds.fraction (e.g. `1436735381.000000`, journalctl short-unix)
fn normalize_epoch_ts(s: &str) -> Option<NormalizedTimestamp> {
    if s.bytes().all(|b| b.is_ascii_digit()) {
        let nanos = match s.len() {
            10 => s.parse::<i128>().ok()? * 1_000_000_000,
            13 => s.parse::<i128>().ok()? * 1_000_000,
            16 => s.parse::<i128>().ok()? * 1_000,
            _ => return None,
        };
        return nanos_to_normalized(nanos);
    }
    // Decimal seconds: integer part must be at least 9 digits (epoch ≥ ~2001)
    if let Some(dot) = s.find('.') {
        let int_part = &s[..dot];
        let frac_part = &s[dot + 1..];
        if int_part.len() >= 9
            && int_part.bytes().all(|b| b.is_ascii_digit())
            && !frac_part.is_empty()
            && frac_part.bytes().all(|b| b.is_ascii_digit())
        {
            let secs: i64 = int_part.parse().ok()?;
            let frac_len = frac_part.len().min(9);
            let frac_digits: i128 = frac_part[..frac_len].parse().ok()?;
            let frac_nanos = frac_digits * 10i128.pow((9 - frac_len) as u32);
            let nanos = secs as i128 * 1_000_000_000 + frac_nanos;
            return nanos_to_normalized(nanos);
        }
    }
    None
}

fn normalize_apache_error_ts(s: &str) -> Option<NormalizedTimestamp> {
    // "[Mon Jan 15 10:30:00.123456 2024]" or "[Fri Dec 31 23:59:59 2024]"
    if !s.starts_with('[') || !s.ends_with(']') {
        return None;
    }
    let inner = &s[1..s.len() - 1];
    // Skip weekday
    if inner.len() < 4 || inner.as_bytes()[3] != b' ' {
        return None;
    }
    let weekday = &inner[..3];
    if !crate::parser::timestamp::WEEKDAYS.contains(&weekday) {
        return None;
    }
    let after_weekday = &inner[4..];
    // Month
    if after_weekday.len() < 3 {
        return None;
    }
    let month_num = bsd_month_number(&after_weekday[..3])?;
    let rest = after_weekday[3..].trim_start();
    // Day
    let day_end = rest.find(|c: char| !c.is_ascii_digit())?;
    let day: u32 = rest[..day_end].parse().ok()?;
    let after_day = rest[day_end..].trim_start();
    // Time (HH:MM:SS or HH:MM:SS.usecs)
    let (h, m, sec, ms) = parse_hms_frac(after_day)?;
    // Skip past time + optional fractional part to find the year
    let mut pos = 8; // HH:MM:SS
    if pos < after_day.len() && after_day.as_bytes()[pos] == b'.' {
        pos += 1;
        while pos < after_day.len() && after_day.as_bytes()[pos].is_ascii_digit() {
            pos += 1;
        }
    }
    let after_time = after_day[pos..].trim_start();
    let year: u32 = after_time.trim().parse().ok()?;
    Some(NormalizedTimestamp {
        time_of_day: h * 3600 + m * 60 + sec,
        canonical: write_canonical(year, month_num, day, h, m, sec, ms),
    })
}

/// Parse `HH:MM:SS[.frac][,frac]` and return `(h, m, s, ms)` where `ms` is
/// milliseconds (0–999) as an integer.
fn parse_hms_frac(s: &str) -> Option<(u32, u32, u32, u16)> {
    if s.len() < 8 {
        return None;
    }
    let b = s.as_bytes();
    if !b[0].is_ascii_digit()
        || !b[1].is_ascii_digit()
        || b[2] != b':'
        || !b[3].is_ascii_digit()
        || !b[4].is_ascii_digit()
        || b[5] != b':'
        || !b[6].is_ascii_digit()
        || !b[7].is_ascii_digit()
    {
        return None;
    }
    let h = (b[0] - b'0') as u32 * 10 + (b[1] - b'0') as u32;
    let m = (b[3] - b'0') as u32 * 10 + (b[4] - b'0') as u32;
    let sec = (b[6] - b'0') as u32 * 10 + (b[7] - b'0') as u32;

    let ms = if s.len() > 8 && (b[8] == b'.' || b[8] == b',') {
        let start = 9;
        let end = s[start..]
            .find(|c: char| !c.is_ascii_digit())
            .map(|p| p + start)
            .unwrap_or(s.len());
        let frac = &b[start..end];
        let d0 = frac.first().map(|x| (x - b'0') as u16).unwrap_or(0);
        let d1 = frac.get(1).map(|x| (x - b'0') as u16).unwrap_or(0);
        let d2 = frac.get(2).map(|x| (x - b'0') as u16).unwrap_or(0);
        d0 * 100 + d1 * 10 + d2
    } else {
        0
    };
    Some((h, m, sec, ms))
}

/// Fill the year digits (bytes 0–3) of a `CanonicalTs` in place.
fn fill_year(c: &mut CanonicalTs, year: i32) {
    let y = year as u32;
    c[0] = b'0' + (y / 1000) as u8;
    c[1] = b'0' + (y / 100 % 10) as u8;
    c[2] = b'0' + (y / 10 % 10) as u8;
    c[3] = b'0' + (y % 10) as u8;
}

impl DateFilter {
    /// Check if a raw timestamp string (from a parser's `DisplayParts.timestamp`)
    /// passes this filter.
    ///
    /// `year_override` provides the correct year for BSD-format timestamps
    /// (which are stored with year `0000`). Pass `None` to fall back to
    /// stripping the year prefix for BSD-vs-BSD comparisons (existing behaviour).
    pub fn matches(&self, timestamp: &str, year_override: Option<i32>) -> bool {
        let norm = match normalize_log_timestamp(timestamp) {
            Some(n) => n,
            None => return true,
        };
        match self {
            DateFilter::Range { mode, lower, upper } => match mode {
                ComparisonMode::TimeOnly => {
                    let t = norm.time_of_day;
                    t >= lower.time_val.unwrap() && t <= upper.time_val.unwrap()
                }
                ComparisonMode::FullDatetime => {
                    let mut c = norm.canonical;
                    let lo = lower.datetime_val.as_ref().unwrap();
                    let hi = upper.datetime_val.as_ref().unwrap();
                    if &lo[..5] == b"0000-" {
                        // BSD bound: strip year prefix for comparison (year-less filter).
                        let (c_cmp, lo_cmp, hi_cmp): (&[u8], &[u8], &[u8]) =
                            (&c[5..], &lo[5..], &hi[5..]);
                        c_cmp >= lo_cmp && c_cmp <= hi_cmp
                    } else {
                        // Full-year bound: fill in the correct year for BSD timestamps.
                        if &c[..5] == b"0000-" {
                            let y = year_override
                                .unwrap_or_else(|| time::OffsetDateTime::now_utc().year());
                            fill_year(&mut c, y);
                        }
                        c.as_ref() >= lo.as_ref() && c.as_ref() <= hi.as_ref()
                    }
                }
            },
            DateFilter::Comparison { mode, op, bound } => match mode {
                ComparisonMode::TimeOnly => {
                    let t = norm.time_of_day;
                    let b = bound.time_val.unwrap();
                    match op {
                        ComparisonOp::Gt => t > b,
                        ComparisonOp::Ge => t >= b,
                        ComparisonOp::Lt => t < b,
                        ComparisonOp::Le => t <= b,
                    }
                }
                ComparisonMode::FullDatetime => {
                    let mut c = norm.canonical;
                    let b = bound.datetime_val.as_ref().unwrap();
                    if &b[..5] == b"0000-" {
                        // BSD-format bound: strip year prefix (year-less filter).
                        let (c_cmp, b_cmp): (&[u8], &[u8]) = (&c[5..], &b[5..]);
                        match op {
                            ComparisonOp::Gt => c_cmp > b_cmp,
                            ComparisonOp::Ge => c_cmp >= b_cmp,
                            ComparisonOp::Lt => c_cmp < b_cmp,
                            ComparisonOp::Le => c_cmp <= b_cmp,
                        }
                    } else {
                        // Full-year bound: fill in the correct year for BSD timestamps.
                        if &c[..5] == b"0000-" {
                            let y = year_override
                                .unwrap_or_else(|| time::OffsetDateTime::now_utc().year());
                            fill_year(&mut c, y);
                        }
                        match op {
                            ComparisonOp::Gt => c.as_ref() > b.as_ref(),
                            ComparisonOp::Ge => c.as_ref() >= b.as_ref(),
                            ComparisonOp::Lt => c.as_ref() < b.as_ref(),
                            ComparisonOp::Le => c.as_ref() <= b.as_ref(),
                        }
                    }
                }
            },
        }
    }
}

/// Collect all enabled `@date:` filters from the filter list, parsed.
/// Filters that fail to parse are silently skipped.
pub fn extract_date_filters(filter_defs: &[FilterDef]) -> Vec<DateFilter> {
    filter_defs
        .iter()
        .filter(|f| f.enabled && f.pattern.starts_with(DATE_PREFIX))
        .filter_map(|f| {
            let expr = &f.pattern[DATE_PREFIX.len()..];
            parse_date_filter(expr).ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf_as_str(b: &CanonicalTs) -> &str {
        std::str::from_utf8(b).unwrap()
    }

    #[test]
    fn test_parse_bound_time_only_hms() {
        let (b, mode, gran) = parse_bound("01:30:45").unwrap();
        assert_eq!(mode, ComparisonMode::TimeOnly);
        assert_eq!(gran, Granularity::Second);
        assert_eq!(b.time_val, Some(3600 + 30 * 60 + 45));
    }

    #[test]
    fn test_parse_bound_time_only_hm() {
        let (b, mode, gran) = parse_bound("13:00").unwrap();
        assert_eq!(mode, ComparisonMode::TimeOnly);
        assert_eq!(gran, Granularity::Minute);
        assert_eq!(b.time_val, Some(13 * 3600));
    }

    #[test]
    fn test_parse_bound_bsd_date_time() {
        let (b, mode, gran) = parse_bound("Feb 21 01:00:00").unwrap();
        assert_eq!(mode, ComparisonMode::FullDatetime);
        assert_eq!(gran, Granularity::Second);
        assert_eq!(
            b.datetime_val.as_ref().map(buf_as_str),
            Some("0000-02-21 01:00:00.000")
        );
    }

    #[test]
    fn test_parse_bound_bsd_date_only() {
        let (b, mode, gran) = parse_bound("Feb 21").unwrap();
        assert_eq!(mode, ComparisonMode::FullDatetime);
        assert_eq!(gran, Granularity::Day);
        assert_eq!(
            b.datetime_val.as_ref().map(buf_as_str),
            Some("0000-02-21 00:00:00.000")
        );
    }

    #[test]
    fn test_parse_bound_bsd_slash_separator() {
        let (b, mode, gran) = parse_bound("Feb/21").unwrap();
        assert_eq!(mode, ComparisonMode::FullDatetime);
        assert_eq!(gran, Granularity::Day);
        assert_eq!(
            b.datetime_val.as_ref().map(buf_as_str),
            Some("0000-02-21 00:00:00.000")
        );
    }

    #[test]
    fn test_parse_bound_bsd_slash_with_time() {
        let (b, mode, gran) = parse_bound("Feb/21 09:00:00").unwrap();
        assert_eq!(mode, ComparisonMode::FullDatetime);
        assert_eq!(gran, Granularity::Second);
        assert_eq!(
            b.datetime_val.as_ref().map(buf_as_str),
            Some("0000-02-21 09:00:00.000")
        );
    }

    #[test]
    fn test_parse_bound_bsd_slash_with_hm_time() {
        let (b, mode, gran) = parse_bound("Feb/21 09:00").unwrap();
        assert_eq!(mode, ComparisonMode::FullDatetime);
        assert_eq!(gran, Granularity::Minute);
        assert_eq!(
            b.datetime_val.as_ref().map(buf_as_str),
            Some("0000-02-21 09:00:00.000")
        );
    }

    #[test]
    fn test_parse_bound_numeric_slash_date_only() {
        let (b, mode, gran) = parse_bound("02/21").unwrap();
        assert_eq!(mode, ComparisonMode::FullDatetime);
        assert_eq!(gran, Granularity::Day);
        assert_eq!(
            b.datetime_val.as_ref().map(buf_as_str),
            Some("0000-02-21 00:00:00.000")
        );
    }

    #[test]
    fn test_parse_bound_numeric_slash_date_with_year() {
        let (b, mode, gran) = parse_bound("02/21/2024").unwrap();
        assert_eq!(mode, ComparisonMode::FullDatetime);
        assert_eq!(gran, Granularity::Day);
        assert_eq!(
            b.datetime_val.as_ref().map(buf_as_str),
            Some("2024-02-21 00:00:00.000")
        );
    }

    #[test]
    fn test_parse_bound_numeric_slash_with_time() {
        let (b, mode, gran) = parse_bound("02/21 09:00:30").unwrap();
        assert_eq!(mode, ComparisonMode::FullDatetime);
        assert_eq!(gran, Granularity::Second);
        assert_eq!(
            b.datetime_val.as_ref().map(buf_as_str),
            Some("0000-02-21 09:00:30.000")
        );
    }

    #[test]
    fn test_parse_bound_numeric_slash_with_hm_time() {
        let (b, mode, gran) = parse_bound("02/21 09:00").unwrap();
        assert_eq!(mode, ComparisonMode::FullDatetime);
        assert_eq!(gran, Granularity::Minute);
        assert_eq!(
            b.datetime_val.as_ref().map(buf_as_str),
            Some("0000-02-21 09:00:00.000")
        );
    }

    #[test]
    fn test_parse_bound_numeric_slash_year_with_time() {
        let (b, mode, gran) = parse_bound("02/21/2024 09:00").unwrap();
        assert_eq!(mode, ComparisonMode::FullDatetime);
        assert_eq!(gran, Granularity::Minute);
        assert_eq!(
            b.datetime_val.as_ref().map(buf_as_str),
            Some("2024-02-21 09:00:00.000")
        );
    }

    #[test]
    fn test_parse_bound_numeric_dash_date_only() {
        let (b, mode, gran) = parse_bound("02-21").unwrap();
        assert_eq!(mode, ComparisonMode::FullDatetime);
        assert_eq!(gran, Granularity::Day);
        assert_eq!(
            b.datetime_val.as_ref().map(buf_as_str),
            Some("0000-02-21 00:00:00.000")
        );
    }

    #[test]
    fn test_parse_bound_numeric_dash_date_with_year() {
        let (b, mode, gran) = parse_bound("02-21-2024").unwrap();
        assert_eq!(mode, ComparisonMode::FullDatetime);
        assert_eq!(gran, Granularity::Day);
        assert_eq!(
            b.datetime_val.as_ref().map(buf_as_str),
            Some("2024-02-21 00:00:00.000")
        );
    }

    #[test]
    fn test_parse_bound_numeric_dash_with_time() {
        let (b, mode, gran) = parse_bound("02-21 09:00").unwrap();
        assert_eq!(mode, ComparisonMode::FullDatetime);
        assert_eq!(gran, Granularity::Minute);
        assert_eq!(
            b.datetime_val.as_ref().map(buf_as_str),
            Some("0000-02-21 09:00:00.000")
        );
    }

    #[test]
    fn test_parse_bound_numeric_slash_invalid_month() {
        assert!(parse_bound("13/01").is_err());
    }

    #[test]
    fn test_parse_bound_iso_date_only() {
        let (b, mode, gran) = parse_bound("2024-02-22").unwrap();
        assert_eq!(mode, ComparisonMode::FullDatetime);
        assert_eq!(gran, Granularity::Day);
        assert_eq!(
            b.datetime_val.as_ref().map(buf_as_str),
            Some("2024-02-22 00:00:00.000")
        );
    }

    #[test]
    fn test_parse_bound_iso_datetime() {
        let (b, mode, gran) = parse_bound("2024-02-22T10:15:30").unwrap();
        assert_eq!(mode, ComparisonMode::FullDatetime);
        assert_eq!(gran, Granularity::Second);
        assert_eq!(
            b.datetime_val.as_ref().map(buf_as_str),
            Some("2024-02-22 10:15:30.000")
        );
    }

    #[test]
    fn test_parse_bound_iso_datetime_space() {
        let (b, mode, gran) = parse_bound("2024-02-22 10:15:30").unwrap();
        assert_eq!(mode, ComparisonMode::FullDatetime);
        assert_eq!(gran, Granularity::Second);
        assert_eq!(
            b.datetime_val.as_ref().map(buf_as_str),
            Some("2024-02-22 10:15:30.000")
        );
    }

    #[test]
    fn test_parse_bound_iso_datetime_hm() {
        let (b, mode, gran) = parse_bound("2024-02-22 10:15").unwrap();
        assert_eq!(mode, ComparisonMode::FullDatetime);
        assert_eq!(gran, Granularity::Minute);
        assert_eq!(
            b.datetime_val.as_ref().map(buf_as_str),
            Some("2024-02-22 10:15:00.000")
        );
    }

    #[test]
    fn test_parse_bound_empty_error() {
        assert!(parse_bound("").is_err());
    }

    #[test]
    fn test_parse_bound_invalid() {
        assert!(parse_bound("not a date").is_err());
    }

    #[test]
    fn test_parse_bound_invalid_time_values() {
        assert!(parse_bound("25:00:00").is_err());
    }

    // ── parse_date_filter ─────────────────────────────────────────────

    #[test]
    fn test_parse_time_range() {
        let df = parse_date_filter("01:00:00 .. 02:00:00").unwrap();
        assert!(matches!(df, DateFilter::Range { .. }));
    }

    #[test]
    fn test_parse_hm_range() {
        let df = parse_date_filter("01:00 .. 02:00").unwrap();
        assert!(matches!(df, DateFilter::Range { .. }));
    }

    #[test]
    fn test_parse_range_no_spaces_around_dots() {
        let df = parse_date_filter("09:00..10:00").unwrap();
        assert!(matches!(df, DateFilter::Range { .. }));
    }

    #[test]
    fn test_parse_range_no_spaces_iso() {
        let df = parse_date_filter("2024-02-21..2024-02-22").unwrap();
        assert!(matches!(df, DateFilter::Range { .. }));
    }

    #[test]
    fn test_parse_range_numeric_dash_no_spaces() {
        let df = parse_date_filter("03-21..03-25").unwrap();
        assert!(df.matches("Mar 21 12:00:00", None));
        assert!(df.matches("Mar 25 00:00:00", None));
        assert!(df.matches("Mar 25 23:59:59", None)); // whole upper day included
        assert!(!df.matches("Mar 20 23:59:59", None));
        assert!(!df.matches("Mar 26 00:00:00", None));
    }

    #[test]
    fn test_parse_range_numeric_slash_no_spaces() {
        let df = parse_date_filter("03/21..03/25").unwrap();
        assert!(df.matches("Mar 21 12:00:00", None));
        assert!(df.matches("Mar 25 00:00:00", None));
        assert!(df.matches("Mar 25 23:59:59", None)); // whole upper day included
        assert!(!df.matches("Mar 20 23:59:59", None));
        assert!(!df.matches("Mar 26 00:00:00", None));
    }

    #[test]
    fn test_parse_gt_comparison() {
        let df = parse_date_filter("> Feb 21 01:00:00").unwrap();
        assert!(matches!(
            df,
            DateFilter::Comparison {
                op: ComparisonOp::Gt,
                ..
            }
        ));
    }

    #[test]
    fn test_parse_ge_comparison() {
        let df = parse_date_filter(">= 2024-02-22").unwrap();
        assert!(matches!(
            df,
            DateFilter::Comparison {
                op: ComparisonOp::Ge,
                ..
            }
        ));
    }

    #[test]
    fn test_parse_lt_comparison() {
        let df = parse_date_filter("< 2024-02-22T10:15:30").unwrap();
        assert!(matches!(
            df,
            DateFilter::Comparison {
                op: ComparisonOp::Lt,
                ..
            }
        ));
    }

    #[test]
    fn test_parse_le_comparison() {
        let df = parse_date_filter("<= Feb 22").unwrap();
        assert!(matches!(
            df,
            DateFilter::Comparison {
                op: ComparisonOp::Le,
                ..
            }
        ));
    }

    #[test]
    fn test_parse_bsd_range() {
        let df = parse_date_filter("Feb 21 .. Feb 22").unwrap();
        assert!(matches!(df, DateFilter::Range { .. }));
    }

    #[test]
    fn test_parse_iso_range() {
        let df = parse_date_filter("2024-02-21 .. 2024-02-22").unwrap();
        assert!(matches!(df, DateFilter::Range { .. }));
    }

    #[test]
    fn test_parse_empty_error() {
        assert!(parse_date_filter("").is_err());
    }

    // No operator → equals (range), no longer an error.
    #[test]
    fn test_parse_no_operator_becomes_equals() {
        assert!(parse_date_filter("01:00:00").is_ok());
    }

    #[test]
    fn test_parse_mixed_mode_error() {
        // Time-only on left, full date on right
        assert!(parse_date_filter("01:00:00 .. 2024-02-22").is_err());
    }

    #[test]
    fn test_parse_inverted_range_error() {
        assert!(parse_date_filter("02:00:00 .. 01:00:00").is_err());
    }

    // ── equals expansion ──────────────────────────────────────────────

    #[test]
    fn test_equals_time_hms_matches_exact_second() {
        let df = parse_date_filter("09:00:30").unwrap();
        assert!(df.matches("2024-01-01T09:00:30Z", None));
        assert!(!df.matches("2024-01-01T09:00:31Z", None));
        assert!(!df.matches("2024-01-01T09:00:29Z", None));
    }

    #[test]
    fn test_equals_time_hm_matches_whole_minute() {
        let df = parse_date_filter("09:00").unwrap();
        assert!(df.matches("2024-01-01T09:00:00Z", None));
        assert!(df.matches("2024-01-01T09:00:59Z", None));
        assert!(!df.matches("2024-01-01T09:01:00Z", None));
        assert!(!df.matches("2024-01-01T08:59:59Z", None));
    }

    #[test]
    fn test_equals_bsd_date_only_matches_whole_day() {
        let df = parse_date_filter("Feb/21").unwrap();
        assert!(df.matches("Feb 21 00:00:00", None));
        assert!(df.matches("Feb 21 12:30:00", None));
        assert!(df.matches("Feb 21 23:59:59", None));
        assert!(!df.matches("Feb 20 23:59:59", None));
        assert!(!df.matches("Feb 22 00:00:00", None));
    }

    #[test]
    fn test_equals_bsd_slash_date_same_as_space() {
        let df_slash = parse_date_filter("Feb/21").unwrap();
        let df_space = parse_date_filter("Feb 21").unwrap();
        // Both should match the same timestamps
        let ts = "Feb 21 12:00:00";
        assert_eq!(df_slash.matches(ts, None), df_space.matches(ts, None));
    }

    #[test]
    fn test_equals_numeric_slash_date_matches_whole_day() {
        let df = parse_date_filter("02/21").unwrap();
        assert!(df.matches("Feb 21 00:00:00", None));
        assert!(df.matches("Feb 21 23:59:59", None));
        assert!(!df.matches("Feb 20 23:59:59", None));
        assert!(!df.matches("Feb 22 00:00:00", None));
    }

    #[test]
    fn test_equals_numeric_dash_date_matches_whole_day() {
        let df = parse_date_filter("02-21").unwrap();
        assert!(df.matches("Feb 21 00:00:00", None));
        assert!(df.matches("Feb 21 23:59:59", None));
        assert!(!df.matches("Feb 20 23:59:59", None));
        assert!(!df.matches("Feb 22 00:00:00", None));
    }

    #[test]
    fn test_equals_numeric_slash_with_year_matches_whole_day() {
        let df = parse_date_filter("02/21/2024").unwrap();
        assert!(df.matches("2024-02-21T00:00:00Z", None));
        assert!(df.matches("2024-02-21T23:59:59Z", None));
        assert!(!df.matches("2024-02-20T23:59:59Z", None));
        assert!(!df.matches("2024-02-22T00:00:00Z", None));
    }

    #[test]
    fn test_equals_iso_date_only_matches_whole_day() {
        let df = parse_date_filter("2024-02-22").unwrap();
        assert!(df.matches("2024-02-22T00:00:00Z", None));
        assert!(df.matches("2024-02-22T23:59:59Z", None));
        assert!(!df.matches("2024-02-21T23:59:59Z", None));
        assert!(!df.matches("2024-02-23T00:00:00Z", None));
    }

    #[test]
    fn test_equals_iso_datetime_hm_matches_whole_minute() {
        let df = parse_date_filter("2024-02-22 10:15").unwrap();
        assert!(df.matches("2024-02-22T10:15:00Z", None));
        assert!(df.matches("2024-02-22T10:15:59Z", None));
        assert!(!df.matches("2024-02-22T10:16:00Z", None));
        assert!(!df.matches("2024-02-22T10:14:59Z", None));
    }

    #[test]
    fn test_equals_iso_datetime_hms_matches_exact_second() {
        let df = parse_date_filter("2024-02-22 10:15:30").unwrap();
        assert!(df.matches("2024-02-22T10:15:30Z", None));
        assert!(!df.matches("2024-02-22T10:15:31Z", None));
        assert!(!df.matches("2024-02-22T10:15:29Z", None));
    }

    // ── normalize_log_timestamp ───────────────────────────────────────

    #[test]
    fn test_normalize_iso() {
        let n = normalize_log_timestamp("2024-02-22T10:15:30+0000").unwrap();
        assert_eq!(buf_as_str(&n.canonical), "2024-02-22 10:15:30.000");
        assert_eq!(n.time_of_day, 10 * 3600 + 15 * 60 + 30);
    }

    #[test]
    fn test_normalize_iso_with_frac() {
        let n = normalize_log_timestamp("2024-02-22T10:15:30.123456Z").unwrap();
        assert_eq!(buf_as_str(&n.canonical), "2024-02-22 10:15:30.123");
    }

    #[test]
    fn test_normalize_datetime() {
        let n = normalize_log_timestamp("2024-01-15 10:30:00.123").unwrap();
        assert_eq!(buf_as_str(&n.canonical), "2024-01-15 10:30:00.123");
    }

    #[test]
    fn test_normalize_datetime_comma_frac() {
        let n = normalize_log_timestamp("2024-01-15 10:30:00,456").unwrap();
        assert_eq!(buf_as_str(&n.canonical), "2024-01-15 10:30:00.456");
    }

    #[test]
    fn test_normalize_slash() {
        let n = normalize_log_timestamp("2024/01/15 10:30:00").unwrap();
        assert_eq!(buf_as_str(&n.canonical), "2024-01-15 10:30:00.000");
    }

    #[test]
    fn test_normalize_full_journalctl() {
        let n = normalize_log_timestamp("Mon 2024-02-22 10:15:30 UTC").unwrap();
        assert_eq!(buf_as_str(&n.canonical), "2024-02-22 10:15:30.000");
    }

    #[test]
    fn test_normalize_bsd() {
        let n = normalize_log_timestamp("Feb 22 10:15:30").unwrap();
        assert_eq!(buf_as_str(&n.canonical), "0000-02-22 10:15:30.000");
    }

    #[test]
    fn test_normalize_bsd_precise() {
        let n = normalize_log_timestamp("Feb 22 10:15:30.123456").unwrap();
        assert_eq!(buf_as_str(&n.canonical), "0000-02-22 10:15:30.123");
    }

    #[test]
    fn test_normalize_clf() {
        let n = normalize_log_timestamp("10/Oct/2000:13:55:36 -0700").unwrap();
        assert_eq!(buf_as_str(&n.canonical), "2000-10-10 13:55:36.000");
    }

    #[test]
    fn test_normalize_apache_error() {
        let n = normalize_log_timestamp("[Mon Jan 15 10:30:00.123456 2024]").unwrap();
        assert_eq!(buf_as_str(&n.canonical), "2024-01-15 10:30:00.123");
    }

    #[test]
    fn test_normalize_apache_error_no_frac() {
        let n = normalize_log_timestamp("[Fri Dec 31 23:59:59 2024]").unwrap();
        assert_eq!(buf_as_str(&n.canonical), "2024-12-31 23:59:59.000");
    }

    #[test]
    fn test_normalize_dmesg_returns_none() {
        assert!(normalize_log_timestamp("[    0.000000]").is_none());
        assert!(normalize_log_timestamp("[12345.678901]").is_none());
    }

    #[test]
    fn test_normalize_empty_returns_none() {
        assert!(normalize_log_timestamp("").is_none());
    }

    #[test]
    fn test_normalize_garbage_returns_none() {
        assert!(normalize_log_timestamp("not a timestamp").is_none());
    }

    #[test]
    fn test_normalize_nanos_ts() {
        let n = normalize_log_timestamp("1700046010234000000").unwrap();
        assert_eq!(buf_as_str(&n.canonical), "2023-11-15 11:00:10.234");
    }

    #[test]
    fn test_normalize_nanos_ts_zero_millis() {
        let n = normalize_log_timestamp("1700046010000000000").unwrap();
        assert_eq!(buf_as_str(&n.canonical), "2023-11-15 11:00:10.000");
    }

    #[test]
    fn test_normalize_nanos_not_19_digits() {
        // 18 digits → not treated as nano epoch
        assert!(normalize_log_timestamp("170004601023400000").is_none());
    }

    // ── Unix epoch (seconds / milliseconds / microseconds / decimal) ──

    #[test]
    fn test_normalize_epoch_micros_journalctl_json() {
        // journalctl JSON __REALTIME_TIMESTAMP (16 digits = microseconds)
        let n = normalize_log_timestamp("1700046010234000").unwrap();
        assert_eq!(buf_as_str(&n.canonical), "2023-11-15 11:00:10.234");
    }

    #[test]
    fn test_normalize_epoch_millis() {
        // 13-digit millisecond epoch (Pino, Bunyan, etc.)
        let n = normalize_log_timestamp("1700046010234").unwrap();
        assert_eq!(buf_as_str(&n.canonical), "2023-11-15 11:00:10.234");
    }

    #[test]
    fn test_normalize_epoch_secs() {
        // 10-digit second epoch
        let n = normalize_log_timestamp("1700046010").unwrap();
        assert_eq!(buf_as_str(&n.canonical), "2023-11-15 11:00:10.000");
    }

    #[test]
    fn test_normalize_epoch_decimal_secs() {
        // Decimal seconds (journalctl short-unix, syslog unix)
        let n = normalize_log_timestamp("1700046010.234000").unwrap();
        assert_eq!(buf_as_str(&n.canonical), "2023-11-15 11:00:10.234");
    }

    #[test]
    fn test_normalize_epoch_decimal_short_frac() {
        // Fewer fractional digits
        let n = normalize_log_timestamp("1700046010.5").unwrap();
        assert_eq!(buf_as_str(&n.canonical), "2023-11-15 11:00:10.500");
    }

    #[test]
    fn test_normalize_epoch_decimal_rejects_short_integer() {
        // Integer part < 9 digits → not an epoch
        assert!(normalize_log_timestamp("12345678.000000").is_none());
    }

    #[test]
    fn test_canonical_timestamp_nanos() {
        let result = canonical_timestamp("1700046010234000000", None);
        assert_eq!(result, Some("2023-11-15 11:00:10.234".to_string()));
    }

    #[test]
    fn test_canonical_timestamp_iso() {
        let result = canonical_timestamp("2024-02-22T10:15:30Z", None);
        assert!(result.is_some());
        assert!(result.unwrap().starts_with("2024-02-22 10:15:30."));
    }

    #[test]
    fn test_canonical_timestamp_none() {
        assert!(canonical_timestamp("not a ts", None).is_none());
    }

    #[test]
    fn test_canonical_timestamp_bsd_uses_current_year() {
        let result = canonical_timestamp("Feb 14 22:11:26", None).unwrap();
        let current_year = time::OffsetDateTime::now_utc().year();
        assert!(result.starts_with(&format!("{:04}-02-14 22:11:26.", current_year)));
    }

    // ── DateFilter::matches ────────────────────────────────────────────

    #[test]
    fn test_matches_time_range_inside() {
        let df = parse_date_filter("01:00:00 .. 02:00:00").unwrap();
        // 01:30:00 ISO timestamp
        assert!(df.matches("2024-02-22T01:30:00Z", None));
    }

    #[test]
    fn test_matches_time_range_at_lower_bound() {
        let df = parse_date_filter("01:00:00 .. 02:00:00").unwrap();
        assert!(df.matches("2024-02-22T01:00:00Z", None));
    }

    #[test]
    fn test_matches_time_range_at_upper_bound() {
        let df = parse_date_filter("01:00:00 .. 02:00:00").unwrap();
        assert!(df.matches("2024-02-22T02:00:00Z", None));
    }

    #[test]
    fn test_matches_time_range_outside() {
        let df = parse_date_filter("01:00:00 .. 02:00:00").unwrap();
        assert!(!df.matches("2024-02-22T03:00:00Z", None));
    }

    #[test]
    fn test_matches_time_range_no_spaces() {
        let df = parse_date_filter("09:00..10:00").unwrap();
        assert!(df.matches("2024-01-01T09:30:59Z", None));
        assert!(!df.matches("2024-01-01T10:01:00Z", None));
    }

    #[test]
    fn test_matches_gt_comparison() {
        let df = parse_date_filter("> 2024-02-22").unwrap();
        assert!(df.matches("2024-02-23T00:00:00Z", None));
        assert!(!df.matches("2024-02-22T00:00:00Z", None));
        assert!(!df.matches("2024-02-21T23:59:59Z", None));
    }

    #[test]
    fn test_matches_bsd_bound_against_iso_timestamp() {
        // BSD bound has year 0000; ISO log timestamps have a real year.
        // "2024-01-20..." must NOT be > "0000-01-23..." (Jan 20 < Jan 23).
        let df = parse_date_filter("> Jan 23").unwrap();
        assert!(!df.matches("2024-01-20T10:00:00Z", None)); // before Jan 23
        assert!(!df.matches("2024-01-23T00:00:00Z", None)); // exactly Jan 23 (strict >)
        assert!(df.matches("2024-01-25T10:00:00Z", None)); // after Jan 23
    }

    #[test]
    fn test_matches_bsd_range_against_iso_timestamps() {
        let df = parse_date_filter("Jan 20 .. Jan 23").unwrap();
        assert!(!df.matches("2024-01-19T23:59:59Z", None)); // before range
        assert!(df.matches("2024-01-20T00:00:00Z", None)); // lower bound
        assert!(df.matches("2024-01-21T12:00:00Z", None)); // inside
        assert!(df.matches("2024-01-23T00:00:00Z", None)); // upper bound (start of day)
        assert!(df.matches("2024-01-23T23:59:59Z", None)); // upper bound (end of day)
        assert!(!df.matches("2024-01-24T00:00:00Z", None)); // after range
    }

    #[test]
    fn test_matches_ge_comparison() {
        let df = parse_date_filter(">= 2024-02-22").unwrap();
        assert!(df.matches("2024-02-22T00:00:00Z", None));
        assert!(df.matches("2024-02-23T00:00:00Z", None));
        assert!(!df.matches("2024-02-21T23:59:59Z", None));
    }

    #[test]
    fn test_matches_lt_comparison() {
        let df = parse_date_filter("< 2024-02-22").unwrap();
        assert!(df.matches("2024-02-21T23:59:59Z", None));
        assert!(!df.matches("2024-02-22T00:00:00Z", None));
    }

    #[test]
    fn test_matches_le_comparison() {
        let df = parse_date_filter("<= 2024-02-22").unwrap();
        assert!(df.matches("2024-02-22T00:00:00Z", None));
        assert!(!df.matches("2024-02-22T00:00:01Z", None));
    }

    #[test]
    fn test_matches_bsd_date_range() {
        let df = parse_date_filter("Feb 21 .. Feb 22").unwrap();
        assert!(df.matches("Feb 21 12:00:00", None));
        assert!(df.matches("Feb 22 00:00:00", None));
        assert!(df.matches("Feb 22 23:59:59", None)); // whole upper day included
        assert!(!df.matches("Feb 23 00:00:00", None));
    }

    #[test]
    fn test_matches_unparseable_passes_through() {
        let df = parse_date_filter("01:00:00 .. 02:00:00").unwrap();
        assert!(df.matches("not a timestamp", None));
        assert!(df.matches("[    0.000000]", None)); // dmesg
    }

    #[test]
    fn test_matches_hm_range() {
        let df = parse_date_filter("13:00 .. 14:00").unwrap();
        assert!(df.matches("2024-01-01T13:30:00Z", None));
        assert!(!df.matches("2024-01-01T12:30:00Z", None));
    }

    // ── extract_date_filters ──────────────────────────────────────────

    #[test]
    fn test_extract_date_filters_empty() {
        let filters = extract_date_filters(&[]);
        assert!(filters.is_empty());
    }

    #[test]
    fn test_extract_date_filters_skips_non_date() {
        let defs = vec![FilterDef {
            id: 1,
            pattern: "ERROR".to_string(),
            filter_type: crate::filters::FilterType::Include,
            enabled: true,
            color_config: None,
            use_regex: false,
            ignore_case: false,
            group: None,
        }];
        let filters = extract_date_filters(&defs);
        assert!(filters.is_empty());
    }

    #[test]
    fn test_extract_date_filters_parses_date() {
        let defs = vec![FilterDef {
            id: 1,
            pattern: "@date:01:00:00 .. 02:00:00".to_string(),
            filter_type: crate::filters::FilterType::Include,
            enabled: true,
            color_config: None,
            use_regex: false,
            ignore_case: false,
            group: None,
        }];
        let filters = extract_date_filters(&defs);
        assert_eq!(filters.len(), 1);
    }

    #[test]
    fn test_extract_date_filters_skips_disabled() {
        let defs = vec![FilterDef {
            id: 1,
            pattern: "@date:01:00:00 .. 02:00:00".to_string(),
            filter_type: crate::filters::FilterType::Include,
            enabled: false,
            color_config: None,
            use_regex: false,
            ignore_case: false,
            group: None,
        }];
        let filters = extract_date_filters(&defs);
        assert!(filters.is_empty());
    }

    #[test]
    fn test_extract_date_filters_skips_invalid_expr() {
        let defs = vec![FilterDef {
            id: 1,
            pattern: "@date:garbage".to_string(),
            filter_type: crate::filters::FilterType::Include,
            enabled: true,
            color_config: None,
            use_regex: false,
            ignore_case: false,
            group: None,
        }];
        let filters = extract_date_filters(&defs);
        assert!(filters.is_empty());
    }

    // ── Edge cases ────────────────────────────────────────────────────

    #[test]
    fn test_time_only_midnight_boundary() {
        let df = parse_date_filter("00:00:00 .. 23:59:59").unwrap();
        assert!(df.matches("2024-01-01T00:00:00Z", None));
        assert!(df.matches("2024-01-01T23:59:59Z", None));
    }

    #[test]
    fn test_equal_range_bounds() {
        let df = parse_date_filter("01:00:00 .. 01:00:00").unwrap();
        assert!(df.matches("2024-01-01T01:00:00Z", None));
        assert!(!df.matches("2024-01-01T01:00:01Z", None));
    }

    #[test]
    fn test_matches_with_datetime_format() {
        let df = parse_date_filter(">= 2024-01-15 10:30:00").unwrap();
        assert!(df.matches("2024-01-15 10:30:00.123", None));
        assert!(!df.matches("2024-01-15 10:29:59.999", None));
    }

    #[test]
    fn test_matches_with_slash_format() {
        let df = parse_date_filter(">= 2024-01-15").unwrap();
        assert!(df.matches("2024/01/15 10:30:00", None));
    }

    #[test]
    fn test_normalize_iso_no_tz() {
        let n = normalize_log_timestamp("2024-02-22T10:15:30").unwrap();
        assert_eq!(buf_as_str(&n.canonical), "2024-02-22 10:15:30.000");
    }
}
