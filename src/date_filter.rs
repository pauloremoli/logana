// ---------------------------------------------------------------------------
// Date/time filter for log lines
// ---------------------------------------------------------------------------
//
// Parses user expressions like `01:00:00 .. 02:00:00` or `> Feb 21 01:00:00`
// and matches them against timestamps extracted by log format parsers.
//
// Date filters are stored as regular `FilterDef` entries with the pattern
// prefixed by `@date:` and `FilterType::Include`.  The `@date:` prefix is
// intentionally an invalid regex/substring so it never conflicts with text
// filters.

use crate::parser::timestamp::BSD_MONTHS;
use crate::types::FilterDef;

/// The `@date:` prefix used in `FilterDef.pattern` to mark date filters.
pub const DATE_PREFIX: &str = "@date:";

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ComparisonOp {
    Gt,
    Ge,
    Lt,
    Le,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ComparisonMode {
    TimeOnly,
    FullDatetime,
}

#[derive(Debug, Clone)]
pub(crate) struct DateBound {
    /// Seconds since midnight (only meaningful when mode == TimeOnly).
    time_val: Option<u32>,
    /// Canonical `"YYYY-MM-DD HH:MM:SS.ffffff"` string (mode == FullDatetime).
    datetime_val: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) enum DateFilter {
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
    canonical: String,
}

// ---------------------------------------------------------------------------
// User-input parsing helpers
// ---------------------------------------------------------------------------

/// Parse a user-provided date/time token into a `(DateBound, ComparisonMode)`.
///
/// Accepted forms:
///   HH:MM:SS          → TimeOnly
///   HH:MM             → TimeOnly  (seconds = 00)
///   Mmm DD HH:MM:SS   → FullDatetime, year = 0000
///   Mmm DD             → FullDatetime, year = 0000, time = 00:00:00
///   YYYY-MM-DD         → FullDatetime, time = 00:00:00
///   YYYY-MM-DD HH:MM:SS → FullDatetime
///   YYYY-MM-DDTHH:MM:SS → FullDatetime
fn parse_bound(input: &str) -> Result<(DateBound, ComparisonMode), String> {
    let s = input.trim();
    if s.is_empty() {
        return Err("Empty date/time value".to_string());
    }

    // Try HH:MM:SS or HH:MM
    if let Some(secs) = try_parse_time_only(s) {
        return Ok((
            DateBound {
                time_val: Some(secs),
                datetime_val: None,
            },
            ComparisonMode::TimeOnly,
        ));
    }

    // Try BSD month prefix: "Feb 21 HH:MM:SS" or "Feb 21"
    if s.len() >= 3 {
        let month_abbr = &s[..3];
        if let Some(month_num) = bsd_month_number(month_abbr) {
            return parse_bsd_bound(s, month_num);
        }
    }

    // Try YYYY-MM-DD[T| ]HH:MM:SS or YYYY-MM-DD
    if s.len() >= 10
        && s.as_bytes()[4] == b'-'
        && s.as_bytes()[7] == b'-'
        && s.as_bytes()[0].is_ascii_digit()
    {
        return parse_iso_bound(s);
    }

    Err(format!("Unrecognized date/time format: '{}'", s))
}

fn try_parse_time_only(s: &str) -> Option<u32> {
    let b = s.as_bytes();
    // HH:MM:SS (8 chars) or HH:MM (5 chars)
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
    let sec = if b.len() == 8 {
        if b[5] != b':' || !b[6].is_ascii_digit() || !b[7].is_ascii_digit() {
            return None;
        }
        let sec_val: u32 = s[6..8].parse().ok()?;
        if sec_val > 59 {
            return None;
        }
        sec_val
    } else {
        0
    };
    Some(h * 3600 + m * 60 + sec)
}

fn bsd_month_number(abbr: &str) -> Option<u32> {
    BSD_MONTHS
        .iter()
        .position(|&m| m.eq_ignore_ascii_case(abbr))
        .map(|i| i as u32 + 1)
}

fn parse_bsd_bound(s: &str, month_num: u32) -> Result<(DateBound, ComparisonMode), String> {
    // Skip month abbreviation + space
    let rest = s[3..].trim_start();
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
    let (h, m, sec) = if after_day.is_empty() {
        (0, 0, 0)
    } else if let Some(secs) = try_parse_time_only(after_day) {
        (secs / 3600, (secs % 3600) / 60, secs % 60)
    } else {
        return Err(format!("Invalid time in '{}'", s));
    };

    let canonical = format!(
        "0000-{:02}-{:02} {:02}:{:02}:{:02}.000000",
        month_num, day, h, m, sec
    );
    Ok((
        DateBound {
            time_val: None,
            datetime_val: Some(canonical),
        },
        ComparisonMode::FullDatetime,
    ))
}

fn parse_iso_bound(s: &str) -> Result<(DateBound, ComparisonMode), String> {
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
    let (h, m, sec) = if after_date.is_empty() {
        (0u32, 0u32, 0u32)
    } else {
        let sep = after_date.as_bytes()[0];
        if sep == b'T' || sep == b' ' {
            let time_str = &after_date[1..];
            if let Some(secs) = try_parse_time_only(time_str) {
                (secs / 3600, (secs % 3600) / 60, secs % 60)
            } else {
                return Err(format!("Invalid time in '{}'", s));
            }
        } else {
            return Err(format!("Unexpected character after date in '{}'", s));
        }
    };

    let canonical = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.000000",
        year, month, day, h, m, sec
    );
    Ok((
        DateBound {
            time_val: None,
            datetime_val: Some(canonical),
        },
        ComparisonMode::FullDatetime,
    ))
}

// ---------------------------------------------------------------------------
// Public API — parse date filter expression
// ---------------------------------------------------------------------------

/// Parse the expression that follows `@date:`.
///
/// Syntax:
///   `<bound> .. <bound>`               — inclusive range
///   `> <bound>` / `>= <bound>`         — after
///   `< <bound>` / `<= <bound>`         — before
pub(crate) fn parse_date_filter(input: &str) -> Result<DateFilter, String> {
    let s = input.trim();
    if s.is_empty() {
        return Err("Empty date filter expression".to_string());
    }

    // Range: contains " .. "
    if let Some(dot_pos) = s.find(" .. ") {
        let left = &s[..dot_pos];
        let right = &s[dot_pos + 4..];
        let (lower, l_mode) = parse_bound(left)?;
        let (upper, u_mode) = parse_bound(right)?;
        if l_mode != u_mode {
            return Err(
                "Both sides of a range must use the same format (both time-only or both date)"
                    .to_string(),
            );
        }
        // Validate lower <= upper
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
        return Ok(DateFilter::Range {
            mode: l_mode,
            lower,
            upper,
        });
    }

    // Comparison: starts with >=, >, <=, <
    if let Some(rest) = s.strip_prefix(">=") {
        let (bound, mode) = parse_bound(rest)?;
        return Ok(DateFilter::Comparison {
            mode,
            op: ComparisonOp::Ge,
            bound,
        });
    }
    if let Some(rest) = s.strip_prefix('>') {
        let (bound, mode) = parse_bound(rest)?;
        return Ok(DateFilter::Comparison {
            mode,
            op: ComparisonOp::Gt,
            bound,
        });
    }
    if let Some(rest) = s.strip_prefix("<=") {
        let (bound, mode) = parse_bound(rest)?;
        return Ok(DateFilter::Comparison {
            mode,
            op: ComparisonOp::Le,
            bound,
        });
    }
    if let Some(rest) = s.strip_prefix('<') {
        let (bound, mode) = parse_bound(rest)?;
        return Ok(DateFilter::Comparison {
            mode,
            op: ComparisonOp::Lt,
            bound,
        });
    }

    Err(format!(
        "Expected a range (.. ) or comparison (>, >=, <, <=) in '{}'",
        s
    ))
}

// ---------------------------------------------------------------------------
// Timestamp normalization
// ---------------------------------------------------------------------------

/// Normalize a raw timestamp string (as produced by a log format parser's
/// `DisplayParts.timestamp`) into a canonical form for comparison.
///
/// Returns `None` for dmesg-style boot-relative timestamps or unparseable input.
fn normalize_log_timestamp(ts: &str) -> Option<NormalizedTimestamp> {
    let s = ts.trim();
    if s.is_empty() {
        return None;
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
    // "2024-02-22T10:15:30..." → extract date + time
    let date = &s[..10];
    // Replace slashes with dashes if needed (shouldn't happen for ISO but safety)
    let time_part = &s[11..];
    // Find end of time: first non-time char after HH:MM:SS
    let (h, m, sec, frac) = parse_hms_frac(time_part)?;
    let canonical = format!(
        "{} {:02}:{:02}:{:02}.{}",
        date.replace('/', "-"),
        h,
        m,
        sec,
        frac
    );
    let tod = h * 3600 + m * 60 + sec;
    Some(NormalizedTimestamp {
        time_of_day: tod,
        canonical,
    })
}

fn normalize_full_ts(s: &str) -> Option<NormalizedTimestamp> {
    // "Mon 2024-02-22 10:15:30 UTC"
    if s.len() < 23 {
        return None;
    }
    let date = &s[4..14]; // YYYY-MM-DD
    let time_str = &s[15..];
    let (h, m, sec, frac) = parse_hms_frac(time_str)?;
    let canonical = format!("{} {:02}:{:02}:{:02}.{}", date, h, m, sec, frac);
    Some(NormalizedTimestamp {
        time_of_day: h * 3600 + m * 60 + sec,
        canonical,
    })
}

fn normalize_datetime_ts(s: &str) -> Option<NormalizedTimestamp> {
    // "2024-01-15 10:30:00[.mmm]"
    let date = &s[..10];
    let time_str = &s[11..];
    let (h, m, sec, frac) = parse_hms_frac(time_str)?;
    let canonical = format!("{} {:02}:{:02}:{:02}.{}", date, h, m, sec, frac);
    Some(NormalizedTimestamp {
        time_of_day: h * 3600 + m * 60 + sec,
        canonical,
    })
}

fn normalize_slash_ts(s: &str) -> Option<NormalizedTimestamp> {
    // "2024/01/15 10:30:00" → "2024-01-15 10:30:00.000000"
    let date = s[..10].replace('/', "-");
    let time_str = &s[11..];
    let (h, m, sec, frac) = parse_hms_frac(time_str)?;
    let canonical = format!("{} {:02}:{:02}:{:02}.{}", date, h, m, sec, frac);
    Some(NormalizedTimestamp {
        time_of_day: h * 3600 + m * 60 + sec,
        canonical,
    })
}

fn normalize_clf_ts(s: &str) -> Option<NormalizedTimestamp> {
    // "10/Oct/2000:13:55:36 -0700"
    if s.len() < 20 {
        return None;
    }
    let day: u32 = s[..2].parse().ok()?;
    let month_abbr = &s[3..6];
    let month_num = bsd_month_number(month_abbr)?;
    let year: u32 = s[7..11].parse().ok()?;
    if s.as_bytes()[11] != b':' {
        return None;
    }
    let time_str = &s[12..];
    let (h, m, sec, frac) = parse_hms_frac(time_str)?;
    let canonical = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{}",
        year, month_num, day, h, m, sec, frac
    );
    Some(NormalizedTimestamp {
        time_of_day: h * 3600 + m * 60 + sec,
        canonical,
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
    let (h, m, sec, frac) = parse_hms_frac(after_day)?;
    let canonical = format!(
        "0000-{:02}-{:02} {:02}:{:02}:{:02}.{}",
        month_num, day, h, m, sec, frac
    );
    Some(NormalizedTimestamp {
        time_of_day: h * 3600 + m * 60 + sec,
        canonical,
    })
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
    let (h, m, sec, frac) = parse_hms_frac(after_day)?;
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
    let canonical = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{}",
        year, month_num, day, h, m, sec, frac
    );
    Some(NormalizedTimestamp {
        time_of_day: h * 3600 + m * 60 + sec,
        canonical,
    })
}

/// Parse `HH:MM:SS[.frac][,frac]` and return `(h, m, s, frac_padded_to_6)`.
fn parse_hms_frac(s: &str) -> Option<(u32, u32, u32, String)> {
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
    let h: u32 = s[..2].parse().ok()?;
    let m: u32 = s[3..5].parse().ok()?;
    let sec: u32 = s[6..8].parse().ok()?;

    let mut frac = String::new();
    if s.len() > 8 && (b[8] == b'.' || b[8] == b',') {
        let start = 9;
        let end = s[start..]
            .find(|c: char| !c.is_ascii_digit())
            .map(|p| p + start)
            .unwrap_or(s.len());
        frac = s[start..end].to_string();
    }
    // Pad or truncate to 6 digits
    while frac.len() < 6 {
        frac.push('0');
    }
    if frac.len() > 6 {
        frac.truncate(6);
    }
    Some((h, m, sec, frac))
}

// ---------------------------------------------------------------------------
// Matching
// ---------------------------------------------------------------------------

impl DateFilter {
    /// Check if a raw timestamp string (from a parser's `DisplayParts.timestamp`)
    /// passes this filter.
    pub fn matches(&self, timestamp: &str) -> bool {
        let norm = match normalize_log_timestamp(timestamp) {
            Some(n) => n,
            None => return true, // unparseable → pass through
        };
        match self {
            DateFilter::Range { mode, lower, upper } => match mode {
                ComparisonMode::TimeOnly => {
                    let t = norm.time_of_day;
                    t >= lower.time_val.unwrap() && t <= upper.time_val.unwrap()
                }
                ComparisonMode::FullDatetime => {
                    let c = &norm.canonical;
                    c >= lower.datetime_val.as_ref().unwrap()
                        && c <= upper.datetime_val.as_ref().unwrap()
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
                    let c = &norm.canonical;
                    let b = bound.datetime_val.as_ref().unwrap();
                    match op {
                        ComparisonOp::Gt => c > b,
                        ComparisonOp::Ge => c >= b,
                        ComparisonOp::Lt => c < b,
                        ComparisonOp::Le => c <= b,
                    }
                }
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Extract date filters from filter definitions
// ---------------------------------------------------------------------------

/// Collect all enabled `@date:` filters from the filter list, parsed.
/// Filters that fail to parse are silently skipped.
pub(crate) fn extract_date_filters(filter_defs: &[FilterDef]) -> Vec<DateFilter> {
    filter_defs
        .iter()
        .filter(|f| f.enabled && f.pattern.starts_with(DATE_PREFIX))
        .filter_map(|f| {
            let expr = &f.pattern[DATE_PREFIX.len()..];
            parse_date_filter(expr).ok()
        })
        .collect()
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_bound ──────────────────────────────────────────────────

    #[test]
    fn test_parse_bound_time_only_hms() {
        let (b, mode) = parse_bound("01:30:45").unwrap();
        assert_eq!(mode, ComparisonMode::TimeOnly);
        assert_eq!(b.time_val, Some(1 * 3600 + 30 * 60 + 45));
    }

    #[test]
    fn test_parse_bound_time_only_hm() {
        let (b, mode) = parse_bound("13:00").unwrap();
        assert_eq!(mode, ComparisonMode::TimeOnly);
        assert_eq!(b.time_val, Some(13 * 3600));
    }

    #[test]
    fn test_parse_bound_bsd_date_time() {
        let (b, mode) = parse_bound("Feb 21 01:00:00").unwrap();
        assert_eq!(mode, ComparisonMode::FullDatetime);
        assert_eq!(
            b.datetime_val.as_deref(),
            Some("0000-02-21 01:00:00.000000")
        );
    }

    #[test]
    fn test_parse_bound_bsd_date_only() {
        let (b, mode) = parse_bound("Feb 21").unwrap();
        assert_eq!(mode, ComparisonMode::FullDatetime);
        assert_eq!(
            b.datetime_val.as_deref(),
            Some("0000-02-21 00:00:00.000000")
        );
    }

    #[test]
    fn test_parse_bound_iso_date_only() {
        let (b, mode) = parse_bound("2024-02-22").unwrap();
        assert_eq!(mode, ComparisonMode::FullDatetime);
        assert_eq!(
            b.datetime_val.as_deref(),
            Some("2024-02-22 00:00:00.000000")
        );
    }

    #[test]
    fn test_parse_bound_iso_datetime() {
        let (b, mode) = parse_bound("2024-02-22T10:15:30").unwrap();
        assert_eq!(mode, ComparisonMode::FullDatetime);
        assert_eq!(
            b.datetime_val.as_deref(),
            Some("2024-02-22 10:15:30.000000")
        );
    }

    #[test]
    fn test_parse_bound_iso_datetime_space() {
        let (b, mode) = parse_bound("2024-02-22 10:15:30").unwrap();
        assert_eq!(mode, ComparisonMode::FullDatetime);
        assert_eq!(
            b.datetime_val.as_deref(),
            Some("2024-02-22 10:15:30.000000")
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

    #[test]
    fn test_parse_no_operator_error() {
        assert!(parse_date_filter("01:00:00").is_err());
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

    // ── normalize_log_timestamp ───────────────────────────────────────

    #[test]
    fn test_normalize_iso() {
        let n = normalize_log_timestamp("2024-02-22T10:15:30+0000").unwrap();
        assert_eq!(n.canonical, "2024-02-22 10:15:30.000000");
        assert_eq!(n.time_of_day, 10 * 3600 + 15 * 60 + 30);
    }

    #[test]
    fn test_normalize_iso_with_frac() {
        let n = normalize_log_timestamp("2024-02-22T10:15:30.123456Z").unwrap();
        assert_eq!(n.canonical, "2024-02-22 10:15:30.123456");
    }

    #[test]
    fn test_normalize_datetime() {
        let n = normalize_log_timestamp("2024-01-15 10:30:00.123").unwrap();
        assert_eq!(n.canonical, "2024-01-15 10:30:00.123000");
    }

    #[test]
    fn test_normalize_datetime_comma_frac() {
        let n = normalize_log_timestamp("2024-01-15 10:30:00,456").unwrap();
        assert_eq!(n.canonical, "2024-01-15 10:30:00.456000");
    }

    #[test]
    fn test_normalize_slash() {
        let n = normalize_log_timestamp("2024/01/15 10:30:00").unwrap();
        assert_eq!(n.canonical, "2024-01-15 10:30:00.000000");
    }

    #[test]
    fn test_normalize_full_journalctl() {
        let n = normalize_log_timestamp("Mon 2024-02-22 10:15:30 UTC").unwrap();
        assert_eq!(n.canonical, "2024-02-22 10:15:30.000000");
    }

    #[test]
    fn test_normalize_bsd() {
        let n = normalize_log_timestamp("Feb 22 10:15:30").unwrap();
        assert_eq!(n.canonical, "0000-02-22 10:15:30.000000");
    }

    #[test]
    fn test_normalize_bsd_precise() {
        let n = normalize_log_timestamp("Feb 22 10:15:30.123456").unwrap();
        assert_eq!(n.canonical, "0000-02-22 10:15:30.123456");
    }

    #[test]
    fn test_normalize_clf() {
        let n = normalize_log_timestamp("10/Oct/2000:13:55:36 -0700").unwrap();
        assert_eq!(n.canonical, "2000-10-10 13:55:36.000000");
    }

    #[test]
    fn test_normalize_apache_error() {
        let n = normalize_log_timestamp("[Mon Jan 15 10:30:00.123456 2024]").unwrap();
        assert_eq!(n.canonical, "2024-01-15 10:30:00.123456");
    }

    #[test]
    fn test_normalize_apache_error_no_frac() {
        let n = normalize_log_timestamp("[Fri Dec 31 23:59:59 2024]").unwrap();
        assert_eq!(n.canonical, "2024-12-31 23:59:59.000000");
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

    // ── DateFilter::matches ────────────────────────────────────────────

    #[test]
    fn test_matches_time_range_inside() {
        let df = parse_date_filter("01:00:00 .. 02:00:00").unwrap();
        // 01:30:00 ISO timestamp
        assert!(df.matches("2024-02-22T01:30:00Z"));
    }

    #[test]
    fn test_matches_time_range_at_lower_bound() {
        let df = parse_date_filter("01:00:00 .. 02:00:00").unwrap();
        assert!(df.matches("2024-02-22T01:00:00Z"));
    }

    #[test]
    fn test_matches_time_range_at_upper_bound() {
        let df = parse_date_filter("01:00:00 .. 02:00:00").unwrap();
        assert!(df.matches("2024-02-22T02:00:00Z"));
    }

    #[test]
    fn test_matches_time_range_outside() {
        let df = parse_date_filter("01:00:00 .. 02:00:00").unwrap();
        assert!(!df.matches("2024-02-22T03:00:00Z"));
    }

    #[test]
    fn test_matches_gt_comparison() {
        let df = parse_date_filter("> 2024-02-22").unwrap();
        assert!(df.matches("2024-02-23T00:00:00Z"));
        assert!(!df.matches("2024-02-22T00:00:00Z"));
        assert!(!df.matches("2024-02-21T23:59:59Z"));
    }

    #[test]
    fn test_matches_ge_comparison() {
        let df = parse_date_filter(">= 2024-02-22").unwrap();
        assert!(df.matches("2024-02-22T00:00:00Z"));
        assert!(df.matches("2024-02-23T00:00:00Z"));
        assert!(!df.matches("2024-02-21T23:59:59Z"));
    }

    #[test]
    fn test_matches_lt_comparison() {
        let df = parse_date_filter("< 2024-02-22").unwrap();
        assert!(df.matches("2024-02-21T23:59:59Z"));
        assert!(!df.matches("2024-02-22T00:00:00Z"));
    }

    #[test]
    fn test_matches_le_comparison() {
        let df = parse_date_filter("<= 2024-02-22").unwrap();
        assert!(df.matches("2024-02-22T00:00:00Z"));
        assert!(!df.matches("2024-02-22T00:00:01Z"));
    }

    #[test]
    fn test_matches_bsd_date_range() {
        let df = parse_date_filter("Feb 21 .. Feb 22").unwrap();
        assert!(df.matches("Feb 21 12:00:00"));
        assert!(df.matches("Feb 22 00:00:00"));
        assert!(!df.matches("Feb 23 00:00:00"));
    }

    #[test]
    fn test_matches_unparseable_passes_through() {
        let df = parse_date_filter("01:00:00 .. 02:00:00").unwrap();
        assert!(df.matches("not a timestamp"));
        assert!(df.matches("[    0.000000]")); // dmesg
    }

    #[test]
    fn test_matches_hm_range() {
        let df = parse_date_filter("13:00 .. 14:00").unwrap();
        assert!(df.matches("2024-01-01T13:30:00Z"));
        assert!(!df.matches("2024-01-01T12:30:00Z"));
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
            filter_type: crate::types::FilterType::Include,
            enabled: true,
            color_config: None,
        }];
        let filters = extract_date_filters(&defs);
        assert!(filters.is_empty());
    }

    #[test]
    fn test_extract_date_filters_parses_date() {
        let defs = vec![FilterDef {
            id: 1,
            pattern: "@date:01:00:00 .. 02:00:00".to_string(),
            filter_type: crate::types::FilterType::Include,
            enabled: true,
            color_config: None,
        }];
        let filters = extract_date_filters(&defs);
        assert_eq!(filters.len(), 1);
    }

    #[test]
    fn test_extract_date_filters_skips_disabled() {
        let defs = vec![FilterDef {
            id: 1,
            pattern: "@date:01:00:00 .. 02:00:00".to_string(),
            filter_type: crate::types::FilterType::Include,
            enabled: false,
            color_config: None,
        }];
        let filters = extract_date_filters(&defs);
        assert!(filters.is_empty());
    }

    #[test]
    fn test_extract_date_filters_skips_invalid_expr() {
        let defs = vec![FilterDef {
            id: 1,
            pattern: "@date:garbage".to_string(),
            filter_type: crate::types::FilterType::Include,
            enabled: true,
            color_config: None,
        }];
        let filters = extract_date_filters(&defs);
        assert!(filters.is_empty());
    }

    // ── Edge cases ────────────────────────────────────────────────────

    #[test]
    fn test_time_only_midnight_boundary() {
        let df = parse_date_filter("00:00:00 .. 23:59:59").unwrap();
        assert!(df.matches("2024-01-01T00:00:00Z"));
        assert!(df.matches("2024-01-01T23:59:59Z"));
    }

    #[test]
    fn test_equal_range_bounds() {
        let df = parse_date_filter("01:00:00 .. 01:00:00").unwrap();
        assert!(df.matches("2024-01-01T01:00:00Z"));
        assert!(!df.matches("2024-01-01T01:00:01Z"));
    }

    #[test]
    fn test_matches_with_datetime_format() {
        let df = parse_date_filter(">= 2024-01-15 10:30:00").unwrap();
        assert!(df.matches("2024-01-15 10:30:00.123"));
        assert!(!df.matches("2024-01-15 10:29:59.999"));
    }

    #[test]
    fn test_matches_with_slash_format() {
        let df = parse_date_filter(">= 2024-01-15").unwrap();
        assert!(df.matches("2024/01/15 10:30:00"));
    }

    #[test]
    fn test_normalize_iso_no_tz() {
        let n = normalize_log_timestamp("2024-02-22T10:15:30").unwrap();
        assert_eq!(n.canonical, "2024-02-22 10:15:30.000000");
    }
}
