use crate::filters::{FilterDecision, FilterDef, FilterType, StyleId};
use crate::parser::DisplayParts;

pub const FIELD_PREFIX: &str = "@field:";

/// `(field, value)` conditions parsed from a stored field filter expression,
/// alongside its optional free-text condition. Returned by
/// [`parse_field_filter_expr`].
pub type FieldFilterExpr = (Vec<(String, String)>, Option<String>);

/// Joins multiple `key:value` condition segments within a compound field
/// filter's stored expression. Never appears in typed filter values.
const CONDITION_SEP: char = '\u{1F}';
/// Marks a segment as the free-text condition rather than a `key:value` pair.
const TEXT_MARKER: char = '\u{02}';

/// A compiled, ready-to-evaluate field-scoped filter. May combine several
/// `(field, value)` conditions (all must match — AND) with an optional
/// free-text substring condition (also ANDed), e.g. from
/// `:filter --field level=INFO --field component=Draco some text`.
#[derive(Debug, Clone)]
pub struct FieldFilter {
    /// `(field, pattern)` pairs — every one must match for this filter to match.
    pub conditions: Vec<(String, String)>,
    /// Optional free-text substring, ANDed with `conditions` when present —
    /// matched against the full raw line, same as a plain non-field filter.
    pub text: Option<String>,
    /// Whether this is an include or exclude filter.
    pub decision: FilterDecision,
}

/// A field filter paired with a [`StyleId`] for value highlighting in the render path.
/// Parallel to [`crate::filters::DateFilterStyle`].
#[derive(Debug, Clone)]
pub struct FieldFilterStyle {
    pub field_filter: FieldFilter,
    pub style_id: StyleId,
    /// When `true`, only the matched value is highlighted; otherwise the whole line.
    pub match_only: bool,
}

/// Parse a single stored `key:value` condition (part of the expression
/// **after** `FIELD_PREFIX`, already split on [`CONDITION_SEP`] by
/// [`parse_field_filter_expr`]).
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

/// Parse the stored expression (the part **after** `FIELD_PREFIX`) into its
/// `(field, value)` conditions and optional free-text condition.
///
/// A single `key:value` with no [`CONDITION_SEP`] — the pre-existing,
/// single-condition storage format — parses through this same function
/// unchanged, so filters saved before compound support was added keep
/// loading correctly.
pub fn parse_field_filter_expr(expr: &str) -> Result<FieldFilterExpr, String> {
    let mut conditions = Vec::new();
    let mut text = None;
    for segment in expr.split(CONDITION_SEP) {
        if let Some(t) = segment.strip_prefix(TEXT_MARKER) {
            text = Some(t.to_string());
        } else {
            conditions.push(parse_field_filter(segment)?);
        }
    }
    Ok((conditions, text))
}

/// Encode `conditions` (and optional `text`) into a `FIELD_PREFIX`-prefixed
/// stored pattern string. Inverse of [`parse_field_filter_expr`] (applied to
/// the string with `FIELD_PREFIX` stripped). A single condition with no text
/// encodes to exactly the pre-existing `@field:key:value` format.
pub fn encode_field_filter(conditions: &[(String, String)], text: Option<&str>) -> String {
    let mut segments: Vec<String> = conditions.iter().map(|(k, v)| format!("{k}:{v}")).collect();
    if let Some(t) = text {
        segments.push(format!("{TEXT_MARKER}{t}"));
    }
    format!(
        "{FIELD_PREFIX}{}",
        segments.join(&CONDITION_SEP.to_string())
    )
}

/// Whether every one of `ff`'s conditions matches the parsed line, and (if
/// present) its free-text condition is found in the raw line.
pub fn field_filter_matches(ff: &FieldFilter, parts: &DisplayParts<'_>, line: &[u8]) -> bool {
    ff.conditions.iter().all(|(field, pattern)| {
        resolve_field(field, parts).is_some_and(|v| v.contains(pattern.as_str()))
    }) && ff
        .text
        .as_deref()
        .is_none_or(|t| std::str::from_utf8(line).is_ok_and(|s| s.contains(t)))
}

/// Extract enabled `@field:` entries from `filter_defs` as compiled compound
/// filters, preserving the original filter order. Used for per-filter match
/// counting (one count per filter — the whole compound condition matching —
/// not per condition).
pub fn extract_field_filters_ordered(filter_defs: &[FilterDef]) -> Vec<FieldFilter> {
    filter_defs
        .iter()
        .filter(|d| d.enabled)
        .filter_map(|d| {
            let expr = d.pattern.strip_prefix(FIELD_PREFIX)?;
            let (conditions, text) = parse_field_filter_expr(expr).ok()?;
            let decision = match d.filter_type {
                FilterType::Include => FilterDecision::Include,
                FilterType::Exclude => FilterDecision::Exclude,
                FilterType::Highlight => FilterDecision::Highlight,
            };
            Some(FieldFilter {
                conditions,
                text,
                decision,
            })
        })
        .collect()
}

/// Increment per-filter counters for each enabled field filter that matches `parts`/`line`.
/// Entries in `counts` are parallel to `filters` from [`extract_field_filters_ordered`].
pub fn count_field_filter_matches(
    filters: &[FieldFilter],
    parts: Option<&DisplayParts<'_>>,
    line: &[u8],
    counts: &mut [usize],
) {
    let Some(parts) = parts else { return };
    for (i, ff) in filters.iter().enumerate() {
        if field_filter_matches(ff, parts, line)
            && let Some(c) = counts.get_mut(i)
        {
            *c += 1;
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
        let Ok((conditions, text)) = parse_field_filter_expr(expr) else {
            continue;
        };
        // Highlight field filters style matching lines but must never enter
        // the visibility vote, so they contribute to neither list here.
        let decision = match def.filter_type {
            FilterType::Include => FilterDecision::Include,
            FilterType::Exclude => FilterDecision::Exclude,
            FilterType::Highlight => continue,
        };
        let ff = FieldFilter {
            conditions,
            text,
            decision,
        };
        match def.filter_type {
            FilterType::Include => includes.push(ff),
            FilterType::Exclude => excludes.push(ff),
            FilterType::Highlight => unreachable!("skipped above"),
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
pub fn resolve_field<'a>(field: &str, parts: &'a DisplayParts<'a>) -> Option<&'a str> {
    if let Some(span_key) = field.strip_prefix("span.") {
        let span = parts.span.as_ref()?;
        if span_key == "name" {
            return Some(span.name);
        }
        return span
            .fields
            .iter()
            .find(|(k, _)| *k == span_key)
            .map(|(_, v)| *v);
    }
    if let Some(fields_key) = field.strip_prefix("fields.") {
        return parts
            .extra_fields
            .iter()
            .find(|(_, k, _)| *k == fields_key)
            .map(|(_, _, v)| *v);
    }
    match field {
        "level" | "lvl" => parts.level,
        "timestamp" | "ts" | "time" => parts.timestamp,
        "target" => parts.target,
        "message" | "msg" => parts.message,
        other => parts
            .extra_fields
            .iter()
            .find(|(_, k, _)| *k == other)
            .map(|(_, _, v)| *v),
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
    line: &[u8],
) -> bool {
    let Some(parts) = parts else {
        return false; // unparseable → pass through
    };
    excludes
        .iter()
        .any(|ff| field_filter_matches(ff, parts, line))
}

/// Evaluate field include filters and return a [`FieldVote`].
///
/// - `Match` — at least one include filter's conditions (and text, if any) all matched.
/// - `Miss` — at least one field was present and evaluated, but none matched.
/// - `PassThrough` — `parts` is `None` or all relevant fields were absent; the
///   caller should fall back to text-filter-only visibility logic.
pub fn field_include_vote(
    includes: &[FieldFilter],
    parts: Option<&DisplayParts<'_>>,
    line: &[u8],
) -> FieldVote {
    if includes.is_empty() {
        return FieldVote::PassThrough;
    }
    let Some(parts) = parts else {
        return FieldVote::PassThrough; // line could not be parsed → pass through
    };

    // Line was successfully parsed: any filter that matches → Match; otherwise → Miss.
    // A field that is absent counts as not matching (Miss), not as pass-through.
    for ff in includes {
        if field_filter_matches(ff, parts, line) {
            return FieldVote::Match;
        }
    }
    FieldVote::Miss
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filters::{FilterDef, FilterType};
    use crate::parser::DisplayParts;

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

    // ── parse_field_filter_expr / encode_field_filter ───────────────────────

    #[test]
    fn test_parse_field_filter_expr_single_condition_old_format() {
        // No separator present — must parse exactly like the pre-existing
        // single-condition storage format, for backward compatibility with
        // filters already saved by users.
        let (conditions, text) = parse_field_filter_expr("level:INFO").unwrap();
        assert_eq!(conditions, vec![("level".to_string(), "INFO".to_string())]);
        assert_eq!(text, None);
    }

    #[test]
    fn test_parse_field_filter_expr_multiple_conditions() {
        let (conditions, text) =
            parse_field_filter_expr("level:INFO\u{1F}component:Draco").unwrap();
        assert_eq!(
            conditions,
            vec![
                ("level".to_string(), "INFO".to_string()),
                ("component".to_string(), "Draco".to_string()),
            ]
        );
        assert_eq!(text, None);
    }

    #[test]
    fn test_parse_field_filter_expr_with_text() {
        let (conditions, text) =
            parse_field_filter_expr("level:INFO\u{1F}component:Draco\u{1F}\u{02}Power measuments:")
                .unwrap();
        assert_eq!(
            conditions,
            vec![
                ("level".to_string(), "INFO".to_string()),
                ("component".to_string(), "Draco".to_string()),
            ]
        );
        assert_eq!(text.as_deref(), Some("Power measuments:"));
    }

    #[test]
    fn test_parse_field_filter_expr_malformed_condition_errors() {
        assert!(parse_field_filter_expr("level:INFO\u{1F}levelonly").is_err());
    }

    #[test]
    fn test_encode_field_filter_single_condition_matches_old_format() {
        let encoded = encode_field_filter(&[("level".to_string(), "INFO".to_string())], None);
        assert_eq!(encoded, "@field:level:INFO");
    }

    #[test]
    fn test_encode_field_filter_round_trips_multiple_conditions_and_text() {
        let conditions = vec![
            ("level".to_string(), "INFO".to_string()),
            ("component".to_string(), "Draco".to_string()),
        ];
        let encoded = encode_field_filter(&conditions, Some("Power measuments:"));
        let expr = encoded.strip_prefix(FIELD_PREFIX).unwrap();
        let (parsed_conditions, parsed_text) = parse_field_filter_expr(expr).unwrap();
        assert_eq!(parsed_conditions, conditions);
        assert_eq!(parsed_text.as_deref(), Some("Power measuments:"));
    }

    // ── field_filter_matches ─────────────────────────────────────────────────

    fn compound(conditions: &[(&str, &str)], text: Option<&str>) -> FieldFilter {
        FieldFilter {
            conditions: conditions
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            text: text.map(|t| t.to_string()),
            decision: FilterDecision::Include,
        }
    }

    #[test]
    fn test_field_filter_matches_all_conditions_and_text() {
        let ff = compound(
            &[("level", "INFO"), ("component", "Draco")],
            Some("Power measuments:"),
        );
        let parts = make_parts(Some("INFO"), None, None, None, vec![("component", "Draco")]);
        assert!(field_filter_matches(
            &ff,
            &parts,
            b"INFO Draco Power measuments: everything nominal"
        ));
    }

    #[test]
    fn test_field_filter_matches_one_condition_fails() {
        let ff = compound(&[("level", "INFO"), ("component", "Draco")], None);
        let parts = make_parts(
            Some("INFO"),
            None,
            None,
            None,
            vec![("component", "WireBoard")],
        );
        assert!(!field_filter_matches(&ff, &parts, b"INFO WireBoard hi"));
    }

    #[test]
    fn test_field_filter_matches_text_fails() {
        let ff = compound(&[("level", "INFO")], Some("Power measuments:"));
        let parts = make_parts(Some("INFO"), None, None, None, vec![]);
        assert!(!field_filter_matches(&ff, &parts, b"INFO unrelated text"));
    }

    #[test]
    fn test_field_filter_matches_conditions_only_no_text_required() {
        let ff = compound(&[("level", "INFO")], None);
        let parts = make_parts(Some("INFO"), None, None, None, vec![]);
        assert!(field_filter_matches(&ff, &parts, b"anything at all"));
    }

    // ── field_filters_visible ────────────────────────────────────────────────

    fn make_parts<'a>(
        level: Option<&'a str>,
        timestamp: Option<&'a str>,
        message: Option<&'a str>,
        target: Option<&'a str>,
        extra: Vec<(&'a str, &'a str)>,
    ) -> DisplayParts<'a> {
        use crate::parser::FieldSemantic;
        DisplayParts {
            level,
            timestamp,
            target,
            message,
            extra_fields: extra
                .into_iter()
                .map(|(k, v)| (FieldSemantic::Extra, k, v))
                .collect(),
            ..Default::default()
        }
    }

    fn inc(field: &str, pattern: &str) -> FieldFilter {
        FieldFilter {
            conditions: vec![(field.to_string(), pattern.to_string())],
            text: None,
            decision: FilterDecision::Include,
        }
    }

    fn exc(field: &str, pattern: &str) -> FieldFilter {
        FieldFilter {
            conditions: vec![(field.to_string(), pattern.to_string())],
            text: None,
            decision: FilterDecision::Exclude,
        }
    }

    // ── any_field_exclude_matches ─────────────────────────────────────────────

    #[test]
    fn test_exclude_match_hides() {
        let parts = make_parts(Some("debug"), None, None, None, vec![]);
        assert!(any_field_exclude_matches(
            &[exc("level", "debug")],
            Some(&parts),
            b""
        ));
    }

    #[test]
    fn test_exclude_no_match_passes() {
        let parts = make_parts(Some("info"), None, None, None, vec![]);
        assert!(!any_field_exclude_matches(
            &[exc("level", "debug")],
            Some(&parts),
            b""
        ));
    }

    #[test]
    fn test_exclude_parts_none_passthrough() {
        assert!(!any_field_exclude_matches(
            &[exc("level", "debug")],
            None,
            b""
        ));
    }

    #[test]
    fn test_exclude_field_absent_passthrough() {
        // Field does not exist in parts → pass through (not excluded)
        let parts = make_parts(None, None, None, None, vec![]);
        assert!(!any_field_exclude_matches(
            &[exc("level", "debug")],
            Some(&parts),
            b""
        ));
    }

    // ── field_include_vote ────────────────────────────────────────────────────

    #[test]
    fn test_include_match_vote() {
        let parts = make_parts(Some("error"), None, None, None, vec![]);
        assert_eq!(
            field_include_vote(&[inc("level", "error")], Some(&parts), b""),
            FieldVote::Match
        );
    }

    #[test]
    fn test_include_no_match_vote_miss() {
        let parts = make_parts(Some("info"), None, None, None, vec![]);
        assert_eq!(
            field_include_vote(&[inc("level", "error")], Some(&parts), b""),
            FieldVote::Miss
        );
    }

    #[test]
    fn test_include_parts_none_passthrough() {
        assert_eq!(
            field_include_vote(&[inc("level", "error")], None, b""),
            FieldVote::PassThrough
        );
    }

    #[test]
    fn test_include_field_absent_is_miss() {
        // Parsed line where the named field is absent → Miss (not PassThrough).
        // This ensures `filter --field level=error` hides lines that have no `level`.
        let parts = make_parts(None, None, None, None, vec![]);
        assert_eq!(
            field_include_vote(&[inc("level", "error")], Some(&parts), b""),
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
                Some(&parts),
                b""
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
                Some(&parts),
                b""
            ),
            FieldVote::Miss
        );
    }

    #[test]
    fn test_compound_filter_requires_all_parts_others_still_or() {
        // The actual user scenario: `:filter --field level=INFO --field
        // component=Draco Power measuments:` produces ONE compound filter
        // whose 2 field conditions and text must ALL match (AND). A second,
        // separate, unrelated filter alongside it still ORs in as usual.
        let compound = compound(
            &[("level", "INFO"), ("component", "Draco")],
            Some("Power measuments:"),
        );
        let unrelated = inc("target", "auth");
        let includes = [compound, unrelated];

        // Matches every part of the compound filter -> Match.
        let parts = make_parts(Some("INFO"), None, None, None, vec![("component", "Draco")]);
        assert_eq!(
            field_include_vote(
                &includes,
                Some(&parts),
                b"INFO Draco Power measuments: nominal"
            ),
            FieldVote::Match
        );

        // Same fields match, but the text is missing -> compound filter
        // fails, and the unrelated filter (target=auth) doesn't apply here
        // either -> Miss overall.
        assert_eq!(
            field_include_vote(&includes, Some(&parts), b"INFO Draco unrelated text"),
            FieldVote::Miss
        );

        // The unrelated filter alone still matches independently (OR
        // semantics across separate filters is unaffected).
        let parts_auth = make_parts(None, None, None, Some("auth"), vec![]);
        assert_eq!(
            field_include_vote(&includes, Some(&parts_auth), b"anything"),
            FieldVote::Match
        );
    }

    #[test]
    fn test_extra_field_by_key_match() {
        let parts = make_parts(None, None, None, None, vec![("component", "auth")]);
        assert_eq!(
            field_include_vote(&[inc("component", "auth")], Some(&parts), b""),
            FieldVote::Match
        );
    }

    #[test]
    fn test_extra_field_by_key_miss() {
        let parts = make_parts(None, None, None, None, vec![("component", "auth")]);
        assert_eq!(
            field_include_vote(&[inc("component", "api")], Some(&parts), b""),
            FieldVote::Miss
        );
    }

    // ── alias resolution ─────────────────────────────────────────────────────

    #[test]
    fn test_alias_lvl() {
        let parts = make_parts(Some("warn"), None, None, None, vec![]);
        assert_eq!(
            field_include_vote(&[inc("lvl", "warn")], Some(&parts), b""),
            FieldVote::Match
        );
    }

    #[test]
    fn test_alias_ts() {
        let parts = make_parts(None, Some("2024-01-01"), None, None, vec![]);
        assert_eq!(
            field_include_vote(&[inc("ts", "2024")], Some(&parts), b""),
            FieldVote::Match
        );
    }

    #[test]
    fn test_alias_msg() {
        let parts = make_parts(None, None, Some("hello world"), None, vec![]);
        assert_eq!(
            field_include_vote(&[inc("msg", "hello")], Some(&parts), b""),
            FieldVote::Match
        );
    }

    // ── dotted path resolution ────────────────────────────────────────────────

    fn make_parts_with_span<'a>(
        extra: Vec<(&'a str, &'a str)>,
        span_fields: Vec<(&'a str, &'a str)>,
    ) -> DisplayParts<'a> {
        use crate::parser::{FieldSemantic, SpanInfo};
        DisplayParts {
            span: Some(SpanInfo {
                name: "req",
                fields: span_fields,
            }),
            extra_fields: extra
                .into_iter()
                .map(|(k, v)| (FieldSemantic::Extra, k, v))
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn test_span_dotted_path_match() {
        let parts = make_parts_with_span(vec![], vec![("method", "GET")]);
        assert_eq!(
            field_include_vote(&[inc("span.method", "GET")], Some(&parts), b""),
            FieldVote::Match
        );
    }

    #[test]
    fn test_span_name_field_resolves() {
        let parts = make_parts_with_span(vec![], vec![("method", "GET")]);
        assert_eq!(resolve_field("span.name", &parts), Some("req"));
    }

    #[test]
    fn test_span_dotted_path_miss() {
        let parts = make_parts_with_span(vec![], vec![("method", "POST")]);
        assert_eq!(
            field_include_vote(&[inc("span.method", "GET")], Some(&parts), b""),
            FieldVote::Miss
        );
    }

    #[test]
    fn test_span_dotted_path_absent_key() {
        let parts = make_parts_with_span(vec![], vec![("uri", "/")]);
        assert_eq!(
            field_include_vote(&[inc("span.method", "GET")], Some(&parts), b""),
            FieldVote::Miss
        );
    }

    #[test]
    fn test_fields_dotted_path_match() {
        // tracing-subscriber inlines "fields" container into extra_fields with bare keys
        let parts = make_parts(None, None, None, None, vec![("order_id", "42")]);
        assert_eq!(
            field_include_vote(&[inc("fields.order_id", "42")], Some(&parts), b""),
            FieldVote::Match
        );
    }

    #[test]
    fn test_fields_dotted_path_miss() {
        let parts = make_parts(None, None, None, None, vec![("order_id", "99")]);
        assert_eq!(
            field_include_vote(&[inc("fields.order_id", "42")], Some(&parts), b""),
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
            use_regex: false,
            group: None,
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
        assert_eq!(
            inc[0].conditions,
            vec![("level".to_string(), "error".to_string())]
        );
        assert_eq!(
            exc[0].conditions,
            vec![("level".to_string(), "debug".to_string())]
        );
    }

    #[test]
    fn test_extract_field_filters_skips_highlight() {
        let defs = vec![make_def(
            1,
            "@field:level:error",
            FilterType::Highlight,
            true,
        )];
        let (inc, exc) = extract_field_filters(&defs);
        assert!(inc.is_empty(), "highlight must not join the include vote");
        assert!(exc.is_empty(), "highlight must not join the exclude vote");
    }

    // ── extract_field_filters_ordered ─────────────────────────────────────────

    #[test]
    fn test_extract_field_filters_ordered_includes_highlight() {
        // Contrast with test_extract_field_filters_skips_highlight: counting
        // stays type-agnostic even though visibility voting excludes Highlight.
        let defs = vec![make_def(
            1,
            "@field:level:error",
            FilterType::Highlight,
            true,
        )];
        let ordered = extract_field_filters_ordered(&defs);
        assert_eq!(
            ordered
                .iter()
                .map(|f| f.conditions.clone())
                .collect::<Vec<_>>(),
            vec![vec![("level".to_string(), "error".to_string())]]
        );
    }

    #[test]
    fn test_extract_field_filters_ordered_preserves_order() {
        let defs = vec![
            make_def(1, "@field:level:error", FilterType::Include, true),
            make_def(2, "@field:level:debug", FilterType::Exclude, true),
            make_def(3, "@field:target:api", FilterType::Include, true),
        ];
        let ordered = extract_field_filters_ordered(&defs);
        assert_eq!(ordered.len(), 3);
        assert_eq!(
            ordered[0].conditions,
            vec![("level".to_string(), "error".to_string())]
        );
        assert_eq!(
            ordered[1].conditions,
            vec![("level".to_string(), "debug".to_string())]
        );
        assert_eq!(
            ordered[2].conditions,
            vec![("target".to_string(), "api".to_string())]
        );
    }

    #[test]
    fn test_extract_field_filters_ordered_skips_disabled() {
        let defs = vec![
            make_def(1, "@field:level:error", FilterType::Include, true),
            make_def(2, "@field:level:debug", FilterType::Exclude, false),
        ];
        let ordered = extract_field_filters_ordered(&defs);
        assert_eq!(ordered.len(), 1);
        assert_eq!(ordered[0].conditions[0].0, "level");
    }

    // ── count_field_filter_matches ────────────────────────────────────────────

    #[test]
    fn test_count_field_filter_matches_increments_on_match() {
        let parts = make_parts(Some("error"), None, None, None, vec![]);
        let filters = vec![inc("level", "error")];
        let mut counts = vec![0usize];
        count_field_filter_matches(&filters, Some(&parts), b"", &mut counts);
        assert_eq!(counts[0], 1);
    }

    #[test]
    fn test_count_field_filter_matches_no_increment_on_miss() {
        let parts = make_parts(Some("info"), None, None, None, vec![]);
        let filters = vec![inc("level", "error")];
        let mut counts = vec![0usize];
        count_field_filter_matches(&filters, Some(&parts), b"", &mut counts);
        assert_eq!(counts[0], 0);
    }

    #[test]
    fn test_count_field_filter_matches_no_parts_skips() {
        let filters = vec![inc("level", "error")];
        let mut counts = vec![0usize];
        count_field_filter_matches(&filters, None, b"", &mut counts);
        assert_eq!(counts[0], 0);
    }

    #[test]
    fn test_count_field_filter_matches_multiple_filters() {
        let parts = make_parts(Some("error"), None, Some("crash"), None, vec![]);
        let filters = vec![
            inc("level", "error"),
            inc("message", "crash"),
            inc("level", "debug"),
        ];
        let mut counts = vec![0usize; 3];
        count_field_filter_matches(&filters, Some(&parts), b"", &mut counts);
        assert_eq!(counts[0], 1);
        assert_eq!(counts[1], 1);
        assert_eq!(counts[2], 0);
    }
}
