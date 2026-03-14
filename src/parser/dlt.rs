//! DLT (AUTOSAR Diagnostic Log and Trace) text format parser.
//!
//! Parses the text output produced by `dlt-convert -a`. Each line has the form:
//!
//! ```text
//! YYYY/MM/DD HH:MM:SS.ffffff TIMESTAMP ECUID APID CTID TYPE SUBTYPE MODE NOAR PAYLOAD
//! ```
//!
//! Missing fields use `----` as a placeholder.

use super::types::{DisplayParts, LogFormatParser};

#[derive(Debug)]
pub struct DltParser;

fn subtype_to_level(subtype: &str) -> Option<&'static str> {
    match subtype {
        "fatal" => Some("FATAL"),
        "error" => Some("ERROR"),
        "warn" => Some("WARN"),
        "info" => Some("INFO"),
        "debug" => Some("DEBUG"),
        "verbose" => Some("TRACE"),
        _ => None,
    }
}

fn is_dlt_type_keyword(s: &str) -> bool {
    matches!(s, "log" | "trace" | "network" | "control")
}

fn parse_dlt_text_line<'a>(line: &'a [u8]) -> Option<DisplayParts<'a>> {
    let s = std::str::from_utf8(line).ok()?;

    // Minimum: "YYYY/MM/DD HH:MM:SS.ffffff" = 26 chars
    if s.len() < 26 {
        return None;
    }

    // Validate timestamp pattern: YYYY/MM/DD HH:MM:SS.ffffff
    let ts_bytes = s.as_bytes();
    if ts_bytes[4] != b'/'
        || ts_bytes[7] != b'/'
        || ts_bytes[10] != b' '
        || ts_bytes[13] != b':'
        || ts_bytes[16] != b':'
        || ts_bytes[19] != b'.'
    {
        return None;
    }

    // Verify digits in expected positions
    if !ts_bytes[0..4].iter().all(|b| b.is_ascii_digit())
        || !ts_bytes[5..7].iter().all(|b| b.is_ascii_digit())
        || !ts_bytes[8..10].iter().all(|b| b.is_ascii_digit())
        || !ts_bytes[11..13].iter().all(|b| b.is_ascii_digit())
        || !ts_bytes[14..16].iter().all(|b| b.is_ascii_digit())
        || !ts_bytes[17..19].iter().all(|b| b.is_ascii_digit())
        || !ts_bytes[20..26].iter().all(|b| b.is_ascii_digit())
    {
        return None;
    }

    let timestamp = &s[..26];

    // Split remaining by whitespace
    let rest = s[26..].trim_start();
    let mut parts_iter = rest.splitn(10, char::is_whitespace);

    let hw_timestamp = parts_iter.next()?; // DLT timestamp counter
    let mcnt = parts_iter.next()?; // message counter
    let ecu = parts_iter.next()?;
    let apid = parts_iter.next()?;
    let ctid = parts_iter.next()?;
    let msg_type = parts_iter.next()?;
    let subtype = parts_iter.next()?;
    let mode = parts_iter.next()?;
    let _noar = parts_iter.next()?;
    let payload = parts_iter.next().unwrap_or("");

    if !is_dlt_type_keyword(msg_type) {
        return None;
    }

    let level = if msg_type == "log" {
        subtype_to_level(subtype)
    } else {
        None
    };

    let target = if apid != "----" { Some(apid) } else { None };

    let mut extra_fields = Vec::new();
    if hw_timestamp != "0" && hw_timestamp != "----" {
        extra_fields.push(("hw_ts", hw_timestamp));
    }
    if mcnt != "---" {
        extra_fields.push(("mcnt", mcnt));
    }
    if ecu != "----" {
        extra_fields.push(("ecu", ecu));
    }
    if ctid != "----" {
        extra_fields.push(("ctid", ctid));
    }
    extra_fields.push(("type", msg_type));
    extra_fields.push(("subtype", subtype));
    if mode != "----" {
        extra_fields.push(("mode", mode));
    }

    let message = if payload.is_empty() {
        None
    } else {
        Some(payload)
    };

    Some(DisplayParts {
        timestamp: Some(timestamp),
        level,
        target,
        span: None,
        extra_fields,
        message,
    })
}

impl LogFormatParser for DltParser {
    fn parse_line<'a>(&self, line: &'a [u8]) -> Option<DisplayParts<'a>> {
        parse_dlt_text_line(line)
    }

    fn collect_field_names(&self, _lines: &[&[u8]]) -> Vec<String> {
        vec![
            "timestamp".to_string(),
            "level".to_string(),
            "target".to_string(),
            "hw_ts".to_string(),
            "mcnt".to_string(),
            "ecu".to_string(),
            "ctid".to_string(),
            "type".to_string(),
            "subtype".to_string(),
            "mode".to_string(),
            "message".to_string(),
        ]
    }

    fn detect_score(&self, sample: &[&[u8]]) -> f64 {
        if sample.is_empty() {
            return 0.0;
        }
        let parsed = sample
            .iter()
            .filter(|l| self.parse_line(l).is_some())
            .count();
        if parsed == 0 {
            return 0.0;
        }
        // DLT text is very distinctive; score slightly above 1.0 to beat JSON
        // but below OTLP's 1.5.
        parsed as f64 / sample.len() as f64 * 1.2
    }

    fn name(&self) -> &str {
        "dlt"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL_LINE: &[u8] =
        b"2024/01/15 10:30:45.123456 1234567 000 ECU1 APP1 CTX1 log info verbose 1 Message text here";

    #[test]
    fn test_parse_complete_line() {
        let parser = DltParser;
        let parts = parser.parse_line(FULL_LINE).unwrap();
        assert_eq!(parts.timestamp, Some("2024/01/15 10:30:45.123456"));
        assert_eq!(parts.level, Some("INFO"));
        assert_eq!(parts.target, Some("APP1"));
        assert_eq!(parts.message, Some("Message text here"));
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|&(k, v)| k == "hw_ts" && v == "1234567")
        );
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|&(k, v)| k == "ecu" && v == "ECU1")
        );
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|&(k, v)| k == "ctid" && v == "CTX1")
        );
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|&(k, v)| k == "type" && v == "log")
        );
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|&(k, v)| k == "subtype" && v == "info")
        );
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|&(k, v)| k == "mode" && v == "verbose")
        );
    }

    #[test]
    fn test_parse_line_with_placeholder_fields() {
        let line = b"2024/01/15 10:30:45.123456 0 000 ---- ---- ---- log info ---- 0 Some payload";
        let parser = DltParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024/01/15 10:30:45.123456"));
        assert_eq!(parts.target, None);
        assert!(!parts.extra_fields.iter().any(|&(k, _)| k == "hw_ts"));
        assert!(!parts.extra_fields.iter().any(|&(k, _)| k == "ecu"));
        assert!(!parts.extra_fields.iter().any(|&(k, _)| k == "ctid"));
        assert!(!parts.extra_fields.iter().any(|&(k, _)| k == "mode"));
    }

    #[test]
    fn test_subtype_fatal_to_level() {
        let line =
            b"2024/01/15 10:30:45.123456 1234567 000 ECU1 APP1 CTX1 log fatal verbose 1 Fatal error";
        let parts = DltParser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("FATAL"));
    }

    #[test]
    fn test_subtype_error_to_level() {
        let line =
            b"2024/01/15 10:30:45.123456 1234567 000 ECU1 APP1 CTX1 log error verbose 1 An error";
        let parts = DltParser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("ERROR"));
    }

    #[test]
    fn test_subtype_warn_to_level() {
        let line =
            b"2024/01/15 10:30:45.123456 1234567 000 ECU1 APP1 CTX1 log warn verbose 1 A warning";
        let parts = DltParser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("WARN"));
    }

    #[test]
    fn test_subtype_debug_to_level() {
        let line =
            b"2024/01/15 10:30:45.123456 1234567 000 ECU1 APP1 CTX1 log debug verbose 1 Debug msg";
        let parts = DltParser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("DEBUG"));
    }

    #[test]
    fn test_subtype_verbose_to_level() {
        let line =
            b"2024/01/15 10:30:45.123456 1234567 000 ECU1 APP1 CTX1 log verbose verbose 1 Trace msg";
        let parts = DltParser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("TRACE"));
    }

    #[test]
    fn test_non_log_type_no_level() {
        for msg_type in &["trace", "network", "control"] {
            let line = format!(
                "2024/01/15 10:30:45.123456 1234567 000 ECU1 APP1 CTX1 {} info verbose 1 Payload",
                msg_type
            );
            let parts = DltParser.parse_line(line.as_bytes()).unwrap();
            assert_eq!(parts.level, None, "type={} should have no level", msg_type);
        }
    }

    #[test]
    fn test_detect_score_high_for_dlt() {
        let parser = DltParser;
        let lines: Vec<&[u8]> = vec![
            b"2024/01/15 10:30:45.123456 1234567 000 ECU1 APP1 CTX1 log info verbose 1 Msg1",
            b"2024/01/15 10:30:46.123456 1234568 001 ECU1 APP1 CTX1 log warn verbose 1 Msg2",
        ];
        let score = parser.detect_score(&lines);
        assert!(score > 1.0, "DLT text should score > 1.0, got {score}");
    }

    #[test]
    fn test_detect_score_zero_for_non_dlt() {
        let parser = DltParser;
        let lines: Vec<&[u8]> = vec![b"plain text log line", br#"{"level":"INFO","msg":"hello"}"#];
        let score = parser.detect_score(&lines);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_detect_score_empty() {
        assert_eq!(DltParser.detect_score(&[]), 0.0);
    }

    #[test]
    fn test_collect_field_names() {
        let parser = DltParser;
        let names = parser.collect_field_names(&[]);
        assert_eq!(
            names,
            vec![
                "timestamp",
                "level",
                "target",
                "hw_ts",
                "mcnt",
                "ecu",
                "ctid",
                "type",
                "subtype",
                "mode",
                "message"
            ]
        );
    }

    #[test]
    fn test_name() {
        assert_eq!(DltParser.name(), "dlt");
    }

    #[test]
    fn test_empty_payload() {
        let line = b"2024/01/15 10:30:45.123456 1234567 000 ECU1 APP1 CTX1 log info verbose 0 ";
        let parts = DltParser.parse_line(line).unwrap();
        assert_eq!(parts.message, None);
    }

    #[test]
    fn test_multi_word_payload() {
        let line = b"2024/01/15 10:30:45.123456 1234567 000 ECU1 APP1 CTX1 log info verbose 1 This is a multi word payload with spaces";
        let parts = DltParser.parse_line(line).unwrap();
        assert_eq!(
            parts.message,
            Some("This is a multi word payload with spaces")
        );
    }

    #[test]
    fn test_too_short_line_rejected() {
        assert!(DltParser.parse_line(b"short").is_none());
    }

    #[test]
    fn test_invalid_timestamp_rejected() {
        let line = b"not-a-timestamp 1234567 000 ECU1 APP1 CTX1 log info verbose 1 msg";
        assert!(DltParser.parse_line(line).is_none());
    }

    #[test]
    fn test_invalid_type_rejected() {
        let line =
            b"2024/01/15 10:30:45.123456 1234567 000 ECU1 APP1 CTX1 unknown info verbose 1 msg";
        assert!(DltParser.parse_line(line).is_none());
    }
}
