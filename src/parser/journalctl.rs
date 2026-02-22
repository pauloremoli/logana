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

use super::timestamp::{
    is_level_keyword, parse_bsd_precise_timestamp, parse_full_timestamp, parse_iso_timestamp,
};
use super::types::{DisplayParts, LogFormatParser};

/// Zero-copy parser for journalctl text output.
#[derive(Debug)]
pub struct JournalctlParser;

/// Check if a token looks like a plausible hostname (not a level keyword, not
/// a Rust module path, and not a short all-uppercase token that is likely a
/// log level).
fn is_likely_hostname(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    // Reject known level keywords
    if is_level_keyword(token) {
        return false;
    }
    // Reject tokens containing "::" (Rust module paths like myapp::server)
    if token.contains("::") {
        return false;
    }
    // Reject all-uppercase tokens ≤ 8 chars (likely level abbreviations)
    if token.len() <= 8
        && token
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        // Allow single-char hostnames only if they are lowercase (already filtered above)
        return false;
    }
    true
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

    // Validate that the token looks like a hostname
    if !is_likely_hostname(hostname) {
        return None;
    }

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
        let parts = parser.parse_line(line).unwrap();
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "hostname" && *v == "myhost")
        );
        assert_eq!(parts.message, Some("kernel"));
    }

    // ── False positive rejection ──────────────────────────────────────

    #[test]
    fn test_reject_common_log_format_line() {
        // env_logger style: ISO timestamp + level keyword where hostname would be
        let parser = JournalctlParser;
        assert!(
            parser
                .parse_line(b"2024-07-24T10:00:00Z INFO myapp::server: listening on 0.0.0.0:3000")
                .is_none()
        );
    }

    #[test]
    fn test_reject_tracing_fmt_line() {
        let parser = JournalctlParser;
        assert!(
            parser
                .parse_line(b"2024-07-24T10:00:00Z DEBUG myapp::handler: request processed")
                .is_none()
        );
    }

    #[test]
    fn test_reject_level_as_hostname() {
        let parser = JournalctlParser;
        // "WARN" where hostname would be should fail
        assert!(
            parser
                .parse_line(b"2024-07-24T10:00:00Z WARN something: msg")
                .is_none()
        );
    }

    // ── is_likely_hostname ────────────────────────────────────────────

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

    // ── rsyslog RSYSLOG_FileFormat (ISO 8601 with microseconds) ───────

    #[test]
    fn test_rsyslog_file_format_parse() {
        let parser = JournalctlParser;
        let line = b"2026-02-22T00:05:10.113076+01:00 paulo-pc rsyslogd: [origin software=\"rsyslogd\"] rsyslogd was HUPed";
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2026-02-22T00:05:10.113076+01:00"));
        assert_eq!(parts.target, Some("rsyslogd"));
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "hostname" && *v == "paulo-pc")
        );
    }

    #[test]
    fn test_rsyslog_file_format_detect() {
        let lines: Vec<&[u8]> = vec![
            b"2026-02-22T00:05:10.113076+01:00 paulo-pc rsyslogd: [origin] msg",
            b"2026-02-22T00:05:10.119576+01:00 paulo-pc systemd[1]: logrotate.service: Deactivated successfully.",
            b"2026-02-22T00:07:24.887273+01:00 paulo-pc systemd[1]: Starting sysstat-summary.service",
        ];
        let parser = JournalctlParser;
        let score = parser.detect_score(&lines);
        assert!(score > 0.9, "Expected high score, got {}", score);
    }
}
