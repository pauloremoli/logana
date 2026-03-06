//! Log format abstraction: trait, shared types, and span utilities.
//!
//! [`LogFormatParser`] is the core trait implemented by every format parser.
//! [`DisplayParts`] is the zero-copy, format-agnostic representation of a
//! parsed log line. All field slices borrow from the original line bytes.

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

/// Format a `SpanInfo` as a display string.
///
/// - `show_keys = false` → `name: v1, v2` (values only, current default)
/// - `show_keys = true`  → `name: k1=v1 k2=v2` (key=value pairs)
pub fn format_span_col(s: &SpanInfo<'_>, show_keys: bool) -> String {
    if s.fields.is_empty() {
        return s.name.to_string();
    }
    let body: String = if show_keys {
        s.fields
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        s.fields
            .iter()
            .map(|(_, v)| v.to_string())
            .collect::<Vec<_>>()
            .join(" ")
    };
    format!("{}: {}", s.name, body)
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
        assert_eq!(format_span_col(&span, false), "request");
        assert_eq!(format_span_col(&span, true), "request");
    }

    #[test]
    fn test_format_span_col_values_only() {
        let span = SpanInfo {
            name: "request",
            fields: vec![("method", "GET"), ("uri", "/health")],
        };
        assert_eq!(format_span_col(&span, false), "request: GET /health");
    }

    #[test]
    fn test_format_span_col_with_keys() {
        let span = SpanInfo {
            name: "request",
            fields: vec![("method", "GET"), ("uri", "/health")],
        };
        assert_eq!(
            format_span_col(&span, true),
            "request: method=GET uri=/health"
        );
    }
}
