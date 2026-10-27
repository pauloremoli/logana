use crate::commands::auto_complete::regex_search_match;
use crate::filters::{FilterDef, FilterType};

/// Splits a filter row into `(prefix, group_tag, value, suffix)`, where
/// `value` is the filter's pattern text (the part the filter's color
/// highlight should apply to), `group_tag` is the `[name] ` tag (styled with
/// the group's own predefined color, if any — kept separate from `prefix` so
/// it can be colored independently), and `prefix`/`suffix` are the remaining
/// metadata (checkbox, type, field tag, match count).
pub(crate) fn filter_row_parts(
    filter: &FilterDef,
    idx: usize,
    selected: usize,
    match_counts: &[usize],
) -> (String, String, String, String) {
    let status = if filter.enabled { "[x]" } else { "[ ]" };
    let selected_prefix = if idx == selected { ">" } else { " " };
    let is_date = filter.pattern.starts_with(crate::filters::DATE_PREFIX);
    let is_field = filter.pattern.starts_with(crate::filters::FIELD_PREFIX);
    let filter_type_str = if is_date {
        "Date"
    } else {
        match filter.filter_type {
            FilterType::Include => "In",
            FilterType::Exclude => "Out",
            FilterType::Highlight => "H",
        }
    };
    let (display_pattern, field_tag) = if is_date {
        (
            filter.pattern[crate::filters::DATE_PREFIX.len()..].to_string(),
            "",
        )
    } else if is_field {
        let expr = &filter.pattern[crate::filters::FIELD_PREFIX.len()..];
        let value = match crate::filters::parse_field_filter_expr(expr) {
            Ok((conditions, text)) => {
                let mut parts: Vec<String> =
                    conditions.iter().map(|(k, v)| format!("{k}={v}")).collect();
                if let Some(t) = text {
                    parts.push(t);
                }
                parts.join(", ")
            }
            Err(_) => expr.to_string(),
        };
        (
            value,
            if crate::filters::field_filter_is_exact(expr) {
                " [field exact]"
            } else {
                " [field]"
            },
        )
    } else {
        (filter.pattern.clone(), "")
    };
    let group_tag = filter
        .group
        .as_deref()
        .map(|g| format!("[{g}] "))
        .unwrap_or_default();
    let ignore_case_tag = if filter.ignore_case { " [i]" } else { "" };
    let count_str = if filter.enabled {
        let count = match_counts.get(idx).copied().unwrap_or(0);
        format!(" ({})", count)
    } else {
        String::new()
    };
    let prefix = format!("{}{} {}: ", selected_prefix, status, filter_type_str);
    let suffix = format!("{}{}{}", field_tag, ignore_case_tag, count_str);
    (prefix, group_tag, display_pattern, suffix)
}

/// Returns the plain display text for a filter row (no styling).
/// Used both for rendering and for hit-testing wrapped sidebar rows.
pub fn filter_row_display_text(
    filter: &FilterDef,
    idx: usize,
    selected: usize,
    match_counts: &[usize],
) -> String {
    let (prefix, group_tag, value, suffix) = filter_row_parts(filter, idx, selected, match_counts);
    format!("{prefix}{group_tag}{value}{suffix}")
}

/// Indices (into `filters`) of entries whose rendered row text matches the
/// `search` regex, in original list order. Empty `search` means "everything
/// matches" — used both to narrow what the sidebar shows while searching,
/// and by `FilterManagementMode` to interpret navigation keys against the
/// same narrowed list while searching.
pub fn narrowed_filter_indices(
    filters: &[FilterDef],
    match_counts: &[usize],
    search: &str,
) -> Vec<usize> {
    if search.is_empty() {
        return (0..filters.len()).collect();
    }
    filters
        .iter()
        .enumerate()
        .filter(|(idx, filter)| {
            let text = filter_row_display_text(filter, *idx, *idx, match_counts);
            regex_search_match(search, &text)
        })
        .map(|(idx, _)| idx)
        .collect()
}

/// Names (from `group_names`) matching the `search` regex, in original list
/// order. Empty `search` means "everything matches" — `GroupManagementMode`'s
/// counterpart to `narrowed_filter_indices`, returning names directly since
/// groups are selected by name rather than index.
pub fn narrowed_group_names(group_names: &[String], search: &str) -> Vec<String> {
    if search.is_empty() {
        return group_names.to_vec();
    }
    group_names
        .iter()
        .filter(|name| regex_search_match(search, name))
        .cloned()
        .collect()
}
