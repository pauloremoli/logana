//! Log format abstraction: trait, shared types, and span utilities.

#[derive(Debug)]
pub struct SpanInfo<'a> {
    pub name: &'a str,
    pub fields: Vec<(&'a str, &'a str)>,
}

/// Zero-copy representation of a parsed log line. All slices borrow from the
/// original line bytes.
#[derive(Debug, Default)]
pub struct DisplayParts<'a> {
    pub timestamp: Option<&'a str>,
    pub level: Option<&'a str>,
    pub target: Option<&'a str>,
    pub span: Option<SpanInfo<'a>>,
    pub extra_fields: Vec<(&'a str, &'a str)>,
    pub message: Option<&'a str>,
}

pub trait LogFormatParser: Send + Sync + std::fmt::Debug {
    fn parse_line<'a>(&self, line: &'a [u8]) -> Option<DisplayParts<'a>>;

    fn collect_field_names(&self, lines: &[&[u8]]) -> Vec<String>;

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
        parsed as f64 / sample.len() as f64
    }

    fn name(&self) -> &str;
}

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
