//! Field-scoped filter logic for structured log lines.
//!
//! Field filters let users match against specific parsed fields rather than raw line
//! text (e.g. `:filter --field level=error`). They follow the same storage model as
//! date filters: a [`crate::types::FilterDef`] whose pattern starts with `@field:` is
//! saved to SQLite and skipped by `build_filter_manager`. After the text-filter pass
//! (and the date-filter pass), `extract_field_filters` extracts enabled `@field:`
//! entries and evaluates them per line using the detected [`crate::parser::LogFormatParser`].
//!
//! Lines that cannot be parsed, or where the named field is absent, are never hidden
//! by a field filter — they pass through unchanged. This prevents spurious filtering of
//! plain-text lines mixed into an otherwise structured file.
//!
//! Multiple include field filters combine with AND logic (all must match). Any matching
//! exclude field filter hides the line regardless of include filters.
//!
//! ## Syntax
//!
//! Stored pattern: `@field:level:error`  (after stripping the prefix: `level:error`)
//! The first colon separates key from value, so values may contain colons.
//!
//! ## Field aliases
//!
//! - `level` / `lvl`            → `parts.level`
//! - `timestamp` / `ts` / `time`→ `parts.timestamp`
//! - `target`                    → `parts.target`
//! - `message` / `msg`          → `parts.message`
//! - anything else              → linear search of `parts.extra_fields`

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::filters::{FilterDecision, StyleId};
use crate::parser::DisplayParts;
use crate::types::{FilterDef, FilterType};

/// Internal prefix stored in `FilterDef.pattern` for field-scoped filters.
pub const FIELD_PREFIX: &str = "@field:";

/// A compiled, ready-to-evaluate field-scoped filter.
#[derive(Debug, Clone)]
pub struct FieldFilter {
    /// Canonical field name after alias resolution (kept for diagnostics / tests).
    pub field: String,
    /// Substring to match within the resolved field value.
    pub pattern: String,
    /// Whether this is an include or exclude filter.
    pub decision: FilterDecision,
}

/// A field filter paired with a [`StyleId`] for value highlighting in the render path.
/// Parallel to [`crate::date_filter::DateFilterStyle`].
#[derive(Debug, Clone)]
pub struct FieldFilterStyle {
    pub field_filter: FieldFilter,
    pub style_id: StyleId,
    /// When `true`, only the matched value is highlighted; otherwise the whole line.
    pub match_only: bool,
}

/// Parse the stored `key:value` expression (the part **after** `FIELD_PREFIX`).
///
/// The first colon splits key from value, so the value may itself contain colons.
/// Returns `Err` if the key or value is empty, or if no colon is present.
pub fn parse_field_filter(expr: &str) -> Result<(String, String), String> {
    let colon = expr
        .find(':')
        .ok_or_else(|| format!("field filter must be 'key:value', got: {expr}"))?;
    let key = &expr[..colon];
    let value = &expr[colon + 1..];
    if key.is_empty() {
        return Err("field name must not be empty".to_string());
    }
    if value.is_empty() {
        return Err("field value must not be empty".to_string());
    }
    Ok((key.to_string(), value.to_string()))
}

/// Extract enabled `@field:` entries from `filter_defs` as `(field, pattern)` pairs,
/// preserving the original filter order. Used for per-filter match counting.
pub fn extract_field_filters_ordered(filter_defs: &[FilterDef]) -> Vec<(String, String)> {
    filter_defs
        .iter()
        .filter(|d| d.enabled)
        .filter_map(|d| {
            let expr = d.pattern.strip_prefix(FIELD_PREFIX)?;
            parse_field_filter(expr).ok()
        })
        .collect()
}

/// Increment per-filter counters for each enabled field filter that matches `parts`.
/// Entries in `counts` are parallel to `filters` from [`extract_field_filters_ordered`].
pub fn count_field_filter_matches(
    filters: &[(String, String)],
    parts: Option<&DisplayParts<'_>>,
    counts: &[AtomicUsize],
) {
    let Some(parts) = parts else { return };
    for (i, (field, pattern)) in filters.iter().enumerate() {
        if resolve_field(field, parts)
            .map(|v| v.contains(pattern.as_str()))
            .unwrap_or(false)
            && let Some(c) = counts.get(i)
        {
            c.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Extract all enabled `@field:` entries from `filter_defs` and split them into
/// `(includes, excludes)`.  Disabled or malformed entries are silently skipped.
pub fn extract_field_filters(filter_defs: &[FilterDef]) -> (Vec<FieldFilter>, Vec<FieldFilter>) {
    let mut includes = Vec::new();
    let mut excludes = Vec::new();

    for def in filter_defs {
        if !def.enabled {
            continue;
        }
        let Some(expr) = def.pattern.strip_prefix(FIELD_PREFIX) else {
            continue;
        };
        let Ok((field, pattern)) = parse_field_filter(expr) else {
            continue;
        };
        let decision = match def.filter_type {
            FilterType::Include => FilterDecision::Include,
            FilterType::Exclude => FilterDecision::Exclude,
        };
        let ff = FieldFilter {
            field,
            pattern,
            decision,
        };
        match def.filter_type {
            FilterType::Include => includes.push(ff),
            FilterType::Exclude => excludes.push(ff),
        }
    }

    (includes, excludes)
}

/// Resolve a field name (possibly an alias or dotted path) to the corresponding value in `parts`.
///
/// Alias table:
/// - `level` / `lvl`             → `parts.level`
/// - `timestamp` / `ts` / `time` → `parts.timestamp`
/// - `target`                    → `parts.target`
/// - `message` / `msg`           → `parts.message`
/// - `span.<key>`                → linear search of `parts.span.fields` by key
/// - `fields.<key>`              → linear search of `parts.extra_fields` by bare key
///   (tracing-subscriber inlines the `fields` container into `extra_fields`)
/// - anything else               → linear search of `parts.extra_fields` by key
pub(crate) fn resolve_field<'a>(field: &str, parts: &'a DisplayParts<'a>) -> Option<&'a str> {
    if let Some(span_key) = field.strip_prefix("span.") {
        return parts
            .span
            .as_ref()?
            .fields
            .iter()
            .find(|(k, _)| *k == span_key)
            .map(|(_, v)| *v);
    }
    if let Some(fields_key) = field.strip_prefix("fields.") {
        return parts
            .extra_fields
            .iter()
            .find(|(k, _)| *k == fields_key)
            .map(|(_, v)| *v);
    }
    match field {
        "level" | "lvl" => parts.level,
        "timestamp" | "ts" | "time" => parts.timestamp,
        "target" => parts.target,
        "message" | "msg" => parts.message,
        other => parts
            .extra_fields
            .iter()
            .find(|(k, _)| *k == other)
            .map(|(_, v)| *v),
    }
}

/// Result of evaluating field include filters against a parsed line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FieldVote {
    /// At least one field include filter matched.
    Match,
    /// The line was parsed but no include filter matched (field absent or value mismatch).
    Miss,
    /// The line could not be parsed at all (e.g. a stack-trace continuation line) — pass through.
    PassThrough,
}

/// Check whether any field exclude filter matches the parsed line.
///
/// Returns `false` if `parts` is `None` (unparseable line → pass through).
/// Returns `false` if the named field is absent (pass through).
pub fn any_field_exclude_matches(
    excludes: &[FieldFilter],
    parts: Option<&DisplayParts<'_>>,
) -> bool {
    let Some(parts) = parts else {
        return false; // unparseable → pass through
    };
    excludes.iter().any(|ff| {
        resolve_field(&ff.field, parts)
            .map(|v| v.contains(ff.pattern.as_str()))
            .unwrap_or(false) // field absent → pass through (don't exclude)
    })
}

/// Evaluate field include filters and return a [`FieldVote`].
///
/// - `Match` — at least one include filter found the field and the value matched.
/// - `Miss` — at least one field was present and evaluated, but none matched.
/// - `PassThrough` — `parts` is `None` or all relevant fields were absent; the
///   caller should fall back to text-filter-only visibility logic.
pub fn field_include_vote(includes: &[FieldFilter], parts: Option<&DisplayParts<'_>>) -> FieldVote {
    if includes.is_empty() {
        return FieldVote::PassThrough;
    }
    let Some(parts) = parts else {
        return FieldVote::PassThrough; // line could not be parsed → pass through
    };

    // Line was successfully parsed: any filter that matches → Match; otherwise → Miss.
    // A field that is absent counts as not matching (Miss), not as pass-through.
    for ff in includes {
        if resolve_field(&ff.field, parts).is_some_and(|v| v.contains(ff.pattern.as_str())) {
            return FieldVote::Match;
        }
    }
    FieldVote::Miss
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::DisplayParts;
    use crate::types::{FilterDef, FilterType};

    // ── parse_field_filter ───────────────────────────────────────────────────

    #[test]
    fn test_parse_field_filter_valid() {
        let (k, v) = parse_field_filter("level:error").unwrap();
        assert_eq!(k, "level");
        assert_eq!(v, "error");
    }

    #[test]
    fn test_parse_field_filter_colon_in_value() {
        let (k, v) = parse_field_filter("level:info:extra").unwrap();
        assert_eq!(k, "level");
        assert_eq!(v, "info:extra");
    }

    #[test]
    fn test_parse_field_filter_missing_colon() {
        assert!(parse_field_filter("levelonly").is_err());
    }

    #[test]
    fn test_parse_field_filter_empty_key() {
        assert!(parse_field_filter(":error").is_err());
    }

    #[test]
    fn test_parse_field_filter_empty_value() {
        assert!(parse_field_filter("level:").is_err());
    }

    // ── field_filters_visible ────────────────────────────────────────────────

    fn make_parts<'a>(
        level: Option<&'a str>,
        timestamp: Option<&'a str>,
        message: Option<&'a str>,
        target: Option<&'a str>,
        extra: Vec<(&'a str, &'a str)>,
    ) -> DisplayParts<'a> {
        DisplayParts {
            level,
            timestamp,
            target,
            message,
            extra_fields: extra,
            ..Default::default()
        }
    }

    fn inc(field: &str, pattern: &str) -> FieldFilter {
        FieldFilter {
            field: field.to_string(),
            pattern: pattern.to_string(),
            decision: FilterDecision::Include,
        }
    }

    fn exc(field: &str, pattern: &str) -> FieldFilter {
        FieldFilter {
            field: field.to_string(),
            pattern: pattern.to_string(),
            decision: FilterDecision::Exclude,
        }
    }

    // ── any_field_exclude_matches ─────────────────────────────────────────────

    #[test]
    fn test_exclude_match_hides() {
        let parts = make_parts(Some("debug"), None, None, None, vec![]);
        assert!(any_field_exclude_matches(
            &[exc("level", "debug")],
            Some(&parts)
        ));
    }

    #[test]
    fn test_exclude_no_match_passes() {
        let parts = make_parts(Some("info"), None, None, None, vec![]);
        assert!(!any_field_exclude_matches(
            &[exc("level", "debug")],
            Some(&parts)
        ));
    }

    #[test]
    fn test_exclude_parts_none_passthrough() {
        assert!(!any_field_exclude_matches(&[exc("level", "debug")], None));
    }

    #[test]
    fn test_exclude_field_absent_passthrough() {
        // Field does not exist in parts → pass through (not excluded)
        let parts = make_parts(None, None, None, None, vec![]);
        assert!(!any_field_exclude_matches(
            &[exc("level", "debug")],
            Some(&parts)
        ));
    }

    // ── field_include_vote ────────────────────────────────────────────────────

    #[test]
    fn test_include_match_vote() {
        let parts = make_parts(Some("error"), None, None, None, vec![]);
        assert_eq!(
            field_include_vote(&[inc("level", "error")], Some(&parts)),
            FieldVote::Match
        );
    }

    #[test]
    fn test_include_no_match_vote_miss() {
        let parts = make_parts(Some("info"), None, None, None, vec![]);
        assert_eq!(
            field_include_vote(&[inc("level", "error")], Some(&parts)),
            FieldVote::Miss
        );
    }

    #[test]
    fn test_include_parts_none_passthrough() {
        assert_eq!(
            field_include_vote(&[inc("level", "error")], None),
            FieldVote::PassThrough
        );
    }

    #[test]
    fn test_include_field_absent_is_miss() {
        // Parsed line where the named field is absent → Miss (not PassThrough).
        // This ensures `filter --field level=error` hides lines that have no `level`.
        let parts = make_parts(None, None, None, None, vec![]);
        assert_eq!(
            field_include_vote(&[inc("level", "error")], Some(&parts)),
            FieldVote::Miss
        );
    }

    #[test]
    fn test_two_includes_any_match() {
        // OR semantics: first include matches → Match
        let parts = make_parts(Some("error"), None, None, Some("api"), vec![]);
        assert_eq!(
            field_include_vote(
                &[inc("level", "error"), inc("target", "auth")],
                Some(&parts)
            ),
            FieldVote::Match
        );
    }

    #[test]
    fn test_two_includes_neither_match() {
        let parts = make_parts(Some("info"), None, None, Some("api"), vec![]);
        assert_eq!(
            field_include_vote(
                &[inc("level", "error"), inc("target", "auth")],
                Some(&parts)
            ),
            FieldVote::Miss
        );
    }

    #[test]
    fn test_extra_field_by_key_match() {
        let parts = make_parts(None, None, None, None, vec![("component", "auth")]);
        assert_eq!(
            field_include_vote(&[inc("component", "auth")], Some(&parts)),
            FieldVote::Match
        );
    }

    #[test]
    fn test_extra_field_by_key_miss() {
        let parts = make_parts(None, None, None, None, vec![("component", "auth")]);
        assert_eq!(
            field_include_vote(&[inc("component", "api")], Some(&parts)),
            FieldVote::Miss
        );
    }

    // ── alias resolution ─────────────────────────────────────────────────────

    #[test]
    fn test_alias_lvl() {
        let parts = make_parts(Some("warn"), None, None, None, vec![]);
        assert_eq!(
            field_include_vote(&[inc("lvl", "warn")], Some(&parts)),
            FieldVote::Match
        );
    }

    #[test]
    fn test_alias_ts() {
        let parts = make_parts(None, Some("2024-01-01"), None, None, vec![]);
        assert_eq!(
            field_include_vote(&[inc("ts", "2024")], Some(&parts)),
            FieldVote::Match
        );
    }

    #[test]
    fn test_alias_msg() {
        let parts = make_parts(None, None, Some("hello world"), None, vec![]);
        assert_eq!(
            field_include_vote(&[inc("msg", "hello")], Some(&parts)),
            FieldVote::Match
        );
    }

    // ── dotted path resolution ────────────────────────────────────────────────

    fn make_parts_with_span<'a>(
        extra: Vec<(&'a str, &'a str)>,
        span_fields: Vec<(&'a str, &'a str)>,
    ) -> DisplayParts<'a> {
        use crate::parser::SpanInfo;
        DisplayParts {
            span: Some(SpanInfo {
                name: "req",
                fields: span_fields,
            }),
            extra_fields: extra,
            ..Default::default()
        }
    }

    #[test]
    fn test_span_dotted_path_match() {
        let parts = make_parts_with_span(vec![], vec![("method", "GET")]);
        assert_eq!(
            field_include_vote(&[inc("span.method", "GET")], Some(&parts)),
            FieldVote::Match
        );
    }

    #[test]
    fn test_span_dotted_path_miss() {
        let parts = make_parts_with_span(vec![], vec![("method", "POST")]);
        assert_eq!(
            field_include_vote(&[inc("span.method", "GET")], Some(&parts)),
            FieldVote::Miss
        );
    }

    #[test]
    fn test_span_dotted_path_absent_key() {
        let parts = make_parts_with_span(vec![], vec![("uri", "/")]);
        assert_eq!(
            field_include_vote(&[inc("span.method", "GET")], Some(&parts)),
            FieldVote::Miss
        );
    }

    #[test]
    fn test_fields_dotted_path_match() {
        // tracing-subscriber inlines "fields" container into extra_fields with bare keys
        let parts = make_parts(None, None, None, None, vec![("order_id", "42")]);
        assert_eq!(
            field_include_vote(&[inc("fields.order_id", "42")], Some(&parts)),
            FieldVote::Match
        );
    }

    #[test]
    fn test_fields_dotted_path_miss() {
        let parts = make_parts(None, None, None, None, vec![("order_id", "99")]);
        assert_eq!(
            field_include_vote(&[inc("fields.order_id", "42")], Some(&parts)),
            FieldVote::Miss
        );
    }

    // ── extract_field_filters ────────────────────────────────────────────────

    fn make_def(id: usize, pattern: &str, filter_type: FilterType, enabled: bool) -> FilterDef {
        FilterDef {
            id,
            pattern: pattern.to_string(),
            filter_type,
            enabled,
            color_config: None,
        }
    }

    #[test]
    fn test_extract_disabled_skipped() {
        let defs = vec![make_def(
            1,
            "@field:level:error",
            FilterType::Include,
            false,
        )];
        let (inc, exc) = extract_field_filters(&defs);
        assert!(inc.is_empty());
        assert!(exc.is_empty());
    }

    #[test]
    fn test_extract_non_field_prefix_skipped() {
        let defs = vec![make_def(1, "level=error", FilterType::Include, true)];
        let (inc, exc) = extract_field_filters(&defs);
        assert!(inc.is_empty());
        assert!(exc.is_empty());
    }

    #[test]
    fn test_extract_malformed_skipped() {
        let defs = vec![make_def(1, "@field:levelonly", FilterType::Include, true)];
        let (inc, exc) = extract_field_filters(&defs);
        assert!(inc.is_empty());
        assert!(exc.is_empty());
    }

    #[test]
    fn test_extract_include_exclude_split() {
        let defs = vec![
            make_def(1, "@field:level:error", FilterType::Include, true),
            make_def(2, "@field:level:debug", FilterType::Exclude, true),
        ];
        let (inc, exc) = extract_field_filters(&defs);
        assert_eq!(inc.len(), 1);
        assert_eq!(exc.len(), 1);
        assert_eq!(inc[0].field, "level");
        assert_eq!(inc[0].pattern, "error");
        assert_eq!(exc[0].pattern, "debug");
    }

    // ── extract_field_filters_ordered ─────────────────────────────────────────

    #[test]
    fn test_extract_field_filters_ordered_preserves_order() {
        let defs = vec![
            make_def(1, "@field:level:error", FilterType::Include, true),
            make_def(2, "@field:level:debug", FilterType::Exclude, true),
            make_def(3, "@field:target:api", FilterType::Include, true),
        ];
        let ordered = extract_field_filters_ordered(&defs);
        assert_eq!(ordered.len(), 3);
        assert_eq!(ordered[0], ("level".to_string(), "error".to_string()));
        assert_eq!(ordered[1], ("level".to_string(), "debug".to_string()));
        assert_eq!(ordered[2], ("target".to_string(), "api".to_string()));
    }

    #[test]
    fn test_extract_field_filters_ordered_skips_disabled() {
        let defs = vec![
            make_def(1, "@field:level:error", FilterType::Include, true),
            make_def(2, "@field:level:debug", FilterType::Exclude, false),
        ];
        let ordered = extract_field_filters_ordered(&defs);
        assert_eq!(ordered.len(), 1);
        assert_eq!(ordered[0].0, "level");
    }

    // ── count_field_filter_matches ────────────────────────────────────────────

    #[test]
    fn test_count_field_filter_matches_increments_on_match() {
        use std::sync::atomic::AtomicUsize;
        let parts = make_parts(Some("error"), None, None, None, vec![]);
        let filters = vec![("level".to_string(), "error".to_string())];
        let counts = vec![AtomicUsize::new(0)];
        count_field_filter_matches(&filters, Some(&parts), &counts);
        assert_eq!(counts[0].load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_count_field_filter_matches_no_increment_on_miss() {
        use std::sync::atomic::AtomicUsize;
        let parts = make_parts(Some("info"), None, None, None, vec![]);
        let filters = vec![("level".to_string(), "error".to_string())];
        let counts = vec![AtomicUsize::new(0)];
        count_field_filter_matches(&filters, Some(&parts), &counts);
        assert_eq!(counts[0].load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_count_field_filter_matches_no_parts_skips() {
        use std::sync::atomic::AtomicUsize;
        let filters = vec![("level".to_string(), "error".to_string())];
        let counts = vec![AtomicUsize::new(0)];
        count_field_filter_matches(&filters, None, &counts);
        assert_eq!(counts[0].load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_count_field_filter_matches_multiple_filters() {
        use std::sync::atomic::AtomicUsize;
        let parts = make_parts(Some("error"), None, Some("crash"), None, vec![]);
        let filters = vec![
            ("level".to_string(), "error".to_string()),
            ("message".to_string(), "crash".to_string()),
            ("level".to_string(), "debug".to_string()),
        ];
        let counts = vec![
            AtomicUsize::new(0),
            AtomicUsize::new(0),
            AtomicUsize::new(0),
        ];
        count_field_filter_matches(&filters, Some(&parts), &counts);
        assert_eq!(counts[0].load(Ordering::Relaxed), 1);
        assert_eq!(counts[1].load(Ordering::Relaxed), 1);
        assert_eq!(counts[2].load(Ordering::Relaxed), 0);
    }
}
