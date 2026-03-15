//! Log format abstraction: trait, shared types, and span utilities.

use std::collections::HashSet;

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
        // Empty lines are structural delimiters (e.g. SSE separators), not
        // log records. Exclude them from both numerator and denominator so they
        // don't dilute the score.
        let non_empty: Vec<&[u8]> = sample.iter().copied().filter(|l| !l.is_empty()).collect();
        if non_empty.is_empty() {
            return 0.0;
        }
        let parsed = non_empty
            .iter()
            .filter(|l| self.parse_line(l).is_some())
            .count();
        if parsed == 0 {
            return 0.0;
        }
        parsed as f64 / non_empty.len() as f64
    }

    fn name(&self) -> &str;

    /// Returns field names that should be hidden by default when this format is
    /// first detected. Parsers override this to suppress noisy internal fields
    /// (e.g. journalctl JSON exports dozens of systemd-internal fields that are
    /// not visible in the default `short` output mode).
    ///
    /// Called once at tab creation with the format-detection sample. Returns an
    /// empty set by default (show all fields).
    fn default_hidden_fields(&self, _sample: &[&[u8]]) -> HashSet<String> {
        HashSet::new()
    }
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
