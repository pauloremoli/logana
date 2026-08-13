use super::schema::LogSchema;
use super::timestamp::normalize_level;
use super::types::{DisplayParts, FieldSemantic, LogFormatParser, push_extra_field, push_field_as};
use std::collections::HashSet;

pub static SCHEMA_LOGFMT: LogSchema = LogSchema {
    name: "logfmt",
    detect_keys: &[],
    timestamp_keys: &["time", "ts", "t", "timestamp", "@timestamp", "datetime"],
    level_keys: &["level", "lvl", "severity"],
    target_keys: &[
        "source",
        "module",
        "logger",
        "component",
        "service",
        "caller",
        "name",
        "target",
    ],
    message_keys: &["message", "msg", "log", "text"],
    extra_semantics: &[],
    level_transform: None,
    keep_visible_extras: &[],
};

#[derive(Debug)]
pub struct LogfmtParser {
    pub schema: &'static LogSchema,
}

impl Default for LogfmtParser {
    fn default() -> Self {
        Self {
            schema: &SCHEMA_LOGFMT,
        }
    }
}

fn parse_pair(s: &str, mut pos: usize) -> Option<(&str, &str, usize)> {
    let b = s.as_bytes();

    while pos < b.len() && b[pos] == b' ' {
        pos += 1;
    }
    if pos >= b.len() {
        return None;
    }

    let key_start = pos;
    while pos < b.len() && b[pos] != b'=' && b[pos] != b' ' {
        pos += 1;
    }
    if pos >= b.len() || b[pos] != b'=' {
        return None;
    }
    let key = &s[key_start..pos];
    if key.is_empty() {
        return None;
    }
    pos += 1;

    if pos < b.len() && b[pos] == b'"' {
        pos += 1;
        let val_start = pos;
        while pos < b.len() {
            if b[pos] == b'\\' && pos + 1 < b.len() {
                pos += 2;
            } else if b[pos] == b'"' {
                break;
            } else {
                pos += 1;
            }
        }
        let value = &s[val_start..pos];
        if pos < b.len() && b[pos] == b'"' {
            pos += 1;
        }
        Some((key, value, pos))
    } else {
        let val_start = pos;
        while pos < b.len() && b[pos] != b' ' {
            pos += 1;
        }
        Some((key, &s[val_start..pos], pos))
    }
}

/// Count pairs and check for known keys (case-insensitive) in a single pass.
fn analyze_pairs(s: &str, schema: &LogSchema) -> (usize, bool) {
    let mut count = 0;
    let mut has_known = false;
    let mut pos = 0;
    while let Some((key, _, new_pos)) = parse_pair(s, pos) {
        count += 1;
        if !has_known {
            let k = key.to_ascii_lowercase();
            has_known = !matches!(schema.classify_key(k.as_str()), FieldSemantic::Extra);
        }
        pos = new_pos;
    }
    (count, has_known)
}

fn parse_logfmt_line<'a>(s: &'a str, schema: &LogSchema) -> Option<DisplayParts<'a>> {
    let mut parts = DisplayParts::default();
    let mut pos = 0;
    let mut pair_count = 0;

    while let Some((key, value, new_pos)) = parse_pair(s, pos) {
        pair_count += 1;
        let k_lower = key.to_ascii_lowercase();

        match schema.classify_key(k_lower.as_str()) {
            FieldSemantic::Timestamp if parts.timestamp.is_none() => {
                parts.timestamp = Some(value);
            }
            FieldSemantic::Level if parts.level.is_none() => {
                parts.level = Some(normalize_level(value).unwrap_or(value));
            }
            FieldSemantic::Message if parts.message.is_none() => {
                parts.message = Some(value);
            }
            FieldSemantic::Target if parts.target.is_none() => {
                parts.target = Some(value);
            }
            FieldSemantic::Extra => {
                push_extra_field(&mut parts.extra_fields, key, value);
            }
            semantic => {
                push_field_as(&mut parts.extra_fields, semantic, value);
            }
        }

        pos = new_pos;
    }

    if pair_count >= 3 { Some(parts) } else { None }
}

impl LogFormatParser for LogfmtParser {
    fn parse_line<'a>(&self, line: &'a [u8]) -> Option<DisplayParts<'a>> {
        let s = std::str::from_utf8(line).ok()?;
        if s.is_empty() || s.starts_with('{') {
            return None;
        }
        parse_logfmt_line(s, self.schema)
    }

    fn collect_field_names(&self, lines: &[&[u8]]) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut extras = Vec::new();
        let mut has_timestamp = false;
        let mut has_level = false;
        let mut has_target = false;
        let mut has_message = false;

        for &line in lines {
            if let Some(parts) = self.parse_line(line) {
                if parts.timestamp.is_some() {
                    has_timestamp = true;
                }
                if parts.level.is_some() {
                    has_level = true;
                }
                if parts.target.is_some() {
                    has_target = true;
                }
                if parts.message.is_some() {
                    has_message = true;
                }
                for (_, key, _) in &parts.extra_fields {
                    let k = key.to_string();
                    if seen.insert(k.clone()) {
                        extras.push(k);
                    }
                }
            }
        }

        let mut result = Vec::new();
        if has_timestamp {
            result.push("timestamp".to_string());
        }
        if has_level {
            result.push("level".to_string());
        }
        if has_target {
            result.push("target".to_string());
        }
        extras.sort();
        extras.dedup();
        result.extend(extras);
        if has_message {
            result.push("message".to_string());
        }
        result
    }

    fn detect_score(&self, sample: &[&[u8]]) -> f64 {
        if sample.is_empty() {
            return 0.0;
        }
        let mut total_score = 0.0;
        let mut parseable = 0usize;
        for &line in sample {
            let s = match std::str::from_utf8(line) {
                Ok(s) if !s.is_empty() && !s.starts_with('{') => s,
                _ => continue,
            };
            let (pairs, has_known) = analyze_pairs(s, self.schema);
            if pairs >= 3 {
                parseable += 1;
                total_score += if has_known { 1.0 } else { 0.5 };
            }
        }
        if parseable == 0 {
            return 0.0;
        }
        total_score / sample.len() as f64
    }

    fn name(&self) -> &str {
        "logfmt"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parser() -> LogfmtParser {
        LogfmtParser::default()
    }

    #[test]
    fn test_parse_pair_simple() {
        let (k, v, pos) = parse_pair("key=value rest", 0).unwrap();
        assert_eq!(k, "key");
        assert_eq!(v, "value");
        assert_eq!(pos, 9);
    }

    #[test]
    fn test_parse_pair_quoted() {
        let (k, v, _) = parse_pair("msg=\"hello world\" rest", 0).unwrap();
        assert_eq!(k, "msg");
        assert_eq!(v, "hello world");
    }

    #[test]
    fn test_parse_pair_quoted_escaped() {
        let (k, v, _) = parse_pair(r#"msg="say \"hi\"" rest"#, 0).unwrap();
        assert_eq!(k, "msg");
        assert_eq!(v, r#"say \"hi\""#);
    }

    #[test]
    fn test_parse_pair_empty_value() {
        let (k, v, _) = parse_pair("key= next=val", 0).unwrap();
        assert_eq!(k, "key");
        assert_eq!(v, "");
    }

    #[test]
    fn test_parse_pair_empty_quoted_value() {
        let (k, v, _) = parse_pair("key=\"\" next=val", 0).unwrap();
        assert_eq!(k, "key");
        assert_eq!(v, "");
    }

    #[test]
    fn test_parse_pair_no_equals() {
        assert!(parse_pair("no_equals", 0).is_none());
    }

    #[test]
    fn test_parse_line_full() {
        let line = b"time=2024-01-01T00:00:00Z level=info msg=\"request handled\" status=200";
        let parts = parser().parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024-01-01T00:00:00Z"));
        assert_eq!(parts.level, Some("INFO"));
        assert_eq!(parts.message, Some("request handled"));
        assert_eq!(parts.extra_fields.len(), 1);
        assert_eq!(parts.extra_fields[0].1, "status");
        assert_eq!(parts.extra_fields[0].2, "200");
    }

    #[test]
    fn test_parse_line_with_target() {
        let line = b"time=2024-01-01 level=debug source=myapp msg=\"starting\" port=8080";
        let parts = parser().parse_line(line).unwrap();
        assert_eq!(parts.target, Some("myapp"));
        assert_eq!(parts.level, Some("DEBUG"));
    }

    #[test]
    fn test_parse_line_no_known_keys() {
        let line = b"foo=bar baz=qux quux=corge";
        let parts = parser().parse_line(line).unwrap();
        assert!(parts.timestamp.is_none());
        assert!(parts.level.is_none());
        assert_eq!(parts.extra_fields.len(), 3);
    }

    #[test]
    fn test_parse_line_too_few_pairs() {
        let line = b"key=value";
        assert!(parser().parse_line(line).is_none());
    }

    #[test]
    fn test_parse_line_json_rejected() {
        assert!(parser().parse_line(br#"{"level":"INFO"}"#).is_none());
    }

    #[test]
    fn test_parse_line_empty() {
        assert!(parser().parse_line(b"").is_none());
    }

    #[test]
    fn test_parse_line_go_slog_format() {
        let line = b"time=2024-07-24T10:00:00Z level=INFO msg=\"request handled\" method=GET path=/api status=200 duration=12ms";
        let parts = parser().parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024-07-24T10:00:00Z"));
        assert_eq!(parts.level, Some("INFO"));
        assert_eq!(parts.message, Some("request handled"));
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(_, k, v)| *k == "method" && *v == "GET")
        );
    }

    #[test]
    fn test_parse_line_heroku_format() {
        let line = b"at=info method=GET path=\"/\" host=myapp.herokuapp.com request_id=abc fwd=\"1.2.3.4\" dyno=web.1 connect=1ms service=5ms status=200 bytes=1234";
        let parts = parser().parse_line(line).unwrap();
        assert!(parts.extra_fields.len() > 3);
    }

    #[test]
    fn test_detect_score_full_logfmt() {
        let lines: Vec<&[u8]> = vec![
            b"time=2024-01-01 level=info msg=hello",
            b"time=2024-01-02 level=warn msg=world",
        ];
        let score = parser().detect_score(&lines);
        assert!(score > 0.9, "Expected high score, got {}", score);
    }

    #[test]
    fn test_detect_score_unknown_keys_lower() {
        let lines: Vec<&[u8]> = vec![b"foo=bar baz=qux quux=corge"];
        let score = parser().detect_score(&lines);
        assert!(score > 0.0);
        assert!(score <= 0.5);
    }

    #[test]
    fn test_detect_score_none() {
        let lines: Vec<&[u8]> = vec![b"just plain text", b"more text"];
        let score = parser().detect_score(&lines);
        assert!((score - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_detect_score_empty() {
        let lines: Vec<&[u8]> = vec![];
        let score = parser().detect_score(&lines);
        assert!((score - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_collect_field_names() {
        let lines: Vec<&[u8]> = vec![
            b"time=2024-01-01 level=info msg=hello status=200",
            b"time=2024-01-02 level=warn msg=world duration=5ms",
        ];
        let names = parser().collect_field_names(&lines);
        assert_eq!(names[0], "timestamp");
        assert_eq!(names[1], "level");
        assert!(names.contains(&"duration".to_string()));
        assert!(names.contains(&"status".to_string()));
        assert_eq!(*names.last().unwrap(), "message");
    }

    #[test]
    fn test_name() {
        assert_eq!(parser().name(), "logfmt");
    }

    #[test]
    fn test_level_normalization() {
        let line = b"time=2024-01-01 level=warning msg=test extra=val";
        let parts = parser().parse_line(line).unwrap();
        assert_eq!(parts.level, Some("WARN"));
    }

    #[test]
    fn test_level_normalization_debug() {
        let line = b"time=2024-01-01 lvl=DBG msg=test extra=val";
        let parts = parser().parse_line(line).unwrap();
        assert_eq!(parts.level, Some("DEBUG"));
    }
}
