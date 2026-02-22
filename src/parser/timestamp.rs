// ---------------------------------------------------------------------------
// Shared timestamp parsing utilities
// ---------------------------------------------------------------------------
//
// Centralises all timestamp recognition used by the various format parsers.
// Each function returns `Option<(&str, usize)>` where the string is the
// matched timestamp slice and the usize is the number of bytes consumed.

/// Weekday abbreviations used by `journalctl -o short-full`.
pub(crate) const WEEKDAYS: &[&str] = &["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// BSD month abbreviations.
pub(crate) const BSD_MONTHS: &[&str] = &[
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

// ---------------------------------------------------------------------------
// Timestamp parsers — each returns `(timestamp_slice, bytes_consumed)`.
// ---------------------------------------------------------------------------

/// Try ISO 8601 timestamp: `2024-02-22T10:15:30+0000` or `2024-02-22T10:15:30.123456+00:00`.
/// Stops at the first space after the 'T'.
pub(crate) fn parse_iso_timestamp(s: &str) -> Option<(&str, usize)> {
    if s.len() < 19 {
        return None;
    }
    let bytes = s.as_bytes();
    if !bytes[0].is_ascii_digit()
        || !bytes[1].is_ascii_digit()
        || !bytes[2].is_ascii_digit()
        || !bytes[3].is_ascii_digit()
        || bytes[4] != b'-'
    {
        return None;
    }
    if bytes[10] != b'T' {
        return None;
    }
    let end = s[10..].find(' ').map(|p| p + 10).unwrap_or(s.len());
    if end <= 10 {
        return None;
    }
    Some((&s[..end], end))
}

/// Try BSD timestamp with microseconds: `Feb 22 10:15:30.123456`.
/// Returns the full timestamp including microseconds.
pub(crate) fn parse_bsd_precise_timestamp(s: &str) -> Option<(&str, usize)> {
    if s.len() < 16 {
        return None;
    }
    let month = &s[..3];
    if !BSD_MONTHS.contains(&month) {
        return None;
    }
    if s.as_bytes()[3] != b' ' {
        return None;
    }
    let day_end = if s.as_bytes()[4] == b' ' {
        if !s.as_bytes()[5].is_ascii_digit() {
            return None;
        }
        6
    } else if s.as_bytes()[4].is_ascii_digit() && s.as_bytes()[5].is_ascii_digit() {
        6
    } else {
        return None;
    };
    if day_end >= s.len() || s.as_bytes()[day_end] != b' ' {
        return None;
    }
    let time_start = day_end + 1;
    if time_start + 8 > s.len() {
        return None;
    }
    let t = &s[time_start..time_start + 8];
    if t.as_bytes()[2] != b':'
        || t.as_bytes()[5] != b':'
        || !t.as_bytes()[0].is_ascii_digit()
        || !t.as_bytes()[1].is_ascii_digit()
        || !t.as_bytes()[3].is_ascii_digit()
        || !t.as_bytes()[4].is_ascii_digit()
        || !t.as_bytes()[6].is_ascii_digit()
        || !t.as_bytes()[7].is_ascii_digit()
    {
        return None;
    }
    let after_time = time_start + 8;
    if after_time >= s.len() || s.as_bytes()[after_time] != b'.' {
        return None;
    }
    let mut end = after_time + 1;
    while end < s.len() && s.as_bytes()[end].is_ascii_digit() {
        end += 1;
    }
    if end == after_time + 1 {
        return None;
    }
    Some((&s[..end], end))
}

/// Try short-full timestamp: `Mon 2024-02-22 10:15:30 UTC`.
/// Format: `Www YYYY-MM-DD HH:MM:SS TZ`
pub(crate) fn parse_full_timestamp(s: &str) -> Option<(&str, usize)> {
    if s.len() < 27 {
        return None;
    }
    let weekday = &s[..3];
    if !WEEKDAYS.contains(&weekday) {
        return None;
    }
    if s.as_bytes()[3] != b' ' {
        return None;
    }
    let date_part = &s[4..14];
    let db = date_part.as_bytes();
    if !db[0].is_ascii_digit()
        || !db[1].is_ascii_digit()
        || !db[2].is_ascii_digit()
        || !db[3].is_ascii_digit()
        || db[4] != b'-'
        || !db[5].is_ascii_digit()
        || !db[6].is_ascii_digit()
        || db[7] != b'-'
        || !db[8].is_ascii_digit()
        || !db[9].is_ascii_digit()
    {
        return None;
    }
    if s.as_bytes()[14] != b' ' {
        return None;
    }
    if s.len() < 23 {
        return None;
    }
    let t = &s[15..23];
    if t.as_bytes()[2] != b':'
        || t.as_bytes()[5] != b':'
        || !t.as_bytes()[0].is_ascii_digit()
        || !t.as_bytes()[1].is_ascii_digit()
        || !t.as_bytes()[3].is_ascii_digit()
        || !t.as_bytes()[4].is_ascii_digit()
        || !t.as_bytes()[6].is_ascii_digit()
        || !t.as_bytes()[7].is_ascii_digit()
    {
        return None;
    }
    if s.as_bytes()[23] != b' ' {
        return None;
    }
    let tz_start = 24;
    let tz_end = s[tz_start..]
        .find(' ')
        .map(|p| p + tz_start)
        .unwrap_or(s.len());
    if tz_end <= tz_start {
        return None;
    }
    Some((&s[..tz_end], tz_end))
}

/// Parse `YYYY-MM-DD HH:MM:SS[.mmm][,mmm]` (logback, Python, Spring Boot).
/// Supports both `.` and `,` as fractional separator.
/// Also handles optional timezone suffix like `+00:00` or `Z`.
pub(crate) fn parse_datetime_timestamp(s: &str) -> Option<(&str, usize)> {
    // Minimum: "YYYY-MM-DD HH:MM:SS" = 19 chars
    if s.len() < 19 {
        return None;
    }
    let b = s.as_bytes();
    // YYYY-MM-DD
    if !b[0].is_ascii_digit()
        || !b[1].is_ascii_digit()
        || !b[2].is_ascii_digit()
        || !b[3].is_ascii_digit()
        || b[4] != b'-'
        || !b[5].is_ascii_digit()
        || !b[6].is_ascii_digit()
        || b[7] != b'-'
        || !b[8].is_ascii_digit()
        || !b[9].is_ascii_digit()
    {
        return None;
    }
    // Space (not 'T' — that's ISO handled elsewhere)
    if b[10] != b' ' {
        return None;
    }
    // HH:MM:SS
    if !b[11].is_ascii_digit()
        || !b[12].is_ascii_digit()
        || b[13] != b':'
        || !b[14].is_ascii_digit()
        || !b[15].is_ascii_digit()
        || b[16] != b':'
        || !b[17].is_ascii_digit()
        || !b[18].is_ascii_digit()
    {
        return None;
    }
    let mut end = 19;
    // Optional fractional seconds: `.mmm` or `,mmm`
    if end < s.len() && (b[end] == b'.' || b[end] == b',') {
        end += 1;
        while end < s.len() && b[end].is_ascii_digit() {
            end += 1;
        }
    }
    // Optional timezone: Z, +HH:MM, +HHMM, +HH
    if end < s.len() {
        if b[end] == b'Z' {
            end += 1;
        } else if (b[end] == b'+' || b[end] == b'-') && end + 2 < s.len() {
            let tz_start = end;
            end += 1;
            // digits
            while end < s.len() && (b[end].is_ascii_digit() || b[end] == b':') {
                end += 1;
            }
            // Validate we consumed something meaningful (at least +HH)
            if end - tz_start < 3 {
                end = tz_start; // revert
            }
        }
    }
    Some((&s[..end], end))
}

/// Parse `YYYY/MM/DD HH:MM:SS[.frac]` (nginx error log, Go standard log).
pub(crate) fn parse_slash_datetime(s: &str) -> Option<(&str, usize)> {
    // Minimum: "YYYY/MM/DD HH:MM:SS" = 19 chars
    if s.len() < 19 {
        return None;
    }
    let b = s.as_bytes();
    if !b[0].is_ascii_digit()
        || !b[1].is_ascii_digit()
        || !b[2].is_ascii_digit()
        || !b[3].is_ascii_digit()
        || b[4] != b'/'
        || !b[5].is_ascii_digit()
        || !b[6].is_ascii_digit()
        || b[7] != b'/'
        || !b[8].is_ascii_digit()
        || !b[9].is_ascii_digit()
        || b[10] != b' '
    {
        return None;
    }
    if !b[11].is_ascii_digit()
        || !b[12].is_ascii_digit()
        || b[13] != b':'
        || !b[14].is_ascii_digit()
        || !b[15].is_ascii_digit()
        || b[16] != b':'
        || !b[17].is_ascii_digit()
        || !b[18].is_ascii_digit()
    {
        return None;
    }
    let mut end = 19;
    // Optional fractional seconds
    if end < s.len() && b[end] == b'.' {
        end += 1;
        while end < s.len() && b[end].is_ascii_digit() {
            end += 1;
        }
    }
    Some((&s[..end], end))
}

/// Parse dmesg kernel timestamp: `[ seconds.usecs]` or `[seconds.usecs]`.
/// Handles variable-width seconds with optional leading spaces.
pub(crate) fn parse_dmesg_timestamp(s: &str) -> Option<(&str, usize)> {
    if s.len() < 4 {
        return None;
    }
    let b = s.as_bytes();
    if b[0] != b'[' {
        return None;
    }
    // Skip spaces after '['
    let mut pos = 1;
    while pos < b.len() && b[pos] == b' ' {
        pos += 1;
    }
    // Need at least one digit
    if pos >= b.len() || !b[pos].is_ascii_digit() {
        return None;
    }
    // Consume digits (seconds)
    while pos < b.len() && b[pos].is_ascii_digit() {
        pos += 1;
    }
    // Must have a dot
    if pos >= b.len() || b[pos] != b'.' {
        return None;
    }
    pos += 1;
    // Consume fractional digits
    let frac_start = pos;
    while pos < b.len() && b[pos].is_ascii_digit() {
        pos += 1;
    }
    if pos == frac_start {
        return None;
    }
    // Must end with ']'
    if pos >= b.len() || b[pos] != b']' {
        return None;
    }
    pos += 1;
    Some((&s[..pos], pos))
}

/// Parse Apache 2.4 error log timestamp: `[Www Mmm DD HH:MM:SS.usecs YYYY]` or
/// `[Www Mmm DD HH:MM:SS YYYY]`.
pub(crate) fn parse_apache_error_timestamp(s: &str) -> Option<(&str, usize)> {
    // Minimum: "[Mon Jan 01 00:00:00 2024]" = 26 chars
    if s.len() < 26 {
        return None;
    }
    let b = s.as_bytes();
    if b[0] != b'[' {
        return None;
    }
    let weekday = &s[1..4];
    if !WEEKDAYS.contains(&weekday) {
        return None;
    }
    if b[4] != b' ' {
        return None;
    }
    let month = &s[5..8];
    if !BSD_MONTHS.contains(&month) {
        return None;
    }
    if b[8] != b' ' {
        return None;
    }
    // Find closing bracket
    let close = s[1..].find(']')?;
    let ts = &s[..close + 2]; // includes [ and ]
    Some((ts, close + 2))
}

// ---------------------------------------------------------------------------
// Level normalization
// ---------------------------------------------------------------------------

/// Map a level keyword (case-insensitive) to a canonical uppercase string.
/// Returns `None` for unrecognized tokens.
pub(crate) fn normalize_level(token: &str) -> Option<&'static str> {
    match token.to_ascii_uppercase().as_str() {
        "TRACE" | "TRC" => Some("TRACE"),
        "DEBUG" | "DBG" => Some("DEBUG"),
        "INFO" | "INF" => Some("INFO"),
        "NOTICE" => Some("NOTICE"),
        "WARN" | "WARNING" | "WRN" => Some("WARN"),
        "ERROR" | "ERR" => Some("ERROR"),
        "FATAL" | "FTL" | "CRITICAL" | "CRIT" | "EMERG" | "ALERT" => Some("FATAL"),
        _ => None,
    }
}

/// Check if a token looks like a known log level keyword (case-insensitive).
pub(crate) fn is_level_keyword(token: &str) -> bool {
    normalize_level(token).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ISO timestamp ─────────────────────────────────────────────────

    #[test]
    fn test_parse_iso_timestamp_basic() {
        let (ts, consumed) = parse_iso_timestamp("2024-02-22T10:15:30+0000 rest").unwrap();
        assert_eq!(ts, "2024-02-22T10:15:30+0000");
        assert_eq!(consumed, 24);
    }

    #[test]
    fn test_parse_iso_timestamp_with_colon_offset() {
        let (ts, _) = parse_iso_timestamp("2024-02-22T10:15:30+00:00 rest").unwrap();
        assert_eq!(ts, "2024-02-22T10:15:30+00:00");
    }

    #[test]
    fn test_parse_iso_timestamp_with_microseconds() {
        let (ts, _) = parse_iso_timestamp("2024-02-22T10:15:30.123456+0000 rest").unwrap();
        assert_eq!(ts, "2024-02-22T10:15:30.123456+0000");
    }

    #[test]
    fn test_parse_iso_timestamp_z_suffix() {
        let (ts, _) = parse_iso_timestamp("2024-02-22T10:15:30Z rest").unwrap();
        assert_eq!(ts, "2024-02-22T10:15:30Z");
    }

    #[test]
    fn test_parse_iso_timestamp_too_short() {
        assert!(parse_iso_timestamp("2024-02-22T10:1").is_none());
    }

    #[test]
    fn test_parse_iso_timestamp_not_iso() {
        assert!(parse_iso_timestamp("Feb 22 10:15:30 host").is_none());
    }

    // ── BSD precise timestamp ─────────────────────────────────────────

    #[test]
    fn test_parse_bsd_precise_basic() {
        let (ts, consumed) = parse_bsd_precise_timestamp("Feb 22 10:15:30.123456 rest").unwrap();
        assert_eq!(ts, "Feb 22 10:15:30.123456");
        assert_eq!(consumed, 22);
    }

    #[test]
    fn test_parse_bsd_precise_single_digit_day() {
        let (ts, _) = parse_bsd_precise_timestamp("Feb  5 10:15:30.123456 rest").unwrap();
        assert_eq!(ts, "Feb  5 10:15:30.123456");
    }

    #[test]
    fn test_parse_bsd_precise_no_dot_returns_none() {
        assert!(parse_bsd_precise_timestamp("Feb 22 10:15:30 host").is_none());
    }

    // ── Full timestamp ────────────────────────────────────────────────

    #[test]
    fn test_parse_full_timestamp_basic() {
        let (ts, consumed) = parse_full_timestamp("Mon 2024-02-22 10:15:30 UTC rest").unwrap();
        assert_eq!(ts, "Mon 2024-02-22 10:15:30 UTC");
        assert_eq!(consumed, 27);
    }

    #[test]
    fn test_parse_full_timestamp_long_tz() {
        let (ts, _) = parse_full_timestamp("Fri 2024-12-31 23:59:59 Europe/Berlin rest").unwrap();
        assert_eq!(ts, "Fri 2024-12-31 23:59:59 Europe/Berlin");
    }

    #[test]
    fn test_parse_full_timestamp_not_weekday() {
        assert!(parse_full_timestamp("Xxx 2024-02-22 10:15:30 UTC rest").is_none());
    }

    // ── Datetime timestamp ────────────────────────────────────────────

    #[test]
    fn test_parse_datetime_basic() {
        let (ts, consumed) = parse_datetime_timestamp("2024-01-15 10:30:00 rest").unwrap();
        assert_eq!(ts, "2024-01-15 10:30:00");
        assert_eq!(consumed, 19);
    }

    #[test]
    fn test_parse_datetime_with_millis_dot() {
        let (ts, _) = parse_datetime_timestamp("2024-01-15 10:30:00.123 rest").unwrap();
        assert_eq!(ts, "2024-01-15 10:30:00.123");
    }

    #[test]
    fn test_parse_datetime_with_millis_comma() {
        let (ts, _) = parse_datetime_timestamp("2024-01-15 10:30:00,456 rest").unwrap();
        assert_eq!(ts, "2024-01-15 10:30:00,456");
    }

    #[test]
    fn test_parse_datetime_with_timezone() {
        let (ts, _) = parse_datetime_timestamp("2024-01-15 10:30:00.123+05:30 rest").unwrap();
        assert_eq!(ts, "2024-01-15 10:30:00.123+05:30");
    }

    #[test]
    fn test_parse_datetime_with_z() {
        let (ts, _) = parse_datetime_timestamp("2024-01-15 10:30:00Z rest").unwrap();
        assert_eq!(ts, "2024-01-15 10:30:00Z");
    }

    #[test]
    fn test_parse_datetime_not_datetime() {
        assert!(parse_datetime_timestamp("not a timestamp").is_none());
    }

    #[test]
    fn test_parse_datetime_iso_not_datetime() {
        // ISO timestamps have 'T' not ' ' at position 10
        assert!(parse_datetime_timestamp("2024-01-15T10:30:00 rest").is_none());
    }

    // ── Slash datetime ────────────────────────────────────────────────

    #[test]
    fn test_parse_slash_datetime_basic() {
        let (ts, consumed) = parse_slash_datetime("2024/01/15 10:30:00 rest").unwrap();
        assert_eq!(ts, "2024/01/15 10:30:00");
        assert_eq!(consumed, 19);
    }

    #[test]
    fn test_parse_slash_datetime_with_frac() {
        let (ts, _) = parse_slash_datetime("2024/01/15 10:30:00.123456 rest").unwrap();
        assert_eq!(ts, "2024/01/15 10:30:00.123456");
    }

    #[test]
    fn test_parse_slash_datetime_not_slash() {
        assert!(parse_slash_datetime("2024-01-15 10:30:00 rest").is_none());
    }

    // ── dmesg timestamp ───────────────────────────────────────────────

    #[test]
    fn test_parse_dmesg_basic() {
        let (ts, consumed) = parse_dmesg_timestamp("[    0.000000] rest").unwrap();
        assert_eq!(ts, "[    0.000000]");
        assert_eq!(consumed, 14);
    }

    #[test]
    fn test_parse_dmesg_large_seconds() {
        let (ts, _) = parse_dmesg_timestamp("[12345.678901] rest").unwrap();
        assert_eq!(ts, "[12345.678901]");
    }

    #[test]
    fn test_parse_dmesg_no_bracket() {
        assert!(parse_dmesg_timestamp("12345.678901] rest").is_none());
    }

    #[test]
    fn test_parse_dmesg_no_dot() {
        assert!(parse_dmesg_timestamp("[12345] rest").is_none());
    }

    // ── Apache error timestamp ────────────────────────────────────────

    #[test]
    fn test_parse_apache_error_basic() {
        let (ts, consumed) =
            parse_apache_error_timestamp("[Mon Jan 15 10:30:00.123456 2024] rest").unwrap();
        assert_eq!(ts, "[Mon Jan 15 10:30:00.123456 2024]");
        assert_eq!(consumed, 33);
    }

    #[test]
    fn test_parse_apache_error_no_frac() {
        let (ts, _) = parse_apache_error_timestamp("[Fri Dec 31 23:59:59 2024] rest").unwrap();
        assert_eq!(ts, "[Fri Dec 31 23:59:59 2024]");
    }

    #[test]
    fn test_parse_apache_error_not_bracket() {
        assert!(parse_apache_error_timestamp("Mon Jan 15 10:30:00 2024] rest").is_none());
    }

    #[test]
    fn test_parse_apache_error_bad_weekday() {
        assert!(parse_apache_error_timestamp("[Xxx Jan 15 10:30:00 2024] rest").is_none());
    }

    #[test]
    fn test_parse_apache_error_bad_month() {
        assert!(parse_apache_error_timestamp("[Mon Xxx 15 10:30:00 2024] rest").is_none());
    }

    // ── normalize_level ───────────────────────────────────────────────

    #[test]
    fn test_normalize_level_standard() {
        assert_eq!(normalize_level("INFO"), Some("INFO"));
        assert_eq!(normalize_level("info"), Some("INFO"));
        assert_eq!(normalize_level("WARN"), Some("WARN"));
        assert_eq!(normalize_level("WARNING"), Some("WARN"));
        assert_eq!(normalize_level("ERROR"), Some("ERROR"));
        assert_eq!(normalize_level("ERR"), Some("ERROR"));
        assert_eq!(normalize_level("DEBUG"), Some("DEBUG"));
        assert_eq!(normalize_level("DBG"), Some("DEBUG"));
        assert_eq!(normalize_level("TRACE"), Some("TRACE"));
        assert_eq!(normalize_level("TRC"), Some("TRACE"));
    }

    #[test]
    fn test_normalize_level_fatal_family() {
        assert_eq!(normalize_level("FATAL"), Some("FATAL"));
        assert_eq!(normalize_level("FTL"), Some("FATAL"));
        assert_eq!(normalize_level("CRITICAL"), Some("FATAL"));
        assert_eq!(normalize_level("CRIT"), Some("FATAL"));
        assert_eq!(normalize_level("EMERG"), Some("FATAL"));
        assert_eq!(normalize_level("ALERT"), Some("FATAL"));
    }

    #[test]
    fn test_normalize_level_notice() {
        assert_eq!(normalize_level("NOTICE"), Some("NOTICE"));
        assert_eq!(normalize_level("notice"), Some("NOTICE"));
    }

    #[test]
    fn test_normalize_level_unknown() {
        assert_eq!(normalize_level("myapp"), None);
        assert_eq!(normalize_level("server"), None);
        assert_eq!(normalize_level(""), None);
    }

    #[test]
    fn test_is_level_keyword() {
        assert!(is_level_keyword("INFO"));
        assert!(is_level_keyword("warn"));
        assert!(!is_level_keyword("myhost"));
        assert!(!is_level_keyword("sshd"));
    }
}
