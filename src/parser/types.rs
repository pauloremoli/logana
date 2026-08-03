//! Log format abstraction: trait, shared types, and span utilities.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Notice,
    Warning,
    Error,
    Fatal,
    #[default]
    Unknown,
}

impl LogLevel {
    pub fn parse_level(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "trace" | "trc" => LogLevel::Trace,
            "debug" | "dbg" => LogLevel::Debug,
            "info" | "inf" => LogLevel::Info,
            "notice" => LogLevel::Notice,
            "warn" | "warning" | "wrn" => LogLevel::Warning,
            "error" | "err" => LogLevel::Error,
            "fatal" | "ftl" | "critical" | "crit" | "emerg" | "alert" => LogLevel::Fatal,
            _ => LogLevel::Unknown,
        }
    }

    pub fn detect_from_bytes(line: &[u8]) -> Self {
        let mut i = 0;
        while i + 4 <= line.len() {
            let w4 = [
                line[i].to_ascii_uppercase(),
                line[i + 1].to_ascii_uppercase(),
                line[i + 2].to_ascii_uppercase(),
                line[i + 3].to_ascii_uppercase(),
            ];
            if w4 == *b"FATA" && i + 5 <= line.len() && line[i + 4].eq_ignore_ascii_case(&b'L') {
                return LogLevel::Fatal;
            }
            if w4 == *b"CRIT" {
                return LogLevel::Fatal;
            }
            if w4 == *b"EMER" && i + 5 <= line.len() && line[i + 4].eq_ignore_ascii_case(&b'G') {
                return LogLevel::Fatal;
            }
            if w4 == *b"ALER" && i + 5 <= line.len() && line[i + 4].eq_ignore_ascii_case(&b'T') {
                return LogLevel::Fatal;
            }
            if w4 == *b"ERRO" && i + 5 <= line.len() && line[i + 4].eq_ignore_ascii_case(&b'R') {
                return LogLevel::Error;
            }
            if w4 == *b"WARN" {
                return LogLevel::Warning;
            }
            if w4 == *b"NOTI"
                && i + 6 <= line.len()
                && line[i + 4].eq_ignore_ascii_case(&b'C')
                && line[i + 5].eq_ignore_ascii_case(&b'E')
            {
                return LogLevel::Notice;
            }
            if w4 == *b"INFO" {
                return LogLevel::Info;
            }
            if w4 == *b"DEBU" && i + 5 <= line.len() && line[i + 4].eq_ignore_ascii_case(&b'G') {
                return LogLevel::Debug;
            }
            if w4 == *b"TRAC" && i + 5 <= line.len() && line[i + 4].eq_ignore_ascii_case(&b'E') {
                return LogLevel::Trace;
            }
            i += 1;
        }
        LogLevel::Unknown
    }
}

/// Semantic meaning of a log field key, shared across all parsers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldSemantic {
    Timestamp,
    Level,
    Target,
    Span,
    Message,
    Hostname,
    Pid,
    Thread,
    Facility,
    MsgId,
    TraceId,
    SpanId,
    HttpStatus,
    HttpBytes,
    HttpReferer,
    HttpUserAgent,
    HttpIdent,
    HttpAuthUser,
    Component,
    Feature,
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
            FieldSemantic::Component => "component",
            FieldSemantic::Feature => "feature",
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

/// One piece of a compiled custom-schema `template`: literal text to
/// reproduce verbatim, or a placeholder for a field's value, keyed by the
/// canonical name `resolve_field` understands (not the raw template
/// placeholder name — e.g. a `{service}` placeholder mapped to the `target`
/// role is stored as `Field { canonical_name: "target", .. }`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateSegment {
    Literal(String),
    Field {
        canonical_name: String,
        ignored: bool,
    },
}

pub trait LogFormatParser: Send + Sync + std::fmt::Debug {
    fn parse_line<'a>(&self, line: &'a [u8]) -> Option<DisplayParts<'a>>;

    fn parse_timestamp<'a>(&self, line: &'a [u8]) -> Option<&'a str> {
        self.parse_line(line).and_then(|p| p.timestamp)
    }

    /// Classifies a raw captured `level` value into a [`LogLevel`], for level
    /// coloring and the `e`/`w` error/warning navigation keys. Parsers whose
    /// schema declares its own error/warning values (see `CustomParser`)
    /// override this to consult that mapping before falling back to the
    /// built-in keyword matching in `LogLevel::parse_level`.
    fn classify_level(&self, raw: &str) -> LogLevel {
        LogLevel::parse_level(raw)
    }

    /// Returns `true` when this schema's continuation lines (lines the
    /// generic continuation grouping attaches to the previous matched
    /// record — see `build_continuation_map`) should be folded into that
    /// record's `message` field, so field filters and the structured fields
    /// panel can see their content. Defaults to `false`; only a `CustomParser`
    /// built from a schema with `"multiline": true` returns `true`.
    fn merges_continuation_into_message(&self) -> bool {
        false
    }

    /// Returns this parser's compiled template segments (literal text +
    /// canonical field placeholders, in the template's own order), for
    /// parsers built from a custom schema `template`. `None` for every other
    /// parser, including a `pattern`-based custom schema (a raw regex has no
    /// literal skeleton to reconstruct a line from).
    fn template_segments(&self) -> Option<&[TemplateSegment]> {
        None
    }

    /// Returns `false` when this format's timestamps do not include a year
    /// (e.g. BSD syslog `"Feb 22 10:15:30"`), meaning a `YearMap` must be
    /// applied to recover the correct year.  Defaults to `true`.
    fn timestamp_has_year(&self) -> bool {
        true
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

    /// Returns `true` when the parsed `level` field is derived from data that
    /// does not appear verbatim in the raw line bytes (e.g. a numeric syslog
    /// priority code).  When `true`, the filter pipeline also runs text filters
    /// against the normalized level string so that a filter like "INFO" matches
    /// syslog lines whose raw bytes only contain `<134>`.
    fn has_synthetic_level(&self) -> bool {
        false
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
        assert_eq!(FieldSemantic::Component.canonical_name(), "component");
        assert_eq!(FieldSemantic::Feature.canonical_name(), "feature");
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

    #[test]
    fn test_log_level_from_str() {
        assert_eq!(LogLevel::parse_level("trace"), LogLevel::Trace);
        assert_eq!(LogLevel::parse_level("TRC"), LogLevel::Trace);
        assert_eq!(LogLevel::parse_level("debug"), LogLevel::Debug);
        assert_eq!(LogLevel::parse_level("DBG"), LogLevel::Debug);
        assert_eq!(LogLevel::parse_level("info"), LogLevel::Info);
        assert_eq!(LogLevel::parse_level("INFO"), LogLevel::Info);
        assert_eq!(LogLevel::parse_level("INF"), LogLevel::Info);
        assert_eq!(LogLevel::parse_level("notice"), LogLevel::Notice);
        assert_eq!(LogLevel::parse_level("warn"), LogLevel::Warning);
        assert_eq!(LogLevel::parse_level("WARNING"), LogLevel::Warning);
        assert_eq!(LogLevel::parse_level("WRN"), LogLevel::Warning);
        assert_eq!(LogLevel::parse_level("error"), LogLevel::Error);
        assert_eq!(LogLevel::parse_level("ERR"), LogLevel::Error);
        assert_eq!(LogLevel::parse_level("fatal"), LogLevel::Fatal);
        assert_eq!(LogLevel::parse_level("FTL"), LogLevel::Fatal);
        assert_eq!(LogLevel::parse_level("critical"), LogLevel::Fatal);
        assert_eq!(LogLevel::parse_level("CRIT"), LogLevel::Fatal);
        assert_eq!(LogLevel::parse_level("emerg"), LogLevel::Fatal);
        assert_eq!(LogLevel::parse_level("alert"), LogLevel::Fatal);
        assert_eq!(LogLevel::parse_level("unknown"), LogLevel::Unknown);
    }

    #[test]
    fn test_log_level_detect_from_bytes() {
        assert_eq!(
            LogLevel::detect_from_bytes(b"some INFO message"),
            LogLevel::Info
        );
        assert_eq!(
            LogLevel::detect_from_bytes(b"WARN: disk full"),
            LogLevel::Warning
        );
        assert_eq!(
            LogLevel::detect_from_bytes(b"ERROR: connection lost"),
            LogLevel::Error
        );
        assert_eq!(
            LogLevel::detect_from_bytes(b"DEBUG: value=5"),
            LogLevel::Debug
        );
        assert_eq!(
            LogLevel::detect_from_bytes(b"plain log line"),
            LogLevel::Unknown
        );
        assert_eq!(
            LogLevel::detect_from_bytes(b"error happened"),
            LogLevel::Error
        );
        assert_eq!(
            LogLevel::detect_from_bytes(b"warn about something"),
            LogLevel::Warning
        );
        assert_eq!(
            LogLevel::detect_from_bytes(b"TRACE entering function"),
            LogLevel::Trace
        );
        assert_eq!(
            LogLevel::detect_from_bytes(b"NOTICE system event"),
            LogLevel::Notice
        );
        assert_eq!(
            LogLevel::detect_from_bytes(b"FATAL system crash"),
            LogLevel::Fatal
        );
        assert_eq!(
            LogLevel::detect_from_bytes(b"CRITICAL out of memory"),
            LogLevel::Fatal
        );
        assert_eq!(
            LogLevel::detect_from_bytes(b"EMERG kernel panic"),
            LogLevel::Fatal
        );
        assert_eq!(
            LogLevel::detect_from_bytes(b"ALERT security breach"),
            LogLevel::Fatal
        );
    }
}
