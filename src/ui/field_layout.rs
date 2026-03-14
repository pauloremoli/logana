use std::collections::HashSet;

use unicode_width::UnicodeWidthChar;

use crate::parser::{DisplayParts, LogFormatParser, SpanInfo, format_span_col};
use crate::types::FieldLayout;

/// Number of terminal rows a line occupies when word-wrapped to `inner_width` columns.
///
/// Simulates ratatui `Wrap { trim: false }` behavior:
/// - Words that fit on the current row are placed there.
/// - Words that don't fit are moved to the next row.
/// - Words wider than `inner_width` are split at character boundaries across rows.
///
/// Returns 1 when `inner_width` is 0 or the line is empty.
pub(crate) fn line_row_count(bytes: &[u8], inner_width: usize) -> usize {
    if inner_width == 0 {
        return 1;
    }
    let text = std::str::from_utf8(bytes).unwrap_or("");
    if text.is_empty() {
        return 1;
    }

    let mut rows = 1usize;
    let mut col = 0usize; // current column (unicode width)
    let mut word_w = 0usize; // accumulated width of the current non-whitespace word

    for ch in text.chars() {
        if ch.is_ascii_whitespace() {
            // Commit the pending word before placing the whitespace character.
            if word_w > 0 {
                if col > 0 && col + word_w > inner_width {
                    // Word doesn't fit on current row: move to next.
                    rows += 1;
                    col = 0;
                }
                if col == 0 && word_w > inner_width {
                    // Word is wider than a full row: split at character boundaries.
                    rows += word_w.div_ceil(inner_width) - 1;
                    col = word_w % inner_width;
                } else {
                    col += word_w;
                }
                word_w = 0;
            }
            let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
            if col + cw > inner_width {
                rows += 1;
                col = cw; // trim: false — keep the space on the new row
            } else {
                col += cw;
            }
        } else {
            word_w += UnicodeWidthChar::width(ch).unwrap_or(0);
        }
    }

    // Commit any remaining word.
    if word_w > 0 {
        if col > 0 && col + word_w > inner_width {
            rows += 1;
            col = 0;
        }
        if col == 0 && word_w > inner_width {
            rows += word_w.div_ceil(inner_width) - 1;
        }
    }

    rows
}

/// Simulate word-wrap of `text` into a box of `width` columns and return the
/// number of lines that result. Used to size the status bar dynamically.
pub(crate) fn count_wrapped_lines(text: &str, width: usize) -> usize {
    if width == 0 {
        return 1;
    }
    let mut lines = 1usize;
    let mut col = 0usize;
    for word in text.split_whitespace() {
        let wl = word.len();
        if col == 0 {
            col = wl;
        } else if col + 1 + wl > width {
            lines += 1;
            col = wl;
        } else {
            col += 1 + wl;
        }
    }
    lines
}

/// Row count for a line, using the structured rendering width when a parser is
/// available. In wrap mode with structured log formats (e.g. JSON tracing logs),
/// raw bytes can be much longer than the rendered output, causing `line_row_count`
/// on raw bytes to underestimate how many lines fit in the viewport. This function
/// uses the actual rendered-column text width instead.
pub(crate) fn effective_row_count(
    line_bytes: &[u8],
    inner_width: usize,
    parser: Option<&dyn LogFormatParser>,
    layout: &FieldLayout,
    hidden_fields: &HashSet<String>,
    show_keys: bool,
) -> usize {
    if let Some(p) = parser
        && let Some(parts) = p.parse_line(line_bytes)
    {
        let cols = apply_field_layout(&parts, layout, hidden_fields, show_keys);
        if !cols.is_empty() {
            let rendered = cols.join(" ");
            return line_row_count(rendered.as_bytes(), inner_width);
        }
    }
    line_row_count(line_bytes, inner_width)
}

// ---------------------------------------------------------------------------
// Structured field layout helpers
// ---------------------------------------------------------------------------

pub(crate) fn get_col(p: &DisplayParts<'_>, name: &str, show_keys: bool) -> Option<String> {
    match name {
        "span" => p.span.as_ref().map(|s| format_span_col(s, show_keys)),
        n => {
            // Resolve dotted span sub-field names (e.g. "span.name", "span.method").
            if let Some(suffix) = n.strip_prefix("span.") {
                return p.span.as_ref().and_then(|s| {
                    if suffix == "name" {
                        Some(s.name.to_string())
                    } else {
                        s.fields
                            .iter()
                            .find(|(k, _)| *k == suffix)
                            .map(|(_, v)| v.to_string())
                    }
                });
            }
            // Resolve dotted fields sub-field names (e.g. "fields.message", "fields.count").
            if let Some(suffix) = n.strip_prefix("fields.") {
                return if crate::parser::MESSAGE_KEYS.contains(&suffix) {
                    p.message.map(|s| s.to_string())
                } else {
                    p.extra_fields
                        .iter()
                        .find(|(k, _)| *k == suffix)
                        .map(|(_, v)| v.to_string())
                };
            }
            // Resolve all known aliases to their canonical DisplayParts slots.
            if crate::parser::TIMESTAMP_KEYS.contains(&n) {
                return p.timestamp.map(|s| s.to_string());
            }
            if crate::parser::LEVEL_KEYS.contains(&n) {
                return p.level.map(|l| format!("{:<5}", l));
            }
            if crate::parser::TARGET_KEYS.contains(&n) {
                return p.target.map(|s| s.to_string());
            }
            if crate::parser::MESSAGE_KEYS.contains(&n) {
                return p.message.map(|s| s.to_string());
            }
            p.extra_fields
                .iter()
                .find(|(k, _)| *k == n)
                .map(|(_, v)| v.to_string())
        }
    }
}

#[cfg(test)]
fn default_cols(p: &DisplayParts<'_>, show_keys: bool) -> Vec<String> {
    let mut cols = Vec::new();
    if let Some(ts) = p.timestamp {
        cols.push(ts.to_string());
    }
    if let Some(lvl) = p.level {
        cols.push(format!("{:<5}", lvl));
    }
    if let Some(tgt) = p.target {
        cols.push(tgt.to_string());
    }
    if let Some(span) = &p.span {
        cols.push(format_span_col(span, show_keys));
    }
    for (key, value) in &p.extra_fields {
        if show_keys {
            cols.push(format!("{key}={value}"));
        } else {
            cols.push(value.to_string());
        }
    }
    if let Some(msg) = p.message {
        cols.push(msg.to_string());
    }
    cols
}

/// Render a span, filtering out sub-fields whose keys are in `excluded_keys`.
fn render_span(s: &SpanInfo<'_>, excluded_keys: &HashSet<&str>, show_keys: bool) -> String {
    if excluded_keys.is_empty() {
        return format_span_col(s, show_keys);
    }
    let visible_fields: Vec<(&str, &str)> = s
        .fields
        .iter()
        .filter(|(k, _)| !excluded_keys.contains(k))
        .copied()
        .collect();
    let filtered = SpanInfo {
        name: s.name,
        fields: visible_fields,
    };
    format_span_col(&filtered, show_keys)
}

pub(crate) fn apply_field_layout(
    p: &DisplayParts<'_>,
    layout: &FieldLayout,
    hidden_fields: &HashSet<String>,
    show_keys: bool,
) -> Vec<String> {
    let excluded_keys: HashSet<&str> = hidden_fields
        .iter()
        .filter_map(|h| h.strip_prefix("span."))
        .collect();

    if let Some(names) = &layout.columns {
        // Explicit layout — filter hidden column names and span sub-fields.
        names
            .iter()
            .filter(|name| !hidden_fields.contains(name.as_str()))
            .filter_map(|name| {
                if name == "span" {
                    p.span
                        .as_ref()
                        .map(|s| render_span(s, &excluded_keys, show_keys))
                } else {
                    get_col(p, name, show_keys)
                }
            })
            .collect()
    } else {
        // Default layout — rebuild without hidden fields.
        // Check all aliases for each canonical slot so that hiding by raw key
        // (e.g. "lvl") works in the default (no explicit layout) path too.
        let mut cols = Vec::new();
        if !crate::parser::TIMESTAMP_KEYS
            .iter()
            .any(|k| hidden_fields.contains(*k))
            && let Some(ts) = p.timestamp
        {
            cols.push(ts.to_string());
        }
        if !crate::parser::LEVEL_KEYS
            .iter()
            .any(|k| hidden_fields.contains(*k))
            && let Some(lvl) = p.level
        {
            cols.push(format!("{:<5}", lvl));
        }
        if !crate::parser::TARGET_KEYS
            .iter()
            .any(|k| hidden_fields.contains(*k))
            && let Some(tgt) = p.target
        {
            cols.push(tgt.to_string());
        }
        if !hidden_fields.contains("span")
            && let Some(span) = &p.span
        {
            cols.push(render_span(span, &excluded_keys, show_keys));
        }
        for (key, value) in &p.extra_fields {
            if !hidden_fields.contains(*key) {
                if show_keys {
                    cols.push(format!("{key}={value}"));
                } else {
                    cols.push(value.to_string());
                }
            }
        }
        if !crate::parser::MESSAGE_KEYS
            .iter()
            .any(|k| hidden_fields.contains(*k))
            && let Some(msg) = p.message
        {
            cols.push(msg.to_string());
        }
        cols
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{JsonParser, SpanInfo};

    // -----------------------------------------------------------------------
    // line_row_count
    // -----------------------------------------------------------------------

    #[test]
    fn test_line_row_count_zero_width() {
        assert_eq!(line_row_count(b"hello", 0), 1);
    }

    #[test]
    fn test_line_row_count_empty_line() {
        assert_eq!(line_row_count(b"", 80), 1);
    }

    #[test]
    fn test_line_row_count_fits_in_one_row() {
        assert_eq!(line_row_count(b"hello", 80), 1);
    }

    #[test]
    fn test_line_row_count_wraps_to_two_rows() {
        // Single word, 10 chars in width 6 → ceil(10/6) = 2 (same as char-wrap for single words)
        assert_eq!(line_row_count(b"0123456789", 6), 2);
    }

    #[test]
    fn test_line_row_count_word_wrap_exceeds_char_wrap() {
        // "hello world test abc" (20 chars) in width 7:
        //   char-wrap: ceil(20/7) = 3 rows
        //   word-wrap: "hello"(5) → col 5; space → col 6;
        //              "world"(5) doesn't fit → row 2; space → col 6;
        //              "test"(4) doesn't fit → row 3; space → col 5;
        //              "abc"(3) doesn't fit → row 4.
        assert_eq!(line_row_count(b"hello world test abc", 7), 4);
    }

    #[test]
    fn test_line_row_count_long_word_spans_many_rows() {
        // Single word of 15 chars in width 5 → ceil(15/5) = 3 rows
        assert_eq!(line_row_count(b"aaaaaaaaaaaaaaa", 5), 3);
    }

    #[test]
    fn test_line_row_count_long_word_plus_short_word() {
        // "aaaaaaaaaa b" — long word (10 chars) in width 7, then " b"
        // Long word: col=0, 10>7 → rows += ceil(10/7)-1=1 → rows=2, col=10%7=3
        // space: col+1=4 ≤ 7 → col=4
        // "b" (1 char): col+1=5 ≤ 7 → col=5
        // Result: 2 rows
        assert_eq!(line_row_count(b"aaaaaaaaaa b", 7), 2);
    }

    #[test]
    fn test_line_row_count_exact_width() {
        assert_eq!(line_row_count(b"12345", 5), 1);
    }

    // -----------------------------------------------------------------------
    // count_wrapped_lines
    // -----------------------------------------------------------------------

    #[test]
    fn test_count_wrapped_lines_empty() {
        assert_eq!(count_wrapped_lines("", 80), 1);
    }

    #[test]
    fn test_count_wrapped_lines_zero_width() {
        assert_eq!(count_wrapped_lines("hello world", 0), 1);
    }

    #[test]
    fn test_count_wrapped_lines_single_word() {
        assert_eq!(count_wrapped_lines("hello", 80), 1);
    }

    #[test]
    fn test_count_wrapped_lines_wraps() {
        // "hello world" with width 6 → "hello" (5) then "world" (5) doesn't fit on same line
        assert!(count_wrapped_lines("hello world", 6) >= 2);
    }

    #[test]
    fn test_count_wrapped_lines_exact_fit() {
        // "ab cd" = 5 chars content, width 5 → fits in 1 line
        assert_eq!(count_wrapped_lines("ab cd", 5), 1);
    }

    // -----------------------------------------------------------------------
    // get_col
    // -----------------------------------------------------------------------

    fn make_parts<'a>() -> DisplayParts<'a> {
        DisplayParts {
            timestamp: Some("2024-01-01T00:00:00Z"),
            level: Some("INFO"),
            target: Some("myapp"),
            span: Some(SpanInfo {
                name: "handler",
                fields: vec![("method", "GET")],
            }),
            extra_fields: vec![("count", "42")],
            message: Some("hello world"),
        }
    }

    #[test]
    fn test_get_col_timestamp() {
        let p = make_parts();
        assert_eq!(
            get_col(&p, "timestamp", false),
            Some("2024-01-01T00:00:00Z".to_string())
        );
    }

    #[test]
    fn test_get_col_level() {
        let p = make_parts();
        let result = get_col(&p, "level", false).unwrap();
        assert!(result.starts_with("INFO"));
    }

    #[test]
    fn test_get_col_message() {
        let p = make_parts();
        assert_eq!(
            get_col(&p, "message", false),
            Some("hello world".to_string())
        );
    }

    #[test]
    fn test_get_col_span_name() {
        let p = make_parts();
        assert_eq!(get_col(&p, "span.name", false), Some("handler".to_string()));
    }

    #[test]
    fn test_get_col_dotted_span_field() {
        let p = make_parts();
        assert_eq!(get_col(&p, "span.method", false), Some("GET".to_string()));
    }

    #[test]
    fn test_get_col_dotted_fields_field() {
        let p = make_parts();
        // "fields.message" should resolve to the message slot
        assert_eq!(
            get_col(&p, "fields.message", false),
            Some("hello world".to_string())
        );
    }

    #[test]
    fn test_get_col_extra_field() {
        let p = make_parts();
        assert_eq!(get_col(&p, "count", false), Some("42".to_string()));
    }

    #[test]
    fn test_get_col_unknown_returns_none() {
        let p = make_parts();
        assert_eq!(get_col(&p, "nonexistent", false), None);
    }

    #[test]
    fn test_get_col_alias_resolution() {
        let p = make_parts();
        // "lvl" is an alias for level
        assert!(get_col(&p, "lvl", false).is_some());
        // "msg" is an alias for message
        assert!(get_col(&p, "msg", false).is_some());
        // "ts" is an alias for timestamp
        assert!(get_col(&p, "ts", false).is_some());
    }

    #[test]
    fn test_get_col_span_show_keys() {
        let p = make_parts();
        // show_keys=false: values only
        assert_eq!(get_col(&p, "span", false), Some("handler: GET".to_string())); // single value, no separator difference
        // show_keys=true: key=value pairs
        assert_eq!(
            get_col(&p, "span", true),
            Some("handler: method=GET".to_string())
        );
    }

    // -----------------------------------------------------------------------
    // default_cols
    // -----------------------------------------------------------------------

    #[test]
    fn test_default_cols_all_fields() {
        let p = make_parts();
        let cols = default_cols(&p, false);
        // Should have: timestamp, level, target, span, extra(count), message = 6
        assert_eq!(cols.len(), 6);
        assert!(cols[0].contains("2024"));
        assert!(cols[1].starts_with("INFO"));
        assert_eq!(cols[2], "myapp");
        assert!(cols[5].contains("hello world"));
    }

    #[test]
    fn test_default_cols_minimal() {
        let p = DisplayParts {
            timestamp: None,
            level: None,
            target: None,
            span: None,
            extra_fields: vec![],
            message: Some("only message"),
        };
        let cols = default_cols(&p, false);
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0], "only message");
    }

    // -----------------------------------------------------------------------
    // apply_field_layout
    // -----------------------------------------------------------------------

    #[test]
    fn test_apply_field_layout_default_no_hidden() {
        let p = make_parts();
        let layout = FieldLayout::default();
        let hidden = HashSet::new();
        let cols = apply_field_layout(&p, &layout, &hidden, false);
        assert_eq!(cols.len(), 6);
    }

    #[test]
    fn test_apply_field_layout_explicit_columns() {
        let p = make_parts();
        let layout = FieldLayout {
            columns: Some(vec!["level".to_string(), "message".to_string()]),
        };
        let hidden = HashSet::new();
        let cols = apply_field_layout(&p, &layout, &hidden, false);
        assert_eq!(cols.len(), 2);
    }

    #[test]
    fn test_apply_field_layout_hidden_fields_default() {
        let p = make_parts();
        let layout = FieldLayout::default();
        let mut hidden = HashSet::new();
        hidden.insert("timestamp".to_string());
        let cols = apply_field_layout(&p, &layout, &hidden, false);
        // Should have 5 (all minus timestamp)
        assert_eq!(cols.len(), 5);
    }

    #[test]
    fn test_apply_field_layout_hidden_fields_explicit() {
        let p = make_parts();
        let layout = FieldLayout {
            columns: Some(vec![
                "timestamp".to_string(),
                "level".to_string(),
                "message".to_string(),
            ]),
        };
        let mut hidden = HashSet::new();
        hidden.insert("timestamp".to_string());
        let cols = apply_field_layout(&p, &layout, &hidden, false);
        assert_eq!(cols.len(), 2); // level + message
    }

    // -----------------------------------------------------------------------
    // effective_row_count
    // -----------------------------------------------------------------------

    #[test]
    fn test_effective_row_count_no_parser_uses_raw_bytes() {
        let hidden = HashSet::new();
        let layout = FieldLayout::default();
        assert_eq!(
            effective_row_count(b"hello world", 80, None, &layout, &hidden, false),
            1
        );
        // ceil(11/5) = 3
        assert_eq!(
            effective_row_count(b"hello world", 5, None, &layout, &hidden, false),
            3
        );
    }

    #[test]
    fn test_effective_row_count_with_parser_uses_rendered_width() {
        // A JSON line that is long in raw bytes but short when structured-rendered.
        let json = br#"{"timestamp":"2024-01-01T00:00:00Z","level":"INFO","target":"app","fields":{"message":"ok"}}"#;
        let parser = JsonParser;
        let layout = FieldLayout::default();
        let hidden = HashSet::new();
        // Raw bytes are ~94 chars; at width=20 that's 5 rows.
        assert_eq!(line_row_count(json, 20), 5);
        // Structured render is much shorter; effective_row_count should be < 5.
        let result = effective_row_count(json, 20, Some(&parser), &layout, &hidden, false);
        assert!(
            result < 5,
            "structured rendering should produce fewer rows than raw bytes"
        );
    }

    #[test]
    fn test_effective_row_count_parse_failure_falls_back_to_raw() {
        let parser = JsonParser;
        let layout = FieldLayout::default();
        let hidden = HashSet::new();
        // Non-JSON input: parse returns None → falls back to raw byte width.
        let raw = b"plain text log line that is not json";
        assert_eq!(
            effective_row_count(raw, 20, Some(&parser), &layout, &hidden, false),
            line_row_count(raw, 20)
        );
    }

    #[test]
    fn test_effective_row_count_all_hidden_falls_back_to_raw() {
        let parser = JsonParser;
        let layout = FieldLayout::default();
        // Hide every known field so cols is empty → falls back to raw.
        let mut hidden = HashSet::new();
        for key in ["timestamp", "level", "target", "message"] {
            hidden.insert(key.to_string());
        }
        let json = br#"{"timestamp":"2024-01-01T00:00:00Z","level":"INFO","target":"app","fields":{"message":"ok"}}"#;
        let raw_rows = line_row_count(json, 20);
        assert_eq!(
            effective_row_count(json, 20, Some(&parser), &layout, &hidden, false),
            raw_rows
        );
    }

    #[test]
    fn test_hiding_span_subfield_filters_it_from_default_layout() {
        let p = DisplayParts {
            timestamp: Some("2024-01-01T00:00:00Z"),
            level: Some("INFO"),
            target: Some("app"),
            span: Some(SpanInfo {
                name: "request",
                fields: vec![("request_id", "abc-123"), ("method", "GET")],
            }),
            extra_fields: vec![],
            message: Some("hello"),
        };
        let layout = FieldLayout::default();
        let mut hidden = HashSet::new();
        hidden.insert("span.request_id".to_string());
        let cols = apply_field_layout(&p, &layout, &hidden, true);
        let span_col = cols.iter().find(|c| c.contains("request")).unwrap();
        assert!(
            !span_col.contains("request_id"),
            "hidden span sub-field should not appear: {span_col}"
        );
        assert!(
            span_col.contains("method"),
            "non-hidden span sub-field should still appear: {span_col}"
        );
    }

    #[test]
    fn test_hiding_span_subfield_via_hidden_fields_explicit_layout() {
        let p = DisplayParts {
            timestamp: None,
            level: None,
            target: None,
            span: Some(SpanInfo {
                name: "request",
                fields: vec![("request_id", "abc-123"), ("method", "GET")],
            }),
            extra_fields: vec![],
            message: None,
        };
        let layout = FieldLayout {
            columns: Some(vec!["span".to_string()]),
        };
        let mut hidden = HashSet::new();
        hidden.insert("span.request_id".to_string());
        let cols = apply_field_layout(&p, &layout, &hidden, true);
        assert_eq!(cols.len(), 1);
        assert!(
            !cols[0].contains("request_id"),
            "hidden span sub-field should not appear in explicit layout: {}",
            cols[0]
        );
        assert!(cols[0].contains("method"));
    }

    #[test]
    fn test_hiding_span_subfield_via_select_fields() {
        // Simulates the select-fields path: field_layout.columns has all fields
        // ordered, and span.request_id is disabled via hidden_fields.
        let p = DisplayParts {
            timestamp: None,
            level: None,
            target: None,
            span: Some(SpanInfo {
                name: "request",
                fields: vec![("request_id", "abc-123"), ("method", "GET")],
            }),
            extra_fields: vec![],
            message: None,
        };
        let layout = FieldLayout {
            columns: Some(vec![
                "span".to_string(),
                "span.request_id".to_string(),
                "span.method".to_string(),
            ]),
        };
        let mut hidden = HashSet::new();
        hidden.insert("span.request_id".to_string());
        let cols = apply_field_layout(&p, &layout, &hidden, true);
        // "span" column should render without request_id (it is in hidden_fields)
        let span_col = cols.iter().find(|c| c.contains("request")).unwrap();
        assert!(
            !span_col.contains("request_id"),
            "disabled span sub-field should be filtered: {span_col}"
        );
        assert!(span_col.contains("method"));
    }

    #[test]
    fn test_hiding_all_span_subfields_leaves_span_name() {
        let p = DisplayParts {
            timestamp: None,
            level: None,
            target: None,
            span: Some(SpanInfo {
                name: "request",
                fields: vec![("request_id", "abc-123")],
            }),
            extra_fields: vec![],
            message: None,
        };
        let layout = FieldLayout::default();
        let mut hidden = HashSet::new();
        hidden.insert("span.request_id".to_string());
        let cols = apply_field_layout(&p, &layout, &hidden, false);
        assert!(
            cols.iter().any(|c| c == "request"),
            "span name should remain when all sub-fields are hidden"
        );
    }

    #[test]
    fn test_apply_field_layout_hidden_alias() {
        let p = make_parts();
        let layout = FieldLayout::default();
        let mut hidden = HashSet::new();
        // "lvl" is an alias for level — hiding it should hide the level column
        hidden.insert("lvl".to_string());
        let cols = apply_field_layout(&p, &layout, &hidden, false);
        // Should have 5 (all minus level)
        assert_eq!(cols.len(), 5);
    }
}
