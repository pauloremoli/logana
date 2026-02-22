// ---------------------------------------------------------------------------
// Log format abstraction: trait, types, detection
// ---------------------------------------------------------------------------

/// Span context extracted from a structured log line (e.g. tracing JSON).
#[derive(Debug)]
pub struct SpanInfo<'a> {
    /// Value of the `name` key inside the span object.
    pub name: &'a str,
    /// All other span fields in document order: `(key, value)`.
    pub fields: Vec<(&'a str, &'a str)>,
}

/// Format-agnostic structured representation of a parsed log line, ready for
/// display. All string slices borrow from the original line bytes — no heap
/// allocation during parsing.
#[derive(Debug, Default)]
pub struct DisplayParts<'a> {
    pub timestamp: Option<&'a str>,
    pub level: Option<&'a str>,
    pub target: Option<&'a str>,
    /// Current span context (from a `span` nested object), if present.
    pub span: Option<SpanInfo<'a>>,
    /// Unknown fields in original document order: `(key, value)`.
    pub extra_fields: Vec<(&'a str, &'a str)>,
    pub message: Option<&'a str>,
}

/// A log format parser that can parse lines, detect format, and discover field
/// names. Implementations must be object-safe (no generic methods).
pub trait LogFormatParser: Send + Sync + std::fmt::Debug {
    /// Zero-copy parse: returns borrowed slices into `line`. None = not this format.
    fn parse_line<'a>(&self, line: &'a [u8]) -> Option<DisplayParts<'a>>;

    /// Discover field names by sampling lines.
    /// Returns canonical names first, then extras sorted alphabetically.
    fn collect_field_names(&self, lines: &[&[u8]]) -> Vec<String>;

    /// Confidence score for a sample of lines (0.0 = not this format, 1.0 = certain).
    fn detect_score(&self, sample: &[&[u8]]) -> f64;

    /// Human-readable format name (e.g. "json", "syslog").
    fn name(&self) -> &str;
}

/// Sample lines, try all registered parsers, return best match.
pub fn detect_format(sample: &[&[u8]]) -> Option<Box<dyn LogFormatParser>> {
    use crate::log_line::JsonParser;
    use crate::syslog::SyslogParser;

    if sample.is_empty() {
        return None;
    }

    let parsers: Vec<Box<dyn LogFormatParser>> = vec![Box::new(JsonParser), Box::new(SyslogParser)];

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

/// Format a `SpanInfo` as a display string: `name: k=v, k=v` or just `name`.
pub fn format_span_col(s: &SpanInfo<'_>) -> String {
    if s.fields.is_empty() {
        return s.name.to_string();
    }
    let kv = s
        .fields
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{}: {}", s.name, kv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_parts_new_all_none() {
        let p = DisplayParts::default();
        assert!(p.timestamp.is_none());
        assert!(p.level.is_none());
        assert!(p.target.is_none());
        assert!(p.span.is_none());
        assert!(p.extra_fields.is_empty());
        assert!(p.message.is_none());
    }

    #[test]
    fn test_format_span_col_name_only() {
        let span = SpanInfo {
            name: "request",
            fields: vec![],
        };
        assert_eq!(format_span_col(&span), "request");
    }

    #[test]
    fn test_format_span_col_with_fields() {
        let span = SpanInfo {
            name: "request",
            fields: vec![("method", "GET"), ("uri", "/health")],
        };
        assert_eq!(format_span_col(&span), "request: method=GET, uri=/health");
    }

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
}
