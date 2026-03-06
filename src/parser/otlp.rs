//! OpenTelemetry (OTLP) log format parser.
//!
//! Handles two common serialization variants, both detected automatically:
//!
//! - **OTLP/JSON (protobuf-JSON encoding)**
//!   Fields: `timeUnixNano`, `severityNumber`/`severityText`,
//!   `body` as `{"stringValue":"…"}`, `attributes` as
//!   `[{"key":"k","value":{"stringValue":"v"}}]`.
//!
//! - **OTel SDK JSON** (Python, Node, Java, etc.)
//!   Fields: `timestamp` (ISO), `severity_text`/`severity_number`,
//!   `body` as a direct string, `attributes` as a flat `{"key":"value"}` object.
//!
//! Detection scores above 1.0 (up to 1.5) so the parser beats the generic
//! JSON parser when OTLP-specific fields are present.

use std::collections::HashSet;

use super::json::parse_json_line;
use super::types::{DisplayParts, LogFormatParser};

/// OTLP log format parser.
#[derive(Debug)]
pub struct OtlpParser;

// ---------------------------------------------------------------------------
// Detection helpers
// ---------------------------------------------------------------------------

/// Returns `true` when the field list looks like an OTel log record.
fn is_otlp_record(fields: &[super::json::JsonField<'_>]) -> bool {
    let has_time_unix_nano = fields
        .iter()
        .any(|f| matches!(f.key, "timeUnixNano" | "observedTimeUnixNano"));
    let has_severity = fields.iter().any(|f| {
        matches!(
            f.key,
            "severityNumber"
                | "severityText"
                | "severity_number"
                | "severity_text"
                | "SeverityText"
                | "Severity"
        )
    });
    let has_body = fields.iter().any(|f| matches!(f.key, "body" | "Body"));

    // Nanosecond timestamp is OTLP/JSON-only; severity+body covers SDK variants.
    has_time_unix_nano || (has_severity && has_body)
}

// ---------------------------------------------------------------------------
// Value extraction helpers
// ---------------------------------------------------------------------------

/// Map an OTLP `severityNumber` (1–24) to a canonical level string.
fn severity_number_to_level(num_str: &str) -> Option<&'static str> {
    let n: u32 = num_str.trim().parse().ok()?;
    Some(match n {
        1..=4 => "TRACE",
        5..=8 => "DEBUG",
        9..=12 => "INFO",
        13..=16 => "WARN",
        17..=20 => "ERROR",
        21..=24 => "FATAL",
        _ => return None,
    })
}

/// Extract a scalar string from an OTLP `AnyValue` object such as
/// `{"stringValue":"GET"}`, `{"intValue":"42"}`, `{"boolValue":"true"}`.
fn extract_any_value_str<'a>(value: &'a str) -> Option<&'a str> {
    let fields = parse_json_line(value.as_bytes())?;
    for f in &fields {
        if matches!(
            f.key,
            "stringValue" | "intValue" | "doubleValue" | "boolValue" | "Value"
        ) {
            return Some(f.value);
        }
    }
    None
}

/// Returns `true` for attribute keys that should fill the `target` slot.
fn is_target_attr(key: &str) -> bool {
    matches!(
        key,
        "service.name" | "code.namespace" | "logger" | "component" | "module"
    )
}

// ---------------------------------------------------------------------------
// Attribute parsing
// ---------------------------------------------------------------------------

/// Parse OTLP array-style attributes:
/// `[{"key":"k","value":{"stringValue":"v"}}, …]`
///
/// Also handles the Go SDK variant:
/// `[{"Key":"k","Value":{"Type":"STRING","Value":"v"}}, …]`
fn parse_otlp_attr_array<'a>(array_str: &'a str) -> Vec<(&'a str, &'a str)> {
    let mut result = Vec::new();
    let bytes = array_str.as_bytes();
    if bytes.is_empty() || bytes[0] != b'[' {
        return result;
    }

    let mut i = 1usize; // skip '['
    while i < bytes.len() {
        // Skip whitespace and commas between elements.
        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n' | b',') {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] == b']' {
            break;
        }
        if bytes[i] != b'{' {
            break;
        }

        // Find the closing `}` for this element, respecting nesting and strings.
        let start = i;
        let mut depth = 0usize;
        let mut in_str = false;
        while i < bytes.len() {
            match bytes[i] {
                b'"' => in_str = !in_str,
                b'\\' if in_str => {
                    i += 1; // skip escaped char; outer i++ skips the backslash
                }
                b'{' if !in_str => depth += 1,
                b'}' if !in_str => {
                    depth -= 1;
                    if depth == 0 {
                        i += 1;
                        break;
                    }
                }
                _ => {}
            }
            i += 1;
        }

        let obj_bytes = &bytes[start..i];
        if let Some(obj_fields) = parse_json_line(obj_bytes) {
            let mut key_str: Option<&'a str> = None;
            let mut val_str: Option<&'a str> = None;
            for f in &obj_fields {
                match f.key {
                    "key" | "Key" if f.value_is_string => key_str = Some(f.value),
                    "value" | "Value" if !f.value_is_string => {
                        val_str = extract_any_value_str(f.value);
                    }
                    _ => {}
                }
            }
            if let (Some(k), Some(v)) = (key_str, val_str) {
                result.push((k, v));
            }
        }
    }
    result
}

/// Parse OTel SDK flat-dict attributes: `{"http.method":"GET","service.name":"svc"}`
fn parse_sdk_attr_dict<'a>(dict_str: &'a str) -> Vec<(&'a str, &'a str)> {
    parse_json_line(dict_str.as_bytes())
        .unwrap_or_default()
        .into_iter()
        .map(|f| (f.key, f.value))
        .collect()
}

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

fn classify_otlp_fields<'a>(fields: &[super::json::JsonField<'a>]) -> DisplayParts<'a> {
    let mut timestamp: Option<&'a str> = None;
    let mut level_text: Option<&'a str> = None;
    let mut level_num: Option<&'a str> = None;
    let mut message: Option<&'a str> = None;
    let mut target: Option<&'a str> = None;
    let mut extra_fields: Vec<(&'a str, &'a str)> = Vec::new();

    for f in fields {
        match f.key {
            // Timestamps — nanosecond (OTLP/JSON) or ISO (SDK).
            // Prefer timeUnixNano; observed* is fallback.
            "timeUnixNano" => {
                timestamp = Some(f.value);
            }
            "timestamp" | "Timestamp" if timestamp.is_none() => {
                timestamp = Some(f.value);
            }
            "observedTimeUnixNano" | "observed_timestamp" if timestamp.is_none() => {
                timestamp = Some(f.value);
            }

            // Level text form takes priority over numeric.
            "severityText" | "severity_text" | "SeverityText"
                if level_text.is_none() && !f.value.is_empty() =>
            {
                level_text = Some(f.value);
            }

            "severityNumber" | "severity_number" | "Severity" if level_num.is_none() => {
                level_num = Some(f.value);
            }

            // Body: direct string (SDK) or AnyValue object (OTLP/JSON).
            "body" | "Body" if message.is_none() => {
                if f.value_is_string {
                    message = Some(f.value);
                } else {
                    message = extract_any_value_str(f.value);
                }
            }

            // Attributes: array (OTLP/JSON, Go SDK) or flat dict (Python/JS SDK).
            "attributes" | "Attributes" => {
                let attrs = if f.value.trim_start().starts_with('[') {
                    parse_otlp_attr_array(f.value)
                } else {
                    parse_sdk_attr_dict(f.value)
                };
                for (k, v) in attrs {
                    if is_target_attr(k) && target.is_none() {
                        target = Some(v);
                    } else {
                        extra_fields.push((k, v));
                    }
                }
            }

            // Resource block — mine it for service.name.
            "resource" | "Resource" => {
                if let Some(res_fields) = parse_json_line(f.value.as_bytes()) {
                    for rf in &res_fields {
                        if rf.key == "attributes" {
                            let attrs = if rf.value.trim_start().starts_with('[') {
                                parse_otlp_attr_array(rf.value)
                            } else {
                                parse_sdk_attr_dict(rf.value)
                            };
                            for (k, v) in attrs {
                                if k == "service.name" && target.is_none() {
                                    target = Some(v);
                                }
                            }
                        }
                    }
                }
            }

            // Trace context — expose as extra fields.
            "traceId" | "trace_id" | "TraceID" => {
                extra_fields.push(("traceId", f.value));
            }
            "spanId" | "span_id" | "SpanID" => {
                extra_fields.push(("spanId", f.value));
            }

            // Internal / bookkeeping fields — skip.
            "flags"
            | "traceFlags"
            | "trace_flags"
            | "TraceFlags"
            | "droppedAttributesCount"
            | "InstrumentationScope"
            | "schemaUrl" => {}

            _ => {
                extra_fields.push((f.key, f.value));
            }
        }
    }

    // Derive level from numeric severity when text form is absent.
    let level: Option<&'a str> = level_text.or_else(|| {
        level_num
            .and_then(severity_number_to_level)
            .map(|s| -> &'a str { s })
    });

    DisplayParts {
        timestamp,
        level,
        target,
        span: None,
        extra_fields,
        message,
    }
}

// ---------------------------------------------------------------------------
// LogFormatParser impl
// ---------------------------------------------------------------------------

impl LogFormatParser for OtlpParser {
    fn parse_line<'a>(&self, line: &'a [u8]) -> Option<DisplayParts<'a>> {
        let fields = parse_json_line(line)?;
        if !is_otlp_record(&fields) {
            return None;
        }
        Some(classify_otlp_fields(&fields))
    }

    fn collect_field_names(&self, lines: &[&[u8]]) -> Vec<String> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut extras: Vec<String> = Vec::new();

        for &line in lines {
            if let Some(fields) = parse_json_line(line) {
                if !is_otlp_record(&fields) {
                    continue;
                }
                let parts = classify_otlp_fields(&fields);
                for (k, _) in parts.extra_fields {
                    let key = k.to_string();
                    if seen.insert(key.clone()) {
                        extras.push(key);
                    }
                }
            }
        }

        let mut result = vec![
            "timestamp".to_string(),
            "level".to_string(),
            "target".to_string(),
        ];
        extras.sort();
        extras.dedup();
        result.extend(extras);
        result.push("message".to_string());
        result
    }

    fn detect_score(&self, sample: &[&[u8]]) -> f64 {
        if sample.is_empty() {
            return 0.0;
        }
        let otlp_count = sample
            .iter()
            .filter(|&&line| {
                parse_json_line(line)
                    .map(|f| is_otlp_record(&f))
                    .unwrap_or(false)
            })
            .count();
        if otlp_count == 0 {
            return 0.0;
        }
        // Score above 1.0 to take priority over the generic JSON parser when
        // OTLP-specific fields are present.
        otlp_count as f64 / sample.len() as f64 * 1.5
    }

    fn name(&self) -> &str {
        "otlp"
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── severity_number_to_level ─────────────────────────────────────────────

    #[test]
    fn test_severity_number_trace() {
        assert_eq!(severity_number_to_level("1"), Some("TRACE"));
        assert_eq!(severity_number_to_level("4"), Some("TRACE"));
    }

    #[test]
    fn test_severity_number_debug() {
        assert_eq!(severity_number_to_level("5"), Some("DEBUG"));
        assert_eq!(severity_number_to_level("8"), Some("DEBUG"));
    }

    #[test]
    fn test_severity_number_info() {
        assert_eq!(severity_number_to_level("9"), Some("INFO"));
        assert_eq!(severity_number_to_level("12"), Some("INFO"));
    }

    #[test]
    fn test_severity_number_warn() {
        assert_eq!(severity_number_to_level("13"), Some("WARN"));
        assert_eq!(severity_number_to_level("16"), Some("WARN"));
    }

    #[test]
    fn test_severity_number_error() {
        assert_eq!(severity_number_to_level("17"), Some("ERROR"));
        assert_eq!(severity_number_to_level("20"), Some("ERROR"));
    }

    #[test]
    fn test_severity_number_fatal() {
        assert_eq!(severity_number_to_level("21"), Some("FATAL"));
        assert_eq!(severity_number_to_level("24"), Some("FATAL"));
    }

    #[test]
    fn test_severity_number_out_of_range() {
        assert_eq!(severity_number_to_level("0"), None);
        assert_eq!(severity_number_to_level("25"), None);
        assert_eq!(severity_number_to_level("abc"), None);
    }

    // ── extract_any_value_str ────────────────────────────────────────────────

    #[test]
    fn test_extract_any_value_string_value() {
        assert_eq!(
            extract_any_value_str(r#"{"stringValue":"hello world"}"#),
            Some("hello world")
        );
    }

    #[test]
    fn test_extract_any_value_int_value() {
        assert_eq!(extract_any_value_str(r#"{"intValue":"42"}"#), Some("42"));
    }

    #[test]
    fn test_extract_any_value_bool_value() {
        assert_eq!(
            extract_any_value_str(r#"{"boolValue":"true"}"#),
            Some("true")
        );
    }

    #[test]
    fn test_extract_any_value_empty_object() {
        assert_eq!(extract_any_value_str(r#"{"other":"x"}"#), None);
    }

    // ── parse_otlp_attr_array ────────────────────────────────────────────────

    #[test]
    fn test_parse_otlp_attr_array_string_values() {
        let input = r#"[{"key":"http.method","value":{"stringValue":"GET"}},{"key":"service.name","value":{"stringValue":"my-svc"}}]"#;
        let attrs = parse_otlp_attr_array(input);
        assert_eq!(attrs.len(), 2);
        assert_eq!(attrs[0], ("http.method", "GET"));
        assert_eq!(attrs[1], ("service.name", "my-svc"));
    }

    #[test]
    fn test_parse_otlp_attr_array_int_value() {
        let input = r#"[{"key":"http.status_code","value":{"intValue":"200"}}]"#;
        let attrs = parse_otlp_attr_array(input);
        assert_eq!(attrs[0], ("http.status_code", "200"));
    }

    #[test]
    fn test_parse_otlp_attr_array_empty() {
        assert!(parse_otlp_attr_array("[]").is_empty());
    }

    #[test]
    fn test_parse_otlp_attr_array_not_array() {
        assert!(parse_otlp_attr_array(r#"{"key":"v"}"#).is_empty());
    }

    // ── parse_sdk_attr_dict ──────────────────────────────────────────────────

    #[test]
    fn test_parse_sdk_attr_dict() {
        let input = r#"{"http.method":"GET","service.name":"svc","http.status_code":"200"}"#;
        let attrs = parse_sdk_attr_dict(input);
        assert!(attrs.iter().any(|&(k, v)| k == "http.method" && v == "GET"));
        assert!(
            attrs
                .iter()
                .any(|&(k, v)| k == "service.name" && v == "svc")
        );
    }

    // ── parse_line: OTLP/JSON ────────────────────────────────────────────────

    #[test]
    fn test_parse_otlp_json_full() {
        let line = br#"{"timeUnixNano":"1700046000000000000","severityNumber":9,"severityText":"INFO","body":{"stringValue":"request completed"},"attributes":[{"key":"http.method","value":{"stringValue":"GET"}},{"key":"service.name","value":{"stringValue":"my-svc"}}],"traceId":"abc123","spanId":"def456"}"#;
        let parser = OtlpParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("1700046000000000000"));
        assert_eq!(parts.level, Some("INFO"));
        assert_eq!(parts.target, Some("my-svc"));
        assert_eq!(parts.message, Some("request completed"));
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|&(k, v)| k == "http.method" && v == "GET")
        );
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|&(k, v)| k == "traceId" && v == "abc123")
        );
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|&(k, v)| k == "spanId" && v == "def456")
        );
    }

    #[test]
    fn test_parse_otlp_json_severity_number_fallback() {
        // severityText is empty string → fall back to severityNumber
        let line = br#"{"timeUnixNano":"1700046000000000000","severityNumber":17,"severityText":"","body":{"stringValue":"oh no"}}"#;
        let parser = OtlpParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("ERROR"));
    }

    #[test]
    fn test_parse_otlp_json_body_string_value_object() {
        let line = br#"{"timeUnixNano":"1","severityText":"WARN","body":{"stringValue":"disk full"}}"#;
        let parser = OtlpParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.message, Some("disk full"));
    }

    // ── parse_line: OTel SDK JSON ────────────────────────────────────────────

    #[test]
    fn test_parse_sdk_json_flat() {
        let line = br#"{"timestamp":"2024-01-15T10:00:00Z","severity_text":"INFO","severity_number":9,"body":"user logged in","attributes":{"user.id":"u123","service.name":"auth-svc"},"trace_id":"abc","span_id":"def"}"#;
        let parser = OtlpParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024-01-15T10:00:00Z"));
        assert_eq!(parts.level, Some("INFO"));
        assert_eq!(parts.target, Some("auth-svc"));
        assert_eq!(parts.message, Some("user logged in"));
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|&(k, v)| k == "user.id" && v == "u123")
        );
    }

    #[test]
    fn test_parse_sdk_json_direct_body_string() {
        let line = br#"{"severity_text":"DEBUG","body":"cache miss","attributes":{}}"#;
        let parser = OtlpParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("DEBUG"));
        assert_eq!(parts.message, Some("cache miss"));
    }

    #[test]
    fn test_parse_sdk_json_resource_service_name() {
        let line = br#"{"severity_text":"INFO","body":"started","resource":{"attributes":{"service.name":"worker"}}}"#;
        let parser = OtlpParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.target, Some("worker"));
    }

    // ── Non-OTLP lines are rejected ──────────────────────────────────────────

    #[test]
    fn test_parse_plain_json_not_otlp() {
        let line = br#"{"level":"INFO","msg":"hello","time":"2024-01-01"}"#;
        let parser = OtlpParser;
        assert!(parser.parse_line(line).is_none());
    }

    #[test]
    fn test_parse_non_json_not_otlp() {
        let parser = OtlpParser;
        assert!(parser.parse_line(b"plain text log line").is_none());
    }

    // ── detect_score ─────────────────────────────────────────────────────────

    #[test]
    fn test_detect_score_otlp_lines() {
        let parser = OtlpParser;
        let lines: Vec<&[u8]> = vec![
            br#"{"timeUnixNano":"1","severityText":"INFO","body":{"stringValue":"a"}}"#,
            br#"{"timeUnixNano":"2","severityText":"WARN","body":{"stringValue":"b"}}"#,
        ];
        let score = parser.detect_score(&lines);
        assert!(score > 1.0, "OTLP should score > 1.0, got {score}");
    }

    #[test]
    fn test_detect_score_plain_json_zero() {
        let parser = OtlpParser;
        let lines: Vec<&[u8]> = vec![
            br#"{"level":"INFO","msg":"hello"}"#,
            br#"{"level":"WARN","msg":"world"}"#,
        ];
        let score = parser.detect_score(&lines);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_detect_score_empty_sample() {
        let parser = OtlpParser;
        assert_eq!(parser.detect_score(&[]), 0.0);
    }

    // ── collect_field_names ──────────────────────────────────────────────────

    #[test]
    fn test_collect_field_names_includes_attributes() {
        let parser = OtlpParser;
        let lines: Vec<&[u8]> = vec![
            br#"{"timeUnixNano":"1","severityText":"INFO","body":{"stringValue":"ok"},"attributes":[{"key":"http.method","value":{"stringValue":"GET"}},{"key":"http.status_code","value":{"intValue":"200"}}]}"#,
        ];
        let names = parser.collect_field_names(&lines);
        assert_eq!(names[0], "timestamp");
        assert_eq!(names[1], "level");
        assert_eq!(names[2], "target");
        assert!(names.contains(&"http.method".to_string()));
        assert!(names.contains(&"http.status_code".to_string()));
        assert_eq!(*names.last().unwrap(), "message");
    }

    // ── detect_format integration ────────────────────────────────────────────

    #[test]
    fn test_detect_format_prefers_otlp_over_json() {
        use crate::parser::detect_format;
        let lines: Vec<&[u8]> = vec![
            br#"{"timeUnixNano":"1700046000000000000","severityNumber":9,"severityText":"INFO","body":{"stringValue":"request completed"}}"#,
            br#"{"timeUnixNano":"1700046001000000000","severityNumber":13,"severityText":"WARN","body":{"stringValue":"slow query"}}"#,
        ];
        let parser = detect_format(&lines).unwrap();
        assert_eq!(parser.name(), "otlp");
    }

    #[test]
    fn test_detect_format_sdk_json() {
        use crate::parser::detect_format;
        let lines: Vec<&[u8]> = vec![
            br#"{"timestamp":"2024-01-15T10:00:00Z","severity_text":"INFO","body":"user logged in","attributes":{"service.name":"auth"}}"#,
            br#"{"timestamp":"2024-01-15T10:00:01Z","severity_text":"WARN","body":"slow query","attributes":{"service.name":"auth"}}"#,
        ];
        let parser = detect_format(&lines).unwrap();
        assert_eq!(parser.name(), "otlp");
    }

    #[test]
    fn test_otlp_name() {
        assert_eq!(OtlpParser.name(), "otlp");
    }
}
