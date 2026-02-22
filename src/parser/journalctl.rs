// ---------------------------------------------------------------------------
// Journalctl text output parser
// ---------------------------------------------------------------------------
//
// Handles `journalctl` output formats that the SyslogParser does not cover:
//   - short-iso:     2024-02-22T10:15:30+0000 hostname unit[pid]: message
//   - short-precise: Feb 22 10:15:30.123456 hostname unit[pid]: message
//   - short-full:    Mon 2024-02-22 10:15:30 UTC hostname unit[pid]: message
//
// The default `journalctl -o short` output (BSD timestamp without priority)
// is already handled by `SyslogParser`, so `JournalctlParser` yields a lower
// detect_score when the input looks like plain BSD syslog.

use std::collections::HashSet;

use super::types::{DisplayParts, LogFormatParser};

/// Zero-copy parser for journalctl text output.
#[derive(Debug)]
pub struct JournalctlParser;

/// Weekday abbreviations used by `journalctl -o short-full`.
const WEEKDAYS: &[&str] = &["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// BSD month abbreviations (shared with syslog, but we avoid cross-module dependency).
const BSD_MONTHS: &[&str] = &[
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

// ---------------------------------------------------------------------------
// Timestamp parsers — each returns `(timestamp_slice, bytes_consumed)`.
// ---------------------------------------------------------------------------

/// Try ISO 8601 timestamp: `2024-02-22T10:15:30+0000` or `2024-02-22T10:15:30.123456+00:00`.
/// Stops at the first space after the 'T'.
fn parse_iso_timestamp(s: &str) -> Option<(&str, usize)> {
    // Minimum: "YYYY-MM-DDTHH:MM:SS" = 19 chars
    if s.len() < 19 {
        return None;
    }
    let bytes = s.as_bytes();
    // Must start with 4 digits (year)
    if !bytes[0].is_ascii_digit()
        || !bytes[1].is_ascii_digit()
        || !bytes[2].is_ascii_digit()
        || !bytes[3].is_ascii_digit()
        || bytes[4] != b'-'
    {
        return None;
    }
    // Must contain 'T' at position 10
    if bytes[10] != b'T' {
        return None;
    }
    // Find the end: first space after position 10
    let end = s[10..].find(' ').map(|p| p + 10).unwrap_or(s.len());
    if end <= 10 {
        return None;
    }
    Some((&s[..end], end))
}

/// Try BSD timestamp with microseconds: `Feb 22 10:15:30.123456`.
/// Returns the full timestamp including microseconds.
fn parse_bsd_precise_timestamp(s: &str) -> Option<(&str, usize)> {
    // Minimum: "Mmm DD HH:MM:SS.U" = 16 chars (at least one microsecond digit)
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
    // Day: " D" or "DD"
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
    // HH:MM:SS
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
    // Must have a '.' for microseconds (this distinguishes from plain BSD)
    if after_time >= s.len() || s.as_bytes()[after_time] != b'.' {
        return None;
    }
    // Consume digits after the dot
    let mut end = after_time + 1;
    while end < s.len() && s.as_bytes()[end].is_ascii_digit() {
        end += 1;
    }
    // Must have consumed at least one digit after '.'
    if end == after_time + 1 {
        return None;
    }
    Some((&s[..end], end))
}

/// Try short-full timestamp: `Mon 2024-02-22 10:15:30 UTC`.
/// Format: `Www YYYY-MM-DD HH:MM:SS TZ`
fn parse_full_timestamp(s: &str) -> Option<(&str, usize)> {
    // Minimum: "Mon 2024-02-22 10:15:30 UTC" = 27 chars
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
    // YYYY-MM-DD at offset 4
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
    // HH:MM:SS at offset 15
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
    // Space then timezone
    if s.as_bytes()[23] != b' ' {
        return None;
    }
    // Timezone: consume non-space chars
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

/// Extract hostname + unit[pid]: message after a timestamp.
/// Input `rest` should start right after the timestamp (leading space expected).
fn parse_host_unit_message<'a>(rest: &'a str) -> Option<DisplayParts<'a>> {
    let rest = rest.strip_prefix(' ')?;
    if rest.is_empty() {
        return None;
    }

    let mut parts = DisplayParts::default();

    // hostname (next space-delimited token)
    let space = rest.find(' ')?;
    let hostname = &rest[..space];
    parts.extra_fields.push(("hostname", hostname));
    let rest = &rest[space + 1..];

    if rest.is_empty() {
        return Some(parts);
    }

    // unit[pid]: message
    if let Some(colon_pos) = rest.find(": ") {
        let tag = &rest[..colon_pos];
        extract_unit_pid(tag, &mut parts);
        let message = &rest[colon_pos + 2..];
        if !message.is_empty() {
            parts.message = Some(message);
        }
    } else if let Some(colon_pos) = rest.find(':') {
        let tag = &rest[..colon_pos];
        extract_unit_pid(tag, &mut parts);
        let message = rest[colon_pos + 1..].trim_start();
        if !message.is_empty() {
            parts.message = Some(message);
        }
    } else {
        // No colon — treat as message
        parts.message = Some(rest);
    }

    Some(parts)
}

/// Extract unit name and optional PID from a tag like `sshd[1234]` or `kernel`.
fn extract_unit_pid<'a>(tag: &'a str, parts: &mut DisplayParts<'a>) {
    if let Some(bracket_start) = tag.find('[') {
        let unit = &tag[..bracket_start];
        parts.target = Some(unit);
        if let Some(bracket_end) = tag[bracket_start..].find(']') {
            let pid = &tag[bracket_start + 1..bracket_start + bracket_end];
            parts.extra_fields.push(("pid", pid));
        }
    } else {
        parts.target = Some(tag);
    }
}

impl LogFormatParser for JournalctlParser {
    fn parse_line<'a>(&self, line: &'a [u8]) -> Option<DisplayParts<'a>> {
        let s = std::str::from_utf8(line).ok()?;
        if s.is_empty() {
            return None;
        }

        // Skip journalctl header/footer lines
        if s.starts_with("-- ") {
            return None;
        }

        // Try short-full first (weekday prefix is most distinctive)
        if let Some((timestamp, consumed)) = parse_full_timestamp(s) {
            let mut parts = parse_host_unit_message(&s[consumed..])?;
            parts.timestamp = Some(timestamp);
            return Some(parts);
        }

        // Try ISO 8601 timestamp (short-iso)
        if let Some((timestamp, consumed)) = parse_iso_timestamp(s) {
            let mut parts = parse_host_unit_message(&s[consumed..])?;
            parts.timestamp = Some(timestamp);
            return Some(parts);
        }

        // Try BSD with microseconds (short-precise)
        if let Some((timestamp, consumed)) = parse_bsd_precise_timestamp(s) {
            let mut parts = parse_host_unit_message(&s[consumed..])?;
            parts.timestamp = Some(timestamp);
            return Some(parts);
        }

        None
    }

    fn collect_field_names(&self, lines: &[&[u8]]) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut extras = Vec::new();

        for &line in lines {
            if let Some(parts) = self.parse_line(line) {
                for (key, _) in &parts.extra_fields {
                    let k = key.to_string();
                    if seen.insert(k.clone()) {
                        extras.push(k);
                    }
                }
            }
        }

        let mut result = vec!["timestamp".to_string(), "target".to_string()];
        extras.sort();
        extras.dedup();
        result.extend(extras);
        result.push("message".to_string());
        result
    }

    fn detect_score(&self, sample: &[&[u8]]) -> f64 {
        if sample.is_empty() {
            return 0.0;
        }
        let mut parsed = 0usize;
        for &line in sample {
            if self.parse_line(line).is_some() {
                parsed += 1;
            }
        }
        if parsed == 0 {
            return 0.0;
        }
        parsed as f64 / sample.len() as f64
    }

    fn name(&self) -> &str {
        "journalctl"
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── ISO timestamp parsing ──────────────────────────────────────────

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

    // ── BSD precise timestamp parsing ──────────────────────────────────

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
        // Plain BSD timestamp without microseconds should NOT match
        assert!(parse_bsd_precise_timestamp("Feb 22 10:15:30 host").is_none());
    }

    #[test]
    fn test_parse_bsd_precise_not_bsd() {
        assert!(parse_bsd_precise_timestamp("2024-02-22T10:15:30 host").is_none());
    }

    // ── Full timestamp parsing ─────────────────────────────────────────

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

    #[test]
    fn test_parse_full_timestamp_too_short() {
        assert!(parse_full_timestamp("Mon 2024-02-22 10:15:30").is_none());
    }

    // ── JournalctlParser: short-iso format ─────────────────────────────

    #[test]
    fn test_short_iso_full_line() {
        let line = b"2024-02-22T10:15:30+0000 myhost sshd[1234]: Accepted password for user";
        let parser = JournalctlParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024-02-22T10:15:30+0000"));
        assert_eq!(parts.target, Some("sshd"));
        assert_eq!(parts.message, Some("Accepted password for user"));
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "hostname" && *v == "myhost")
        );
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "pid" && *v == "1234")
        );
    }

    #[test]
    fn test_short_iso_no_pid() {
        let line = b"2024-02-22T10:15:30+0000 myhost kernel: something happened";
        let parser = JournalctlParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.target, Some("kernel"));
        assert!(!parts.extra_fields.iter().any(|(k, _)| *k == "pid"));
        assert_eq!(parts.message, Some("something happened"));
    }

    #[test]
    fn test_short_iso_z_suffix() {
        let line = b"2024-02-22T10:15:30Z myhost systemd[1]: Started Service.";
        let parser = JournalctlParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024-02-22T10:15:30Z"));
        assert_eq!(parts.target, Some("systemd"));
    }

    // ── JournalctlParser: short-precise format ─────────────────────────

    #[test]
    fn test_short_precise_full_line() {
        let line = b"Feb 22 10:15:30.123456 myhost sshd[5678]: Connection closed";
        let parser = JournalctlParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("Feb 22 10:15:30.123456"));
        assert_eq!(parts.target, Some("sshd"));
        assert_eq!(parts.message, Some("Connection closed"));
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "pid" && *v == "5678")
        );
    }

    #[test]
    fn test_short_precise_single_digit_day() {
        let line = b"Feb  5 10:15:30.999 myhost app: msg";
        let parser = JournalctlParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("Feb  5 10:15:30.999"));
    }

    // ── JournalctlParser: short-full format ────────────────────────────

    #[test]
    fn test_short_full_full_line() {
        let line = b"Mon 2024-02-22 10:15:30 UTC myhost sshd[1234]: Accepted key";
        let parser = JournalctlParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("Mon 2024-02-22 10:15:30 UTC"));
        assert_eq!(parts.target, Some("sshd"));
        assert_eq!(parts.message, Some("Accepted key"));
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "hostname" && *v == "myhost")
        );
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "pid" && *v == "1234")
        );
    }

    #[test]
    fn test_short_full_long_timezone() {
        let line = b"Fri 2024-12-31 23:59:59 Europe/Berlin myhost app[99]: year end";
        let parser = JournalctlParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(
            parts.timestamp,
            Some("Fri 2024-12-31 23:59:59 Europe/Berlin")
        );
        assert_eq!(parts.target, Some("app"));
    }

    // ── Edge cases ─────────────────────────────────────────────────────

    #[test]
    fn test_parse_empty_line() {
        let parser = JournalctlParser;
        assert!(parser.parse_line(b"").is_none());
    }

    #[test]
    fn test_parse_journal_header_skipped() {
        let parser = JournalctlParser;
        assert!(
            parser
                .parse_line(b"-- Journal begins at Mon 2024-01-01 00:00:00 UTC. --")
                .is_none()
        );
    }

    #[test]
    fn test_parse_no_entries_skipped() {
        let parser = JournalctlParser;
        assert!(parser.parse_line(b"-- No entries --").is_none());
    }

    #[test]
    fn test_parse_plain_text_not_journalctl() {
        let parser = JournalctlParser;
        assert!(parser.parse_line(b"just plain text").is_none());
    }

    #[test]
    fn test_parse_json_not_journalctl() {
        let parser = JournalctlParser;
        assert!(
            parser
                .parse_line(br#"{"level":"INFO","msg":"hello"}"#)
                .is_none()
        );
    }

    #[test]
    fn test_parse_syslog_with_priority_not_journalctl() {
        let parser = JournalctlParser;
        // Syslog with <PRI> prefix should not match journalctl
        assert!(
            parser
                .parse_line(b"<134>Oct 11 22:14:15 myhost sshd[1234]: msg")
                .is_none()
        );
    }

    #[test]
    fn test_unit_without_colon() {
        let line = b"2024-02-22T10:15:30+0000 myhost kernel";
        let parser = JournalctlParser;
        // hostname consumed, then "kernel" has no colon — treated as message
        let parts = parser.parse_line(line).unwrap();
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "hostname" && *v == "myhost")
        );
        assert_eq!(parts.message, Some("kernel"));
    }

    // ── detect_score ───────────────────────────────────────────────────

    #[test]
    fn test_detect_score_all_journalctl() {
        let parser = JournalctlParser;
        let lines: Vec<&[u8]> = vec![
            b"2024-02-22T10:15:30+0000 host1 sshd[1]: msg1",
            b"2024-02-22T10:15:31+0000 host1 sshd[1]: msg2",
        ];
        let score = parser.detect_score(&lines);
        assert!((score - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_detect_score_mixed() {
        let parser = JournalctlParser;
        let lines: Vec<&[u8]> = vec![
            b"2024-02-22T10:15:30+0000 host1 sshd[1]: msg1",
            b"not journalctl at all",
        ];
        let score = parser.detect_score(&lines);
        assert!((score - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_detect_score_none() {
        let parser = JournalctlParser;
        let lines: Vec<&[u8]> = vec![b"plain text", b"more plain text"];
        let score = parser.detect_score(&lines);
        assert!((score - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_detect_score_empty() {
        let parser = JournalctlParser;
        let lines: Vec<&[u8]> = vec![];
        let score = parser.detect_score(&lines);
        assert!((score - 0.0).abs() < 0.001);
    }

    // ── collect_field_names ────────────────────────────────────────────

    #[test]
    fn test_collect_field_names_iso() {
        let parser = JournalctlParser;
        let lines: Vec<&[u8]> = vec![b"2024-02-22T10:15:30+0000 myhost sshd[1234]: msg"];
        let names = parser.collect_field_names(&lines);
        assert_eq!(names[0], "timestamp");
        assert_eq!(names[1], "target");
        assert!(names.contains(&"hostname".to_string()));
        assert!(names.contains(&"pid".to_string()));
        assert_eq!(*names.last().unwrap(), "message");
    }

    #[test]
    fn test_collect_field_names_no_pid() {
        let parser = JournalctlParser;
        let lines: Vec<&[u8]> = vec![b"2024-02-22T10:15:30+0000 myhost kernel: msg"];
        let names = parser.collect_field_names(&lines);
        assert!(names.contains(&"hostname".to_string()));
        assert!(!names.contains(&"pid".to_string()));
    }

    // ── name ───────────────────────────────────────────────────────────

    #[test]
    fn test_name() {
        let parser = JournalctlParser;
        assert_eq!(parser.name(), "journalctl");
    }
}
