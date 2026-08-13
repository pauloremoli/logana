//! Journalctl text output parser.
//!
//! Supported output modes (journalctl --output):
//! - `short`             BSD timestamp without fractional seconds
//! - `short-iso`         ISO-8601 timestamp
//! - `short-iso-precise` ISO-8601 with microseconds (handled by iso parser)
//! - `short-full`        Weekday + date + time + timezone
//! - `short-precise`     BSD timestamp with microseconds
//! - `short-monotonic`   Boot-relative monotonic clock in brackets
//! - `short-unix`        Unix epoch with decimal seconds
//! - `with-unit`         Like `short` but identifies source by unit name
//!
//! Not supported (multi-line formats): `verbose`, `export`, `json-pretty`.

use std::collections::HashSet;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

use super::timestamp::{
    is_level_keyword, parse_bsd_plain_timestamp, parse_bsd_precise_timestamp, parse_full_timestamp,
    parse_iso_timestamp, parse_monotonic_timestamp, parse_unix_timestamp,
};
use super::types::{DisplayParts, FieldSemantic, LogFormatParser, push_field_as};

#[derive(Debug, Clone, Copy)]
enum JournalFormat {
    Full,
    Iso,
    BsdPrecise,
    BsdPlain,
    Monotonic,
    Unix,
}

impl JournalFormat {
    fn index(self) -> usize {
        self as usize
    }

    fn from_index(i: usize) -> Self {
        match i {
            0 => Self::Full,
            1 => Self::Iso,
            2 => Self::BsdPrecise,
            3 => Self::BsdPlain,
            4 => Self::Monotonic,
            _ => Self::Unix,
        }
    }
}

const MIN_SAMPLES: u32 = 50;

fn detect_journal_timestamp(s: &str) -> Option<(JournalFormat, &str)> {
    if let Some((ts, _)) = parse_full_timestamp(s) {
        return Some((JournalFormat::Full, ts));
    }
    if let Some((ts, _)) = parse_iso_timestamp(s) {
        return Some((JournalFormat::Iso, ts));
    }
    if let Some((ts, _)) = parse_bsd_precise_timestamp(s) {
        return Some((JournalFormat::BsdPrecise, ts));
    }
    if let Some((ts, _)) = parse_bsd_plain_timestamp(s) {
        return Some((JournalFormat::BsdPlain, ts));
    }
    if let Some((ts, _)) = parse_monotonic_timestamp(s) {
        return Some((JournalFormat::Monotonic, ts));
    }
    parse_unix_timestamp(s).map(|(ts, _)| (JournalFormat::Unix, ts))
}

fn extract_journal_timestamp(s: &str, fmt: JournalFormat) -> Option<&str> {
    match fmt {
        JournalFormat::Full => parse_full_timestamp(s).map(|(ts, _)| ts),
        JournalFormat::Iso => parse_iso_timestamp(s).map(|(ts, _)| ts),
        JournalFormat::BsdPrecise => parse_bsd_precise_timestamp(s).map(|(ts, _)| ts),
        JournalFormat::BsdPlain => parse_bsd_plain_timestamp(s).map(|(ts, _)| ts),
        JournalFormat::Monotonic => parse_monotonic_timestamp(s).map(|(ts, _)| ts),
        JournalFormat::Unix => parse_unix_timestamp(s).map(|(ts, _)| ts),
    }
}

#[derive(Debug, Default)]
pub struct JournalctlParser {
    format: OnceLock<JournalFormat>,
    fmt_counts: [AtomicU32; 6],
    fmt_total: AtomicU32,
}

fn parse_journal_line_by_format<'a>(s: &'a str, fmt: JournalFormat) -> Option<DisplayParts<'a>> {
    let (timestamp, consumed) = match fmt {
        JournalFormat::Full => parse_full_timestamp(s)?,
        JournalFormat::Iso => parse_iso_timestamp(s)?,
        JournalFormat::BsdPrecise => parse_bsd_precise_timestamp(s)?,
        JournalFormat::BsdPlain => parse_bsd_plain_timestamp(s)?,
        JournalFormat::Monotonic => parse_monotonic_timestamp(s)?,
        JournalFormat::Unix => parse_unix_timestamp(s)?,
    };
    let mut parts = parse_host_unit_message(&s[consumed..])?;
    parts.timestamp = Some(timestamp);
    Some(parts)
}

/// Rejects known level keywords, `::` module paths, and short all-uppercase
/// tokens to prevent false positives against common-log format lines.
fn is_likely_hostname(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    if is_level_keyword(token) {
        return false;
    }
    if token.contains("::") {
        return false;
    }
    if token.len() <= 8
        && token
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        return false;
    }
    true
}

fn parse_host_unit_message<'a>(rest: &'a str) -> Option<DisplayParts<'a>> {
    let rest = rest.strip_prefix(' ')?;
    if rest.is_empty() {
        return None;
    }

    let mut parts = DisplayParts::default();

    let space = rest.find(' ')?;
    let hostname = &rest[..space];

    if !is_likely_hostname(hostname) {
        return None;
    }

    push_field_as(&mut parts.extra_fields, FieldSemantic::Hostname, hostname);
    let rest = &rest[space + 1..];

    if rest.is_empty() {
        return Some(parts);
    }

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
        parts.message = Some(rest);
    }

    Some(parts)
}

fn extract_unit_pid<'a>(tag: &'a str, parts: &mut DisplayParts<'a>) {
    if let Some(bracket_start) = tag.find('[') {
        let unit = &tag[..bracket_start];
        parts.target = Some(unit);
        if let Some(bracket_end) = tag[bracket_start..].find(']') {
            let pid = &tag[bracket_start + 1..bracket_start + bracket_end];
            push_field_as(&mut parts.extra_fields, FieldSemantic::Pid, pid);
        }
    } else {
        parts.target = Some(tag);
    }
}

impl JournalctlParser {
    fn record_format(&self, fmt: JournalFormat) {
        self.fmt_counts[fmt.index()].fetch_add(1, Ordering::Relaxed);
        let total = self.fmt_total.fetch_add(1, Ordering::Relaxed) + 1;
        if total >= MIN_SAMPLES && self.format.get().is_none() {
            let winner = (0..6)
                .max_by_key(|&i| self.fmt_counts[i].load(Ordering::Relaxed))
                .unwrap_or(0);
            let _ = self.format.set(JournalFormat::from_index(winner));
        }
    }
}

impl LogFormatParser for JournalctlParser {
    fn timestamp_has_year(&self) -> bool {
        !matches!(
            self.format.get(),
            Some(JournalFormat::BsdPrecise | JournalFormat::BsdPlain)
        )
    }

    fn parse_timestamp<'a>(&self, line: &'a [u8]) -> Option<&'a str> {
        let s = std::str::from_utf8(line).ok()?;
        if s.is_empty() || s.starts_with("-- ") {
            return None;
        }
        if let Some(&fmt) = self.format.get() {
            return extract_journal_timestamp(s, fmt);
        }
        let (fmt, ts) = detect_journal_timestamp(s)?;
        self.record_format(fmt);
        Some(ts)
    }

    fn parse_line<'a>(&self, line: &'a [u8]) -> Option<DisplayParts<'a>> {
        let s = std::str::from_utf8(line).ok()?;
        if s.is_empty() || s.starts_with("-- ") {
            return None;
        }

        if let Some(&fmt) = self.format.get() {
            let result = parse_journal_line_by_format(s, fmt);
            if result.is_some() {
                return result;
            }
        }

        type TsFn = fn(&str) -> Option<(&str, usize)>;
        let parsers: &[(JournalFormat, TsFn)] = &[
            (JournalFormat::Full, parse_full_timestamp),
            (JournalFormat::Iso, parse_iso_timestamp),
            (JournalFormat::BsdPrecise, parse_bsd_precise_timestamp),
            (JournalFormat::BsdPlain, parse_bsd_plain_timestamp),
            (JournalFormat::Monotonic, parse_monotonic_timestamp),
            (JournalFormat::Unix, parse_unix_timestamp),
        ];
        for &(fmt, parser) in parsers {
            if let Some((timestamp, consumed)) = parser(s)
                && let Some(mut parts) = parse_host_unit_message(&s[consumed..])
            {
                parts.timestamp = Some(timestamp);
                self.record_format(fmt);
                return Some(parts);
            }
        }

        None
    }

    fn collect_field_names(&self, lines: &[&[u8]]) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut extras = Vec::new();

        for &line in lines {
            if let Some(parts) = self.parse_line(line) {
                for (_, key, _) in &parts.extra_fields {
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

    /// Slight penalty relative to SyslogParser (1.0) so that lines shared
    /// between the two formats (ISO timestamp, plain BSD) resolve to syslog.
    /// Journalctl-exclusive formats (full, monotonic, unix, precise BSD) still
    /// win because only this parser matches them.
    fn detection_weight(&self) -> f64 {
        0.99
    }

    fn name(&self) -> &str {
        "journalctl"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_short_iso_full_line() {
        let line = b"2024-02-22T10:15:30+0000 myhost sshd[1234]: Accepted password for user";
        let parser = JournalctlParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024-02-22T10:15:30+0000"));
        assert_eq!(parts.target, Some("sshd"));
        assert_eq!(parts.message, Some("Accepted password for user"));
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(_, k, v)| *k == "hostname" && *v == "myhost")
        );
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(_, k, v)| *k == "pid" && *v == "1234")
        );
    }

    #[test]
    fn test_short_iso_no_pid() {
        let line = b"2024-02-22T10:15:30+0000 myhost kernel: something happened";
        let parser = JournalctlParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.target, Some("kernel"));
        assert!(!parts.extra_fields.iter().any(|(_, k, _)| *k == "pid"));
        assert_eq!(parts.message, Some("something happened"));
    }

    #[test]
    fn test_short_iso_z_suffix() {
        let line = b"2024-02-22T10:15:30Z myhost systemd[1]: Started Service.";
        let parser = JournalctlParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024-02-22T10:15:30Z"));
        assert_eq!(parts.target, Some("systemd"));
    }

    #[test]
    fn test_short_precise_full_line() {
        let line = b"Feb 22 10:15:30.123456 myhost sshd[5678]: Connection closed";
        let parser = JournalctlParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("Feb 22 10:15:30.123456"));
        assert_eq!(parts.target, Some("sshd"));
        assert_eq!(parts.message, Some("Connection closed"));
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(_, k, v)| *k == "pid" && *v == "5678")
        );
    }

    #[test]
    fn test_short_precise_single_digit_day() {
        let line = b"Feb  5 10:15:30.999 myhost app: msg";
        let parser = JournalctlParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("Feb  5 10:15:30.999"));
    }

    #[test]
    fn test_short_full_full_line() {
        let line = b"Mon 2024-02-22 10:15:30 UTC myhost sshd[1234]: Accepted key";
        let parser = JournalctlParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("Mon 2024-02-22 10:15:30 UTC"));
        assert_eq!(parts.target, Some("sshd"));
        assert_eq!(parts.message, Some("Accepted key"));
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(_, k, v)| *k == "hostname" && *v == "myhost")
        );
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(_, k, v)| *k == "pid" && *v == "1234")
        );
    }

    #[test]
    fn test_short_full_long_timezone() {
        let line = b"Fri 2024-12-31 23:59:59 Europe/Berlin myhost app[99]: year end";
        let parser = JournalctlParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(
            parts.timestamp,
            Some("Fri 2024-12-31 23:59:59 Europe/Berlin")
        );
        assert_eq!(parts.target, Some("app"));
    }

    #[test]
    fn test_parse_empty_line() {
        let parser = JournalctlParser::default();
        assert!(parser.parse_line(b"").is_none());
    }

    #[test]
    fn test_parse_journal_header_skipped() {
        let parser = JournalctlParser::default();
        assert!(
            parser
                .parse_line(b"-- Journal begins at Mon 2024-01-01 00:00:00 UTC. --")
                .is_none()
        );
    }

    #[test]
    fn test_parse_no_entries_skipped() {
        let parser = JournalctlParser::default();
        assert!(parser.parse_line(b"-- No entries --").is_none());
    }

    #[test]
    fn test_parse_plain_text_not_journalctl() {
        let parser = JournalctlParser::default();
        assert!(parser.parse_line(b"just plain text").is_none());
    }

    #[test]
    fn test_parse_json_not_journalctl() {
        let parser = JournalctlParser::default();
        assert!(
            parser
                .parse_line(br#"{"level":"INFO","msg":"hello"}"#)
                .is_none()
        );
    }

    #[test]
    fn test_parse_syslog_with_priority_not_journalctl() {
        let parser = JournalctlParser::default();
        assert!(
            parser
                .parse_line(b"<134>Oct 11 22:14:15 myhost sshd[1234]: msg")
                .is_none()
        );
    }

    #[test]
    fn test_unit_without_colon() {
        let line = b"2024-02-22T10:15:30+0000 myhost kernel";
        let parser = JournalctlParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(_, k, v)| *k == "hostname" && *v == "myhost")
        );
        assert_eq!(parts.message, Some("kernel"));
    }

    #[test]
    fn test_short_full_line() {
        let line = b"Jul 12 22:23:01 hostname sshd[1234]: Accepted password";
        let parser = JournalctlParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("Jul 12 22:23:01"));
        assert_eq!(parts.target, Some("sshd"));
        assert_eq!(parts.message, Some("Accepted password"));
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(_, k, v)| *k == "hostname" && *v == "hostname")
        );
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(_, k, v)| *k == "pid" && *v == "1234")
        );
    }

    #[test]
    fn test_short_single_digit_day() {
        let line = b"Feb  5 10:15:30 myhost kernel: something";
        let parser = JournalctlParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("Feb  5 10:15:30"));
        assert_eq!(parts.message, Some("something"));
    }

    #[test]
    fn test_short_does_not_match_precise() {
        // short-precise (with dot) should NOT be matched by the plain BSD parser
        let line = b"Feb 22 10:15:30.123456 myhost sshd[5678]: msg";
        let parser = JournalctlParser::default();
        let parts = parser.parse_line(line).unwrap();
        // It should still parse via parse_bsd_precise_timestamp
        assert_eq!(parts.timestamp, Some("Feb 22 10:15:30.123456"));
    }

    #[test]
    fn test_short_monotonic_basic() {
        let line = b"[     0.000000] myhost sshd[1]: Started";
        let parser = JournalctlParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("[     0.000000]"));
        assert_eq!(parts.target, Some("sshd"));
        assert_eq!(parts.message, Some("Started"));
    }

    #[test]
    fn test_short_monotonic_large_value() {
        let line = b"[12345.678901] myhost kernel: IRQ routing";
        let parser = JournalctlParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("[12345.678901]"));
        assert_eq!(parts.target, Some("kernel"));
    }

    #[test]
    fn test_short_unix_basic() {
        let line = b"1436735381.000000 myhost sshd[1234]: Connection closed";
        let parser = JournalctlParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("1436735381.000000"));
        assert_eq!(parts.target, Some("sshd"));
        assert_eq!(parts.message, Some("Connection closed"));
    }

    #[test]
    fn test_short_unix_with_pid() {
        let line = b"1700000000.123456 myhost systemd[1]: Started service";
        let parser = JournalctlParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("1700000000.123456"));
        assert_eq!(parts.target, Some("systemd"));
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(_, k, v)| *k == "pid" && *v == "1")
        );
    }

    #[test]
    fn test_reject_common_log_format_line() {
        // env_logger style: ISO timestamp + level keyword where hostname would be
        let parser = JournalctlParser::default();
        assert!(
            parser
                .parse_line(b"2024-07-24T10:00:00Z INFO myapp::server: listening on 0.0.0.0:3000")
                .is_none()
        );
    }

    #[test]
    fn test_reject_tracing_fmt_line() {
        let parser = JournalctlParser::default();
        assert!(
            parser
                .parse_line(b"2024-07-24T10:00:00Z DEBUG myapp::handler: request processed")
                .is_none()
        );
    }

    #[test]
    fn test_reject_level_as_hostname() {
        let parser = JournalctlParser::default();
        // "WARN" where hostname would be should fail
        assert!(
            parser
                .parse_line(b"2024-07-24T10:00:00Z WARN something: msg")
                .is_none()
        );
    }

    #[test]
    fn test_is_likely_hostname_valid() {
        assert!(is_likely_hostname("myhost"));
        assert!(is_likely_hostname("server-01"));
        assert!(is_likely_hostname("ip-172-31-0-1"));
        assert!(is_likely_hostname("paulo-pc"));
    }

    #[test]
    fn test_is_likely_hostname_rejects_levels() {
        assert!(!is_likely_hostname("INFO"));
        assert!(!is_likely_hostname("WARN"));
        assert!(!is_likely_hostname("ERROR"));
        assert!(!is_likely_hostname("DEBUG"));
        assert!(!is_likely_hostname("TRACE"));
    }

    #[test]
    fn test_is_likely_hostname_rejects_module_paths() {
        assert!(!is_likely_hostname("myapp::server"));
        assert!(!is_likely_hostname("crate::module::func"));
    }

    #[test]
    fn test_is_likely_hostname_rejects_short_uppercase() {
        assert!(!is_likely_hostname("ABC"));
        assert!(!is_likely_hostname("MYAPP"));
    }

    #[test]
    fn test_detect_score_short_format() {
        let parser = JournalctlParser::default();
        let lines: Vec<&[u8]> = vec![
            b"Jul 12 22:23:01 host1 sshd[1]: msg1",
            b"Jul 12 22:23:02 host1 sshd[1]: msg2",
        ];
        let score = parser.detect_score(&lines);
        assert!((score - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_detect_score_short_monotonic() {
        let parser = JournalctlParser::default();
        let lines: Vec<&[u8]> = vec![
            b"[     0.000000] host1 sshd[1]: msg1",
            b"[12345.678901] host1 kernel: msg2",
        ];
        let score = parser.detect_score(&lines);
        assert!((score - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_detect_score_short_unix() {
        let parser = JournalctlParser::default();
        let lines: Vec<&[u8]> = vec![
            b"1436735381.000000 host1 sshd[1]: msg1",
            b"1436735382.000001 host1 sshd[1]: msg2",
        ];
        let score = parser.detect_score(&lines);
        assert!((score - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_detect_score_all_journalctl() {
        let parser = JournalctlParser::default();
        let lines: Vec<&[u8]> = vec![
            b"2024-02-22T10:15:30+0000 host1 sshd[1]: msg1",
            b"2024-02-22T10:15:31+0000 host1 sshd[1]: msg2",
        ];
        let score = parser.detect_score(&lines);
        assert!((score - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_detect_score_mixed() {
        let parser = JournalctlParser::default();
        let lines: Vec<&[u8]> = vec![
            b"2024-02-22T10:15:30+0000 host1 sshd[1]: msg1",
            b"not journalctl at all",
        ];
        let score = parser.detect_score(&lines);
        assert!((score - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_detect_score_none() {
        let parser = JournalctlParser::default();
        let lines: Vec<&[u8]> = vec![b"plain text", b"more plain text"];
        let score = parser.detect_score(&lines);
        assert!((score - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_detect_score_empty() {
        let parser = JournalctlParser::default();
        let lines: Vec<&[u8]> = vec![];
        let score = parser.detect_score(&lines);
        assert!((score - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_collect_field_names_iso() {
        let parser = JournalctlParser::default();
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
        let parser = JournalctlParser::default();
        let lines: Vec<&[u8]> = vec![b"2024-02-22T10:15:30+0000 myhost kernel: msg"];
        let names = parser.collect_field_names(&lines);
        assert!(names.contains(&"hostname".to_string()));
        assert!(!names.contains(&"pid".to_string()));
    }

    #[test]
    fn test_name() {
        let parser = JournalctlParser::default();
        assert_eq!(parser.name(), "journalctl");
    }

    #[test]
    fn test_short_default_format() {
        let line = b"Mar 15 10:00:00 hostname app[42]: something happened";
        let parser = JournalctlParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("Mar 15 10:00:00"));
        assert_eq!(parts.target, Some("app"));
        assert_eq!(parts.message, Some("something happened"));
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(_, k, v)| *k == "hostname" && *v == "hostname")
        );
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(_, k, v)| *k == "pid" && *v == "42")
        );
    }

    #[test]
    fn test_short_default_single_digit_day() {
        let line = b"Mar  5 10:00:00 hostname app: msg";
        let parser = JournalctlParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("Mar  5 10:00:00"));
        assert_eq!(parts.target, Some("app"));
        assert_eq!(parts.message, Some("msg"));
    }

    #[test]
    fn test_collect_field_names_short_format() {
        let parser = JournalctlParser::default();
        let lines: Vec<&[u8]> = vec![b"Mar 15 10:00:00 myhost sshd[1234]: Accepted password"];
        let names = parser.collect_field_names(&lines);
        assert_eq!(names[0], "timestamp");
        assert_eq!(names[1], "target");
        assert!(names.contains(&"hostname".to_string()));
        assert!(names.contains(&"pid".to_string()));
        assert_eq!(*names.last().unwrap(), "message");
    }

    #[test]
    fn test_detect_score_short_format_month_abbrev() {
        let parser = JournalctlParser::default();
        let lines: Vec<&[u8]> = vec![
            b"Mar 15 10:00:00 myhost sshd[1234]: Accepted password",
            b"Mar 15 10:00:01 myhost sshd[1234]: Session opened",
        ];
        let score = parser.detect_score(&lines);
        assert!((score - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_rsyslog_file_format_parse() {
        let parser = JournalctlParser::default();
        let line = b"2026-02-22T00:05:10.113076+01:00 my-pc rsyslogd: [origin software=\"rsyslogd\"] rsyslogd was HUPed";
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2026-02-22T00:05:10.113076+01:00"));
        assert_eq!(parts.target, Some("rsyslogd"));
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(_, k, v)| *k == "hostname" && *v == "my-pc")
        );
    }

    #[test]
    fn test_rsyslog_file_format_detect() {
        let lines: Vec<&[u8]> = vec![
            b"2026-02-22T00:05:10.113076+01:00 my-pc rsyslogd: [origin] msg",
            b"2026-02-22T00:05:10.119576+01:00 my-pc systemd[1]: logrotate.service: Deactivated successfully.",
            b"2026-02-22T00:07:24.887273+01:00 my-pc systemd[1]: Starting sysstat-summary.service",
        ];
        let parser = JournalctlParser::default();
        let score = parser.detect_score(&lines);
        assert!(score > 0.9, "Expected high score, got {}", score);
    }

    #[test]
    fn test_parse_timestamp_iso() {
        let line = b"2024-02-22T10:15:30+0000 myhost sshd[1234]: msg";
        let parser = JournalctlParser::default();
        assert_eq!(
            parser.parse_timestamp(line),
            Some("2024-02-22T10:15:30+0000")
        );
    }

    #[test]
    fn test_parse_timestamp_matches_parse_line_iso() {
        let line = b"2024-02-22T10:15:30+0000 myhost sshd[1234]: msg";
        let parser = JournalctlParser::default();
        assert_eq!(
            parser.parse_timestamp(line),
            parser.parse_line(line).and_then(|p| p.timestamp)
        );
    }

    #[test]
    fn test_parse_timestamp_full() {
        let line = b"Mon 2024-02-22 10:15:30 UTC myhost sshd[1]: msg";
        let parser = JournalctlParser::default();
        assert_eq!(
            parser.parse_timestamp(line),
            parser.parse_line(line).and_then(|p| p.timestamp)
        );
    }

    #[test]
    fn test_parse_timestamp_bsd() {
        let line = b"Feb 22 10:15:30 myhost sshd[1]: msg";
        let parser = JournalctlParser::default();
        assert_eq!(
            parser.parse_timestamp(line),
            parser.parse_line(line).and_then(|p| p.timestamp)
        );
    }

    #[test]
    fn test_parse_timestamp_separator_line_returns_none() {
        let parser = JournalctlParser::default();
        assert!(parser.parse_timestamp(b"-- Reboot --").is_none());
    }

    #[test]
    fn test_parse_timestamp_plain_text_returns_none() {
        let parser = JournalctlParser::default();
        assert!(parser.parse_timestamp(b"just plain text").is_none());
    }

    #[test]
    fn test_format_winner_not_set_before_min_samples() {
        let parser = JournalctlParser::default();
        let line = b"Feb 22 10:15:30 myhost sshd[1]: msg";
        for _ in 0..MIN_SAMPLES - 1 {
            parser.parse_line(line).unwrap();
        }
        assert!(parser.format.get().is_none());
    }

    #[test]
    fn test_format_winner_set_after_min_samples() {
        let parser = JournalctlParser::default();
        let line = b"Feb 22 10:15:30 myhost sshd[1]: msg";
        for _ in 0..MIN_SAMPLES {
            parser.parse_line(line).unwrap();
        }
        assert!(parser.format.get().is_some());
    }

    #[test]
    fn test_format_consistent_output_before_and_after_lock() {
        let parser = JournalctlParser::default();
        let line = b"2024-07-24T10:00:00+00:00 myhost sshd[1]: msg";
        let before = parser.parse_line(line).unwrap();
        for _ in 0..MIN_SAMPLES {
            parser.parse_line(line).unwrap();
        }
        let after = parser.parse_line(line).unwrap();
        assert_eq!(before.timestamp, after.timestamp);
    }
}
