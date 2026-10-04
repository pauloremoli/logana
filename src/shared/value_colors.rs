use regex::Regex;
use std::sync::LazyLock;

use crate::filters::StyleId;
use crate::theme::ValueColors;

pub const VALUE_STYLE_HTTP_GET: StyleId = 242;
pub const VALUE_STYLE_HTTP_POST: StyleId = 243;
pub const VALUE_STYLE_HTTP_PUT: StyleId = 244;
pub const VALUE_STYLE_HTTP_DELETE: StyleId = 245;
pub const VALUE_STYLE_HTTP_PATCH: StyleId = 246;
pub const VALUE_STYLE_HTTP_OTHER: StyleId = 247;
pub const VALUE_STYLE_STATUS_2XX: StyleId = 248;
pub const VALUE_STYLE_STATUS_3XX: StyleId = 249;
pub const VALUE_STYLE_STATUS_4XX: StyleId = 250;
pub const VALUE_STYLE_STATUS_5XX: StyleId = 251;
pub const VALUE_STYLE_IP: StyleId = 252;
pub const VALUE_STYLE_UUID: StyleId = 253;

static HTTP_METHOD_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(GET|POST|PUT|DELETE|PATCH|HEAD|OPTIONS)\b").unwrap());

static STATUS_CODE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b([1-5]\d{2})\b").unwrap());

static IPV4_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3})\b").unwrap());

static UUID_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b").unwrap()
});

static IPV6_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:[0-9a-f]{1,4}:){7}[0-9a-f]{1,4}\b|(?i)\b(?:[0-9a-f]{1,4}:){1,7}:\b|(?i)\b(?:[0-9a-f]{1,4}:){1,6}:[0-9a-f]{1,4}\b|(?i)\b(?:[0-9a-f]{1,4}:){1,5}(?::[0-9a-f]{1,4}){1,2}\b|(?i)\b(?:[0-9a-f]{1,4}:){1,4}(?::[0-9a-f]{1,4}){1,3}\b|(?i)\b(?:[0-9a-f]{1,4}:){1,3}(?::[0-9a-f]{1,4}){1,4}\b|(?i)\b(?:[0-9a-f]{1,4}:){1,2}(?::[0-9a-f]{1,4}){1,5}\b|(?i)\b[0-9a-f]{1,4}:(?::[0-9a-f]{1,4}){1,6}\b|(?i)::(?:[0-9a-f]{1,4}:){0,5}[0-9a-f]{1,4}\b|(?i)\b(?:[0-9a-f]{1,4}:){1,5}:(?:\d{1,3}\.){3}\d{1,3}\b|::1\b",
    )
    .unwrap()
});

fn http_method_style_id(method: &str) -> StyleId {
    match method {
        "GET" => VALUE_STYLE_HTTP_GET,
        "POST" => VALUE_STYLE_HTTP_POST,
        "PUT" => VALUE_STYLE_HTTP_PUT,
        "DELETE" => VALUE_STYLE_HTTP_DELETE,
        "PATCH" => VALUE_STYLE_HTTP_PATCH,
        _ => VALUE_STYLE_HTTP_OTHER,
    }
}

fn http_method_key(method: &str) -> &'static str {
    match method {
        "GET" => "http_get",
        "POST" => "http_post",
        "PUT" => "http_put",
        "DELETE" => "http_delete",
        "PATCH" => "http_patch",
        _ => "http_other",
    }
}

fn status_code_style_id(code: &str) -> Option<StyleId> {
    match code.as_bytes()[0] {
        b'2' => Some(VALUE_STYLE_STATUS_2XX),
        b'3' => Some(VALUE_STYLE_STATUS_3XX),
        b'4' => Some(VALUE_STYLE_STATUS_4XX),
        b'5' => Some(VALUE_STYLE_STATUS_5XX),
        _ => None,
    }
}

fn status_code_key(code: &str) -> &'static str {
    match code.as_bytes()[0] {
        b'2' => "status_2xx",
        b'3' => "status_3xx",
        b'4' => "status_4xx",
        b'5' => "status_5xx",
        _ => "status_1xx",
    }
}

pub fn collect_value_color_spans(text: &str, colors: &ValueColors) -> Vec<(usize, usize, StyleId)> {
    let mut matches: Vec<(usize, usize, StyleId)> = Vec::new();

    for m in HTTP_METHOD_RE.find_iter(text) {
        let key = http_method_key(m.as_str());
        if !colors.is_disabled(key) {
            matches.push((m.start(), m.end(), http_method_style_id(m.as_str())));
        }
    }

    let text_bytes = text.as_bytes();
    for m in STATUS_CODE_RE.find_iter(text) {
        // Skip matches adjacent to '.' or '/' — avoids coloring version numbers
        // (e.g. "537.36", "Chrome/120.0.0.0") and path segments ("/300/").
        let before = if m.start() > 0 {
            text_bytes[m.start() - 1]
        } else {
            b' '
        };
        let after = if m.end() < text_bytes.len() {
            text_bytes[m.end()]
        } else {
            b' '
        };
        if before == b'.' || before == b'/' || after == b'.' || after == b'/' {
            continue;
        }

        let key = status_code_key(m.as_str());
        if !colors.is_disabled(key)
            && let Some(sid) = status_code_style_id(m.as_str())
        {
            matches.push((m.start(), m.end(), sid));
        }
    }

    for m in IPV4_RE.find_iter(text) {
        if !colors.is_disabled("ip_address") {
            // Skip matches preceded by '/' — avoids coloring version numbers
            // like "Chrome/120.0.0.0".
            let before = if m.start() > 0 {
                text_bytes[m.start() - 1]
            } else {
                b' '
            };
            if before == b'/' {
                continue;
            }
            // Validate each octet is 0..=255
            let valid = m
                .as_str()
                .split('.')
                .all(|part| part.len() <= 3 && part.parse::<u16>().is_ok_and(|n| n <= 255));
            if valid {
                matches.push((m.start(), m.end(), VALUE_STYLE_IP));
            }
        }
    }

    for m in UUID_RE.find_iter(text) {
        if !colors.is_disabled("uuid") {
            matches.push((m.start(), m.end(), VALUE_STYLE_UUID));
        }
    }

    for m in IPV6_RE.find_iter(text) {
        if !colors.is_disabled("ip_address") {
            matches.push((m.start(), m.end(), VALUE_STYLE_IP));
        }
    }

    // Sort by start position; for overlapping matches, keep the first.
    matches.sort_by_key(|m| m.0);

    // Remove overlapping matches — keep the first one encountered.
    let mut deduped: Vec<(usize, usize, StyleId)> = Vec::with_capacity(matches.len());
    let mut last_end = 0;
    for m in matches {
        if m.0 >= last_end {
            last_end = m.1;
            deduped.push(m);
        }
    }
    deduped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_colors() -> ValueColors {
        ValueColors::default()
    }

    #[test]
    fn test_http_get_style_id() {
        let colors = default_colors();
        let spans = collect_value_color_spans("GET /api/users HTTP/1.1", &colors);

        assert!(!spans.is_empty());
        assert_eq!(spans[0].0, 0);
        assert_eq!(spans[0].1, 3);
        assert_eq!(spans[0].2, VALUE_STYLE_HTTP_GET);
    }

    #[test]
    fn test_http_post_style_id() {
        let colors = default_colors();
        let spans = collect_value_color_spans("POST /api/data HTTP/1.1", &colors);

        assert!(!spans.is_empty());
        assert_eq!(spans[0].2, VALUE_STYLE_HTTP_POST);
    }

    #[test]
    fn test_http_put_style_id() {
        let colors = default_colors();
        let spans = collect_value_color_spans("PUT /api/item/1", &colors);

        assert!(!spans.is_empty());
        assert_eq!(spans[0].2, VALUE_STYLE_HTTP_PUT);
    }

    #[test]
    fn test_http_delete_style_id() {
        let colors = default_colors();
        let spans = collect_value_color_spans("DELETE /api/item/42", &colors);

        assert!(!spans.is_empty());
        assert_eq!(spans[0].2, VALUE_STYLE_HTTP_DELETE);
    }

    #[test]
    fn test_http_patch_style_id() {
        let colors = default_colors();
        let spans = collect_value_color_spans("PATCH /api/item/1", &colors);

        assert!(!spans.is_empty());
        assert_eq!(spans[0].2, VALUE_STYLE_HTTP_PATCH);
    }

    #[test]
    fn test_http_head_other_style_id() {
        let colors = default_colors();
        let spans = collect_value_color_spans("HEAD /health", &colors);

        assert!(!spans.is_empty());
        assert_eq!(spans[0].2, VALUE_STYLE_HTTP_OTHER);
    }

    #[test]
    fn test_status_2xx_style_id() {
        let colors = default_colors();
        let spans = collect_value_color_spans("HTTP/1.1 200 OK", &colors);

        let status = spans.iter().find(|s| s.2 == VALUE_STYLE_STATUS_2XX);
        assert!(status.is_some());
    }

    #[test]
    fn test_status_3xx_style_id() {
        let colors = default_colors();
        let spans = collect_value_color_spans("redirected 301 Moved", &colors);

        let status = spans.iter().find(|s| s.2 == VALUE_STYLE_STATUS_3XX);
        assert!(status.is_some());
    }

    #[test]
    fn test_status_4xx_style_id() {
        let colors = default_colors();
        let spans = collect_value_color_spans("responded with 404 Not Found", &colors);

        let status = spans.iter().find(|s| s.2 == VALUE_STYLE_STATUS_4XX);
        assert!(status.is_some());
    }

    #[test]
    fn test_status_5xx_style_id() {
        let colors = default_colors();
        let spans = collect_value_color_spans("error 500 Internal Server Error", &colors);

        let status = spans.iter().find(|s| s.2 == VALUE_STYLE_STATUS_5XX);
        assert!(status.is_some());
    }

    #[test]
    fn test_status_1xx_not_colorized() {
        let colors = default_colors();
        let spans = collect_value_color_spans("status 100 Continue", &colors);

        assert!(spans.is_empty());
    }

    #[test]
    fn test_ipv4_style_id() {
        let colors = default_colors();
        let spans = collect_value_color_spans("request from 192.168.1.1 received", &colors);

        let ip = spans.iter().find(|s| s.2 == VALUE_STYLE_IP);
        assert!(ip.is_some());
    }

    #[test]
    fn test_ipv4_invalid_octet_not_colorized() {
        let colors = default_colors();
        let spans = collect_value_color_spans("addr 999.999.999.999 invalid", &colors);

        assert!(!spans.iter().any(|s| s.2 == VALUE_STYLE_IP));
    }

    #[test]
    fn test_ipv6_loopback_style_id() {
        let colors = default_colors();
        let spans = collect_value_color_spans("listening on ::1 port 8080", &colors);

        let ip = spans.iter().find(|s| s.2 == VALUE_STYLE_IP);
        assert!(ip.is_some());
    }

    #[test]
    fn test_uuid_style_id() {
        let colors = default_colors();
        let spans = collect_value_color_spans(
            "request_id=550e8400-e29b-41d4-a716-446655440000 processed",
            &colors,
        );

        let uuid = spans.iter().find(|s| s.2 == VALUE_STYLE_UUID);
        assert!(uuid.is_some());
    }

    #[test]
    fn test_uuid_uppercase_style_id() {
        let colors = default_colors();
        let spans = collect_value_color_spans("id: 550E8400-E29B-41D4-A716-446655440000", &colors);

        let uuid = spans.iter().find(|s| s.2 == VALUE_STYLE_UUID);
        assert!(uuid.is_some());
    }

    #[test]
    fn test_no_matches_returns_empty() {
        let colors = default_colors();
        let spans = collect_value_color_spans("just a regular log line with no patterns", &colors);

        assert!(spans.is_empty());
    }

    #[test]
    fn test_multiple_matches() {
        let colors = default_colors();
        let spans = collect_value_color_spans("GET /api 200 from 10.0.0.1", &colors);

        assert!(spans.iter().any(|s| s.2 == VALUE_STYLE_HTTP_GET));
        assert!(spans.iter().any(|s| s.2 == VALUE_STYLE_STATUS_2XX));
        assert!(spans.iter().any(|s| s.2 == VALUE_STYLE_IP));
    }

    #[test]
    fn test_uuid_mixed_with_other_values() {
        let colors = default_colors();
        let spans = collect_value_color_spans(
            "GET /api/item/550e8400-e29b-41d4-a716-446655440000 200 from 10.0.0.1",
            &colors,
        );

        assert!(spans.iter().any(|s| s.2 == VALUE_STYLE_HTTP_GET));
        assert!(spans.iter().any(|s| s.2 == VALUE_STYLE_UUID));
        assert!(spans.iter().any(|s| s.2 == VALUE_STYLE_STATUS_2XX));
        assert!(spans.iter().any(|s| s.2 == VALUE_STYLE_IP));
    }

    #[test]
    fn test_status_code_not_in_version_number() {
        let colors = default_colors();
        let spans = collect_value_color_spans("AppleWebKit/537.36 Chrome/120.0.0.0", &colors);

        assert!(!spans.iter().any(|s| {
            s.2 == VALUE_STYLE_STATUS_3XX
                || s.2 == VALUE_STYLE_STATUS_2XX
                || s.2 == VALUE_STYLE_STATUS_5XX
        }));
    }

    #[test]
    fn test_status_code_not_in_path_segment() {
        let colors = default_colors();
        let spans = collect_value_color_spans("GET /api/v2/items/300/details HTTP/1.1", &colors);

        assert!(!spans.iter().any(|s| s.2 == VALUE_STYLE_STATUS_3XX));
    }

    #[test]
    fn test_status_code_standalone_still_works() {
        let colors = default_colors();
        let spans = collect_value_color_spans("responded 404 Not Found", &colors);

        assert!(spans.iter().any(|s| s.2 == VALUE_STYLE_STATUS_4XX));
    }

    #[test]
    fn test_disabled_category_skipped() {
        let mut colors = default_colors();
        colors.disabled.insert("http_get".to_string());
        let spans = collect_value_color_spans("GET /api 200", &colors);

        assert!(!spans.iter().any(|s| s.2 == VALUE_STYLE_HTTP_GET));
        assert!(spans.iter().any(|s| s.2 == VALUE_STYLE_STATUS_2XX));
    }

    #[test]
    fn test_disabled_all_categories_no_spans() {
        let mut colors = default_colors();
        for group in colors.grouped_categories(None) {
            for (key, _, _) in group.children {
                colors.disabled.insert(key.to_string());
            }
        }
        let spans = collect_value_color_spans(
            "GET /api 200 from 10.0.0.1 id=550e8400-e29b-41d4-a716-446655440000",
            &colors,
        );

        assert!(spans.is_empty());
    }

    #[test]
    fn test_no_overlapping_spans() {
        let colors = default_colors();
        let spans = collect_value_color_spans("GET /api 200 from 10.0.0.1", &colors);

        let mut last_end = 0;
        for (start, end, _) in &spans {
            assert!(
                *start >= last_end,
                "overlapping span at {start}..{end}, previous ended at {last_end}"
            );
            last_end = *end;
        }
    }

    #[test]
    fn test_spans_sorted_by_start() {
        let colors = default_colors();
        let spans = collect_value_color_spans("GET /api 200 from 10.0.0.1", &colors);

        let starts: Vec<usize> = spans.iter().map(|s| s.0).collect();
        let mut sorted = starts.clone();
        sorted.sort();
        assert_eq!(starts, sorted);
    }
}
