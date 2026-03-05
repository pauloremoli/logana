//! Structured field layout helpers for log line rendering.
//!
//! [`apply_field_layout`] converts a [`DisplayParts`] and a [`FieldLayout`]
//! into an ordered `Vec<String>` of column values. [`line_row_count`] computes
//! wrap-aware terminal row height for a line. [`default_cols`] and [`get_col`]
//! handle column name resolution including all canonical key aliases.

use std::collections::HashSet;

use unicode_width::UnicodeWidthStr;

use crate::parser::{DisplayParts, LogFormatParser, format_span_col};
use crate::types::FieldLayout;

/// Number of terminal rows a line occupies when wrapped to `inner_width` columns.
/// Returns 1 when `inner_width` is 0 or the line is empty.
pub(crate) fn line_row_count(bytes: &[u8], inner_width: usize) -> usize {
    if inner_width == 0 {
        return 1;
    }
    let w = UnicodeWidthStr::width(std::str::from_utf8(bytes).unwrap_or(""));
    if w == 0 { 1 } else { w.div_ceil(inner_width) }
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
) -> usize {
    if let Some(p) = parser
        && let Some(parts) = p.parse_line(line_bytes)
    {
        let cols = apply_field_layout(&parts, layout, hidden_fields);
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

pub(crate) fn get_col(p: &DisplayParts<'_>, name: &str) -> Option<String> {
    match name {
        "span" => p.span.as_ref().map(format_span_col),
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

fn default_cols(p: &DisplayParts<'_>) -> Vec<String> {
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
        cols.push(format_span_col(span));
    }
    for (_key, value) in &p.extra_fields {
        cols.push(value.to_string());
    }
    if let Some(msg) = p.message {
        cols.push(msg.to_string());
    }
    cols
}

pub(crate) fn apply_field_layout(
    p: &DisplayParts<'_>,
    layout: &FieldLayout,
    hidden_fields: &HashSet<String>,
) -> Vec<String> {
    let cols = match &layout.columns {
        None => default_cols(p),
        Some(names) => names.iter().filter_map(|name| get_col(p, name)).collect(),
    };
    if hidden_fields.is_empty() {
        cols
    } else if let Some(names) = &layout.columns {
        // Explicit layout — re-filter, excluding hidden names.
        names
            .iter()
            .filter(|name| !hidden_fields.contains(name.as_str()))
            .filter_map(|name| get_col(p, name))
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
            cols.push(format_span_col(span));
        }
        for (key, value) in &p.extra_fields {
            if !hidden_fields.contains(*key) {
                cols.push(value.to_string());
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
        // 10 chars in width 6 → ceil(10/6) = 2
        assert_eq!(line_row_count(b"0123456789", 6), 2);
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
            get_col(&p, "timestamp"),
            Some("2024-01-01T00:00:00Z".to_string())
        );
    }

    #[test]
    fn test_get_col_level() {
        let p = make_parts();
        let result = get_col(&p, "level").unwrap();
        assert!(result.starts_with("INFO"));
    }

    #[test]
    fn test_get_col_message() {
        let p = make_parts();
        assert_eq!(get_col(&p, "message"), Some("hello world".to_string()));
    }

    #[test]
    fn test_get_col_span_name() {
        let p = make_parts();
        assert_eq!(get_col(&p, "span.name"), Some("handler".to_string()));
    }

    #[test]
    fn test_get_col_dotted_span_field() {
        let p = make_parts();
        assert_eq!(get_col(&p, "span.method"), Some("GET".to_string()));
    }

    #[test]
    fn test_get_col_dotted_fields_field() {
        let p = make_parts();
        // "fields.message" should resolve to the message slot
        assert_eq!(
            get_col(&p, "fields.message"),
            Some("hello world".to_string())
        );
    }

    #[test]
    fn test_get_col_extra_field() {
        let p = make_parts();
        assert_eq!(get_col(&p, "count"), Some("42".to_string()));
    }

    #[test]
    fn test_get_col_unknown_returns_none() {
        let p = make_parts();
        assert_eq!(get_col(&p, "nonexistent"), None);
    }

    #[test]
    fn test_get_col_alias_resolution() {
        let p = make_parts();
        // "lvl" is an alias for level
        assert!(get_col(&p, "lvl").is_some());
        // "msg" is an alias for message
        assert!(get_col(&p, "msg").is_some());
        // "ts" is an alias for timestamp
        assert!(get_col(&p, "ts").is_some());
    }

    // -----------------------------------------------------------------------
    // default_cols
    // -----------------------------------------------------------------------

    #[test]
    fn test_default_cols_all_fields() {
        let p = make_parts();
        let cols = default_cols(&p);
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
        let cols = default_cols(&p);
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
        let cols = apply_field_layout(&p, &layout, &hidden);
        assert_eq!(cols.len(), 6);
    }

    #[test]
    fn test_apply_field_layout_explicit_columns() {
        let p = make_parts();
        let layout = FieldLayout {
            columns: Some(vec!["level".to_string(), "message".to_string()]),
            columns_order: None,
        };
        let hidden = HashSet::new();
        let cols = apply_field_layout(&p, &layout, &hidden);
        assert_eq!(cols.len(), 2);
    }

    #[test]
    fn test_apply_field_layout_hidden_fields_default() {
        let p = make_parts();
        let layout = FieldLayout::default();
        let mut hidden = HashSet::new();
        hidden.insert("timestamp".to_string());
        let cols = apply_field_layout(&p, &layout, &hidden);
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
            columns_order: None,
        };
        let mut hidden = HashSet::new();
        hidden.insert("timestamp".to_string());
        let cols = apply_field_layout(&p, &layout, &hidden);
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
            effective_row_count(b"hello world", 80, None, &layout, &hidden),
            1
        );
        // ceil(11/5) = 3
        assert_eq!(
            effective_row_count(b"hello world", 5, None, &layout, &hidden),
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
        let result = effective_row_count(json, 20, Some(&parser), &layout, &hidden);
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
            effective_row_count(raw, 20, Some(&parser), &layout, &hidden),
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
            effective_row_count(json, 20, Some(&parser), &layout, &hidden),
            raw_rows
        );
    }

    #[test]
    fn test_apply_field_layout_hidden_alias() {
        let p = make_parts();
        let layout = FieldLayout::default();
        let mut hidden = HashSet::new();
        // "lvl" is an alias for level — hiding it should hide the level column
        hidden.insert("lvl".to_string());
        let cols = apply_field_layout(&p, &layout, &hidden);
        // Should have 5 (all minus level)
        assert_eq!(cols.len(), 5);
    }
}
