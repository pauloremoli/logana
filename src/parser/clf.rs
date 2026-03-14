//! Common Log Format (CLF) and Combined Log Format parser.

use std::collections::HashSet;

use super::types::{DisplayParts, LogFormatParser};

#[derive(Debug)]
pub struct ClfParser;

const CLF_MONTHS: &[&str] = &[
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn parse_clf_line(s: &str) -> Option<DisplayParts<'_>> {
    let mut pos = 0;

    let (host, new_pos) = next_token(s, pos)?;
    pos = new_pos;

    let (ident, new_pos) = next_token(s, pos)?;
    pos = new_pos;

    let (authuser, new_pos) = next_token(s, pos)?;
    pos = new_pos;

    pos = skip_spaces(s, pos);
    if pos >= s.len() || s.as_bytes()[pos] != b'[' {
        return None;
    }
    pos += 1;
    let date_start = pos;
    let close_bracket = s[pos..].find(']')?;
    let date = &s[date_start..pos + close_bracket];
    pos = pos + close_bracket + 1;

    if !validate_clf_date(date) {
        return None;
    }

    pos = skip_spaces(s, pos);
    let (request, new_pos) = read_quoted(s, pos)?;
    pos = new_pos;

    let (status, new_pos) = next_token(s, pos)?;
    pos = new_pos;
    if status != "-" && (status.len() != 3 || !status.as_bytes().iter().all(|b| b.is_ascii_digit()))
    {
        return None;
    }

    let (bytes_str, new_pos) = next_token(s, pos)?;
    pos = new_pos;

    let mut parts = DisplayParts {
        timestamp: Some(date),
        message: Some(request),
        target: Some(host),
        ..Default::default()
    };

    if ident != "-" {
        parts.extra_fields.push(("ident", ident));
    }
    if authuser != "-" {
        parts.extra_fields.push(("authuser", authuser));
    }
    parts.extra_fields.push(("status", status));
    if bytes_str != "-" {
        parts.extra_fields.push(("bytes", bytes_str));
    }

    pos = skip_spaces(s, pos);
    if pos < s.len()
        && s.as_bytes()[pos] == b'"'
        && let Some((referer, new_pos)) = read_quoted(s, pos)
    {
        pos = new_pos;
        if referer != "-" {
            parts.extra_fields.push(("referer", referer));
        }

        pos = skip_spaces(s, pos);
        if pos < s.len()
            && s.as_bytes()[pos] == b'"'
            && let Some((user_agent, _)) = read_quoted(s, pos)
            && user_agent != "-"
        {
            parts.extra_fields.push(("user_agent", user_agent));
        }
    }

    Some(parts)
}

fn validate_clf_date(date: &str) -> bool {
    if date.len() < 26 {
        return false;
    }
    let b = date.as_bytes();
    if !b[0].is_ascii_digit() || !b[1].is_ascii_digit() || b[2] != b'/' {
        return false;
    }
    let month = &date[3..6];
    if !CLF_MONTHS.contains(&month) || b[6] != b'/' {
        return false;
    }
    if !b[7].is_ascii_digit()
        || !b[8].is_ascii_digit()
        || !b[9].is_ascii_digit()
        || !b[10].is_ascii_digit()
        || b[11] != b':'
    {
        return false;
    }
    if !b[12].is_ascii_digit()
        || !b[13].is_ascii_digit()
        || b[14] != b':'
        || !b[15].is_ascii_digit()
        || !b[16].is_ascii_digit()
        || b[17] != b':'
        || !b[18].is_ascii_digit()
        || !b[19].is_ascii_digit()
    {
        return false;
    }
    if b[20] != b' ' || !matches!(b[21], b'+' | b'-') {
        return false;
    }
    if !b[22].is_ascii_digit()
        || !b[23].is_ascii_digit()
        || !b[24].is_ascii_digit()
        || !b[25].is_ascii_digit()
    {
        return false;
    }
    true
}

fn next_token(s: &str, pos: usize) -> Option<(&str, usize)> {
    let start = skip_spaces(s, pos);
    if start >= s.len() {
        return None;
    }
    let end = s[start..].find(' ').map(|p| p + start).unwrap_or(s.len());
    let after = skip_spaces(s, end);
    Some((&s[start..end], after))
}

fn read_quoted(s: &str, pos: usize) -> Option<(&str, usize)> {
    if pos >= s.len() || s.as_bytes()[pos] != b'"' {
        return None;
    }
    let start = pos + 1;
    let mut end = start;
    let bytes = s.as_bytes();
    while end < bytes.len() {
        if bytes[end] == b'\\' && end + 1 < bytes.len() {
            end += 2;
        } else if bytes[end] == b'"' {
            break;
        } else {
            end += 1;
        }
    }
    if end >= bytes.len() {
        return None;
    }
    Some((&s[start..end], end + 1))
}

fn skip_spaces(s: &str, mut pos: usize) -> usize {
    let bytes = s.as_bytes();
    while pos < bytes.len() && bytes[pos] == b' ' {
        pos += 1;
    }
    pos
}

impl LogFormatParser for ClfParser {
    fn parse_line<'a>(&self, line: &'a [u8]) -> Option<DisplayParts<'a>> {
        let s = std::str::from_utf8(line).ok()?;
        if s.is_empty() {
            return None;
        }
        parse_clf_line(s)
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
        result.extend(extras);
        result.push("message".to_string());
        result
    }

    fn name(&self) -> &str {
        "clf"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clf_full_line() {
        let line =
            b"127.0.0.1 - frank [10/Oct/2000:13:55:36 -0700] \"GET /apache_pb.gif HTTP/1.0\" 200 2326";
        let parser = ClfParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("10/Oct/2000:13:55:36 -0700"));
        assert_eq!(parts.target, Some("127.0.0.1"));
        assert_eq!(parts.message, Some("GET /apache_pb.gif HTTP/1.0"));
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "authuser" && *v == "frank")
        );
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "status" && *v == "200")
        );
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "bytes" && *v == "2326")
        );
        // ident is "-" so should be absent
        assert!(!parts.extra_fields.iter().any(|(k, _)| *k == "ident"));
    }

    #[test]
    fn test_clf_dash_fields() {
        let line = b"192.168.1.1 - - [01/Jan/2024:00:00:00 +0000] \"POST /api HTTP/1.1\" 201 -";
        let parser = ClfParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.target, Some("192.168.1.1"));
        assert_eq!(parts.message, Some("POST /api HTTP/1.1"));
        assert!(!parts.extra_fields.iter().any(|(k, _)| *k == "ident"));
        assert!(!parts.extra_fields.iter().any(|(k, _)| *k == "authuser"));
        assert!(!parts.extra_fields.iter().any(|(k, _)| *k == "bytes"));
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "status" && *v == "201")
        );
    }

    #[test]
    fn test_clf_with_ident() {
        let line =
            b"10.0.0.1 user-id admin [15/Feb/2024:08:30:00 +0100] \"DELETE /item/5 HTTP/2\" 204 0";
        let parser = ClfParser;
        let parts = parser.parse_line(line).unwrap();
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "ident" && *v == "user-id")
        );
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "authuser" && *v == "admin")
        );
    }

    // ── Combined Log Format ────────────────────────────────────────────

    #[test]
    fn test_combined_full_line() {
        let line = b"127.0.0.1 - jane [10/Oct/2000:13:55:36 -0700] \"GET /index.html HTTP/1.0\" 200 2326 \"http://www.example.com/start.html\" \"Mozilla/4.08 [en] (Win98; I ;Nav)\"";
        let parser = ClfParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("10/Oct/2000:13:55:36 -0700"));
        assert_eq!(parts.target, Some("127.0.0.1"));
        assert_eq!(parts.message, Some("GET /index.html HTTP/1.0"));
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "referer" && *v == "http://www.example.com/start.html")
        );
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "user_agent" && *v == "Mozilla/4.08 [en] (Win98; I ;Nav)")
        );
    }

    #[test]
    fn test_combined_dash_referer_and_agent() {
        let line =
            b"10.0.0.1 - - [01/Jan/2024:00:00:00 +0000] \"GET / HTTP/1.1\" 200 512 \"-\" \"-\"";
        let parser = ClfParser;
        let parts = parser.parse_line(line).unwrap();
        // "-" values should be omitted
        assert!(!parts.extra_fields.iter().any(|(k, _)| *k == "referer"));
        assert!(!parts.extra_fields.iter().any(|(k, _)| *k == "user_agent"));
    }

    #[test]
    fn test_combined_only_referer_no_agent() {
        let line = b"10.0.0.1 - - [01/Jan/2024:00:00:00 +0000] \"GET / HTTP/1.1\" 200 512 \"http://example.com\"";
        let parser = ClfParser;
        let parts = parser.parse_line(line).unwrap();
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "referer" && *v == "http://example.com")
        );
        assert!(!parts.extra_fields.iter().any(|(k, _)| *k == "user_agent"));
    }

    // ── Date validation ────────────────────────────────────────────────

    #[test]
    fn test_validate_clf_date_valid() {
        assert!(validate_clf_date("10/Oct/2000:13:55:36 -0700"));
        assert!(validate_clf_date("01/Jan/2024:00:00:00 +0000"));
        assert!(validate_clf_date("31/Dec/1999:23:59:59 +1200"));
    }

    #[test]
    fn test_validate_clf_date_invalid_month() {
        assert!(!validate_clf_date("10/Xxx/2000:13:55:36 -0700"));
    }

    #[test]
    fn test_validate_clf_date_too_short() {
        assert!(!validate_clf_date("10/Oct/2000:13:55:36"));
    }

    #[test]
    fn test_validate_clf_date_bad_format() {
        assert!(!validate_clf_date("2024-01-15T10:00:00+0000aaaa"));
    }

    // ── Edge cases ─────────────────────────────────────────────────────

    #[test]
    fn test_parse_empty_line() {
        let parser = ClfParser;
        assert!(parser.parse_line(b"").is_none());
    }

    #[test]
    fn test_parse_json_not_clf() {
        let parser = ClfParser;
        assert!(
            parser
                .parse_line(br#"{"level":"INFO","msg":"hello"}"#)
                .is_none()
        );
    }

    #[test]
    fn test_parse_plain_text_not_clf() {
        let parser = ClfParser;
        assert!(parser.parse_line(b"just plain text").is_none());
    }

    #[test]
    fn test_parse_syslog_not_clf() {
        let parser = ClfParser;
        assert!(
            parser
                .parse_line(b"<134>Oct 11 22:14:15 myhost sshd[1234]: msg")
                .is_none()
        );
    }

    #[test]
    fn test_parse_invalid_status_code() {
        let parser = ClfParser;
        // "abc" is not a valid status
        assert!(
            parser
                .parse_line(
                    b"127.0.0.1 - - [10/Oct/2000:13:55:36 -0700] \"GET / HTTP/1.0\" abc 100"
                )
                .is_none()
        );
    }

    #[test]
    fn test_parse_missing_request_quotes() {
        let parser = ClfParser;
        // Missing quotes around request
        assert!(
            parser
                .parse_line(b"127.0.0.1 - - [10/Oct/2000:13:55:36 -0700] GET / HTTP/1.0 200 100")
                .is_none()
        );
    }

    #[test]
    fn test_parse_unclosed_bracket() {
        let parser = ClfParser;
        assert!(
            parser
                .parse_line(b"127.0.0.1 - - [10/Oct/2000:13:55:36 -0700 \"GET / HTTP/1.0\" 200 100")
                .is_none()
        );
    }

    #[test]
    fn test_dash_status() {
        let parser = ClfParser;
        let line = b"127.0.0.1 - - [01/Jan/2024:00:00:00 +0000] \"GET / HTTP/1.1\" - 0";
        let parts = parser.parse_line(line).unwrap();
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "status" && *v == "-")
        );
    }

    // ── detect_score ───────────────────────────────────────────────────

    #[test]
    fn test_detect_score_all_clf() {
        let parser = ClfParser;
        let lines: Vec<&[u8]> = vec![
            b"127.0.0.1 - - [10/Oct/2000:13:55:36 -0700] \"GET /a HTTP/1.0\" 200 100",
            b"10.0.0.1 - user [10/Oct/2000:13:55:37 -0700] \"POST /b HTTP/1.1\" 201 50",
        ];
        let score = parser.detect_score(&lines);
        assert!((score - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_detect_score_mixed() {
        let parser = ClfParser;
        let lines: Vec<&[u8]> = vec![
            b"127.0.0.1 - - [10/Oct/2000:13:55:36 -0700] \"GET / HTTP/1.0\" 200 100",
            b"not clf at all",
        ];
        let score = parser.detect_score(&lines);
        assert!((score - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_detect_score_none() {
        let parser = ClfParser;
        let lines: Vec<&[u8]> = vec![b"plain text", b"more text"];
        let score = parser.detect_score(&lines);
        assert!((score - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_detect_score_empty() {
        let parser = ClfParser;
        let lines: Vec<&[u8]> = vec![];
        let score = parser.detect_score(&lines);
        assert!((score - 0.0).abs() < 0.001);
    }

    // ── collect_field_names ────────────────────────────────────────────

    #[test]
    fn test_collect_field_names_clf() {
        let parser = ClfParser;
        let lines: Vec<&[u8]> =
            vec![b"127.0.0.1 - frank [10/Oct/2000:13:55:36 -0700] \"GET / HTTP/1.0\" 200 2326"];
        let names = parser.collect_field_names(&lines);
        assert_eq!(names[0], "timestamp");
        assert_eq!(names[1], "target");
        assert!(names.contains(&"authuser".to_string()));
        assert!(names.contains(&"status".to_string()));
        assert!(names.contains(&"bytes".to_string()));
        assert_eq!(*names.last().unwrap(), "message");
    }

    #[test]
    fn test_collect_field_names_combined() {
        let parser = ClfParser;
        let lines: Vec<&[u8]> = vec![
            b"127.0.0.1 - - [10/Oct/2000:13:55:36 -0700] \"GET / HTTP/1.0\" 200 100 \"http://example.com\" \"Mozilla/5.0\"",
        ];
        let names = parser.collect_field_names(&lines);
        assert!(names.contains(&"referer".to_string()));
        assert!(names.contains(&"user_agent".to_string()));
    }

    // ── name ───────────────────────────────────────────────────────────

    #[test]
    fn test_name() {
        let parser = ClfParser;
        assert_eq!(parser.name(), "clf");
    }
}
