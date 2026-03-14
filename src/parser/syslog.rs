//! Syslog parser supporting RFC 3164 (BSD) and RFC 5424.

use std::collections::HashSet;

use super::types::{DisplayParts, LogFormatParser};

#[derive(Debug)]
pub struct SyslogParser;

const FACILITY_NAMES: &[&str] = &[
    "kern", "user", "mail", "daemon", "auth", "syslog", "lpr", "news", "uucp", "cron", "authpriv",
    "ftp", "ntp", "audit", "alert", "clock", "local0", "local1", "local2", "local3", "local4",
    "local5", "local6", "local7",
];

fn severity_to_level(severity: u8) -> &'static str {
    match severity {
        0..=3 => "ERROR",
        4 => "WARN",
        5..=6 => "INFO",
        7 => "DEBUG",
        _ => "UNKNOWN",
    }
}

fn parse_priority(line: &[u8]) -> Option<(u8, usize)> {
    if line.is_empty() || line[0] != b'<' {
        return None;
    }
    let close = line[1..].iter().position(|&b| b == b'>')?;
    if close == 0 || close > 3 {
        return None;
    }
    let pri_str = std::str::from_utf8(&line[1..1 + close]).ok()?;
    let pri: u8 = pri_str.parse().ok()?;
    Some((pri, close + 2))
}

const BSD_MONTHS: &[&str] = &[
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn parse_bsd_timestamp(s: &str) -> Option<(&str, usize)> {
    if s.len() < 15 {
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
    let time_part = &s[time_start..time_start + 8];
    if time_part.as_bytes()[2] != b':'
        || time_part.as_bytes()[5] != b':'
        || !time_part.as_bytes()[0].is_ascii_digit()
        || !time_part.as_bytes()[1].is_ascii_digit()
        || !time_part.as_bytes()[3].is_ascii_digit()
        || !time_part.as_bytes()[4].is_ascii_digit()
        || !time_part.as_bytes()[6].is_ascii_digit()
        || !time_part.as_bytes()[7].is_ascii_digit()
    {
        return None;
    }
    let end = time_start + 8;
    Some((&s[..end], end))
}

fn parse_rfc5424<'a>(s: &'a str, priority: u8) -> Option<DisplayParts<'a>> {
    let severity = priority & 0x07;
    let facility = priority >> 3;

    if s.is_empty() || !s.as_bytes()[0].is_ascii_digit() {
        return None;
    }
    let ver_end = s.find(' ')?;
    let rest = &s[ver_end + 1..];

    let (timestamp, rest) = next_token(rest)?;
    let (hostname, rest) = next_token(rest)?;
    let (app_name, rest) = next_token(rest)?;
    let (procid, rest) = next_token(rest)?;
    let (msgid, rest) = next_token(rest)?;

    let mut parts = DisplayParts::default();
    if timestamp != "-" {
        parts.timestamp = Some(timestamp);
    }
    parts.level = Some(severity_to_level(severity));
    if app_name != "-" {
        parts.target = Some(app_name);
    }

    if hostname != "-" {
        parts.extra_fields.push(("hostname", hostname));
    }
    if procid != "-" {
        parts.extra_fields.push(("pid", procid));
    }
    if facility < FACILITY_NAMES.len() as u8 {
        parts
            .extra_fields
            .push(("facility", FACILITY_NAMES[facility as usize]));
    }
    if msgid != "-" {
        parts.extra_fields.push(("msgid", msgid));
    }

    let rest = rest.trim_start();
    if rest.is_empty() {
        return Some(parts);
    }

    let msg_start;
    if rest.starts_with('[') {
        let mut pos = 0;
        let rest_bytes = rest.as_bytes();
        while pos < rest_bytes.len() && rest_bytes[pos] == b'[' {
            let sd_start = pos + 1;
            pos += 1;
            while pos < rest_bytes.len() && rest_bytes[pos] != b']' {
                if rest_bytes[pos] == b'"' {
                    pos += 1;
                    while pos < rest_bytes.len() {
                        if rest_bytes[pos] == b'\\' {
                            pos += 2;
                        } else if rest_bytes[pos] == b'"' {
                            pos += 1;
                            break;
                        } else {
                            pos += 1;
                        }
                    }
                } else {
                    pos += 1;
                }
            }
            if pos < rest_bytes.len() && rest_bytes[pos] == b']' {
                let sd_content = &rest[sd_start..pos];
                parse_sd_params(sd_content, &mut parts);
                pos += 1;
                while pos < rest_bytes.len() && rest_bytes[pos] == b' ' {
                    pos += 1;
                }
            } else {
                break;
            }
        }
        msg_start = pos;
    } else if rest.starts_with('-') {
        msg_start = if rest.len() > 1 && rest.as_bytes()[1] == b' ' {
            2
        } else {
            1
        };
    } else {
        msg_start = 0;
    }

    let msg = rest[msg_start..].trim_start();
    if !msg.is_empty() {
        let msg = msg.strip_prefix('\u{FEFF}').unwrap_or(msg);
        if !msg.is_empty() {
            parts.message = Some(msg);
        }
    }

    Some(parts)
}

fn parse_sd_params<'a>(content: &'a str, parts: &mut DisplayParts<'a>) {
    let rest = match content.find(' ') {
        Some(pos) => &content[pos + 1..],
        None => return,
    };

    let mut pos = 0;
    let bytes = rest.as_bytes();
    while pos < bytes.len() {
        while pos < bytes.len() && bytes[pos] == b' ' {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }
        let name_start = pos;
        while pos < bytes.len() && bytes[pos] != b'=' {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }
        let param_name = &rest[name_start..pos];
        pos += 1;

        if pos >= bytes.len() || bytes[pos] != b'"' {
            break;
        }
        pos += 1;
        let val_start = pos;
        while pos < bytes.len() {
            if bytes[pos] == b'\\' {
                pos += 2;
            } else if bytes[pos] == b'"' {
                break;
            } else {
                pos += 1;
            }
        }
        let param_val = &rest[val_start..pos];
        if pos < bytes.len() {
            pos += 1;
        }

        parts.extra_fields.push((param_name, param_val));
    }
}

/// Extract `target[pid]: message` from a tag+message region.
fn extract_tag_and_message<'a>(rest: &'a str, parts: &mut DisplayParts<'a>) {
    if let Some(colon_pos) = rest.find(": ") {
        extract_unit_pid(&rest[..colon_pos], parts);
        let message = &rest[colon_pos + 2..];
        if !message.is_empty() {
            parts.message = Some(message);
        }
    } else if let Some(colon_pos) = rest.find(':') {
        extract_unit_pid(&rest[..colon_pos], parts);
        let message = rest[colon_pos + 1..].trim_start();
        if !message.is_empty() {
            parts.message = Some(message);
        }
    } else {
        parts.message = Some(rest);
    }
}

fn extract_unit_pid<'a>(tag: &'a str, parts: &mut DisplayParts<'a>) {
    if let Some(bracket_start) = tag.find('[') {
        parts.target = Some(&tag[..bracket_start]);
        if let Some(bracket_end) = tag[bracket_start..].find(']') {
            parts
                .extra_fields
                .push(("pid", &tag[bracket_start + 1..bracket_start + bracket_end]));
        }
    } else {
        parts.target = Some(tag);
    }
}

fn parse_rfc3164_inner<'a>(s: &'a str, priority: Option<u8>) -> Option<DisplayParts<'a>> {
    let (timestamp, ts_end) = parse_bsd_timestamp(s)?;

    let mut parts = DisplayParts {
        timestamp: Some(timestamp),
        level: priority.map(|p| severity_to_level(p & 0x07)),
        ..Default::default()
    };

    let rest = s[ts_end..].strip_prefix(' ').unwrap_or(&s[ts_end..]);
    if rest.is_empty() {
        return Some(parts);
    }

    let (hostname, rest) = next_token(rest)?;
    parts.extra_fields.push(("hostname", hostname));

    if rest.is_empty() {
        return Some(parts);
    }

    extract_tag_and_message(rest, &mut parts);

    if let Some(pri) = priority {
        let facility = pri >> 3;
        if facility < FACILITY_NAMES.len() as u8 {
            parts
                .extra_fields
                .push(("facility", FACILITY_NAMES[facility as usize]));
        }
    }

    Some(parts)
}

fn next_token(s: &str) -> Option<(&str, &str)> {
    if s.is_empty() {
        return None;
    }
    match s.find(' ') {
        Some(pos) => Some((&s[..pos], &s[pos + 1..])),
        None => Some((s, "")),
    }
}

impl LogFormatParser for SyslogParser {
    fn parse_line<'a>(&self, line: &'a [u8]) -> Option<DisplayParts<'a>> {
        let s = std::str::from_utf8(line).ok()?;
        if s.is_empty() {
            return None;
        }

        if let Some((priority, consumed)) = parse_priority(line) {
            let rest = &s[consumed..];
            if let Some(parts) = parse_rfc5424(rest, priority) {
                return Some(parts);
            }
            if let Some(parts) = parse_rfc3164_inner(rest, Some(priority)) {
                return Some(parts);
            }
        }

        parse_rfc3164_inner(s, None)
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

        // Canonical fields first, then discovered extras
        let mut result = vec![
            "timestamp".to_string(),
            "level".to_string(),
            "target".to_string(),
        ];
        extras.sort();
        extras.dedup();
        result.extend(extras);
        result.push("message".to_string());
        result
    }

    fn name(&self) -> &str {
        "syslog"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_priority_valid() {
        assert_eq!(parse_priority(b"<134>rest"), Some((134, 5)));
        assert_eq!(parse_priority(b"<0>rest"), Some((0, 3)));
        assert_eq!(parse_priority(b"<13>rest"), Some((13, 4)));
    }

    #[test]
    fn test_parse_priority_invalid() {
        assert!(parse_priority(b"no angle").is_none());
        assert!(parse_priority(b"<>rest").is_none());
        assert!(parse_priority(b"<1234>rest").is_none()); // too many digits
        assert!(parse_priority(b"").is_none());
    }

    #[test]
    fn test_severity_to_level_mapping() {
        assert_eq!(severity_to_level(0), "ERROR"); // emerg
        assert_eq!(severity_to_level(1), "ERROR"); // alert
        assert_eq!(severity_to_level(2), "ERROR"); // crit
        assert_eq!(severity_to_level(3), "ERROR"); // err
        assert_eq!(severity_to_level(4), "WARN"); // warning
        assert_eq!(severity_to_level(5), "INFO"); // notice
        assert_eq!(severity_to_level(6), "INFO"); // info
        assert_eq!(severity_to_level(7), "DEBUG"); // debug
    }

    // ── RFC 3164 parsing ─────────────────────────────────────────────────

    #[test]
    fn test_rfc3164_full() {
        let line = b"<134>Oct 11 22:14:15 myhost sshd[1234]: Accepted password for user";
        let parser = SyslogParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("Oct 11 22:14:15"));
        assert_eq!(parts.level, Some("INFO")); // severity 6
        assert_eq!(parts.target, Some("sshd"));
        assert_eq!(parts.message, Some("Accepted password for user"));
        // extras: hostname, pid, facility
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
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "facility" && *v == "local0")
        );
    }

    #[test]
    fn test_rfc3164_no_pid() {
        let line = b"<134>Oct 11 22:14:15 myhost sshd: Accepted password for user";
        let parser = SyslogParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.target, Some("sshd"));
        assert!(!parts.extra_fields.iter().any(|(k, _)| *k == "pid"));
        assert_eq!(parts.message, Some("Accepted password for user"));
    }

    #[test]
    fn test_rfc3164_single_digit_day() {
        let line = b"<134>Oct  5 22:14:15 myhost sshd[1234]: message";
        let parser = SyslogParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("Oct  5 22:14:15"));
    }

    #[test]
    fn test_rfc3164_no_priority() {
        let line = b"Oct 11 22:14:15 myhost sshd[1234]: Accepted password for user";
        let parser = SyslogParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("Oct 11 22:14:15"));
        assert_eq!(parts.target, Some("sshd"));
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
        assert_eq!(parts.message, Some("Accepted password for user"));
        // No level without priority
        assert!(parts.level.is_none());
    }

    #[test]
    fn test_rfc3164_error_severity() {
        // priority = 11 → facility 1 (user), severity 3 (err)
        let line = b"<11>Oct 11 22:14:15 myhost app: error message";
        let parser = SyslogParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("ERROR"));
    }

    #[test]
    fn test_rfc3164_warning_severity() {
        // priority = 12 → facility 1 (user), severity 4 (warning)
        let line = b"<12>Oct 11 22:14:15 myhost app: warn message";
        let parser = SyslogParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("WARN"));
    }

    #[test]
    fn test_rfc3164_debug_severity() {
        // priority = 15 → facility 1 (user), severity 7 (debug)
        let line = b"<15>Oct 11 22:14:15 myhost app: debug message";
        let parser = SyslogParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("DEBUG"));
    }

    // ── RFC 5424 parsing ─────────────────────────────────────────────────

    #[test]
    fn test_rfc5424_full() {
        let line = b"<165>1 2003-10-11T22:14:15.003Z mymachine.example.com evntslog - ID47 [exampleSDID@32473 iut=\"3\" eventSource=\"Application\"] An application event log entry...";
        let parser = SyslogParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2003-10-11T22:14:15.003Z"));
        assert_eq!(parts.level, Some("INFO")); // severity 5 (notice)
        assert_eq!(parts.target, Some("evntslog"));
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "hostname" && *v == "mymachine.example.com")
        );
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "msgid" && *v == "ID47")
        );
        // SD params
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "iut" && *v == "3")
        );
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "eventSource" && *v == "Application")
        );
        assert_eq!(parts.message, Some("An application event log entry..."));
    }

    #[test]
    fn test_rfc5424_nil_fields() {
        let line = b"<134>1 2003-10-11T22:14:15.003Z - - - - - No structured data";
        let parser = SyslogParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2003-10-11T22:14:15.003Z"));
        assert_eq!(parts.level, Some("INFO")); // severity 6
        assert!(parts.target.is_none()); // app_name is "-"
        assert_eq!(parts.message, Some("No structured data"));
    }

    #[test]
    fn test_rfc5424_no_message() {
        let line = b"<134>1 2003-10-11T22:14:15.003Z myhost myapp 1234 - -";
        let parser = SyslogParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2003-10-11T22:14:15.003Z"));
        assert_eq!(parts.target, Some("myapp"));
        assert!(parts.message.is_none());
    }

    #[test]
    fn test_rfc5424_multiple_sd_elements() {
        let line =
            b"<165>1 2003-10-11T22:14:15.003Z host app - - [sdA@1 a=\"1\"][sdB@1 b=\"2\"] msg";
        let parser = SyslogParser;
        let parts = parser.parse_line(line).unwrap();
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "a" && *v == "1")
        );
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "b" && *v == "2")
        );
        assert_eq!(parts.message, Some("msg"));
    }

    // ── detect_score ─────────────────────────────────────────────────────

    #[test]
    fn test_detect_score_all_syslog() {
        let parser = SyslogParser;
        let lines: Vec<&[u8]> = vec![
            b"<134>Oct 11 22:14:15 myhost sshd[1234]: msg1",
            b"<134>Oct 11 22:14:16 myhost sshd[1234]: msg2",
        ];
        let score = parser.detect_score(&lines);
        assert!((score - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_detect_score_mixed() {
        let parser = SyslogParser;
        let lines: Vec<&[u8]> = vec![
            b"<134>Oct 11 22:14:15 myhost sshd[1234]: msg1",
            b"not syslog at all",
        ];
        let score = parser.detect_score(&lines);
        assert!((score - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_detect_score_none_syslog() {
        let parser = SyslogParser;
        let lines: Vec<&[u8]> = vec![b"plain text", b"more plain text"];
        let score = parser.detect_score(&lines);
        assert!((score - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_detect_score_empty() {
        let parser = SyslogParser;
        let lines: Vec<&[u8]> = vec![];
        let score = parser.detect_score(&lines);
        assert!((score - 0.0).abs() < 0.001);
    }

    // ── collect_field_names ──────────────────────────────────────────────

    #[test]
    fn test_collect_field_names_rfc3164() {
        let parser = SyslogParser;
        let lines: Vec<&[u8]> = vec![b"<134>Oct 11 22:14:15 myhost sshd[1234]: msg"];
        let names = parser.collect_field_names(&lines);
        assert!(names.contains(&"timestamp".to_string()));
        assert!(names.contains(&"level".to_string()));
        assert!(names.contains(&"target".to_string()));
        assert!(names.contains(&"message".to_string()));
        assert!(names.contains(&"hostname".to_string()));
        assert!(names.contains(&"pid".to_string()));
        assert!(names.contains(&"facility".to_string()));
    }

    #[test]
    fn test_collect_field_names_rfc5424_with_sd() {
        let parser = SyslogParser;
        let lines: Vec<&[u8]> =
            vec![b"<165>1 2003-10-11T22:14:15.003Z host app - ID47 [sd@1 key=\"val\"] msg"];
        let names = parser.collect_field_names(&lines);
        assert!(names.contains(&"hostname".to_string()));
        assert!(names.contains(&"msgid".to_string()));
        assert!(names.contains(&"key".to_string()));
    }

    // ── Edge cases ───────────────────────────────────────────────────────

    #[test]
    fn test_parse_empty_line() {
        let parser = SyslogParser;
        assert!(parser.parse_line(b"").is_none());
    }

    #[test]
    fn test_parse_json_line_not_syslog() {
        let parser = SyslogParser;
        assert!(
            parser
                .parse_line(br#"{"level":"INFO","msg":"hello"}"#)
                .is_none()
        );
    }

    #[test]
    fn test_parse_plain_text_not_syslog() {
        let parser = SyslogParser;
        assert!(parser.parse_line(b"just plain text").is_none());
    }
}
