pub mod date_filter;
pub mod field_filter;
pub mod manager;

pub use date_filter::{
    CanonicalTs, ComparisonMode, ComparisonOp, DATE_PREFIX, DateBound, DateFilter, DateFilterStyle,
    canonical_timestamp, extract_date_filters, parse_date_filter, timestamp_to_canonical,
};
pub(crate) use date_filter::{bsd_month_from_timestamp, system_time_to_date};
pub use field_filter::{
    FIELD_PREFIX, FieldFilter, FieldFilterStyle, FieldVote, any_field_exclude_matches,
    count_field_filter_matches, encode_field_filter, extract_field_filters,
    extract_field_filters_ordered, field_filter_matches, field_include_vote, parse_field_filter,
    parse_field_filter_expr, resolve_field,
};
pub use manager::{
    CURRENT_SEARCH_STYLE_ID, ColorConfig, Filter, FilterDecision, FilterDef, FilterInsertOptions,
    FilterManager, FilterOptions, FilterType, MatchCollector, MatchSpan, RegexFilter,
    SEARCH_STYLE_ID, StyleId, SubstringFilter, build_filter, is_regex_pattern, known_groups,
    render_line,
};
