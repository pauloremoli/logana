//! Log format detection and parser registry.

pub mod clf;
pub mod common_log;
pub mod dlt;
pub mod dlt_binary;
pub mod journalctl;
pub mod json;
pub mod logfmt;
pub mod otlp;
pub mod schema;
pub mod syslog;
pub mod timestamp;
pub mod types;

pub use clf::ClfParser;
pub use common_log::CommonLogParser;
pub use dlt::DltParser;
pub use journalctl::JournalctlParser;
pub use json::{
    JsonField, JsonParser, LogLine, build_display_json, parse_json_line, strip_json_prefixes,
};
pub use logfmt::{LogfmtParser, SCHEMA_LOGFMT};
pub use otlp::OtlpParser;
pub use schema::LogSchema;
pub use syslog::SyslogParser;
pub use types::LogLevel;
pub use types::{DisplayParts, FieldSemantic, LogFormatParser, SpanInfo, format_span_col};
pub use types::{push_extra_field, push_field_as};

pub fn detect_format(sample: &[&[u8]]) -> Option<Box<dyn LogFormatParser>> {
    if sample.is_empty() {
        return None;
    }

    let mut parsers: Vec<Box<dyn LogFormatParser>> = vec![
        // OtlpParser scores up to 1.5 to beat JsonParser on OTLP files
        Box::new(OtlpParser),
        // DltParser scores up to 1.2 — DLT text is highly distinctive
        Box::new(DltParser),
    ];
    parsers.extend(JsonParser::all_variants());
    parsers.extend([
        Box::new(SyslogParser::default()) as Box<dyn LogFormatParser>,
        Box::new(JournalctlParser::default()),
        Box::new(ClfParser),
        Box::new(LogfmtParser::default()),
        // CommonLogParser last — broadest catch-all with 0.95× score penalty
        Box::new(CommonLogParser::default()),
    ]);

    let non_empty: Vec<&[u8]> = sample.iter().copied().filter(|l| !l.is_empty()).collect();
    if non_empty.is_empty() {
        return None;
    }
    let n = non_empty.len();
    let p = parsers.len();

    // matches[pi][li]: line li is a detection match for parser pi.
    // Uses matches_for_detection, which may be stricter than parse_line — e.g.
    // JSON schema parsers only count a line when it contains their required keys.
    let matches: Vec<Vec<bool>> = parsers
        .iter()
        .map(|parser| {
            non_empty
                .iter()
                .map(|l| parser.matches_for_detection(l))
                .collect()
        })
        .collect();

    // line_weight[li]: contribution of line li to any matching parser.
    //   1.0 → exclusively matched by one parser  (decisive signal)
    //   1/N → matched by N parsers               (ambiguous, lower weight)
    let line_weight: Vec<f64> = (0..n)
        .map(|li| {
            let hits = (0..p).filter(|&pi| matches[pi][li]).count();
            if hits == 0 { 0.0 } else { 1.0 / hits as f64 }
        })
        .collect();

    // Score each parser: exclusivity-weighted match ratio × format-specific multiplier.
    // Use >= so that, on a tie, the last parser in the list wins — same behaviour as
    // the previous max_by, preserving existing tests.
    let mut best_pi: Option<usize> = None;
    let mut best_score = 0.0f64;
    for pi in 0..p {
        let score: f64 = (0..n)
            .filter(|&li| matches[pi][li])
            .map(|li| line_weight[li])
            .sum::<f64>()
            / n as f64
            * parsers[pi].detection_weight();
        if score >= best_score && score > 0.0 {
            best_score = score;
            best_pi = Some(pi);
        }
    }

    best_pi.map(|i| {
        let mut parsers = parsers;
        parsers.remove(i)
    })
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
        // ISO-timestamp hostname tag: message is also rsyslog format → syslog wins
        assert_eq!(parser.name(), "syslog");
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
        // rsyslog ISO format — syslog wins over journalctl
        assert_eq!(parser.name(), "syslog");
    }

    #[test]
    fn test_detect_format_nano_timestamp_common_log() {
        let lines: Vec<&[u8]> = vec![
            b"1700046000000000000 INFO  api-gateway host.name=prod-host-01 server started on 0.0.0.0:8080",
            b"1700046001123000000 INFO  api-gateway http.method=GET http.route=/api/users spanId=00f067aa0ba902b7 request received",
        ];
        let parser = detect_format(&lines).unwrap();
        assert_eq!(parser.name(), "common-log");
        let parts = parser.parse_line(lines[0]).unwrap();
        assert_eq!(parts.timestamp, Some("1700046000000000000"));
        assert_eq!(parts.level, Some("INFO"));
    }

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
        assert_eq!(parser.name(), "gelf");
        let parts = parser.parse_line(lines[0]).unwrap();
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
        let lines: Vec<&[u8]> = vec![
            br#"{"timeUnixNano":"1700000000000000000","severityNumber":9,"body":{"stringValue":"msg"},"attributes":[]}"#,
            br#"{"timeUnixNano":"1700000001000000000","severityNumber":13,"body":{"stringValue":"warn"},"attributes":[]}"#,
        ];
        let parser = detect_format(&lines).unwrap();
        assert_eq!(parser.name(), "otlp");
    }

    #[test]
    fn test_detect_format_journalctl_json() {
        let lines: Vec<&[u8]> = vec![
            br#"{"MESSAGE":"Accepted password","PRIORITY":"6","__REALTIME_TIMESTAMP":"1699","_HOSTNAME":"myhost","SYSLOG_IDENTIFIER":"sshd"}"#,
            br#"{"MESSAGE":"Session opened","PRIORITY":"6","__REALTIME_TIMESTAMP":"1700","_HOSTNAME":"myhost","SYSLOG_IDENTIFIER":"sshd"}"#,
        ];
        let parser = detect_format(&lines).unwrap();
        assert_eq!(parser.name(), "journalctl-json");
        let parts = parser.parse_line(lines[0]).unwrap();
        assert_eq!(parts.message, Some("Accepted password"));
        assert_eq!(parts.level, Some("INFO"));
    }

    #[test]
    fn test_detect_format_tracing_json() {
        let lines: Vec<&[u8]> = vec![
            br#"{"timestamp":"2024-01-01T00:00:00Z","level":"INFO","target":"myapp","fields":{"message":"server started"}}"#,
            br#"{"timestamp":"2024-01-01T00:00:01Z","level":"WARN","target":"myapp","fields":{"message":"slow query"}}"#,
        ];
        let parser = detect_format(&lines).unwrap();
        assert_eq!(parser.name(), "tracing-json");
        let parts = parser.parse_line(lines[0]).unwrap();
        assert_eq!(parts.message, Some("server started"));
    }

    // ── New journalctl format detection tests ─────────────────────────

    #[test]
    fn test_detect_format_journalctl_short() {
        // Plain BSD lines without priority — syslog does not claim these for
        // detection (shared with journalctl short format), so journalctl wins.
        let lines: Vec<&[u8]> = vec![
            b"Jul 12 22:23:01 myhost sshd[1234]: Accepted password",
            b"Jul 12 22:23:02 myhost sshd[1234]: Session opened",
        ];
        let parser = detect_format(&lines).unwrap();
        assert_eq!(parser.name(), "journalctl");
    }

    #[test]
    fn test_detect_format_journalctl_short_monotonic() {
        let lines: Vec<&[u8]> = vec![
            b"[     0.000000] myhost sshd[1]: msg1",
            b"[12345.678901] myhost kernel: msg2",
        ];
        let parser = detect_format(&lines).unwrap();
        assert_eq!(parser.name(), "journalctl");
    }

    #[test]
    fn test_detect_format_journalctl_short_unix() {
        let lines: Vec<&[u8]> = vec![
            b"1436735381.000000 myhost sshd[1234]: msg1",
            b"1436735382.000001 myhost sshd[1234]: msg2",
        ];
        let parser = detect_format(&lines).unwrap();
        assert_eq!(parser.name(), "journalctl");
    }

    #[test]
    fn test_detect_format_json_sse() {
        let lines: Vec<&[u8]> = vec![
            br#"data: {"level":"INFO","msg":"hello"}"#,
            br#"data: {"level":"WARN","msg":"world"}"#,
        ];
        let parser = detect_format(&lines).unwrap();
        assert_eq!(parser.name(), "json");
    }

    #[test]
    fn test_detect_format_json_seq() {
        let mut line1 = vec![0x1eu8];
        line1.extend_from_slice(br#"{"level":"INFO","msg":"hello"}"#);
        let mut line2 = vec![0x1eu8];
        line2.extend_from_slice(br#"{"level":"WARN","msg":"world"}"#);
        let lines: Vec<&[u8]> = vec![&line1, &line2];
        let parser = detect_format(&lines).unwrap();
        assert_eq!(parser.name(), "json");
    }

    #[test]
    fn test_detect_format_syslog_wins_when_priority_lines_present() {
        // Priority-prefixed lines are exclusively syslog; plain BSD lines are
        // ambiguous (both syslog and journalctl parse them).  Exclusivity
        // weighting should tip the balance toward syslog.
        let lines: Vec<&[u8]> = vec![
            b"<134>Oct 11 22:14:15 myhost sshd[1234]: Accepted password",
            b"Oct 11 22:14:16 myhost sshd[1234]: Session opened", // ambiguous
            b"<30>Oct 11 22:14:17 myhost systemd[1]: Started cron",
        ];
        let parser = detect_format(&lines).unwrap();
        assert_eq!(parser.name(), "syslog");
    }

    // ── Priority: specific parsers beat common-log ────────────────────

    #[test]
    fn test_detect_format_journalctl_wins_plain_bsd_without_priority() {
        // Plain BSD lines without a priority prefix are shared with journalctl
        // short format; syslog deliberately does not claim them for detection,
        // so journalctl wins (e.g. `journalctl --output short | logana`).
        let lines: Vec<&[u8]> = vec![
            b"Mar 15 10:00:00 myhost sshd[1234]: Accepted password",
            b"Mar 15 10:00:01 myhost sshd[1234]: Session opened",
        ];
        let parser = detect_format(&lines).unwrap();
        assert_eq!(parser.name(), "journalctl");
    }

    #[test]
    fn test_syslog_beats_common_log_on_iso_format() {
        // rsyslog ISO format — syslog wins over both journalctl and common-log
        let lines: Vec<&[u8]> = vec![
            b"2024-02-22T10:15:30+0000 myhost sshd[1234]: msg1",
            b"2024-02-22T10:15:31+0000 myhost sshd[1234]: msg2",
        ];
        let parser = detect_format(&lines).unwrap();
        assert_eq!(parser.name(), "syslog");
    }
}
