// ---------------------------------------------------------------------------
// Web server error log parser (nginx error + Apache 2.4 error)
// ---------------------------------------------------------------------------
//
// nginx error:
//   2024/01/15 10:30:00 [error] 1234#5678: *99 message, client: 1.2.3.4, server: example.com
//
// Apache 2.4 error:
//   [Mon Jan 15 10:30:00.123456 2024] [module:level] [pid 1234:tid 5678] message

use std::collections::HashSet;

use super::timestamp::{normalize_level, parse_apache_error_timestamp, parse_slash_datetime};
use super::types::{DisplayParts, LogFormatParser};

/// Zero-copy parser for nginx and Apache error logs.
#[derive(Debug)]
pub struct WebErrorParser;

/// Parse an nginx error log line.
/// Format: `YYYY/MM/DD HH:MM:SS [level] pid#tid: *connid message, key: val, ...`
fn parse_nginx_error(s: &str) -> Option<DisplayParts<'_>> {
    let (timestamp, consumed) = parse_slash_datetime(s)?;

    let rest = s.get(consumed..)?.strip_prefix(' ')?;
    if rest.is_empty() {
        return None;
    }

    // [level]
    if !rest.starts_with('[') {
        return None;
    }
    let close_bracket = rest.find(']')?;
    let level_str = &rest[1..close_bracket];
    let level = normalize_level(level_str).unwrap_or(level_str);
    let rest = &rest[close_bracket + 1..];
    let rest = rest.strip_prefix(' ').unwrap_or(rest);

    let mut parts = DisplayParts {
        timestamp: Some(timestamp),
        level: Some(level),
        ..Default::default()
    };

    // pid#tid:
    if let Some(colon_pos) = rest.find(": ") {
        let pid_tid = &rest[..colon_pos];
        if let Some(hash) = pid_tid.find('#') {
            let pid = &pid_tid[..hash];
            let tid = &pid_tid[hash + 1..];
            if !pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit()) {
                parts.extra_fields.push(("pid", pid));
            }
            if !tid.is_empty() && tid.bytes().all(|b| b.is_ascii_digit()) {
                parts.extra_fields.push(("tid", tid));
            }
        }
        let rest = &rest[colon_pos + 2..];

        // Optional *connid prefix
        let msg = if let Some(stripped) = rest.strip_prefix('*') {
            if let Some(space) = stripped.find(' ') {
                &stripped[space + 1..]
            } else {
                stripped
            }
        } else {
            rest
        };

        // Parse trailing key: value pairs (nginx appends client, server, etc.)
        let (message, extras) = split_nginx_kv(msg);
        if !message.is_empty() {
            parts.message = Some(message);
        }
        for (k, v) in extras {
            parts.extra_fields.push((k, v));
        }
    } else if !rest.is_empty() {
        parts.message = Some(rest);
    }

    Some(parts)
}

/// Split an nginx message into the main message and trailing `key: value` pairs.
/// Returns (message, Vec<(key, value)>).
fn split_nginx_kv(s: &str) -> (&str, Vec<(&str, &str)>) {
    let mut extras = Vec::new();
    // Work backwards from known nginx suffix keys
    let known_keys = [
        "client", "server", "request", "upstream", "host", "referrer",
    ];
    let mut msg_end = s.len();

    // Find the last occurrence of ", key: " patterns
    let mut search_pos = s.len();
    while search_pos > 0 {
        if let Some(comma_pos) = s[..search_pos].rfind(", ") {
            let after_comma = &s[comma_pos + 2..];
            if let Some(colon_pos) = after_comma.find(": ") {
                let key = &after_comma[..colon_pos];
                if known_keys.contains(&key) {
                    let value_start = comma_pos + 2 + colon_pos + 2;
                    // Value extends to end or next ", key: "
                    let value_end = if msg_end > value_start {
                        msg_end
                    } else {
                        s.len()
                    };
                    let value = s[value_start..value_end].trim_end_matches(", ");
                    extras.push((key, value));
                    msg_end = comma_pos;
                    search_pos = comma_pos;
                    continue;
                }
            }
            search_pos = comma_pos;
        } else {
            break;
        }
    }

    extras.reverse();
    (&s[..msg_end], extras)
}

/// Parse an Apache 2.4 error log line.
/// Format: `[Www Mmm DD HH:MM:SS.us YYYY] [module:level] [pid N:tid N] message`
fn parse_apache_error(s: &str) -> Option<DisplayParts<'_>> {
    let (timestamp, consumed) = parse_apache_error_timestamp(s)?;

    let rest = s.get(consumed..)?.strip_prefix(' ')?;
    if rest.is_empty() {
        return None;
    }

    // [module:level]
    if !rest.starts_with('[') {
        return None;
    }
    let close = rest.find(']')?;
    let module_level = &rest[1..close];
    let rest = &rest[close + 1..];
    let rest = rest.strip_prefix(' ').unwrap_or(rest);

    let mut parts = DisplayParts {
        timestamp: Some(timestamp),
        ..Default::default()
    };

    // Parse module:level
    if let Some(colon_pos) = module_level.find(':') {
        let module = &module_level[..colon_pos];
        let level_str = &module_level[colon_pos + 1..];
        parts.level = Some(normalize_level(level_str).unwrap_or(level_str));
        if !module.is_empty() {
            parts.extra_fields.push(("module", module));
        }
    } else {
        // Just a level or module
        if let Some(normalized) = normalize_level(module_level) {
            parts.level = Some(normalized);
        } else {
            parts.extra_fields.push(("module", module_level));
        }
    }

    // Optional [pid N:tid N] or [pid N]
    if rest.starts_with('[')
        && let Some(close) = rest.find(']')
    {
        let pid_section = &rest[1..close];
        parse_apache_pid_tid(pid_section, &mut parts);
        let rest = &rest[close + 1..];
        let rest = rest.strip_prefix(' ').unwrap_or(rest);

        // Optional [client IP:port]
        if rest.starts_with("[client ")
            && let Some(close) = rest.find(']')
        {
            let client = &rest[8..close];
            parts.extra_fields.push(("client", client));
            let rest = &rest[close + 1..];
            let rest = rest.strip_prefix(' ').unwrap_or(rest);
            if !rest.is_empty() {
                parts.message = Some(rest);
            }
            return Some(parts);
        }

        if !rest.is_empty() {
            parts.message = Some(rest);
        }
        return Some(parts);
    }

    if !rest.is_empty() {
        parts.message = Some(rest);
    }

    Some(parts)
}

/// Parse Apache pid/tid section: `pid N:tid N` or `pid N`
fn parse_apache_pid_tid<'a>(section: &'a str, parts: &mut DisplayParts<'a>) {
    if let Some(stripped) = section.strip_prefix("pid ") {
        if let Some(colon) = stripped.find(":tid ") {
            let pid = &stripped[..colon];
            let tid = &stripped[colon + 5..];
            parts.extra_fields.push(("pid", pid));
            parts.extra_fields.push(("tid", tid));
        } else {
            parts.extra_fields.push(("pid", stripped));
        }
    }
}

impl LogFormatParser for WebErrorParser {
    fn parse_line<'a>(&self, line: &'a [u8]) -> Option<DisplayParts<'a>> {
        let s = std::str::from_utf8(line).ok()?;
        if s.is_empty() {
            return None;
        }

        // Try nginx error first (starts with slash-date)
        if let Some(parts) = parse_nginx_error(s) {
            return Some(parts);
        }

        // Try Apache 2.4 error (starts with [weekday ...)
        if let Some(parts) = parse_apache_error(s) {
            return Some(parts);
        }

        None
    }

    fn collect_field_names(&self, lines: &[&[u8]]) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut extras = Vec::new();
        let mut has_level = false;
        let mut has_target = false;

        for &line in lines {
            if let Some(parts) = self.parse_line(line) {
                if parts.level.is_some() {
                    has_level = true;
                }
                if parts.target.is_some() {
                    has_target = true;
                }
                for (key, _) in &parts.extra_fields {
                    let k = key.to_string();
                    if seen.insert(k.clone()) {
                        extras.push(k);
                    }
                }
            }
        }

        let mut result = vec!["timestamp".to_string()];
        if has_level {
            result.push("level".to_string());
        }
        if has_target {
            result.push("target".to_string());
        }
        // Preserve discovery order for extras
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
        "web-error"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── nginx error parsing ───────────────────────────────────────────

    #[test]
    fn test_nginx_error_basic() {
        let line =
            b"2024/01/15 10:30:00 [error] 1234#5678: *99 open() failed, client: 1.2.3.4, server: example.com";
        let parser = WebErrorParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024/01/15 10:30:00"));
        assert_eq!(parts.level, Some("ERROR"));
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
                .any(|(k, v)| *k == "tid" && *v == "5678")
        );
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "client" && *v == "1.2.3.4")
        );
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "server" && *v == "example.com")
        );
        assert_eq!(parts.message, Some("open() failed"));
    }

    #[test]
    fn test_nginx_error_warn() {
        let line = b"2024/01/15 10:30:00 [warn] 1#0: msg here";
        let parser = WebErrorParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("WARN"));
    }

    #[test]
    fn test_nginx_error_crit() {
        let line = b"2024/01/15 10:30:00 [crit] 1#0: something critical";
        let parser = WebErrorParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("FATAL"));
    }

    #[test]
    fn test_nginx_error_no_connid() {
        let line = b"2024/01/15 10:30:00 [error] 1#0: some msg without connid";
        let parser = WebErrorParser;
        let parts = parser.parse_line(line).unwrap();
        assert!(parts.message.is_some());
    }

    #[test]
    fn test_nginx_error_with_fractional() {
        let line = b"2024/01/15 10:30:00.123 [error] 1#0: msg";
        let parser = WebErrorParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024/01/15 10:30:00.123"));
    }

    // ── Apache error parsing ──────────────────────────────────────────

    #[test]
    fn test_apache_error_basic() {
        let line = b"[Mon Jan 15 10:30:00.123456 2024] [core:error] [pid 1234:tid 5678] [client 192.168.1.1:54321] File does not exist";
        let parser = WebErrorParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("[Mon Jan 15 10:30:00.123456 2024]"));
        assert_eq!(parts.level, Some("ERROR"));
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "module" && *v == "core")
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
                .any(|(k, v)| *k == "tid" && *v == "5678")
        );
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "client" && *v == "192.168.1.1:54321")
        );
        assert_eq!(parts.message, Some("File does not exist"));
    }

    #[test]
    fn test_apache_error_no_client() {
        let line = b"[Fri Dec 31 23:59:59 2024] [mpm_event:notice] [pid 100] AH00489: Apache/2.4 configured";
        let parser = WebErrorParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("NOTICE"));
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "module" && *v == "mpm_event")
        );
        assert!(parts.message.is_some());
    }

    #[test]
    fn test_apache_error_warn_level() {
        let line = b"[Mon Jan 15 10:30:00 2024] [ssl:warn] [pid 1] AH01909: warning";
        let parser = WebErrorParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("WARN"));
    }

    // ── Negative cases ────────────────────────────────────────────────

    #[test]
    fn test_parse_empty() {
        let parser = WebErrorParser;
        assert!(parser.parse_line(b"").is_none());
    }

    #[test]
    fn test_parse_not_web_error() {
        let parser = WebErrorParser;
        assert!(parser.parse_line(b"just plain text").is_none());
    }

    #[test]
    fn test_parse_json_not_web_error() {
        let parser = WebErrorParser;
        assert!(
            parser
                .parse_line(br#"{"level":"INFO","msg":"hello"}"#)
                .is_none()
        );
    }

    #[test]
    fn test_parse_clf_not_web_error() {
        let parser = WebErrorParser;
        assert!(
            parser
                .parse_line(
                    b"127.0.0.1 - - [10/Oct/2000:13:55:36 -0700] \"GET / HTTP/1.0\" 200 100"
                )
                .is_none()
        );
    }

    // ── detect_score ─────────────────────────────────────────────────

    #[test]
    fn test_detect_score_all_nginx() {
        let parser = WebErrorParser;
        let lines: Vec<&[u8]> = vec![
            b"2024/01/15 10:30:00 [error] 1#0: msg1",
            b"2024/01/15 10:30:01 [warn] 1#0: msg2",
        ];
        let score = parser.detect_score(&lines);
        assert!((score - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_detect_score_all_apache() {
        let parser = WebErrorParser;
        let lines: Vec<&[u8]> = vec![
            b"[Mon Jan 15 10:30:00 2024] [core:error] [pid 1] msg1",
            b"[Mon Jan 15 10:30:01 2024] [ssl:warn] [pid 1] msg2",
        ];
        let score = parser.detect_score(&lines);
        assert!((score - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_detect_score_none() {
        let parser = WebErrorParser;
        let lines: Vec<&[u8]> = vec![b"plain text", b"more text"];
        let score = parser.detect_score(&lines);
        assert!((score - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_detect_score_empty() {
        let parser = WebErrorParser;
        let lines: Vec<&[u8]> = vec![];
        let score = parser.detect_score(&lines);
        assert!((score - 0.0).abs() < 0.001);
    }

    // ── collect_field_names ──────────────────────────────────────────

    #[test]
    fn test_collect_field_names_nginx() {
        let parser = WebErrorParser;
        let lines: Vec<&[u8]> =
            vec![b"2024/01/15 10:30:00 [error] 1234#5678: *99 msg, client: 1.2.3.4"];
        let names = parser.collect_field_names(&lines);
        assert!(names.contains(&"timestamp".to_string()));
        assert!(names.contains(&"level".to_string()));
        assert!(names.contains(&"message".to_string()));
    }

    #[test]
    fn test_collect_field_names_apache() {
        let parser = WebErrorParser;
        let lines: Vec<&[u8]> = vec![b"[Mon Jan 15 10:30:00 2024] [core:error] [pid 1:tid 2] msg"];
        let names = parser.collect_field_names(&lines);
        assert!(names.contains(&"timestamp".to_string()));
        assert!(names.contains(&"level".to_string()));
        assert!(names.contains(&"module".to_string()));
    }

    // ── name ─────────────────────────────────────────────────────────

    #[test]
    fn test_name() {
        let parser = WebErrorParser;
        assert_eq!(parser.name(), "web-error");
    }

    // ── split_nginx_kv ───────────────────────────────────────────────

    #[test]
    fn test_split_nginx_kv_with_pairs() {
        let (msg, extras) = split_nginx_kv("open() failed, client: 1.2.3.4, server: example.com");
        assert_eq!(msg, "open() failed");
        assert_eq!(extras.len(), 2);
        assert_eq!(extras[0], ("client", "1.2.3.4"));
        assert_eq!(extras[1], ("server", "example.com"));
    }

    #[test]
    fn test_split_nginx_kv_no_pairs() {
        let (msg, extras) = split_nginx_kv("just a message");
        assert_eq!(msg, "just a message");
        assert!(extras.is_empty());
    }

    #[test]
    fn test_split_nginx_kv_request_pair() {
        let (msg, extras) = split_nginx_kv(
            "upstream timed out, client: 1.2.3.4, server: s, request: \"GET /api HTTP/1.1\"",
        );
        assert_eq!(msg, "upstream timed out");
        assert!(extras.iter().any(|(k, _)| *k == "request"));
    }
}
