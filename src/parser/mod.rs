pub mod clf;
pub mod journalctl;
pub mod json;
pub mod syslog;
pub mod types;

// Re-export all public items for backward-compatible access via `crate::parser::*`.
pub use clf::ClfParser;
pub use journalctl::JournalctlParser;
pub use json::{
    JsonField, JsonParser, LEVEL_KEYS, LogFormat, LogLine, MESSAGE_KEYS, TARGET_KEYS,
    TIMESTAMP_KEYS, build_display_json, classify_json_fields, classify_json_fields_all,
    detect_json_format, parse_json_line,
};
pub use syslog::SyslogParser;
pub use types::{DisplayParts, LogFormatParser, SpanInfo, format_span_col};

/// Sample lines, try all registered parsers, return best match.
pub fn detect_format(sample: &[&[u8]]) -> Option<Box<dyn LogFormatParser>> {
    if sample.is_empty() {
        return None;
    }

    let parsers: Vec<Box<dyn LogFormatParser>> = vec![
        Box::new(JsonParser),
        Box::new(SyslogParser),
        Box::new(JournalctlParser),
        Box::new(ClfParser),
    ];

    parsers
        .into_iter()
        .map(|p| {
            let score = p.detect_score(sample);
            (p, score)
        })
        .filter(|(_, s)| *s > 0.0)
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
        .map(|(p, _)| p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_format_json() {
        let lines: Vec<&[u8]> = vec![
            br#"{"level":"INFO","msg":"hello"}"#,
            br#"{"level":"WARN","msg":"world"}"#,
        ];
        let parser = detect_format(&lines).unwrap();
        assert_eq!(parser.name(), "json");
    }

    #[test]
    fn test_detect_format_syslog_rfc3164() {
        let lines: Vec<&[u8]> = vec![
            b"<134>Oct 11 22:14:15 myhost sshd[1234]: Accepted password for user",
            b"<134>Oct 11 22:14:16 myhost sshd[1234]: Session opened",
        ];
        let parser = detect_format(&lines).unwrap();
        assert_eq!(parser.name(), "syslog");
    }

    #[test]
    fn test_detect_format_syslog_rfc5424() {
        let lines: Vec<&[u8]> = vec![
            b"<165>1 2003-10-11T22:14:15.003Z mymachine.example.com evntslog - ID47 [exampleSDID@32473 iut=\"3\" eventSource=\"App\"] BOMAn application event log entry...",
        ];
        let parser = detect_format(&lines).unwrap();
        assert_eq!(parser.name(), "syslog");
    }

    #[test]
    fn test_detect_format_raw_text() {
        let lines: Vec<&[u8]> = vec![b"plain text log line 1", b"plain text log line 2"];
        assert!(detect_format(&lines).is_none());
    }

    #[test]
    fn test_detect_format_empty_sample() {
        let lines: Vec<&[u8]> = vec![];
        assert!(detect_format(&lines).is_none());
    }

    #[test]
    fn test_detect_format_mixed_json_wins() {
        // Mostly JSON with one non-JSON line
        let lines: Vec<&[u8]> = vec![
            br#"{"level":"INFO","msg":"hello"}"#,
            b"not json",
            br#"{"level":"WARN","msg":"world"}"#,
            br#"{"level":"ERROR","msg":"fail"}"#,
        ];
        let parser = detect_format(&lines).unwrap();
        assert_eq!(parser.name(), "json");
    }

    #[test]
    fn test_detect_format_journalctl_short_iso() {
        let lines: Vec<&[u8]> = vec![
            b"2024-02-22T10:15:30+0000 myhost sshd[1234]: Accepted password",
            b"2024-02-22T10:15:31+0000 myhost sshd[1234]: Session opened",
        ];
        let parser = detect_format(&lines).unwrap();
        assert_eq!(parser.name(), "journalctl");
    }

    #[test]
    fn test_detect_format_journalctl_short_precise() {
        let lines: Vec<&[u8]> = vec![
            b"Feb 22 10:15:30.123456 myhost sshd[1234]: msg1",
            b"Feb 22 10:15:31.654321 myhost sshd[1234]: msg2",
        ];
        let parser = detect_format(&lines).unwrap();
        assert_eq!(parser.name(), "journalctl");
    }

    #[test]
    fn test_detect_format_journalctl_short_full() {
        let lines: Vec<&[u8]> = vec![
            b"Mon 2024-02-22 10:15:30 UTC myhost sshd[1234]: msg1",
            b"Mon 2024-02-22 10:15:31 UTC myhost sshd[1234]: msg2",
        ];
        let parser = detect_format(&lines).unwrap();
        assert_eq!(parser.name(), "journalctl");
    }

    #[test]
    fn test_detect_format_clf() {
        let lines: Vec<&[u8]> = vec![
            b"127.0.0.1 - frank [10/Oct/2000:13:55:36 -0700] \"GET /a HTTP/1.0\" 200 2326",
            b"10.0.0.1 - - [10/Oct/2000:13:55:37 -0700] \"POST /b HTTP/1.1\" 201 50",
        ];
        let parser = detect_format(&lines).unwrap();
        assert_eq!(parser.name(), "clf");
    }

    #[test]
    fn test_detect_format_combined() {
        let lines: Vec<&[u8]> = vec![
            b"127.0.0.1 - - [10/Oct/2000:13:55:36 -0700] \"GET / HTTP/1.0\" 200 100 \"http://example.com\" \"Mozilla/5.0\"",
        ];
        let parser = detect_format(&lines).unwrap();
        assert_eq!(parser.name(), "clf");
    }
}
