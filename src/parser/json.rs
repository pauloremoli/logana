//! JSON log parser — schema-driven, one instance per log format.
//!
//! Multiple `JsonParser` instances are registered in the format registry,
//! each carrying a `LogSchema` that defines key→slot mappings for one format
//! (tracing-subscriber, journalctl JSON, GELF, generic JSON, …).
//!
//! JSON wrapper prefixes supported transparently:
//! - `json-sse`  `data: {...}`
//! - `json-seq`  RS (0x1E) + `{...}`

use std::collections::HashSet;

use super::schema::LogSchema;
use super::types::{
    DisplayParts, FieldSemantic, LogFormatParser, SpanInfo, push_extra_field, push_field_as,
};

#[derive(Debug)]
pub struct LogLine<'a> {
    pub text: &'a [u8],
    pub level: Option<&'a str>,
    pub timestamp: Option<&'a str>,
}

impl<'a> LogLine<'a> {
    pub fn parse(line: &'a [u8]) -> Self {
        if line.is_empty() {
            return LogLine {
                text: line,
                level: None,
                timestamp: None,
            };
        }

        let mut timestamp = None;
        let mut level = None;
        let mut current_pos = 0;

        if line.len() > 1
            && line[0] == b'['
            && let Some(ts_end_bracket) = line[1..].iter().position(|&b| b == b']')
        {
            timestamp = std::str::from_utf8(&line[1..ts_end_bracket + 1]).ok();
            current_pos = ts_end_bracket + 2;
            while current_pos < line.len() && line[current_pos] == b' ' {
                current_pos += 1;
            }
        }

        if current_pos < line.len() {
            if let Some(level_space) = line[current_pos..].iter().position(|&b| b == b' ') {
                level = std::str::from_utf8(&line[current_pos..current_pos + level_space]).ok();
            } else {
                level = std::str::from_utf8(&line[current_pos..]).ok();
            }
        }

        LogLine {
            text: line,
            level,
            timestamp,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct JsonField<'a> {
    pub key: &'a str,
    pub value: &'a str,
    pub value_is_string: bool,
}

/// Parse a JSON log line into zero-copy key-value fields.
/// Nested objects/arrays are captured verbatim, not recursed into.
pub fn parse_json_line(line: &[u8]) -> Option<Vec<JsonField<'_>>> {
    if line.is_empty() || line[0] != b'{' {
        return None;
    }

    let mut pos = 1;
    let mut fields = Vec::new();

    loop {
        pos += skip_ws(line, pos);

        if pos >= line.len() || line[pos] == b'}' {
            break;
        }

        if line[pos] != b'"' {
            return None;
        }
        pos += 1;
        let key = read_string(line, &mut pos)?;

        pos += skip_ws(line, pos);
        if pos >= line.len() || line[pos] != b':' {
            return None;
        }
        pos += 1;
        pos += skip_ws(line, pos);

        let (value, value_is_string) = read_value(line, &mut pos)?;

        fields.push(JsonField {
            key,
            value,
            value_is_string,
        });

        pos += skip_ws(line, pos);
        if pos < line.len() && line[pos] == b',' {
            pos += 1;
        }
    }

    if fields.is_empty() {
        None
    } else {
        Some(fields)
    }
}

pub fn build_display_json(
    fields: &[JsonField<'_>],
    hidden_names: &HashSet<String>,
    hidden_indices: &HashSet<usize>,
) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(fields.len());
    for (idx, field) in fields.iter().enumerate() {
        if hidden_indices.contains(&idx) || hidden_names.contains(field.key) {
            continue;
        }
        if field.value_is_string && (field.value.contains(' ') || field.value.is_empty()) {
            parts.push(format!("{}=\"{}\"", field.key, field.value));
        } else {
            parts.push(format!("{}={}", field.key, field.value));
        }
    }
    parts.join(" ")
}

/// Strip a `json-sse` `data: ` prefix if present.
fn strip_sse_prefix(line: &[u8]) -> &[u8] {
    line.strip_prefix(b"data: ").unwrap_or(line)
}

/// Strip a `json-seq` RS (0x1E) prefix if present.
fn strip_seq_prefix(line: &[u8]) -> &[u8] {
    if line.first() == Some(&0x1e) {
        &line[1..]
    } else {
        line
    }
}

/// Strip all known journalctl JSON wrapper prefixes (SSE and seq).
pub fn strip_json_prefixes(line: &[u8]) -> &[u8] {
    strip_seq_prefix(strip_sse_prefix(line))
}

/// Schema-driven JSON parser. One instance per log format in the registry.
#[derive(Debug)]
pub struct JsonParser {
    pub schema: &'static LogSchema,
    /// Key whose object value is unpacked into extra fields (e.g. `"fields"` for tracing-subscriber).
    pub fields_container: Option<&'static str>,
    /// Key whose object value is parsed as a span (e.g. `"span"` for tracing-subscriber).
    pub span_key: Option<&'static str>,
    /// Multiplier applied to the base detection score (>1.0 prioritises this parser).
    pub score_weight: f64,
}

impl JsonParser {
    fn classify_fields<'a>(
        &self,
        fields: &[JsonField<'a>],
        hidden_names: &HashSet<String>,
        hidden_indices: &HashSet<usize>,
    ) -> DisplayParts<'a> {
        let mut parts = DisplayParts::default();

        for (idx, field) in fields.iter().enumerate() {
            if hidden_indices.contains(&idx) || hidden_names.contains(field.key) {
                continue;
            }

            if let Some(container) = self.fields_container
                && field.key == container
                && !field.value_is_string
            {
                if let Some(sub_fields) = parse_json_line(field.value.as_bytes()) {
                    for sub in &sub_fields {
                        match self.schema.classify_key(sub.key) {
                            FieldSemantic::Message => {
                                parts.message.get_or_insert(sub.value);
                            }
                            FieldSemantic::Extra => {
                                push_extra_field(&mut parts.extra_fields, sub.key, sub.value);
                            }
                            semantic => {
                                push_field_as(&mut parts.extra_fields, semantic, sub.value);
                            }
                        }
                    }
                }
                continue;
            }

            if let Some(span_key) = self.span_key {
                if field.key == span_key && !field.value_is_string {
                    if let Some(sub_fields) = parse_json_line(field.value.as_bytes()) {
                        let mut span_name = "";
                        let mut span_fields: Vec<(&str, &str)> = Vec::new();
                        for sub in &sub_fields {
                            if sub.key == "name" {
                                span_name = sub.value;
                            } else {
                                span_fields.push((sub.key, sub.value));
                            }
                        }
                        parts.span = Some(SpanInfo {
                            name: span_name,
                            fields: span_fields,
                        });
                    }
                    continue;
                }
                if field.key == "spans" {
                    continue;
                }
            }

            match self.schema.classify_key(field.key) {
                FieldSemantic::Timestamp => {
                    parts.timestamp.get_or_insert(field.value);
                }
                FieldSemantic::Level => {
                    let value = if let Some(transform) = self.schema.level_transform {
                        transform(field.value).unwrap_or(field.value)
                    } else {
                        field.value
                    };
                    parts.level.get_or_insert(value);
                }
                FieldSemantic::Target => {
                    parts.target.get_or_insert(field.value);
                }
                FieldSemantic::Message => {
                    parts.message.get_or_insert(field.value);
                }
                FieldSemantic::Extra => {
                    push_extra_field(&mut parts.extra_fields, field.key, field.value);
                }
                semantic => {
                    push_field_as(&mut parts.extra_fields, semantic, field.value);
                }
            }
        }

        parts
    }
}

impl LogFormatParser for JsonParser {
    fn parse_line<'a>(&self, line: &'a [u8]) -> Option<DisplayParts<'a>> {
        let line = strip_json_prefixes(line);
        let fields = parse_json_line(line)?;
        Some(self.classify_fields(&fields, &HashSet::new(), &HashSet::new()))
    }

    fn collect_field_names(&self, lines: &[&[u8]]) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut timestamp_seen = false;
        let mut level_seen = false;
        let mut target_seen = false;
        let mut message_seen = false;
        let mut extras: Vec<String> = Vec::new();

        for &line in lines {
            let line = strip_json_prefixes(line);
            if let Some(fields) = parse_json_line(line) {
                for field in &fields {
                    let key = field.key;

                    if let Some(container) = self.fields_container
                        && key == container
                        && !field.value_is_string
                    {
                        if let Some(subs) = parse_json_line(field.value.as_bytes()) {
                            for sub in &subs {
                                let dotted = format!("{container}.{}", sub.key);
                                if seen.insert(dotted.clone()) {
                                    extras.push(dotted);
                                }
                            }
                        }
                        continue;
                    }

                    if let Some(span_key) = self.span_key {
                        if key == span_key && !field.value_is_string {
                            if let Some(subs) = parse_json_line(field.value.as_bytes()) {
                                for sub in &subs {
                                    let dotted = format!("{span_key}.{}", sub.key);
                                    if seen.insert(dotted.clone()) {
                                        extras.push(dotted);
                                    }
                                }
                            }
                            continue;
                        }
                        if key == "spans" {
                            continue;
                        }
                    }

                    if seen.insert(key.to_string()) {
                        match self.schema.classify_key(key) {
                            FieldSemantic::Timestamp => timestamp_seen = true,
                            FieldSemantic::Level => level_seen = true,
                            FieldSemantic::Target => target_seen = true,
                            FieldSemantic::Message => message_seen = true,
                            sem => {
                                let canonical = sem.to_string();
                                if canonical.is_empty() {
                                    extras.push(key.to_string());
                                } else {
                                    extras.push(canonical);
                                }
                            }
                        }
                    }
                }
            }
        }

        let mut result: Vec<String> = Vec::new();
        if timestamp_seen {
            result.push("timestamp".to_string());
        }
        if level_seen {
            result.push("level".to_string());
        }
        if target_seen {
            result.push("target".to_string());
        }
        extras.sort();
        extras.dedup();
        result.extend(extras);
        if message_seen {
            result.push("message".to_string());
        }
        result
    }

    fn detect_score(&self, sample: &[&[u8]]) -> f64 {
        let non_empty: Vec<&[u8]> = sample.iter().copied().filter(|l| !l.is_empty()).collect();
        if non_empty.is_empty() {
            return 0.0;
        }
        let matched = non_empty
            .iter()
            .filter(|&&l| {
                let l = strip_json_prefixes(l);
                if let Some(fields) = parse_json_line(l) {
                    let field_keys: Vec<&str> = fields.iter().map(|f| f.key).collect();
                    self.schema.matches_detect_keys(&field_keys)
                } else {
                    false
                }
            })
            .count();
        if matched == 0 {
            return 0.0;
        }
        (matched as f64 / non_empty.len() as f64) * self.score_weight
    }

    fn matches_for_detection(&self, line: &[u8]) -> bool {
        let l = strip_json_prefixes(line);
        if let Some(fields) = parse_json_line(l) {
            let field_keys: Vec<&str> = fields.iter().map(|f| f.key).collect();
            self.schema.matches_detect_keys(&field_keys)
        } else {
            false
        }
    }

    fn detection_weight(&self) -> f64 {
        self.score_weight
    }

    fn name(&self) -> &str {
        self.schema.name
    }

    fn default_hidden_fields(&self, sample: &[&[u8]]) -> HashSet<String> {
        if self.schema.keep_visible_extras.is_empty() {
            return HashSet::new();
        }
        self.collect_field_names(sample)
            .into_iter()
            .filter(|name| {
                let is_primary =
                    matches!(name.as_str(), "timestamp" | "level" | "target" | "message");
                let is_kept = self.schema.keep_visible_extras.contains(&name.as_str());
                !is_primary && !is_kept
            })
            .collect()
    }
}

fn skip_ws(line: &[u8], mut pos: usize) -> usize {
    let start = pos;
    while pos < line.len() && matches!(line[pos], b' ' | b'\t' | b'\r' | b'\n') {
        pos += 1;
    }
    pos - start
}

fn read_string<'a>(line: &'a [u8], pos: &mut usize) -> Option<&'a str> {
    let start = *pos;
    loop {
        if *pos >= line.len() {
            return None;
        }
        match line[*pos] {
            b'"' => {
                let s = std::str::from_utf8(&line[start..*pos]).ok()?;
                *pos += 1;
                return Some(s);
            }
            b'\\' => *pos += 2,
            _ => *pos += 1,
        }
    }
}

fn read_value<'a>(line: &'a [u8], pos: &mut usize) -> Option<(&'a str, bool)> {
    if *pos >= line.len() {
        return None;
    }
    match line[*pos] {
        b'"' => {
            *pos += 1;
            let value = read_string(line, pos)?;
            Some((value, true))
        }
        b'{' | b'[' => {
            let start = *pos;
            let open = line[*pos];
            let close = if open == b'{' { b'}' } else { b']' };
            let mut depth = 0usize;
            loop {
                if *pos >= line.len() {
                    break;
                }
                let c = line[*pos];
                if c == b'"' {
                    *pos += 1;
                    while *pos < line.len() {
                        let sc = line[*pos];
                        *pos += 1;
                        if sc == b'"' {
                            break;
                        }
                        if sc == b'\\' {
                            *pos += 1;
                        }
                    }
                } else {
                    *pos += 1;
                    if c == open {
                        depth += 1;
                    } else if c == close {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                }
            }
            let value = std::str::from_utf8(&line[start..*pos]).ok()?;
            Some((value, false))
        }
        _ => {
            let start = *pos;
            while *pos < line.len()
                && !matches!(
                    line[*pos],
                    b',' | b'}' | b']' | b' ' | b'\t' | b'\r' | b'\n'
                )
            {
                *pos += 1;
            }
            let value = std::str::from_utf8(&line[start..*pos]).ok()?.trim();
            Some((value, false))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::schema::{
        SCHEMA_GELF, SCHEMA_GENERIC_JSON, SCHEMA_JOURNALCTL_JSON, SCHEMA_TRACING,
    };

    fn generic_parser() -> JsonParser {
        JsonParser {
            schema: &SCHEMA_GENERIC_JSON,
            fields_container: None,
            span_key: None,
            score_weight: 1.0,
        }
    }

    fn tracing_parser() -> JsonParser {
        JsonParser {
            schema: &SCHEMA_TRACING,
            fields_container: Some("fields"),
            span_key: Some("span"),
            score_weight: 1.1,
        }
    }

    fn journalctl_parser() -> JsonParser {
        JsonParser {
            schema: &SCHEMA_JOURNALCTL_JSON,
            fields_container: None,
            span_key: None,
            score_weight: 1.2,
        }
    }

    fn gelf_parser() -> JsonParser {
        JsonParser {
            schema: &SCHEMA_GELF,
            fields_container: None,
            span_key: None,
            score_weight: 1.05,
        }
    }

    #[test]
    fn test_parse_log_line_full() {
        let line = b"[2024-07-24T10:00:00Z] INFO myhost: everything is fine";
        let log_line = LogLine::parse(line);
        assert_eq!(log_line.timestamp, Some("2024-07-24T10:00:00Z"));
        assert_eq!(log_line.level, Some("INFO"));
        assert_eq!(log_line.text, line);
    }

    #[test]
    fn test_parse_log_line_no_level_no_host() {
        let line = b"[2024-07-24T10:00:00Z] some message without level or host";
        let log_line = LogLine::parse(line);
        assert_eq!(log_line.timestamp, Some("2024-07-24T10:00:00Z"));
        assert_eq!(log_line.level, Some("some"));
        assert_eq!(log_line.text, line);
    }

    #[test]
    fn test_parse_log_line_empty() {
        let line = b"";
        let log_line = LogLine::parse(line);
        assert_eq!(log_line.timestamp, None);
        assert_eq!(log_line.level, None);
        assert_eq!(log_line.text, line);
    }

    #[test]
    fn test_parse_log_line_only_timestamp() {
        let line = b"[2024-07-24T10:00:00Z]";
        let log_line = LogLine::parse(line);
        assert_eq!(log_line.timestamp, Some("2024-07-24T10:00:00Z"));
        assert_eq!(log_line.level, None);
        assert_eq!(log_line.text, line);
    }

    #[test]
    fn test_parse_log_line_no_timestamp_bracket() {
        let line = b"2024-07-24T10:00:00Z INFO message";
        let log_line = LogLine::parse(line);
        assert_eq!(log_line.timestamp, None);
        assert_eq!(log_line.level, Some("2024-07-24T10:00:00Z"));
        assert_eq!(log_line.text, line);
    }

    // ── parse_json_line ──────────────────────────────────────────────────────

    #[test]
    fn test_parse_json_plain_not_json() {
        assert!(parse_json_line(b"not json").is_none());
        assert!(parse_json_line(b"").is_none());
        assert!(parse_json_line(b"[]").is_none());
    }

    #[test]
    fn test_parse_json_empty_object() {
        assert!(parse_json_line(b"{}").is_none());
    }

    #[test]
    fn test_parse_json_simple_string_fields() {
        let line = br#"{"level":"INFO","msg":"hello"}"#;
        let fields = parse_json_line(line).unwrap();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].key, "level");
        assert_eq!(fields[0].value, "INFO");
        assert!(fields[0].value_is_string);
        assert_eq!(fields[1].key, "msg");
        assert_eq!(fields[1].value, "hello");
        assert!(fields[1].value_is_string);
    }

    #[test]
    fn test_parse_json_number_and_bool_values() {
        let line = br#"{"pid":1234,"active":true,"score":3.14}"#;
        let fields = parse_json_line(line).unwrap();
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].key, "pid");
        assert_eq!(fields[0].value, "1234");
        assert!(!fields[0].value_is_string);
        assert_eq!(fields[1].key, "active");
        assert_eq!(fields[1].value, "true");
        assert_eq!(fields[2].key, "score");
        assert_eq!(fields[2].value, "3.14");
    }

    #[test]
    fn test_parse_json_null_value() {
        let line = br#"{"error":null,"msg":"ok"}"#;
        let fields = parse_json_line(line).unwrap();
        assert_eq!(fields[0].key, "error");
        assert_eq!(fields[0].value, "null");
        assert!(!fields[0].value_is_string);
    }

    #[test]
    fn test_parse_json_journalctl_format() {
        let line = br#"{"__REALTIME_TIMESTAMP":"1699999999000000","PRIORITY":"6","_HOSTNAME":"myhost","SYSLOG_IDENTIFIER":"sshd","MESSAGE":"Accepted password for user"}"#;
        let fields = parse_json_line(line).unwrap();
        assert!(
            fields
                .iter()
                .any(|f| f.key == "MESSAGE" && f.value == "Accepted password for user")
        );
        assert!(fields.iter().any(|f| f.key == "PRIORITY" && f.value == "6"));
        assert!(
            fields
                .iter()
                .any(|f| f.key == "_HOSTNAME" && f.value == "myhost")
        );
    }

    #[test]
    fn test_parse_json_syslog_format() {
        let line = br#"{"time":"2024-01-15T10:00:00Z","level":"INFO","hostname":"myhost","app":"nginx","message":"GET /health 200"}"#;
        let fields = parse_json_line(line).unwrap();
        assert!(
            fields
                .iter()
                .any(|f| f.key == "message" && f.value == "GET /health 200")
        );
        assert!(fields.iter().any(|f| f.key == "level" && f.value == "INFO"));
        assert!(
            fields
                .iter()
                .any(|f| f.key == "hostname" && f.value == "myhost")
        );
    }

    #[test]
    fn test_parse_json_escaped_string() {
        let line = br#"{"msg":"hello \"world\"","level":"DEBUG"}"#;
        let fields = parse_json_line(line).unwrap();
        assert_eq!(fields[0].key, "msg");
        assert_eq!(fields[0].value, r#"hello \"world\""#);
    }

    #[test]
    fn test_parse_json_nested_object_value_captured_verbatim() {
        let line = br#"{"meta":{"host":"a","pid":1},"level":"INFO"}"#;
        let fields = parse_json_line(line).unwrap();
        assert_eq!(fields[0].key, "meta");
        assert_eq!(fields[0].value, r#"{"host":"a","pid":1}"#);
        assert!(!fields[0].value_is_string);
        assert_eq!(fields[1].key, "level");
        assert_eq!(fields[1].value, "INFO");
    }

    #[test]
    fn test_parse_json_array_value_captured_verbatim() {
        let line = br#"{"tags":["a","b","c"],"level":"WARN"}"#;
        let fields = parse_json_line(line).unwrap();
        assert_eq!(fields[0].key, "tags");
        assert_eq!(fields[0].value, r#"["a","b","c"]"#);
        assert!(!fields[0].value_is_string);
    }

    #[test]
    fn test_parse_json_whitespace_around_separators() {
        let line = b"{ \"level\" : \"INFO\" , \"msg\" : \"ok\" }";
        let fields = parse_json_line(line).unwrap();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].key, "level");
        assert_eq!(fields[0].value, "INFO");
        assert_eq!(fields[1].key, "msg");
        assert_eq!(fields[1].value, "ok");
    }

    #[test]
    fn test_parse_json_preserves_field_order() {
        let line = br#"{"z":"last","a":"first","m":"mid"}"#;
        let fields = parse_json_line(line).unwrap();
        assert_eq!(fields[0].key, "z");
        assert_eq!(fields[1].key, "a");
        assert_eq!(fields[2].key, "m");
    }

    // ── build_display_json ───────────────────────────────────────────────────

    #[test]
    fn test_build_display_json_no_hidden() {
        let line = br#"{"level":"INFO","msg":"hello"}"#;
        let fields = parse_json_line(line).unwrap();
        let display = build_display_json(&fields, &HashSet::new(), &HashSet::new());
        assert!(display.contains("level=INFO"));
        assert!(display.contains("msg=hello"));
    }

    #[test]
    fn test_build_display_json_hide_by_name() {
        let line = br#"{"level":"INFO","msg":"hello","pid":42}"#;
        let fields = parse_json_line(line).unwrap();
        let mut hidden = HashSet::new();
        hidden.insert("pid".to_string());
        let display = build_display_json(&fields, &hidden, &HashSet::new());
        assert!(display.contains("level=INFO"));
        assert!(display.contains("msg=hello"));
        assert!(!display.contains("pid="));
    }

    #[test]
    fn test_build_display_json_hide_by_index() {
        let line = br#"{"level":"INFO","msg":"hello","pid":42}"#;
        let fields = parse_json_line(line).unwrap();
        let mut hidden_idx = HashSet::new();
        hidden_idx.insert(0usize);
        let display = build_display_json(&fields, &HashSet::new(), &hidden_idx);
        assert!(!display.contains("level="));
        assert!(display.contains("msg=hello"));
        assert!(display.contains("pid=42"));
    }

    #[test]
    fn test_build_display_json_hide_all_fields_produces_empty_object() {
        let line = br#"{"level":"INFO"}"#;
        let fields = parse_json_line(line).unwrap();
        let mut hidden = HashSet::new();
        hidden.insert("level".to_string());
        let display = build_display_json(&fields, &hidden, &HashSet::new());
        assert_eq!(display, "");
    }

    #[test]
    fn test_build_display_json_non_string_value_no_quotes() {
        let line = br#"{"pid":1234,"ok":true}"#;
        let fields = parse_json_line(line).unwrap();
        let display = build_display_json(&fields, &HashSet::new(), &HashSet::new());
        assert!(display.contains("pid=1234"));
        assert!(display.contains("ok=true"));
        assert!(!display.contains("pid=\"1234\""));
    }

    #[test]
    fn test_build_display_json_journalctl_hide_cursor_and_timestamp() {
        let line = br#"{"__CURSOR":"s=abc","__REALTIME_TIMESTAMP":"1699","MESSAGE":"hello","PRIORITY":"6"}"#;
        let fields = parse_json_line(line).unwrap();
        let mut hidden = HashSet::new();
        hidden.insert("__CURSOR".to_string());
        hidden.insert("__REALTIME_TIMESTAMP".to_string());
        let display = build_display_json(&fields, &hidden, &HashSet::new());
        assert!(!display.contains("__CURSOR"));
        assert!(!display.contains("__REALTIME_TIMESTAMP"));
        assert!(display.contains("MESSAGE=hello"));
        assert!(display.contains("PRIORITY=6"));
    }

    // ── JsonParser::classify_fields ──────────────────────────────────────────

    #[test]
    fn test_classify_known_fields_extracted() {
        let parser = generic_parser();
        let line = br#"{"timestamp":"2024-01-01T00:00:00Z","level":"INFO","target":"myapp","message":"hello"}"#;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024-01-01T00:00:00Z"));
        assert_eq!(parts.level, Some("INFO"));
        assert_eq!(parts.target, Some("myapp"));
        assert_eq!(parts.message, Some("hello"));
        assert!(parts.extra_fields.is_empty());
    }

    #[test]
    fn test_classify_unknown_fields_go_to_extra() {
        let parser = generic_parser();
        let line = br#"{"level":"WARN","request_id":"abc","msg":"hi"}"#;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("WARN"));
        assert_eq!(parts.message, Some("hi"));
        assert_eq!(parts.extra_fields.len(), 1);
        assert_eq!(parts.extra_fields[0].1, "request_id");
        assert_eq!(parts.extra_fields[0].2, "abc");
    }

    #[test]
    fn test_classify_extra_fields_preserve_order() {
        let parser = generic_parser();
        let line = br#"{"msg":"hi","z_field":"z","a_field":"a","level":"INFO"}"#;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.extra_fields[0].1, "z_field");
        assert_eq!(parts.extra_fields[1].1, "a_field");
    }

    #[test]
    fn test_classify_hidden_by_name_excluded() {
        let parser = generic_parser();
        let line = br#"{"level":"INFO","request_id":"abc","msg":"hi"}"#;
        let fields = parse_json_line(line).unwrap();
        let mut hidden = HashSet::new();
        hidden.insert("request_id".to_string());
        let parts = parser.classify_fields(&fields, &hidden, &HashSet::new());
        assert!(parts.extra_fields.is_empty());
        assert_eq!(parts.message, Some("hi"));
    }

    #[test]
    fn test_classify_hidden_by_index_excluded() {
        let parser = generic_parser();
        let line = br#"{"level":"INFO","request_id":"abc","msg":"hi"}"#;
        let fields = parse_json_line(line).unwrap();
        let mut hidden_idx = HashSet::new();
        hidden_idx.insert(1usize);
        let parts = parser.classify_fields(&fields, &HashSet::new(), &hidden_idx);
        assert!(parts.extra_fields.is_empty());
    }

    #[test]
    fn test_classify_journalctl_format() {
        let parser = journalctl_parser();
        let line = br#"{"__REALTIME_TIMESTAMP":"1699999999","PRIORITY":"6","SYSLOG_IDENTIFIER":"sshd","MESSAGE":"Accepted"}"#;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("1699999999"));
        assert_eq!(parts.level, Some("INFO"));
        assert_eq!(parts.target, Some("sshd"));
        assert_eq!(parts.message, Some("Accepted"));
    }

    #[test]
    fn test_classify_duplicate_known_key_drops_second() {
        let parser = generic_parser();
        let line = br#"{"time":"t1","ts":"t2","msg":"hi"}"#;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("t1"));
        assert!(parts.extra_fields.is_empty());
    }

    #[test]
    fn test_classify_all_unknown_fields_only() {
        let parser = generic_parser();
        let line = br#"{"foo":"bar","baz":42}"#;
        let parts = parser.parse_line(line).unwrap();
        assert!(parts.timestamp.is_none());
        assert!(parts.level.is_none());
        assert!(parts.target.is_none());
        assert!(parts.message.is_none());
        assert_eq!(parts.extra_fields.len(), 2);
    }

    #[test]
    fn test_classify_fields_container_extracts_message() {
        let parser = tracing_parser();
        let line = br#"{"level":"INFO","target":"todo_app","fields":{"message":"Listening on 0.0.0.0:3000"}}"#;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.message, Some("Listening on 0.0.0.0:3000"));
        assert!(parts.extra_fields.is_empty());
    }

    #[test]
    fn test_classify_fields_container_extracts_extras_too() {
        let parser = tracing_parser();
        let line = br#"{"level":"INFO","fields":{"message":"todos listed","count":9}}"#;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.message, Some("todos listed"));
        assert_eq!(parts.extra_fields.len(), 1);
        assert_eq!(parts.extra_fields[0].1, "count");
        assert_eq!(parts.extra_fields[0].2, "9");
    }

    #[test]
    fn test_classify_span_extracts_name_and_fields() {
        let parser = tracing_parser();
        let line = br#"{"level":"INFO","span":{"name":"request","method":"GET","uri":"/todos"},"fields":{"message":"ok"}}"#;
        let parts = parser.parse_line(line).unwrap();
        let span = parts.span.unwrap();
        assert_eq!(span.name, "request");
        assert_eq!(span.fields.len(), 2);
        assert!(
            span.fields
                .iter()
                .any(|(k, v)| *k == "method" && *v == "GET")
        );
        assert!(
            span.fields
                .iter()
                .any(|(k, v)| *k == "uri" && *v == "/todos")
        );
    }

    #[test]
    fn test_classify_spans_array_is_skipped() {
        let parser = tracing_parser();
        let line = br#"{"level":"INFO","spans":[{"name":"root"}],"message":"hi"}"#;
        let parts = parser.parse_line(line).unwrap();
        assert!(parts.extra_fields.is_empty());
        assert!(parts.span.is_none());
    }

    #[test]
    fn test_gelf_short_message_classified_as_message() {
        let parser = gelf_parser();
        let line =
            br#"{"version":"1.1","host":"example.org","short_message":"A short message","level":1}"#;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.message, Some("A short message"));
    }

    // ── JsonParser trait impl ────────────────────────────────────────────────

    #[test]
    fn test_json_parser_parse_line() {
        let parser = generic_parser();
        let line = br#"{"timestamp":"2024-01-01","level":"INFO","message":"hello"}"#;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024-01-01"));
        assert_eq!(parts.level, Some("INFO"));
        assert_eq!(parts.message, Some("hello"));
    }

    #[test]
    fn test_json_parser_parse_line_not_json() {
        let parser = generic_parser();
        assert!(parser.parse_line(b"not json").is_none());
    }

    #[test]
    fn test_json_parser_detect_score_generic_matches_all_json() {
        let parser = generic_parser();
        let lines: Vec<&[u8]> = vec![
            br#"{"level":"INFO","msg":"hello"}"#,
            br#"{"level":"WARN","msg":"world"}"#,
        ];
        let score = parser.detect_score(&lines);
        assert!((score - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_json_parser_detect_score_mixed() {
        let parser = generic_parser();
        let lines: Vec<&[u8]> = vec![br#"{"level":"INFO","msg":"hello"}"#, b"not json"];
        let score = parser.detect_score(&lines);
        assert!((score - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_json_parser_detect_score_journalctl_schema_requires_specific_keys() {
        let parser = journalctl_parser();
        let lines: Vec<&[u8]> = vec![
            br#"{"level":"INFO","msg":"hello"}"#,
            br#"{"level":"WARN","msg":"world"}"#,
        ];
        let score = parser.detect_score(&lines);
        assert!(
            (score).abs() < 0.001,
            "journalctl schema should not match generic JSON"
        );
    }

    #[test]
    fn test_json_parser_detect_score_journalctl_matches_journalctl_json() {
        let parser = journalctl_parser();
        let lines: Vec<&[u8]> =
            vec![br#"{"MESSAGE":"hello","PRIORITY":"6","__REALTIME_TIMESTAMP":"123"}"#];
        let score = parser.detect_score(&lines);
        assert!(
            score > 1.0,
            "journalctl schema should score > 1.0 on journalctl JSON"
        );
    }

    #[test]
    fn test_json_parser_collect_field_names_canonical() {
        let parser = generic_parser();
        let lines: Vec<&[u8]> =
            vec![br#"{"timestamp":"2024","level":"INFO","request_id":"abc","message":"hi"}"#];
        let names = parser.collect_field_names(&lines);
        assert_eq!(names[0], "timestamp");
        assert_eq!(names[1], "level");
        assert!(names.contains(&"request_id".to_string()));
        assert_eq!(*names.last().unwrap(), "message");
    }

    #[test]
    fn test_json_parser_collect_field_names_returns_canonical_not_raw() {
        let parser = generic_parser();
        let lines: Vec<&[u8]> = vec![br#"{"ts":"2024","lvl":"INFO","msg":"hi"}"#];
        let names = parser.collect_field_names(&lines);
        assert!(
            names.contains(&"timestamp".to_string()),
            "ts should be normalised to 'timestamp'"
        );
        assert!(
            names.contains(&"level".to_string()),
            "lvl should be normalised to 'level'"
        );
        assert!(
            names.contains(&"message".to_string()),
            "msg should be normalised to 'message'"
        );
    }

    #[test]
    fn test_json_parser_name() {
        assert_eq!(generic_parser().name(), "json");
        assert_eq!(journalctl_parser().name(), "journalctl-json");
        assert_eq!(tracing_parser().name(), "tracing-json");
        assert_eq!(gelf_parser().name(), "gelf");
    }

    // ── json-sse (Server-Sent Events) support ────────────────────────────────

    #[test]
    fn test_json_sse_parse_line() {
        let parser = generic_parser();
        let line = br#"data: {"level":"INFO","msg":"hello"}"#;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("INFO"));
        assert_eq!(parts.message, Some("hello"));
    }

    #[test]
    fn test_json_sse_detect_score() {
        let parser = generic_parser();
        let lines: Vec<&[u8]> = vec![
            br#"data: {"level":"INFO","msg":"hello"}"#,
            br#"data: {"level":"WARN","msg":"world"}"#,
        ];
        let score = parser.detect_score(&lines);
        assert!((score - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_json_sse_collect_field_names() {
        let parser = generic_parser();
        let lines: Vec<&[u8]> = vec![br#"data: {"timestamp":"2024","level":"INFO","msg":"hi"}"#];
        let names = parser.collect_field_names(&lines);
        assert!(names.contains(&"timestamp".to_string()));
        assert!(names.contains(&"level".to_string()));
        assert!(names.contains(&"message".to_string()));
    }

    // ── json-seq (RFC 7464 JSON Sequence) support ────────────────────────────

    #[test]
    fn test_json_seq_parse_line() {
        let parser = generic_parser();
        let mut line = vec![0x1eu8];
        line.extend_from_slice(br#"{"level":"INFO","msg":"hello"}"#);
        let parts = parser.parse_line(&line).unwrap();
        assert_eq!(parts.level, Some("INFO"));
        assert_eq!(parts.message, Some("hello"));
    }

    #[test]
    fn test_json_seq_detect_score() {
        let parser = generic_parser();
        let mut line1 = vec![0x1eu8];
        line1.extend_from_slice(br#"{"level":"INFO","msg":"hello"}"#);
        let mut line2 = vec![0x1eu8];
        line2.extend_from_slice(br#"{"level":"WARN","msg":"world"}"#);
        let lines: Vec<&[u8]> = vec![&line1, &line2];
        let score = parser.detect_score(&lines);
        assert!((score - 1.0).abs() < 0.001);
    }

    // ── journalctl JSON default_hidden_fields ────────────────────────────────

    #[test]
    fn test_default_hidden_fields_journalctl_json() {
        let parser = journalctl_parser();
        let lines: Vec<&[u8]> = vec![
            br#"{"__CURSOR":"s=abc","__REALTIME_TIMESTAMP":"1699","__MONOTONIC_TIMESTAMP":"123","_BOOT_ID":"abc","PRIORITY":"6","_HOSTNAME":"myhost","SYSLOG_IDENTIFIER":"sshd","_PID":"1234","_UID":"1000","_COMM":"sshd","MESSAGE":"Accepted"}"#,
        ];
        let hidden = parser.default_hidden_fields(&lines);
        assert!(hidden.contains("__CURSOR"), "__CURSOR should be hidden");
        assert!(
            hidden.contains("__MONOTONIC_TIMESTAMP"),
            "__MONOTONIC_TIMESTAMP should be hidden"
        );
        assert!(hidden.contains("_BOOT_ID"), "_BOOT_ID should be hidden");
        assert!(hidden.contains("_UID"), "_UID should be hidden");
        assert!(
            !hidden.contains("__REALTIME_TIMESTAMP"),
            "__REALTIME_TIMESTAMP should be visible (timestamp slot)"
        );
        assert!(
            !hidden.contains("PRIORITY"),
            "PRIORITY should be visible (level slot)"
        );
        assert!(
            !hidden.contains("SYSLOG_IDENTIFIER"),
            "SYSLOG_IDENTIFIER should be visible (target slot)"
        );
        assert!(!hidden.contains("hostname"), "hostname should be visible");
        assert!(!hidden.contains("pid"), "pid should be visible");
        assert!(
            !hidden.contains("MESSAGE"),
            "MESSAGE should be visible (message slot)"
        );
    }

    #[test]
    fn test_default_hidden_fields_generic_json_empty() {
        let parser = generic_parser();
        let lines: Vec<&[u8]> =
            vec![br#"{"timestamp":"2024","level":"INFO","message":"hello","request_id":"abc"}"#];
        let hidden = parser.default_hidden_fields(&lines);
        assert!(
            hidden.is_empty(),
            "Generic JSON should have no default hidden fields"
        );
    }

    #[test]
    fn test_default_hidden_fields_journalctl_json_sse() {
        let parser = journalctl_parser();
        let lines: Vec<&[u8]> = vec![
            br#"data: {"__CURSOR":"s=abc","__REALTIME_TIMESTAMP":"1699","PRIORITY":"6","_HOSTNAME":"myhost","SYSLOG_IDENTIFIER":"sshd","_PID":"1234","_BOOT_ID":"xyz","MESSAGE":"Started"}"#,
        ];
        let hidden = parser.default_hidden_fields(&lines);
        assert!(hidden.contains("__CURSOR"));
        assert!(hidden.contains("_BOOT_ID"));
        assert!(!hidden.contains("hostname"));
        assert!(!hidden.contains("pid"));
    }

    // ── strip_json_prefixes ──────────────────────────────────────────────────

    #[test]
    fn test_strip_sse_prefix() {
        assert_eq!(strip_sse_prefix(b"data: {}"), b"{}");
        assert_eq!(strip_sse_prefix(b"{}"), b"{}");
        assert_eq!(strip_sse_prefix(b"data:{}"), b"data:{}");
    }

    #[test]
    fn test_strip_seq_prefix() {
        let with_rs = [0x1eu8, b'{', b'}'];
        assert_eq!(strip_seq_prefix(&with_rs), b"{}");
        assert_eq!(strip_seq_prefix(b"{}"), b"{}");
    }
}
