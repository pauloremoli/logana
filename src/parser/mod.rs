//! Log format detection and parser registry.
//!
//! [`detect_format`] samples up to 200 lines and returns the [`LogFormatParser`]
//! with the highest confidence score. Registered parsers in priority order:
//! [`OtlpParser`] (scores up to 1.5 to beat JSON), [`JsonParser`], [`SyslogParser`],
//! [`JournalctlParser`], [`ClfParser`], [`LogfmtParser`],
//! [`CommonLogParser`] (0.95× score penalty as catch-all).

pub mod clf;
pub mod common_log;
pub mod journalctl;
pub mod json;
pub mod logfmt;
pub mod otlp;
pub mod syslog;
pub(crate) mod timestamp;
pub mod types;

// Re-export all public items for backward-compatible access via `crate::parser::*`.
pub use clf::ClfParser;
pub use common_log::CommonLogParser;
pub use journalctl::JournalctlParser;
pub use json::{
    JsonField, JsonParser, LEVEL_KEYS, LogFormat, LogLine, MESSAGE_KEYS, TARGET_KEYS,
    TIMESTAMP_KEYS, build_display_json, classify_json_fields, classify_json_fields_all,
    detect_json_format, parse_json_line,
};
pub use logfmt::LogfmtParser;
pub use otlp::OtlpParser;
pub use syslog::SyslogParser;
pub use types::{DisplayParts, LogFormatParser, SpanInfo, format_span_col};

/// Sample lines, try all registered parsers, return best match.
pub fn detect_format(sample: &[&[u8]]) -> Option<Box<dyn LogFormatParser>> {
    if sample.is_empty() {
        return None;
    }

    let parsers: Vec<Box<dyn LogFormatParser>> = vec![
        // OtlpParser scores up to 1.5 to beat JsonParser (max 1.0) on OTLP files
        Box::new(OtlpParser),
        Box::new(JsonParser),
        Box::new(SyslogParser),
        Box::new(JournalctlParser),
        Box::new(ClfParser),
        Box::new(LogfmtParser),
        // CommonLogParser last — broadest catch-all with 0.95× score penalty
        Box::new(CommonLogParser),
    ];

    parsers
        .into_iter()
        .map(|p| {
            let score = p.detect_score(sample);
            (p, score)
        })
        .filter(|(_, s)| *s > 0.0)
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
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

    #[test]
    fn test_detect_format_rsyslog_file_format() {
        let lines: Vec<&[u8]> = vec![
            b"2026-02-22T00:05:10.113076+01:00 my-pc rsyslogd: [origin software=\"rsyslogd\"] msg",
            b"2026-02-22T00:05:10.119576+01:00 my-pc systemd[1]: logrotate.service: Deactivated successfully.",
            b"2026-02-22T00:07:24.887273+01:00 my-pc systemd[1]: Starting sysstat-summary.service",
        ];
        let parser = detect_format(&lines).unwrap();
        assert_eq!(parser.name(), "journalctl");
    }

    // ── New format detection tests ────────────────────────────────────

    #[test]
    fn test_detect_format_logfmt() {
        let lines: Vec<&[u8]> = vec![
            b"time=2024-01-01T00:00:00Z level=info msg=\"request handled\" status=200",
            b"time=2024-01-01T00:00:01Z level=warn msg=\"slow query\" duration=500ms",
        ];
        let parser = detect_format(&lines).unwrap();
        assert_eq!(parser.name(), "logfmt");
    }

    #[test]
    fn test_detect_format_common_log_env_logger() {
        let lines: Vec<&[u8]> = vec![
            b"[2024-07-24T10:00:00Z INFO  myapp] Starting server",
            b"[2024-07-24T10:00:01Z WARN  myapp] Low memory",
        ];
        let parser = detect_format(&lines).unwrap();
        assert_eq!(parser.name(), "common-log");
    }

    #[test]
    fn test_detect_format_common_log_python_basic() {
        let lines: Vec<&[u8]> = vec![
            b"INFO:root:Application started",
            b"WARNING:django.server:Not Found: /favicon.ico",
        ];
        let parser = detect_format(&lines).unwrap();
        assert_eq!(parser.name(), "common-log");
    }

    #[test]
    fn test_detect_format_common_log_generic() {
        let lines: Vec<&[u8]> = vec![
            b"2024-07-24T10:00:00Z INFO request processed",
            b"2024-07-24T10:00:01Z ERROR database error",
        ];
        let parser = detect_format(&lines).unwrap();
        // Should be common-log (not journalctl, since "INFO" fails hostname check)
        assert_eq!(parser.name(), "common-log");
    }

    #[test]
    fn test_detect_format_logback() {
        let lines: Vec<&[u8]> = vec![
            b"2024-07-24 10:00:00.123 [main] INFO  com.example.App - Application started",
            b"2024-07-24 10:00:01.456 [main] WARN  com.example.App - Config missing",
        ];
        let parser = detect_format(&lines).unwrap();
        assert_eq!(parser.name(), "common-log");
    }

    #[test]
    fn test_detect_format_spring_boot() {
        let lines: Vec<&[u8]> = vec![
            b"2024-07-24 10:00:00.123  INFO 12345 --- [           main] c.e.MyApp : Started",
            b"2024-07-24 10:00:01.456  WARN 12345 --- [           main] c.e.MyApp : Warning",
        ];
        let parser = detect_format(&lines).unwrap();
        assert_eq!(parser.name(), "common-log");
    }

    #[test]
    fn test_detect_format_gelf_short_message() {
        let lines: Vec<&[u8]> = vec![
            br#"{"version":"1.1","host":"example.org","short_message":"A short message","level":1}"#,
            br#"{"version":"1.1","host":"example.org","short_message":"Another msg","level":6}"#,
        ];
        let parser = detect_format(&lines).unwrap();
        assert_eq!(parser.name(), "json");
        // Verify short_message is classified as message
        let fields = parse_json_line(lines[0]).unwrap();
        let parts = classify_json_fields_all(&fields);
        assert_eq!(parts.message, Some("A short message"));
    }

    #[test]
    fn test_detect_format_otlp_json() {
        let lines: Vec<&[u8]> = vec![
            br#"{"timeUnixNano":"1700000000000000000","severityNumber":9,"severityText":"INFO","body":{"stringValue":"request received"},"attributes":[{"key":"service.name","value":{"stringValue":"my-service"}}]}"#,
            br#"{"timeUnixNano":"1700000001000000000","severityNumber":13,"severityText":"WARN","body":{"stringValue":"slow response"},"attributes":[]}"#,
        ];
        let parser = detect_format(&lines).unwrap();
        assert_eq!(parser.name(), "otlp");
    }

    #[test]
    fn test_detect_format_otel_sdk_json() {
        let lines: Vec<&[u8]> = vec![
            br#"{"timestamp":"2024-01-01T00:00:00.000000Z","severity_text":"INFO","severity_number":9,"body":"request received","attributes":{"service.name":"my-service"}}"#,
            br#"{"timestamp":"2024-01-01T00:00:01.000000Z","severity_text":"WARN","severity_number":13,"body":"slow response","attributes":{}}"#,
        ];
        let parser = detect_format(&lines).unwrap();
        assert_eq!(parser.name(), "otlp");
    }

    #[test]
    fn test_otlp_beats_json() {
        // OTLP lines are valid JSON, but OtlpParser should win
        let lines: Vec<&[u8]> = vec![
            br#"{"timeUnixNano":"1700000000000000000","severityNumber":9,"body":{"stringValue":"msg"},"attributes":[]}"#,
            br#"{"timeUnixNano":"1700000001000000000","severityNumber":13,"body":{"stringValue":"warn"},"attributes":[]}"#,
        ];
        let parser = detect_format(&lines).unwrap();
        assert_eq!(parser.name(), "otlp");
    }

    // ── Priority: specific parsers beat common-log ────────────────────

    #[test]
    fn test_journalctl_beats_common_log() {
        // Lines that journalctl parser can handle (valid hostname)
        let lines: Vec<&[u8]> = vec![
            b"2024-02-22T10:15:30+0000 myhost sshd[1234]: msg1",
            b"2024-02-22T10:15:31+0000 myhost sshd[1234]: msg2",
        ];
        let parser = detect_format(&lines).unwrap();
        assert_eq!(parser.name(), "journalctl");
    }
}
