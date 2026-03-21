//! Log format abstraction: trait, shared types, and span utilities.

use std::collections::HashSet;

/// Semantic meaning of a log field key, shared across all parsers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldSemantic {
    // Canonical display slots
    Timestamp,
    Level,
    Target,
    Span,
    Message,
    // Host / process metadata
    Hostname,
    Pid,
    Thread,
    // Syslog
    Facility,
    MsgId,
    // Distributed tracing
    TraceId,
    SpanId,
    // HTTP / access log (CLF)
    HttpStatus,
    HttpBytes,
    HttpReferer,
    HttpUserAgent,
    HttpIdent,
    HttpAuthUser,
    // Anything not recognised
    Extra,
}

impl FieldSemantic {
    /// Canonical field name for this semantic slot.
    /// Returns `""` for `Extra` and `Span` (no fixed name).
    pub fn canonical_name(self) -> &'static str {
        match self {
            FieldSemantic::Timestamp => "timestamp",
            FieldSemantic::Level => "level",
            FieldSemantic::Target => "target",
            FieldSemantic::Message => "message",
            FieldSemantic::Hostname => "hostname",
            FieldSemantic::Pid => "pid",
            FieldSemantic::Thread => "thread",
            FieldSemantic::Facility => "facility",
            FieldSemantic::MsgId => "msgid",
            FieldSemantic::TraceId => "traceId",
            FieldSemantic::SpanId => "spanId",
            FieldSemantic::HttpStatus => "status",
            FieldSemantic::HttpBytes => "bytes",
            FieldSemantic::HttpReferer => "referer",
            FieldSemantic::HttpUserAgent => "user_agent",
            FieldSemantic::HttpIdent => "ident",
            FieldSemantic::HttpAuthUser => "authuser",
            FieldSemantic::Span | FieldSemantic::Extra => "",
        }
    }
}

impl std::fmt::Display for FieldSemantic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.canonical_name())
    }
}

/// Push a known-semantic field into `extra_fields`.
/// The canonical name for the semantic is used as the key.
/// Use [`push_extra_field`] for unrecognised fields.
pub fn push_field_as<'a>(
    fields: &mut Vec<(FieldSemantic, &'a str, &'a str)>,
    semantic: FieldSemantic,
    val: &'a str,
) {
    fields.push((semantic, semantic.canonical_name(), val));
}

/// Push an unrecognised (`Extra`) field into `extra_fields`, preserving the raw key.
pub fn push_extra_field<'a>(
    fields: &mut Vec<(FieldSemantic, &'a str, &'a str)>,
    key: &'a str,
    val: &'a str,
) {
    fields.push((FieldSemantic::Extra, key, val));
}

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
    pub extra_fields: Vec<(FieldSemantic, &'a str, &'a str)>,
    pub message: Option<&'a str>,
}

pub trait LogFormatParser: Send + Sync + std::fmt::Debug {
    fn parse_line<'a>(&self, line: &'a [u8]) -> Option<DisplayParts<'a>>;

    fn parse_timestamp<'a>(&self, line: &'a [u8]) -> Option<&'a str> {
        self.parse_line(line).and_then(|p| p.timestamp)
    }

    fn collect_field_names(&self, lines: &[&[u8]]) -> Vec<String>;

    /// Returns whether this parser considers `line` a match for format-detection
    /// purposes.  This may be stricter than `parse_line` — for example, JSON
    /// schema parsers only return `true` when the line contains their required
    /// schema keys, even though `parse_line` can render any valid JSON.
    ///
    /// Used by `detect_format` to build the cross-parser exclusivity matrix.
    /// Defaults to `parse_line(line).is_some()`.
    fn matches_for_detection(&self, line: &[u8]) -> bool {
        self.parse_line(line).is_some()
    }

    /// Format-specific multiplier applied to the exclusivity-weighted score in
    /// `detect_format`.  Parsers that need to beat equally-matched alternatives
    /// (e.g. OTLP vs generic JSON) return a value > 1.0; overly-broad parsers
    /// return < 1.0.  Defaults to 1.0.
    fn detection_weight(&self) -> f64 {
        1.0
    }

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

    #[test]
    fn test_field_semantic_canonical_names() {
        assert_eq!(FieldSemantic::Timestamp.canonical_name(), "timestamp");
        assert_eq!(FieldSemantic::Level.canonical_name(), "level");
        assert_eq!(FieldSemantic::Target.canonical_name(), "target");
        assert_eq!(FieldSemantic::Message.canonical_name(), "message");
        assert_eq!(FieldSemantic::Hostname.canonical_name(), "hostname");
        assert_eq!(FieldSemantic::Pid.canonical_name(), "pid");
        assert_eq!(FieldSemantic::Thread.canonical_name(), "thread");
        assert_eq!(FieldSemantic::TraceId.canonical_name(), "traceId");
        assert_eq!(FieldSemantic::SpanId.canonical_name(), "spanId");
        assert_eq!(FieldSemantic::Extra.canonical_name(), "");
        assert_eq!(FieldSemantic::Span.canonical_name(), "");
        // Display delegates to canonical_name
        assert_eq!(FieldSemantic::Pid.to_string(), "pid");
        assert_eq!(FieldSemantic::Extra.to_string(), "");
    }

    #[test]
    fn test_push_field_as_uses_canonical_key() {
        let mut fields: Vec<(FieldSemantic, &str, &str)> = Vec::new();
        push_field_as(&mut fields, FieldSemantic::Pid, "1234");
        push_field_as(&mut fields, FieldSemantic::Hostname, "myhost");
        push_extra_field(&mut fields, "request_id", "abc");
        assert_eq!(fields[0], (FieldSemantic::Pid, "pid", "1234"));
        assert_eq!(fields[1], (FieldSemantic::Hostname, "hostname", "myhost"));
        assert_eq!(fields[2], (FieldSemantic::Extra, "request_id", "abc"));
    }
}
