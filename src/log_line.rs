use std::collections::HashSet;

// ---------------------------------------------------------------------------
// Legacy parser (kept for backward compatibility)
// ---------------------------------------------------------------------------

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

        // Try to parse timestamp
        if line.len() > 1 && line[0] == b'['
            && let Some(ts_end_bracket) = line[1..].iter().position(|&b| b == b']') {
                timestamp = std::str::from_utf8(&line[1..ts_end_bracket + 1]).ok();
                current_pos = ts_end_bracket + 2;
                while current_pos < line.len() && line[current_pos] == b' ' {
                    current_pos += 1;
                }
            }

        // Try to parse level
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

// ---------------------------------------------------------------------------
// Zero-copy JSON log line parsers
// ---------------------------------------------------------------------------

/// A single key-value field extracted from a JSON log line.
/// Both `key` and `value` are string slices directly into the original line bytes —
/// no heap allocation beyond the `Vec` that holds them.
#[derive(Debug, Clone, PartialEq)]
pub struct JsonField<'a> {
    pub key: &'a str,
    /// The field value, stripped of surrounding quotes for string values.
    pub value: &'a str,
    /// `true` when the JSON value was a quoted string.
    pub value_is_string: bool,
}

/// Detected JSON log format, determined by the field names present.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LogFormat {
    /// Not a recognised JSON log format.
    Plain,
    /// `journalctl -o json` output: ALL_CAPS field names, mandatory `MESSAGE` key.
    JournalctlJson,
    /// Structured syslog / application JSON: lowercase `message`, `msg`, `log`, or `text` key.
    SyslogJson,
}

/// Parse a single JSON log line into a list of zero-copy key-value fields.
///
/// Returns `None` for lines that do not start with `{` or are otherwise not
/// valid JSON objects.  Only the subset of JSON used by journalctl and syslog
/// is supported: string, number, boolean, null, and raw nested objects/arrays
/// (nested values are captured verbatim, not recursed into).
///
/// Escape sequences inside string values are preserved verbatim.
pub fn parse_json_line(line: &[u8]) -> Option<Vec<JsonField<'_>>> {
    if line.is_empty() || line[0] != b'{' {
        return None;
    }

    let mut pos = 1; // skip '{'
    let mut fields = Vec::new();

    loop {
        pos += skip_ws(line, pos);

        if pos >= line.len() || line[pos] == b'}' {
            break;
        }

        // Key must be a quoted string
        if line[pos] != b'"' {
            return None;
        }
        pos += 1; // skip opening '"'
        let key = read_string(line, &mut pos)?;

        // Skip optional whitespace, then ':'
        pos += skip_ws(line, pos);
        if pos >= line.len() || line[pos] != b':' {
            return None;
        }
        pos += 1;
        pos += skip_ws(line, pos);

        // Read value
        let (value, value_is_string) = read_value(line, &mut pos)?;

        fields.push(JsonField { key, value, value_is_string });

        // Skip optional comma and whitespace before next key
        pos += skip_ws(line, pos);
        if pos < line.len() && line[pos] == b',' {
            pos += 1;
        }
    }

    if fields.is_empty() { None } else { Some(fields) }
}

/// Detect the JSON log format from a parsed field list.
pub fn detect_json_format(fields: &[JsonField<'_>]) -> LogFormat {
    if fields.iter().any(|f| f.key == "MESSAGE") {
        LogFormat::JournalctlJson
    } else if fields.iter().any(|f| matches!(f.key, "message" | "msg" | "log" | "text")) {
        LogFormat::SyslogJson
    } else {
        LogFormat::Plain
    }
}

/// Build a display string from `fields`, omitting any field whose name is in
/// `hidden_names` or whose 0-based index is in `hidden_indices`.
///
/// Output format: logfmt-style `key=value` pairs separated by two spaces.
/// String values that contain spaces or are empty are double-quoted.
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
    parts.join("  ")
}

// ---------------------------------------------------------------------------
// Structured JSON display
// ---------------------------------------------------------------------------

const TIMESTAMP_KEYS: &[&str] = &[
    "timestamp",
    "time",
    "ts",
    "@timestamp",
    "datetime",
    "_SOURCE_REALTIME_TIMESTAMP",
    "__REALTIME_TIMESTAMP",
];
const LEVEL_KEYS: &[&str] = &["level", "lvl", "severity", "PRIORITY", "log_level"];
const TARGET_KEYS: &[&str] = &[
    "target",
    "module",
    "logger",
    "source",
    "component",
    "service",
    "name",
    "SYSLOG_IDENTIFIER",
    "_COMM",
    "caller",
];
const MESSAGE_KEYS: &[&str] =
    &["message", "msg", "log", "text", "MESSAGE", "body"];

/// Span context extracted from a `span` field of a tracing JSON log line.
#[derive(Debug)]
pub struct SpanInfo {
    /// Value of the `name` key inside the span object.
    pub name: String,
    /// All other span fields in document order: `(key, value)`.
    pub fields: Vec<(String, String)>,
}

/// Structured representation of a parsed JSON log line, ready for display.
/// Known fields are extracted into typed slots; all other visible fields land
/// in `extra_fields` and are rendered before the message.
#[derive(Debug)]
pub struct JsonDisplayParts {
    pub timestamp: Option<String>,
    pub level: Option<String>,
    pub target: Option<String>,
    /// Current span context (from a `span` nested object), if present.
    pub span: Option<SpanInfo>,
    /// Unknown fields in original document order: `(key, value)`.
    pub extra_fields: Vec<(String, String)>,
    pub message: Option<String>,
}

/// Classify `fields` into known slots and extra fields, honouring hidden-field
/// rules. Fields in `hidden_names` or at a 0-based index in `hidden_indices`
/// are omitted entirely. When a known category already has a value, subsequent
/// fields with the same category key are silently dropped (not added to extras).
///
/// Special containers:
/// - `fields` (nested object) — tracing-subscriber format; its contents are
///   inlined: `message`-keyed sub-fields fill the message slot, others go to
///   `extra_fields`.
/// - `span` (nested object) — tracing span context; `name` becomes
///   `SpanInfo::name`, all other sub-fields become `SpanInfo::fields`.
/// - `spans` (array) — parent span stack; skipped (duplicates `span`).
pub fn classify_json_fields(
    fields: &[JsonField<'_>],
    hidden_names: &HashSet<String>,
    hidden_indices: &HashSet<usize>,
) -> JsonDisplayParts {
    let mut parts = JsonDisplayParts {
        timestamp: None,
        level: None,
        target: None,
        span: None,
        extra_fields: Vec::new(),
        message: None,
    };

    for (idx, field) in fields.iter().enumerate() {
        if hidden_indices.contains(&idx) || hidden_names.contains(field.key) {
            continue;
        }

        let key = field.key;
        let val = || field.value.to_string();

        if key == "fields" && !field.value_is_string {
            // tracing-subscriber JSON: "fields" holds message + event-level fields.
            if let Some(sub_fields) = parse_json_line(field.value.as_bytes()) {
                for sub in &sub_fields {
                    if MESSAGE_KEYS.contains(&sub.key) {
                        parts.message.get_or_insert_with(|| sub.value.to_string());
                    } else {
                        parts.extra_fields.push((sub.key.to_string(), sub.value.to_string()));
                    }
                }
            }
        } else if key == "span" && !field.value_is_string {
            // Span context: extract "name" as the span label, rest as span fields.
            if let Some(sub_fields) = parse_json_line(field.value.as_bytes()) {
                let mut span_name = String::new();
                let mut span_fields: Vec<(String, String)> = Vec::new();
                for sub in &sub_fields {
                    if sub.key == "name" {
                        span_name = sub.value.to_string();
                    } else {
                        span_fields.push((sub.key.to_string(), sub.value.to_string()));
                    }
                }
                parts.span = Some(SpanInfo { name: span_name, fields: span_fields });
            }
        } else if key == "spans" {
            // Parent span stack — skip; the current span is already in "span".
        } else if TIMESTAMP_KEYS.contains(&key) {
            parts.timestamp.get_or_insert_with(val);
        } else if LEVEL_KEYS.contains(&key) {
            parts.level.get_or_insert_with(val);
        } else if TARGET_KEYS.contains(&key) {
            parts.target.get_or_insert_with(val);
        } else if MESSAGE_KEYS.contains(&key) {
            parts.message.get_or_insert_with(val);
        } else {
            parts.extra_fields.push((key.to_string(), field.value.to_string()));
        }
    }

    parts
}

// ---------------------------------------------------------------------------
// Parser helpers (private)
// ---------------------------------------------------------------------------

/// Return the number of leading whitespace bytes at `pos`.
fn skip_ws(line: &[u8], mut pos: usize) -> usize {
    let start = pos;
    while pos < line.len() && matches!(line[pos], b' ' | b'\t' | b'\r' | b'\n') {
        pos += 1;
    }
    pos - start
}

/// Read a JSON string starting *after* the opening `"`.
/// Advances `*pos` past the closing `"` on success.
fn read_string<'a>(line: &'a [u8], pos: &mut usize) -> Option<&'a str> {
    let start = *pos;
    loop {
        if *pos >= line.len() {
            return None; // unclosed string
        }
        match line[*pos] {
            b'"' => {
                let s = std::str::from_utf8(&line[start..*pos]).ok()?;
                *pos += 1; // skip closing '"'
                return Some(s);
            }
            b'\\' => *pos += 2, // skip escape + next byte
            _ => *pos += 1,
        }
    }
}

/// Read a JSON value starting at `*pos`. Returns `(value_str, is_string)`.
fn read_value<'a>(line: &'a [u8], pos: &mut usize) -> Option<(&'a str, bool)> {
    if *pos >= line.len() {
        return None;
    }
    match line[*pos] {
        b'"' => {
            *pos += 1; // skip opening '"'
            let value = read_string(line, pos)?;
            Some((value, true))
        }
        b'{' | b'[' => {
            // Nested object or array — capture raw bytes, track depth.
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
                    // Skip string literal inside the nested structure.
                    *pos += 1;
                    while *pos < line.len() {
                        let sc = line[*pos];
                        *pos += 1;
                        if sc == b'"' {
                            break;
                        }
                        if sc == b'\\' {
                            *pos += 1; // skip escaped char
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
            // Number, boolean, null — runs until `,`, `}`, `]`, or whitespace.
            let start = *pos;
            while *pos < line.len()
                && !matches!(line[*pos], b',' | b'}' | b']' | b' ' | b'\t' | b'\r' | b'\n')
            {
                *pos += 1;
            }
            let value = std::str::from_utf8(&line[start..*pos]).ok()?.trim();
            Some((value, false))
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── Legacy LogLine parser ────────────────────────────────────────────────

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
        // An empty object {} has no fields → treated as non-JSON
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
        assert!(fields.iter().any(|f| f.key == "MESSAGE" && f.value == "Accepted password for user"));
        assert!(fields.iter().any(|f| f.key == "PRIORITY" && f.value == "6"));
        assert!(fields.iter().any(|f| f.key == "_HOSTNAME" && f.value == "myhost"));
    }

    #[test]
    fn test_parse_json_syslog_format() {
        let line = br#"{"time":"2024-01-15T10:00:00Z","level":"INFO","hostname":"myhost","app":"nginx","message":"GET /health 200"}"#;
        let fields = parse_json_line(line).unwrap();
        assert!(fields.iter().any(|f| f.key == "message" && f.value == "GET /health 200"));
        assert!(fields.iter().any(|f| f.key == "level" && f.value == "INFO"));
        assert!(fields.iter().any(|f| f.key == "hostname" && f.value == "myhost"));
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

    // ── detect_json_format ───────────────────────────────────────────────────

    #[test]
    fn test_detect_journalctl_format() {
        let line = br#"{"MESSAGE":"hello","PRIORITY":"6","_HOSTNAME":"host"}"#;
        let fields = parse_json_line(line).unwrap();
        assert_eq!(detect_json_format(&fields), LogFormat::JournalctlJson);
    }

    #[test]
    fn test_detect_syslog_json_via_message_key() {
        let line = br#"{"time":"2024","level":"INFO","message":"hello"}"#;
        let fields = parse_json_line(line).unwrap();
        assert_eq!(detect_json_format(&fields), LogFormat::SyslogJson);
    }

    #[test]
    fn test_detect_syslog_json_via_msg_key() {
        let line = br#"{"ts":"2024","level":"WARN","msg":"warn msg"}"#;
        let fields = parse_json_line(line).unwrap();
        assert_eq!(detect_json_format(&fields), LogFormat::SyslogJson);
    }

    #[test]
    fn test_detect_plain_json_format() {
        let line = br#"{"foo":"bar","baz":42}"#;
        let fields = parse_json_line(line).unwrap();
        assert_eq!(detect_json_format(&fields), LogFormat::Plain);
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
        hidden_idx.insert(0usize); // hide "level"
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
        // Values are NOT double-quoted
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

    // ── classify_json_fields ─────────────────────────────────────────────────

    #[test]
    fn test_classify_known_fields_extracted() {
        let line = br#"{"timestamp":"2024-01-01T00:00:00Z","level":"INFO","target":"myapp","message":"hello"}"#;
        let fields = parse_json_line(line).unwrap();
        let parts = classify_json_fields(&fields, &HashSet::new(), &HashSet::new());
        assert_eq!(parts.timestamp.as_deref(), Some("2024-01-01T00:00:00Z"));
        assert_eq!(parts.level.as_deref(), Some("INFO"));
        assert_eq!(parts.target.as_deref(), Some("myapp"));
        assert_eq!(parts.message.as_deref(), Some("hello"));
        assert!(parts.extra_fields.is_empty());
    }

    #[test]
    fn test_classify_unknown_fields_go_to_extra() {
        let line = br#"{"level":"WARN","request_id":"abc","msg":"hi"}"#;
        let fields = parse_json_line(line).unwrap();
        let parts = classify_json_fields(&fields, &HashSet::new(), &HashSet::new());
        assert_eq!(parts.level.as_deref(), Some("WARN"));
        assert_eq!(parts.message.as_deref(), Some("hi"));
        assert_eq!(parts.extra_fields.len(), 1);
        assert_eq!(parts.extra_fields[0], ("request_id".to_string(), "abc".to_string()));
    }

    #[test]
    fn test_classify_extra_fields_preserve_order() {
        let line = br#"{"msg":"hi","z_field":"z","a_field":"a","level":"INFO"}"#;
        let fields = parse_json_line(line).unwrap();
        let parts = classify_json_fields(&fields, &HashSet::new(), &HashSet::new());
        assert_eq!(parts.extra_fields[0].0, "z_field");
        assert_eq!(parts.extra_fields[1].0, "a_field");
    }

    #[test]
    fn test_classify_hidden_by_name_excluded() {
        let line = br#"{"level":"INFO","request_id":"abc","msg":"hi"}"#;
        let fields = parse_json_line(line).unwrap();
        let mut hidden = HashSet::new();
        hidden.insert("request_id".to_string());
        let parts = classify_json_fields(&fields, &hidden, &HashSet::new());
        assert!(parts.extra_fields.is_empty());
        assert_eq!(parts.message.as_deref(), Some("hi"));
    }

    #[test]
    fn test_classify_hidden_by_index_excluded() {
        let line = br#"{"level":"INFO","request_id":"abc","msg":"hi"}"#;
        let fields = parse_json_line(line).unwrap();
        let mut hidden_idx = HashSet::new();
        hidden_idx.insert(1usize); // "request_id"
        let parts = classify_json_fields(&fields, &HashSet::new(), &hidden_idx);
        assert!(parts.extra_fields.is_empty());
    }

    #[test]
    fn test_classify_journalctl_format() {
        let line = br#"{"__REALTIME_TIMESTAMP":"1699999999","PRIORITY":"6","SYSLOG_IDENTIFIER":"sshd","MESSAGE":"Accepted"}"#;
        let fields = parse_json_line(line).unwrap();
        let parts = classify_json_fields(&fields, &HashSet::new(), &HashSet::new());
        assert_eq!(parts.timestamp.as_deref(), Some("1699999999"));
        assert_eq!(parts.level.as_deref(), Some("6"));
        assert_eq!(parts.target.as_deref(), Some("sshd"));
        assert_eq!(parts.message.as_deref(), Some("Accepted"));
    }

    #[test]
    fn test_classify_duplicate_known_key_drops_second() {
        // "time" and "ts" both match timestamp — only the first fills the slot
        let line = br#"{"time":"t1","ts":"t2","msg":"hi"}"#;
        let fields = parse_json_line(line).unwrap();
        let parts = classify_json_fields(&fields, &HashSet::new(), &HashSet::new());
        assert_eq!(parts.timestamp.as_deref(), Some("t1"));
        assert!(parts.extra_fields.is_empty()); // "ts" is silently dropped
    }

    #[test]
    fn test_classify_all_unknown_fields_only() {
        let line = br#"{"foo":"bar","baz":42}"#;
        let fields = parse_json_line(line).unwrap();
        let parts = classify_json_fields(&fields, &HashSet::new(), &HashSet::new());
        assert!(parts.timestamp.is_none());
        assert!(parts.level.is_none());
        assert!(parts.target.is_none());
        assert!(parts.message.is_none());
        assert_eq!(parts.extra_fields.len(), 2);
    }

    #[test]
    fn test_classify_fields_container_extracts_message() {
        // tracing-subscriber JSON format: message inside "fields" object
        let line = br#"{"level":"INFO","target":"todo_app","fields":{"message":"Listening on 0.0.0.0:3000"}}"#;
        let fields = parse_json_line(line).unwrap();
        let parts = classify_json_fields(&fields, &HashSet::new(), &HashSet::new());
        assert_eq!(parts.message.as_deref(), Some("Listening on 0.0.0.0:3000"));
        assert!(parts.extra_fields.is_empty());
    }

    #[test]
    fn test_classify_fields_container_extracts_extras_too() {
        let line = br#"{"level":"INFO","fields":{"message":"todos listed","count":9}}"#;
        let fields = parse_json_line(line).unwrap();
        let parts = classify_json_fields(&fields, &HashSet::new(), &HashSet::new());
        assert_eq!(parts.message.as_deref(), Some("todos listed"));
        assert_eq!(parts.extra_fields.len(), 1);
        assert_eq!(parts.extra_fields[0], ("count".to_string(), "9".to_string()));
    }

    #[test]
    fn test_classify_span_extracts_name_and_fields() {
        let line = br#"{"level":"INFO","span":{"name":"request","method":"GET","uri":"/todos"},"fields":{"message":"ok"}}"#;
        let fields = parse_json_line(line).unwrap();
        let parts = classify_json_fields(&fields, &HashSet::new(), &HashSet::new());
        let span = parts.span.unwrap();
        assert_eq!(span.name, "request");
        assert_eq!(span.fields.len(), 2);
        assert!(span.fields.iter().any(|(k, v)| k == "method" && v == "GET"));
        assert!(span.fields.iter().any(|(k, v)| k == "uri" && v == "/todos"));
    }

    #[test]
    fn test_classify_spans_array_is_skipped() {
        let line = br#"{"level":"INFO","spans":[{"name":"root"}],"msg":"hi"}"#;
        let fields = parse_json_line(line).unwrap();
        let parts = classify_json_fields(&fields, &HashSet::new(), &HashSet::new());
        // "spans" key must not appear in extra_fields
        assert!(parts.extra_fields.is_empty());
        assert!(parts.span.is_none());
    }
}
