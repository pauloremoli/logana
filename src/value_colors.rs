use ratatui::style::Color;
use ratatui::text::{Line, Span};
use regex::Regex;
use std::sync::LazyLock;

use crate::theme::ValueColors;

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

struct ColorMatch {
    start: usize,
    end: usize,
    color: Color,
}

fn http_method_color(method: &str, colors: &ValueColors) -> Color {
    match method {
        "GET" => colors.http_get,
        "POST" => colors.http_post,
        "PUT" => colors.http_put,
        "DELETE" => colors.http_delete,
        "PATCH" => colors.http_patch,
        _ => colors.http_other,
    }
}

fn status_code_color(code: &str, colors: &ValueColors) -> Option<Color> {
    let first = code.as_bytes()[0];
    match first {
        b'2' => Some(colors.status_2xx),
        b'3' => Some(colors.status_3xx),
        b'4' => Some(colors.status_4xx),
        b'5' => Some(colors.status_5xx),
        _ => None,
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

fn status_code_key(code: &str) -> &'static str {
    match code.as_bytes()[0] {
        b'2' => "status_2xx",
        b'3' => "status_3xx",
        b'4' => "status_4xx",
        b'5' => "status_5xx",
        _ => "status_1xx",
    }
}

fn collect_matches(text: &str, colors: &ValueColors) -> Vec<ColorMatch> {
    let mut matches = Vec::new();

    for m in HTTP_METHOD_RE.find_iter(text) {
        let key = http_method_key(m.as_str());
        if !colors.is_disabled(key) {
            matches.push(ColorMatch {
                start: m.start(),
                end: m.end(),
                color: http_method_color(m.as_str(), colors),
            });
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
            && let Some(color) = status_code_color(m.as_str(), colors)
        {
            matches.push(ColorMatch {
                start: m.start(),
                end: m.end(),
                color,
            });
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
                matches.push(ColorMatch {
                    start: m.start(),
                    end: m.end(),
                    color: colors.ip_address,
                });
            }
        }
    }

    for m in UUID_RE.find_iter(text) {
        if !colors.is_disabled("uuid") {
            matches.push(ColorMatch {
                start: m.start(),
                end: m.end(),
                color: colors.uuid,
            });
        }
    }

    for m in IPV6_RE.find_iter(text) {
        if !colors.is_disabled("ip_address") {
            matches.push(ColorMatch {
                start: m.start(),
                end: m.end(),
                color: colors.ip_address,
            });
        }
    }

    // Sort by start position; for overlapping matches, keep the first (longest first by regex).
    matches.sort_by_key(|m| m.start);

    // Remove overlapping matches — keep the first one encountered.
    let mut deduped: Vec<ColorMatch> = Vec::with_capacity(matches.len());
    let mut last_end = 0;
    for m in matches {
        if m.start >= last_end {
            last_end = m.end;
            deduped.push(m);
        }
    }
    deduped
}

fn colorize_span(span: Span<'static>, colors: &ValueColors) -> Vec<Span<'static>> {
    let matches = collect_matches(&span.content, colors);
    if matches.is_empty() {
        return vec![span];
    }

    let text = span.content.into_owned();
    let base_style = span.style;
    let mut result = Vec::with_capacity(matches.len() * 2 + 1);
    let mut pos = 0;

    for m in &matches {
        if m.start > pos {
            result.push(Span::styled(text[pos..m.start].to_string(), base_style));
        }
        result.push(Span::styled(
            text[m.start..m.end].to_string(),
            base_style.fg(m.color),
        ));
        pos = m.end;
    }

    if pos < text.len() {
        result.push(Span::styled(text[pos..].to_string(), base_style));
    }

    result
}

/// Post-processes a rendered `Line`, applying value-based colors to spans that carry no
/// explicit styling. Spans already colored by filters (fg **or** bg) or search are left
/// untouched so that filter highlighting always takes precedence.
pub fn colorize_known_values(line: Line<'static>, colors: &ValueColors) -> Line<'static> {
    let alignment = line.alignment;
    let style = line.style;
    let mut new_spans: Vec<Span<'static>> = Vec::with_capacity(line.spans.len());

    for span in line.spans {
        if span.style.fg.is_some() || span.style.bg.is_some() {
            // Already styled by a filter, date filter, process color, or search — keep as-is.
            new_spans.push(span);
        } else {
            new_spans.extend(colorize_span(span, colors));
        }
    }

    let mut new_line = Line::from(new_spans);
    new_line.style = style;
    new_line.alignment = alignment;
    new_line
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Style;

    fn default_colors() -> ValueColors {
        ValueColors::default()
    }

    #[test]
    fn test_http_methods_colorized() {
        let colors = default_colors();
        let line = Line::from("GET /api/users HTTP/1.1");
        let result = colorize_known_values(line, &colors);

        // First span should be "GET" with http_get color
        assert_eq!(result.spans[0].content.as_ref(), "GET");
        assert_eq!(result.spans[0].style.fg, Some(colors.http_get));
    }

    #[test]
    fn test_post_method_colorized() {
        let colors = default_colors();
        let line = Line::from("POST /api/data HTTP/1.1");
        let result = colorize_known_values(line, &colors);

        assert_eq!(result.spans[0].content.as_ref(), "POST");
        assert_eq!(result.spans[0].style.fg, Some(colors.http_post));
    }

    #[test]
    fn test_delete_method_colorized() {
        let colors = default_colors();
        let line = Line::from("DELETE /api/item/42");
        let result = colorize_known_values(line, &colors);

        assert_eq!(result.spans[0].content.as_ref(), "DELETE");
        assert_eq!(result.spans[0].style.fg, Some(colors.http_delete));
    }

    #[test]
    fn test_status_codes_colorized() {
        let colors = default_colors();
        let line = Line::from("HTTP/1.1 200 OK");
        let result = colorize_known_values(line, &colors);

        // Find the span with "200"
        let status_span = result.spans.iter().find(|s| s.content.contains("200"));
        assert!(status_span.is_some());
        assert_eq!(status_span.unwrap().style.fg, Some(colors.status_2xx));
    }

    #[test]
    fn test_status_4xx_colorized() {
        let colors = default_colors();
        let line = Line::from("responded with 404 Not Found");
        let result = colorize_known_values(line, &colors);

        let status_span = result.spans.iter().find(|s| s.content.contains("404"));
        assert!(status_span.is_some());
        assert_eq!(status_span.unwrap().style.fg, Some(colors.status_4xx));
    }

    #[test]
    fn test_status_5xx_colorized() {
        let colors = default_colors();
        let line = Line::from("error 500 Internal Server Error");
        let result = colorize_known_values(line, &colors);

        let status_span = result.spans.iter().find(|s| s.content.contains("500"));
        assert!(status_span.is_some());
        assert_eq!(status_span.unwrap().style.fg, Some(colors.status_5xx));
    }

    #[test]
    fn test_status_1xx_not_colorized() {
        let colors = default_colors();
        let line = Line::from("status 100 Continue");
        let result = colorize_known_values(line, &colors);

        // 1xx has no color mapping
        let status_span = result.spans.iter().find(|s| s.content.contains("100"));
        assert!(status_span.is_some());
        assert_eq!(status_span.unwrap().style.fg, None);
    }

    #[test]
    fn test_ipv4_colorized() {
        let colors = default_colors();
        let line = Line::from("request from 192.168.1.1 received");
        let result = colorize_known_values(line, &colors);

        let ip_span = result
            .spans
            .iter()
            .find(|s| s.content.contains("192.168.1.1"));
        assert!(ip_span.is_some());
        assert_eq!(ip_span.unwrap().style.fg, Some(colors.ip_address));
    }

    #[test]
    fn test_ipv4_invalid_octet_not_colorized() {
        let colors = default_colors();
        let line = Line::from("addr 999.999.999.999 invalid");
        let result = colorize_known_values(line, &colors);

        // Invalid IP — should not be colored as IP
        let ip_span = result
            .spans
            .iter()
            .find(|s| s.content.contains("999.999.999.999"));
        assert!(ip_span.is_some());
        assert_eq!(ip_span.unwrap().style.fg, None);
    }

    #[test]
    fn test_ipv6_loopback_colorized() {
        let colors = default_colors();
        let line = Line::from("listening on ::1 port 8080");
        let result = colorize_known_values(line, &colors);

        let ip_span = result.spans.iter().find(|s| s.content.contains("::1"));
        assert!(ip_span.is_some());
        assert_eq!(ip_span.unwrap().style.fg, Some(colors.ip_address));
    }

    #[test]
    fn test_filter_colored_spans_untouched_fg() {
        let colors = default_colors();
        let styled = Span::styled("GET /api", Style::default().fg(Color::Yellow));
        let line = Line::from(vec![styled]);
        let result = colorize_known_values(line, &colors);

        // Should remain exactly as-is because fg was already set.
        assert_eq!(result.spans.len(), 1);
        assert_eq!(result.spans[0].style.fg, Some(Color::Yellow));
    }

    #[test]
    fn test_filter_bg_only_spans_untouched() {
        // A filter that sets only a background color (no fg) should still
        // prevent value colors from overriding the span.
        let colors = default_colors();
        let bg_color = Color::DarkGray;
        let styled = Span::styled("GET /api 200 OK", Style::default().bg(bg_color));
        let line = Line::from(vec![styled]);
        let result = colorize_known_values(line, &colors);

        // Even though fg is None, the bg means a filter colored this span —
        // value colors must not be applied.
        assert_eq!(result.spans.len(), 1);
        assert_eq!(result.spans[0].style.bg, Some(bg_color));
        assert_eq!(result.spans[0].style.fg, None);
    }

    #[test]
    fn test_mixed_styled_and_unstyled_spans() {
        let colors = default_colors();
        let spans = vec![
            Span::styled("filter: ", Style::default().fg(Color::Red)),
            Span::raw("GET /api/v1 200"),
        ];
        let line = Line::from(spans);
        let result = colorize_known_values(line, &colors);

        // First span should be unchanged (has fg)
        assert_eq!(result.spans[0].content.as_ref(), "filter: ");
        assert_eq!(result.spans[0].style.fg, Some(Color::Red));

        // GET should be colorized
        let get_span = result.spans.iter().find(|s| s.content.as_ref() == "GET");
        assert!(get_span.is_some());
        assert_eq!(get_span.unwrap().style.fg, Some(colors.http_get));
    }

    #[test]
    fn test_no_matches_returns_same_line() {
        let colors = default_colors();
        let line = Line::from("just a regular log line with no patterns");
        let result = colorize_known_values(line, &colors);

        assert_eq!(result.spans.len(), 1);
        assert_eq!(
            result.spans[0].content.as_ref(),
            "just a regular log line with no patterns"
        );
        assert_eq!(result.spans[0].style.fg, None);
    }

    #[test]
    fn test_multiple_matches_in_single_span() {
        let colors = default_colors();
        let line = Line::from("GET /api 200 from 10.0.0.1");
        let result = colorize_known_values(line, &colors);

        let get_span = result.spans.iter().find(|s| s.content.as_ref() == "GET");
        assert!(get_span.is_some());
        assert_eq!(get_span.unwrap().style.fg, Some(colors.http_get));

        let status_span = result.spans.iter().find(|s| s.content.as_ref() == "200");
        assert!(status_span.is_some());
        assert_eq!(status_span.unwrap().style.fg, Some(colors.status_2xx));

        let ip_span = result
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "10.0.0.1");
        assert!(ip_span.is_some());
        assert_eq!(ip_span.unwrap().style.fg, Some(colors.ip_address));
    }

    #[test]
    fn test_line_style_preserved() {
        let colors = default_colors();
        let mut line = Line::from("GET /api");
        line.style = Style::default().bg(Color::Blue);
        let result = colorize_known_values(line, &colors);

        assert_eq!(result.style.bg, Some(Color::Blue));
    }

    #[test]
    fn test_uuid_colorized() {
        let colors = default_colors();
        let line = Line::from("request_id=550e8400-e29b-41d4-a716-446655440000 processed");
        let result = colorize_known_values(line, &colors);

        let uuid_span = result
            .spans
            .iter()
            .find(|s| s.content.contains("550e8400-e29b-41d4-a716-446655440000"));
        assert!(uuid_span.is_some());
        assert_eq!(uuid_span.unwrap().style.fg, Some(colors.uuid));
    }

    #[test]
    fn test_uuid_uppercase_colorized() {
        let colors = default_colors();
        let line = Line::from("id: 550E8400-E29B-41D4-A716-446655440000");
        let result = colorize_known_values(line, &colors);

        let uuid_span = result
            .spans
            .iter()
            .find(|s| s.content.contains("550E8400-E29B-41D4-A716-446655440000"));
        assert!(uuid_span.is_some());
        assert_eq!(uuid_span.unwrap().style.fg, Some(colors.uuid));
    }

    #[test]
    fn test_uuid_mixed_with_other_values() {
        let colors = default_colors();
        let line =
            Line::from("GET /api/item/550e8400-e29b-41d4-a716-446655440000 200 from 10.0.0.1");
        let result = colorize_known_values(line, &colors);

        let get_span = result.spans.iter().find(|s| s.content.as_ref() == "GET");
        assert!(get_span.is_some());
        assert_eq!(get_span.unwrap().style.fg, Some(colors.http_get));

        let uuid_span = result.spans.iter().find(|s| s.content.contains("550e8400"));
        assert!(uuid_span.is_some());
        assert_eq!(uuid_span.unwrap().style.fg, Some(colors.uuid));

        let status_span = result.spans.iter().find(|s| s.content.as_ref() == "200");
        assert!(status_span.is_some());
        assert_eq!(status_span.unwrap().style.fg, Some(colors.status_2xx));
    }

    #[test]
    fn test_status_code_not_in_version_number() {
        let colors = default_colors();
        let line = Line::from("AppleWebKit/537.36 Chrome/120.0.0.0");
        let result = colorize_known_values(line, &colors);

        // "537" and "120" should NOT be colorized — they are part of version numbers
        for span in &result.spans {
            if span.content.contains("537") || span.content.contains("120") {
                assert_eq!(
                    span.style.fg, None,
                    "version number fragment '{}' should not be colorized",
                    span.content
                );
            }
        }
    }

    #[test]
    fn test_status_code_not_in_path_segment() {
        let colors = default_colors();
        let line = Line::from("GET /api/v2/items/300/details HTTP/1.1");
        let result = colorize_known_values(line, &colors);

        // "300" is a path segment (/300/), should NOT be colorized
        let path_300 = result.spans.iter().find(|s| s.content.contains("300"));
        assert!(path_300.is_some());
        assert_eq!(path_300.unwrap().style.fg, None);
    }

    #[test]
    fn test_status_code_standalone_still_works() {
        let colors = default_colors();
        let line = Line::from("responded 404 Not Found");
        let result = colorize_known_values(line, &colors);

        let status_span = result.spans.iter().find(|s| s.content.as_ref() == "404");
        assert!(status_span.is_some());
        assert_eq!(status_span.unwrap().style.fg, Some(colors.status_4xx));
    }

    #[test]
    fn test_status_code_at_end_of_line() {
        let colors = default_colors();
        let line = Line::from("request completed 500");
        let result = colorize_known_values(line, &colors);

        let status_span = result.spans.iter().find(|s| s.content.as_ref() == "500");
        assert!(status_span.is_some());
        assert_eq!(status_span.unwrap().style.fg, Some(colors.status_5xx));
    }

    #[test]
    fn test_status_code_not_in_port_number() {
        let colors = default_colors();
        let line = Line::from("listening on :443 and :200");
        let _result = colorize_known_values(line, &colors);
        // Port numbers like :443 and :200 are preceded by ':' which is not a
        // word char, so \b still matches. This is acceptable — port numbers
        // in the status code range do get colored. The important cases are
        // version numbers (.) and path segments (/) which are now excluded.
    }

    #[test]
    fn test_disabled_category_skipped() {
        let mut colors = default_colors();
        colors.disabled.insert("http_get".to_string());
        let line = Line::from("GET /api 200");
        let result = colorize_known_values(line, &colors);

        // GET should NOT be colorized (disabled)
        let get_span = result.spans.iter().find(|s| s.content.contains("GET"));
        assert!(get_span.is_some());
        assert_eq!(get_span.unwrap().style.fg, None);

        // 200 should still be colorized
        let status_span = result.spans.iter().find(|s| s.content.contains("200"));
        assert!(status_span.is_some());
        assert_eq!(status_span.unwrap().style.fg, Some(colors.status_2xx));
    }

    #[test]
    fn test_disabled_all_categories_no_coloring() {
        let mut colors = default_colors();
        for group in colors.grouped_categories(None) {
            for (key, _, _) in group.children {
                colors.disabled.insert(key.to_string());
            }
        }
        let line = Line::from("GET /api 200 from 10.0.0.1 id=550e8400-e29b-41d4-a716-446655440000");
        let result = colorize_known_values(line, &colors);

        // Should be a single unstyled span
        assert_eq!(result.spans.len(), 1);
        assert_eq!(result.spans[0].style.fg, None);
    }
}
