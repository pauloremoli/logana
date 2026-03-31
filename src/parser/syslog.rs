use crate::parser::timestamp::MONTHS;

use super::types::{DisplayParts, FieldSemantic, LogFormatParser, push_extra_field, push_field_as};
use std::collections::HashSet;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

#[derive(Debug, Clone, Copy)]
enum SyslogFormat {
    Rfc3164,
    Rfc5424,
    /// rsyslog RSYSLOG_FileFormat: `ISO-timestamp hostname tag[pid]: message`
    /// (ISO 8601 timestamp with no `<PRI>` prefix).
    RsyslogIso,
}

impl SyslogFormat {
    fn index(self) -> usize {
        self as usize
    }

    fn from_index(i: usize) -> Self {
        match i {
            0 => Self::Rfc3164,
            1 => Self::Rfc5424,
            _ => Self::RsyslogIso,
        }
    }
}

const MIN_SAMPLES: u32 = 50;

#[derive(Debug, Default)]
pub struct SyslogParser {
    format: OnceLock<SyslogFormat>,
    fmt_counts: [AtomicU32; 3],
    fmt_total: AtomicU32,
}

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

fn parse_bsd_timestamp(s: &str) -> Option<(&str, usize)> {
    if s.len() < 15 {
        return None;
    }
    let month = &s[..3];
    if !MONTHS.contains(&month) {
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
        push_field_as(&mut parts.extra_fields, FieldSemantic::Hostname, hostname);
    }
    if procid != "-" {
        push_field_as(&mut parts.extra_fields, FieldSemantic::Pid, procid);
    }
    if facility < FACILITY_NAMES.len() as u8 {
        push_field_as(
            &mut parts.extra_fields,
            FieldSemantic::Facility,
            FACILITY_NAMES[facility as usize],
        );
    }
    if msgid != "-" {
        push_field_as(&mut parts.extra_fields, FieldSemantic::MsgId, msgid);
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

        push_extra_field(&mut parts.extra_fields, param_name, param_val);
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
            push_field_as(
                &mut parts.extra_fields,
                FieldSemantic::Pid,
                &tag[bracket_start + 1..bracket_start + bracket_end],
            );
        }
    } else {
        parts.target = Some(tag);
    }
}

fn parse_rfc3164_inner<'a>(s: &'a str, priority: Option<u8>) -> Option<DisplayParts<'a>> {
    let (timestamp, ts_end) = parse_bsd_timestamp(s)?;

    // Reject precise BSD timestamps (`Feb 22 10:15:30.123456 …`) — the fractional
    // second suffix belongs to the journalctl `short-precise` format, not RFC 3164.
    let after_ts = &s[ts_end..];
    if !after_ts.is_empty() && !after_ts.starts_with(' ') {
        return None;
    }

    let mut parts = DisplayParts {
        timestamp: Some(timestamp),
        level: priority.map(|p| severity_to_level(p & 0x07)),
        ..Default::default()
    };

    let rest = after_ts.strip_prefix(' ').unwrap_or(after_ts);
    if rest.is_empty() {
        return Some(parts);
    }

    let (hostname, rest) = next_token(rest)?;
    push_field_as(&mut parts.extra_fields, FieldSemantic::Hostname, hostname);

    if rest.is_empty() {
        return Some(parts);
    }

    extract_tag_and_message(rest, &mut parts);

    if let Some(pri) = priority {
        let facility = pri >> 3;
        if facility < FACILITY_NAMES.len() as u8 {
            push_field_as(
                &mut parts.extra_fields,
                FieldSemantic::Facility,
                FACILITY_NAMES[facility as usize],
            );
        }
    }

    Some(parts)
}

fn is_level_keyword(s: &str) -> bool {
    matches!(
        s,
        "TRACE"
            | "DEBUG"
            | "INFO"
            | "NOTICE"
            | "WARN"
            | "WARNING"
            | "ERROR"
            | "CRITICAL"
            | "FATAL"
            | "PANIC"
            | "EMERG"
            | "ALERT"
            | "CRIT"
            | "ERR"
    )
}

/// Returns false for tokens that look like log-level keywords, Rust module paths,
/// or short all-uppercase identifiers — these indicate common-log format, not a
/// syslog hostname following an ISO timestamp.
fn is_valid_syslog_hostname(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    if is_level_keyword(token) {
        return false;
    }
    if token.contains("::") {
        return false;
    }
    // Reject short all-caps tokens (e.g. "APP", "API", "MYAPP") that are likely
    // application names or log targets, not hostnames.
    if token.len() <= 8
        && token
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        return false;
    }
    true
}

/// Parse rsyslog RSYSLOG_FileFormat: `ISO-timestamp hostname tag[pid]: message`.
/// This is the default rsyslog file output format — no `<PRI>` prefix, ISO 8601
/// timestamp with optional subseconds and timezone offset.
fn parse_rsyslog_iso_inner<'a>(s: &'a str) -> Option<DisplayParts<'a>> {
    let (timestamp, ts_end) = super::timestamp::parse_iso_timestamp(s)?;
    let rest = s[ts_end..].strip_prefix(' ')?;

    let mut parts = DisplayParts {
        timestamp: Some(timestamp),
        ..Default::default()
    };

    if rest.is_empty() {
        return Some(parts);
    }

    let (hostname, rest) = next_token(rest)?;
    if !is_valid_syslog_hostname(hostname) {
        return None;
    }
    push_field_as(&mut parts.extra_fields, FieldSemantic::Hostname, hostname);

    if rest.is_empty() {
        return Some(parts);
    }

    extract_tag_and_message(rest, &mut parts);
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

fn detect_syslog_timestamp<'a>(s: &'a str, line: &'a [u8]) -> Option<(SyslogFormat, &'a str)> {
    let body = if s.starts_with('<') {
        let (_, consumed) = parse_priority(line)?;
        &s[consumed..]
    } else {
        s
    };
    if let Some((ts, ts_end)) = parse_bsd_timestamp(body) {
        // Only accept if followed by space (not a precise BSD timestamp).
        let after = &body[ts_end..];
        if after.is_empty() || after.starts_with(' ') {
            return Some((SyslogFormat::Rfc3164, ts));
        }
    }
    if body
        .as_bytes()
        .first()
        .map(|b| b.is_ascii_digit())
        .unwrap_or(false)
    {
        // Could be RFC 5424 (`<PRI>VER ISO-timestamp …`) or rsyslog ISO (no PRI).
        if s.starts_with('<') {
            let ver_end = body.find(' ')?;
            let rest = &body[ver_end + 1..];
            let (ts, _) = next_token(rest)?;
            if ts != "-" {
                return Some((SyslogFormat::Rfc5424, ts));
            }
        } else if let Some((ts, _)) = super::timestamp::parse_iso_timestamp(body) {
            return Some((SyslogFormat::RsyslogIso, ts));
        }
    }
    None
}

fn extract_syslog_timestamp_rfc3164<'a>(s: &'a str, line: &'a [u8]) -> Option<&'a str> {
    let body = if s.starts_with('<') {
        let (_, consumed) = parse_priority(line)?;
        &s[consumed..]
    } else {
        s
    };
    let (ts, ts_end) = parse_bsd_timestamp(body)?;
    let after = &body[ts_end..];
    if after.is_empty() || after.starts_with(' ') {
        Some(ts)
    } else {
        None
    }
}

fn extract_syslog_timestamp_rfc5424<'a>(s: &'a str, line: &'a [u8]) -> Option<&'a str> {
    let (_, consumed) = parse_priority(line)?;
    let rest = &s[consumed..];
    let ver_end = rest.find(' ')?;
    let rest = &rest[ver_end + 1..];
    let (ts, _) = next_token(rest)?;
    if ts != "-" { Some(ts) } else { None }
}

fn extract_syslog_timestamp_rsyslog_iso(s: &str) -> Option<&str> {
    super::timestamp::parse_iso_timestamp(s).map(|(ts, _)| ts)
}

impl SyslogParser {
    fn record_format(&self, fmt: SyslogFormat) {
        self.fmt_counts[fmt.index()].fetch_add(1, Ordering::Relaxed);
        let total = self.fmt_total.fetch_add(1, Ordering::Relaxed) + 1;
        if total >= MIN_SAMPLES && self.format.get().is_none() {
            let winner = (0..3)
                .max_by_key(|&i| self.fmt_counts[i].load(Ordering::Relaxed))
                .unwrap_or(0);
            let _ = self.format.set(SyslogFormat::from_index(winner));
        }
    }
}

impl LogFormatParser for SyslogParser {
    fn timestamp_has_year(&self) -> bool {
        !matches!(self.format.get(), Some(SyslogFormat::Rfc3164))
    }

    fn parse_timestamp<'a>(&self, line: &'a [u8]) -> Option<&'a str> {
        let s = std::str::from_utf8(line).ok()?;
        if s.is_empty() {
            return None;
        }
        if let Some(&fmt) = self.format.get() {
            return match fmt {
                SyslogFormat::Rfc3164 => extract_syslog_timestamp_rfc3164(s, line),
                SyslogFormat::Rfc5424 => extract_syslog_timestamp_rfc5424(s, line),
                SyslogFormat::RsyslogIso => extract_syslog_timestamp_rsyslog_iso(s),
            };
        }
        let (fmt, ts) = detect_syslog_timestamp(s, line)?;
        self.record_format(fmt);
        Some(ts)
    }

    fn parse_line<'a>(&self, line: &'a [u8]) -> Option<DisplayParts<'a>> {
        let s = std::str::from_utf8(line).ok()?;
        if s.is_empty() {
            return None;
        }

        if let Some(&fmt) = self.format.get() {
            let result = match fmt {
                SyslogFormat::Rfc5424 => {
                    let (priority, consumed) = parse_priority(line)?;
                    parse_rfc5424(&s[consumed..], priority)
                }
                SyslogFormat::Rfc3164 => {
                    if let Some((priority, consumed)) = parse_priority(line) {
                        parse_rfc3164_inner(&s[consumed..], Some(priority))
                    } else {
                        parse_rfc3164_inner(s, None)
                    }
                }
                SyslogFormat::RsyslogIso => parse_rsyslog_iso_inner(s),
            };
            if result.is_some() {
                return result;
            }
        }

        if let Some((priority, consumed)) = parse_priority(line) {
            let rest = &s[consumed..];
            if let Some(parts) = parse_rfc5424(rest, priority) {
                self.record_format(SyslogFormat::Rfc5424);
                return Some(parts);
            }
            if let Some(parts) = parse_rfc3164_inner(rest, Some(priority)) {
                self.record_format(SyslogFormat::Rfc3164);
                return Some(parts);
            }
        }

        if let Some(parts) = parse_rfc3164_inner(s, None) {
            self.record_format(SyslogFormat::Rfc3164);
            return Some(parts);
        }

        if let Some(parts) = parse_rsyslog_iso_inner(s) {
            self.record_format(SyslogFormat::RsyslogIso);
            return Some(parts);
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

    /// Only claim lines that carry a format-specific syslog signal:
    ///   • priority prefix (`<PRI>…`) — exclusively RFC 3164 / RFC 5424
    ///   • ISO timestamp (`YYYY-MM-DDTHH:MM:SS…`) — rsyslog RSYSLOG_FileFormat
    ///
    /// Plain BSD lines without a priority prefix (`Oct 11 22:14:15 host tag: msg`)
    /// are shared with journalctl `--output short` and are intentionally **not**
    /// claimed here so that piped `journalctl` output is still detected as
    /// journalctl.  Those lines can still be *parsed* by `parse_line` once the
    /// format is locked by other lines in the sample.
    fn matches_for_detection(&self, line: &[u8]) -> bool {
        if line.is_empty() {
            return false;
        }
        // Priority-prefixed lines are exclusively syslog.
        if line[0] == b'<' {
            return parse_priority(line).is_some();
        }
        // rsyslog ISO format — ISO timestamp + valid hostname.
        if let Ok(s) = std::str::from_utf8(line) {
            return parse_rsyslog_iso_inner(s).is_some();
        }
        false
    }

    fn name(&self) -> &str {
        "syslog"
    }

    fn has_synthetic_level(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_has_synthetic_level() {
        assert!(SyslogParser::default().has_synthetic_level());
    }

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
        let parser = SyslogParser::default();
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
                .any(|(_, k, v)| *k == "hostname" && *v == "myhost")
        );
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(_, k, v)| *k == "pid" && *v == "1234")
        );
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(_, k, v)| *k == "facility" && *v == "local0")
        );
    }

    #[test]
    fn test_rfc3164_no_pid() {
        let line = b"<134>Oct 11 22:14:15 myhost sshd: Accepted password for user";
        let parser = SyslogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.target, Some("sshd"));
        assert!(!parts.extra_fields.iter().any(|(_, k, _)| *k == "pid"));
        assert_eq!(parts.message, Some("Accepted password for user"));
    }

    #[test]
    fn test_rfc3164_single_digit_day() {
        let line = b"<134>Oct  5 22:14:15 myhost sshd[1234]: message";
        let parser = SyslogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("Oct  5 22:14:15"));
    }

    #[test]
    fn test_rfc3164_no_priority() {
        let line = b"Oct 11 22:14:15 myhost sshd[1234]: Accepted password for user";
        let parser = SyslogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("Oct 11 22:14:15"));
        assert_eq!(parts.target, Some("sshd"));
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
        assert_eq!(parts.message, Some("Accepted password for user"));
        // No level without priority
        assert!(parts.level.is_none());
    }

    #[test]
    fn test_rfc3164_error_severity() {
        // priority = 11 → facility 1 (user), severity 3 (err)
        let line = b"<11>Oct 11 22:14:15 myhost app: error message";
        let parser = SyslogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("ERROR"));
    }

    #[test]
    fn test_rfc3164_warning_severity() {
        // priority = 12 → facility 1 (user), severity 4 (warning)
        let line = b"<12>Oct 11 22:14:15 myhost app: warn message";
        let parser = SyslogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("WARN"));
    }

    #[test]
    fn test_rfc3164_debug_severity() {
        // priority = 15 → facility 1 (user), severity 7 (debug)
        let line = b"<15>Oct 11 22:14:15 myhost app: debug message";
        let parser = SyslogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("DEBUG"));
    }

    // ── RFC 5424 parsing ─────────────────────────────────────────────────

    #[test]
    fn test_rfc5424_full() {
        let line = b"<165>1 2003-10-11T22:14:15.003Z mymachine.example.com evntslog - ID47 [exampleSDID@32473 iut=\"3\" eventSource=\"Application\"] An application event log entry...";
        let parser = SyslogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2003-10-11T22:14:15.003Z"));
        assert_eq!(parts.level, Some("INFO")); // severity 5 (notice)
        assert_eq!(parts.target, Some("evntslog"));
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(_, k, v)| *k == "hostname" && *v == "mymachine.example.com")
        );
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(_, k, v)| *k == "msgid" && *v == "ID47")
        );
        // SD params
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(_, k, v)| *k == "iut" && *v == "3")
        );
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(_, k, v)| *k == "eventSource" && *v == "Application")
        );
        assert_eq!(parts.message, Some("An application event log entry..."));
    }

    #[test]
    fn test_rfc5424_nil_fields() {
        let line = b"<134>1 2003-10-11T22:14:15.003Z - - - - - No structured data";
        let parser = SyslogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2003-10-11T22:14:15.003Z"));
        assert_eq!(parts.level, Some("INFO")); // severity 6
        assert!(parts.target.is_none()); // app_name is "-"
        assert_eq!(parts.message, Some("No structured data"));
    }

    #[test]
    fn test_rfc5424_no_message() {
        let line = b"<134>1 2003-10-11T22:14:15.003Z myhost myapp 1234 - -";
        let parser = SyslogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2003-10-11T22:14:15.003Z"));
        assert_eq!(parts.target, Some("myapp"));
        assert!(parts.message.is_none());
    }

    #[test]
    fn test_rfc5424_multiple_sd_elements() {
        let line =
            b"<165>1 2003-10-11T22:14:15.003Z host app - - [sdA@1 a=\"1\"][sdB@1 b=\"2\"] msg";
        let parser = SyslogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(_, k, v)| *k == "a" && *v == "1")
        );
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(_, k, v)| *k == "b" && *v == "2")
        );
        assert_eq!(parts.message, Some("msg"));
    }

    // ── detect_score ─────────────────────────────────────────────────────

    #[test]
    fn test_detect_score_all_syslog() {
        let parser = SyslogParser::default();
        let lines: Vec<&[u8]> = vec![
            b"<134>Oct 11 22:14:15 myhost sshd[1234]: msg1",
            b"<134>Oct 11 22:14:16 myhost sshd[1234]: msg2",
        ];
        let score = parser.detect_score(&lines);
        assert!((score - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_detect_score_mixed() {
        let parser = SyslogParser::default();
        let lines: Vec<&[u8]> = vec![
            b"<134>Oct 11 22:14:15 myhost sshd[1234]: msg1",
            b"not syslog at all",
        ];
        let score = parser.detect_score(&lines);
        assert!((score - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_detect_score_none_syslog() {
        let parser = SyslogParser::default();
        let lines: Vec<&[u8]> = vec![b"plain text", b"more plain text"];
        let score = parser.detect_score(&lines);
        assert!((score - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_detect_score_empty() {
        let parser = SyslogParser::default();
        let lines: Vec<&[u8]> = vec![];
        let score = parser.detect_score(&lines);
        assert!((score - 0.0).abs() < 0.001);
    }

    // ── collect_field_names ──────────────────────────────────────────────

    #[test]
    fn test_collect_field_names_rfc3164() {
        let parser = SyslogParser::default();
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
        let parser = SyslogParser::default();
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
        let parser = SyslogParser::default();
        assert!(parser.parse_line(b"").is_none());
    }

    #[test]
    fn test_parse_json_line_not_syslog() {
        let parser = SyslogParser::default();
        assert!(
            parser
                .parse_line(br#"{"level":"INFO","msg":"hello"}"#)
                .is_none()
        );
    }

    #[test]
    fn test_parse_plain_text_not_syslog() {
        let parser = SyslogParser::default();
        assert!(parser.parse_line(b"just plain text").is_none());
    }

    // ── parse_timestamp ────────────────────────────────────────────────

    #[test]
    fn test_parse_timestamp_rfc3164_with_priority() {
        let line = b"<134>Oct 11 22:14:15 myhost sshd[1234]: msg";
        let parser = SyslogParser::default();
        assert_eq!(parser.parse_timestamp(line), Some("Oct 11 22:14:15"));
    }

    #[test]
    fn test_parse_timestamp_rfc3164_matches_parse_line() {
        let line = b"<134>Oct 11 22:14:15 myhost sshd[1234]: msg";
        let parser = SyslogParser::default();
        assert_eq!(
            parser.parse_timestamp(line),
            parser.parse_line(line).and_then(|p| p.timestamp)
        );
    }

    #[test]
    fn test_parse_timestamp_rfc5424_matches_parse_line() {
        let line = b"<165>1 2003-10-11T22:14:15.003Z mymachine.example.com evntslog - ID47 - msg";
        let parser = SyslogParser::default();
        assert_eq!(
            parser.parse_timestamp(line),
            parser.parse_line(line).and_then(|p| p.timestamp)
        );
    }

    #[test]
    fn test_parse_timestamp_no_priority_bsd() {
        let line = b"Oct 11 22:14:15 myhost sshd[1234]: msg";
        let parser = SyslogParser::default();
        assert_eq!(parser.parse_timestamp(line), Some("Oct 11 22:14:15"));
    }

    #[test]
    fn test_parse_timestamp_plain_text_returns_none() {
        let parser = SyslogParser::default();
        assert!(parser.parse_timestamp(b"just plain text").is_none());
    }

    #[test]
    fn test_format_winner_not_set_before_min_samples() {
        let parser = SyslogParser::default();
        let line = b"Oct 11 22:14:15 myhost sshd[1234]: msg";
        for _ in 0..MIN_SAMPLES - 1 {
            parser.parse_line(line).unwrap();
        }
        assert!(parser.format.get().is_none());
    }

    #[test]
    fn test_format_winner_set_after_min_samples_rfc3164() {
        let parser = SyslogParser::default();
        let line = b"Oct 11 22:14:15 myhost sshd[1234]: msg";
        for _ in 0..MIN_SAMPLES {
            parser.parse_line(line).unwrap();
        }
        assert!(parser.format.get().is_some());
    }

    #[test]
    fn test_format_winner_set_after_min_samples_rfc5424() {
        let parser = SyslogParser::default();
        let line = b"<165>1 2003-10-11T22:14:15.003Z mymachine.example.com evntslog - ID47 - msg";
        for _ in 0..MIN_SAMPLES {
            parser.parse_line(line).unwrap();
        }
        assert!(parser.format.get().is_some());
    }

    #[test]
    fn test_format_consistent_output_before_and_after_lock() {
        let parser = SyslogParser::default();
        let line = b"<165>1 2003-10-11T22:14:15.003Z mymachine.example.com evntslog - ID47 - msg";
        let before = parser.parse_line(line).unwrap();
        for _ in 0..MIN_SAMPLES {
            parser.parse_line(line).unwrap();
        }
        let after = parser.parse_line(line).unwrap();
        assert_eq!(before.timestamp, after.timestamp);
        assert_eq!(before.level, after.level);
        assert_eq!(before.target, after.target);
    }
}
