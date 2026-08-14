use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
};

use crate::commands::auto_complete::regex_search_match;
use crate::filters::{FilterDef, FilterType, GroupDef};
use crate::theme::Theme;
use crate::ui::field_layout::line_row_count;

pub struct Sidebar<'a> {
    pub filters: &'a [FilterDef],
    pub groups: &'a [GroupDef],
    /// Full, unnarrowed filter list — used for the Groups section's per-group
    /// counts, which must not shrink just because a sidebar search narrowed
    /// `filters` to a subset.
    pub all_filters: &'a [FilterDef],
    /// `LogManager::group_names()` — every known group, sorted.
    pub group_names: &'a [String],
    pub match_counts: &'a [usize],
    pub selected_filter_idx: usize,
    /// Name of the group selected in `GroupManagementMode`, or `None` when
    /// not in that mode (the Groups section shows no highlight then).
    pub selected_group: Option<&'a str>,
    pub filter_enabled: bool,
    pub show_marks_only: bool,
    pub filter_progress: Option<usize>,
    pub show_borders: bool,
    pub is_filter_mode: bool,
    pub is_group_mode: bool,
    /// Whether the bottom Groups section renders at all — toggled via `:ui`.
    pub show_groups: bool,
    /// Row offset to scroll the filter list by, as computed by
    /// [`compute_scroll_offset`] and persisted across frames by the caller.
    pub scroll_offset: usize,
    /// When true, filters act as pure highlighters (visibility bypassed);
    /// shown as a `[HIGHLIGHT]` marker in the title.
    pub highlight_mode: bool,
    /// Live typeahead query narrowing `filters`; shown in the title, empty
    /// when not searching or before anything has been typed.
    pub search: &'a str,
    /// True while search is capturing input. Distinct from `!search.is_empty()`
    /// so the title can show a "type to search..." placeholder the instant
    /// `/` is pressed, before any character is typed — otherwise there's no
    /// visible cue that the app is now waiting for search text.
    pub searching: bool,
    /// Live typeahead query narrowing `group_names`; `GroupManagementMode`'s
    /// counterpart to `search`. Shown in the Groups section title.
    pub group_search: &'a str,
    /// True while group search is capturing input; `GroupManagementMode`'s
    /// counterpart to `searching`.
    pub group_searching: bool,
    pub theme: &'a Theme,
}

/// Splits a filter row into `(prefix, group_tag, value, suffix)`, where
/// `value` is the filter's pattern text (the part the filter's color
/// highlight should apply to), `group_tag` is the `[name] ` tag (styled with
/// the group's own predefined color, if any — kept separate from `prefix` so
/// it can be colored independently), and `prefix`/`suffix` are the remaining
/// metadata (checkbox, type, field tag, match count).
fn filter_row_parts(
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
        (value, " [field]")
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

fn build_filter_row(
    filter: &FilterDef,
    idx: usize,
    selected: usize,
    match_counts: &[usize],
    theme: &Theme,
    groups: &[GroupDef],
) -> Line<'static> {
    let (prefix, group_tag, value, suffix) = filter_row_parts(filter, idx, selected, match_counts);
    let mut default_style = Style::default().fg(theme.text);
    if idx == selected {
        default_style = default_style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
    }
    let mut value_style = default_style;
    if let Some(cfg) = crate::filters::effective_color_config(filter, groups) {
        if let Some(fg) = cfg.fg {
            value_style = value_style.fg(fg);
        }
        if let Some(bg) = cfg.bg {
            value_style = value_style.bg(bg);
        }
    }
    let mut group_tag_style = default_style;
    if let Some(cfg) = filter
        .group
        .as_deref()
        .and_then(|name| crate::filters::group_style(groups, name))
    {
        if let Some(fg) = cfg.fg {
            group_tag_style = group_tag_style.fg(fg);
        }
        if let Some(bg) = cfg.bg {
            group_tag_style = group_tag_style.bg(bg);
        }
    }
    Line::from(vec![
        Span::styled(prefix, default_style),
        Span::styled(group_tag, group_tag_style),
        Span::styled(value, value_style),
        Span::styled(suffix, default_style),
    ])
}

/// Tri-state: all enabled → `Some(true)`, all disabled or no filters →
/// `Some(false)`, mixed → `None`.
fn group_toggle_state(name: &str, all_filters: &[FilterDef]) -> Option<bool> {
    let mut members = all_filters
        .iter()
        .filter(|f| f.group.as_deref() == Some(name));
    let Some(first) = members.next() else {
        return Some(false);
    };
    if members.all(|f| f.enabled == first.enabled) {
        Some(first.enabled)
    } else {
        None
    }
}

/// Builds one Groups-section row: checkbox, name, count. Checkbox stays
/// plain-styled so it doesn't compete with the group's own color.
fn build_group_row(
    name: &str,
    all_filters: &[FilterDef],
    groups: &[GroupDef],
    is_selected: bool,
    theme: &Theme,
) -> Line<'static> {
    let count = all_filters
        .iter()
        .filter(|f| f.group.as_deref() == Some(name))
        .count();
    let status = match group_toggle_state(name, all_filters) {
        Some(true) => "[x]",
        Some(false) => "[ ]",
        None => "[-]",
    };
    let mut default_style = Style::default().fg(theme.text);
    if is_selected {
        default_style = default_style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
    }
    let mut name_style = default_style;
    if let Some(cfg) = crate::filters::group_style(groups, name) {
        if let Some(fg) = cfg.fg {
            name_style = name_style.fg(fg);
        }
        if let Some(bg) = cfg.bg {
            name_style = name_style.bg(bg);
        }
    }
    Line::from(vec![
        Span::styled(format!("{status} "), default_style),
        Span::styled(format!("{name} ({count})"), name_style),
    ])
}

/// Appends the "type to search..." / "/query" cue shared by the Filters and
/// Groups section titles. Gated on `searching` rather than `search.is_empty()`:
/// the moment right after pressing `/` has an empty query but must still show
/// a visible cue that the app is now capturing search text. The placeholder
/// is dimmed to read as a hint rather than active content; the query itself
/// keeps the normal title style once something's been typed.
fn push_search_span(
    spans: &mut Vec<Span<'static>>,
    search: &str,
    searching: bool,
    title_style: Style,
) {
    if !searching {
        return;
    }
    if search.is_empty() {
        spans.push(Span::styled(
            " type to search...",
            title_style.add_modifier(Modifier::DIM),
        ));
    } else {
        spans.push(Span::styled(format!(" /{search}"), title_style));
    }
}

/// Builds the Groups section's title, mirroring [`build_sidebar_title`]'s
/// `"Filters [n/n]"` shape with `"Groups [n]"` (groups have no per-group
/// enabled/disabled count to show, unlike filters).
fn build_groups_title(
    group_count: usize,
    search: &str,
    searching: bool,
    title_style: Style,
) -> Line<'static> {
    let mut spans = vec![Span::styled(format!("Groups [{group_count}]"), title_style)];
    push_search_span(&mut spans, search, searching, title_style);
    Line::from(spans)
}

/// The Groups section never shrinks below this many rows (label included)
/// while it's shown, even with zero groups — keeps its position stable
/// instead of the sidebar jumping around as groups are added/removed.
const MIN_GROUPS_SECTION_HEIGHT: usize = 8;

/// Splits the sidebar's total inner content height into `(filters_height,
/// groups_height)`, reserving at least one row for the filter list.
/// `groups_height` includes one row for the Groups section's own label and
/// never drops below [`MIN_GROUPS_SECTION_HEIGHT`] — callers that want the
/// section hidden entirely must skip this and use `(inner_height, 0)`
/// directly rather than passing `group_count: 0`. Shared by
/// [`Sidebar::render`] and `hit_test_sidebar` so hit-testing agrees with
/// what was rendered.
pub(crate) fn split_sidebar_heights(inner_height: usize, group_count: usize) -> (usize, usize) {
    let desired = (group_count + 1).max(MIN_GROUPS_SECTION_HEIGHT);
    let groups_height = desired.min(inner_height.saturating_sub(1));
    (inner_height.saturating_sub(groups_height), groups_height)
}

/// Content (width, height) of the sidebar's scrollable area for a given
/// outer `area`, matching the block layout built by [`Sidebar::render`]
/// (a title row, plus a 1-cell border or padding gutter on each side).
///
/// Shared with [`crate::ui::input_handler::InputHandler::hit_test_sidebar`]
/// so hit-testing agrees with what was actually rendered.
pub(crate) fn sidebar_inner_dims(area: Rect, show_borders: bool) -> (usize, usize) {
    if show_borders {
        (
            area.width.saturating_sub(2) as usize,
            area.height.saturating_sub(2) as usize,
        )
    } else {
        (
            area.width.saturating_sub(1) as usize,
            area.height.saturating_sub(1) as usize,
        )
    }
}

/// Row offset (in wrapped display rows) to scroll the filter list by, given
/// the viewport it was previously scrolled to (`prev_scroll`). Scrolls just
/// enough to bring the selected filter into a `content_h`-row viewport at
/// wrap width `content_w`; already-visible selections leave the offset
/// unchanged. `hit_test_sidebar` replicates this exact scroll to map a
/// click's screen row back to a filter index.
pub(crate) fn compute_scroll_offset(
    filters: &[FilterDef],
    selected: usize,
    match_counts: &[usize],
    content_w: usize,
    content_h: usize,
    prev_scroll: usize,
) -> usize {
    if content_w == 0 || content_h == 0 {
        return 0;
    }
    let mut row = 0usize;
    let mut selected_start = 0usize;
    let mut selected_end = 0usize;
    for (i, filter) in filters.iter().enumerate() {
        let text = filter_row_display_text(filter, i, selected, match_counts);
        let row_h = line_row_count(text.as_bytes(), content_w);
        if i == selected {
            selected_start = row;
        }
        row += row_h;
        if i == selected {
            selected_end = row;
        }
    }
    let total_rows = row;

    let scroll = if selected_start < prev_scroll {
        selected_start
    } else if selected_end > prev_scroll + content_h {
        selected_end.saturating_sub(content_h)
    } else {
        prev_scroll
    };
    scroll.min(total_rows.saturating_sub(content_h))
}

/// Builds the sidebar's title as a styled [`Line`] rather than a plain
/// `String` so the search placeholder can be dimmed independently of the
/// rest of the title, which otherwise all shares `title_style`.
#[allow(clippy::too_many_arguments)]
fn build_sidebar_title(
    filter_enabled: bool,
    show_marks_only: bool,
    highlight_mode: bool,
    filter_progress: Option<usize>,
    active_count: usize,
    total_count: usize,
    search: &str,
    searching: bool,
    title_style: Style,
) -> Line<'static> {
    let filter_count_suffix = if total_count > 0 {
        format!(" [{}/{}]", active_count, total_count)
    } else {
        String::new()
    };
    let mut prefix = if show_marks_only {
        format!("Filters [MARKS ONLY]{}", filter_count_suffix)
    } else if filter_enabled {
        format!("Filters{}", filter_count_suffix)
    } else {
        format!("Filters [OFF]{}", filter_count_suffix)
    };
    if highlight_mode {
        prefix.push_str(" [HIGHLIGHT]");
    }

    let mut spans = vec![Span::styled(prefix, title_style)];
    push_search_span(&mut spans, search, searching, title_style);

    let progress_suffix = match filter_progress {
        Some(pct) if pct < 100 => Some(format!(" {pct}%")),
        Some(_) => Some(" Indexing\u{2026}".to_string()),
        None => None,
    };
    if let Some(suffix) = progress_suffix {
        spans.push(Span::styled(suffix, title_style));
    }

    Line::from(spans)
}

impl<'a> Widget for Sidebar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let filters_text: Vec<Line> = self
            .filters
            .iter()
            .enumerate()
            .map(|(i, filter)| {
                build_filter_row(
                    filter,
                    i,
                    self.selected_filter_idx,
                    self.match_counts,
                    self.theme,
                    self.groups,
                )
            })
            .collect();

        let active_count = self.filters.iter().filter(|f| f.enabled).count();
        let total_count = self.filters.len();
        let title_style = if self.is_filter_mode {
            Style::default().fg(self.theme.text_highlight_fg)
        } else {
            Style::default().fg(self.theme.border_title)
        };
        let sidebar_title = build_sidebar_title(
            self.filter_enabled,
            self.show_marks_only,
            self.highlight_mode,
            self.filter_progress,
            active_count,
            total_count,
            self.search,
            self.searching,
            title_style,
        );

        let sidebar_block = if self.show_borders {
            let border_style = if self.is_filter_mode {
                Style::default().fg(self.theme.text_highlight_fg)
            } else {
                Style::default().fg(self.theme.border)
            };
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style)
                .title(sidebar_title)
        } else {
            Block::default()
                .borders(Borders::NONE)
                .padding(Padding::new(1, 0, 0, 0))
                .title(sidebar_title)
        };

        let inner_area = sidebar_block.inner(area);
        sidebar_block.render(area, buf);

        let (_, groups_height) = if self.show_groups {
            split_sidebar_heights(inner_area.height as usize, self.group_names.len())
        } else {
            (inner_area.height as usize, 0)
        };
        let [filters_area, groups_area] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(groups_height as u16)])
            .areas(inner_area);

        Paragraph::new(filters_text)
            .wrap(Wrap { trim: false })
            .scroll((self.scroll_offset.min(u16::MAX as usize) as u16, 0))
            .render(filters_area, buf);

        if groups_height > 0 {
            // Narrowed for display only — the section's own height stays
            // anchored to the full group count (above) so it doesn't resize
            // as the user types a query, mirroring why `MIN_GROUPS_SECTION_HEIGHT`
            // exists in the first place.
            let narrowed_names = narrowed_group_names(self.group_names, self.group_search);
            let groups_title_style = if self.is_group_mode {
                Style::default().fg(self.theme.text_highlight_fg)
            } else {
                Style::default().fg(self.theme.border_title)
            };
            let groups_block = Block::default()
                .borders(Borders::NONE)
                .title(build_groups_title(
                    narrowed_names.len(),
                    self.group_search,
                    self.group_searching,
                    groups_title_style,
                ));
            let groups_inner = groups_block.inner(groups_area);
            groups_block.render(groups_area, buf);

            let groups_text: Vec<Line> = narrowed_names
                .iter()
                .map(|name| {
                    let is_selected =
                        self.is_group_mode && self.selected_group == Some(name.as_str());
                    build_group_row(name, self.all_filters, self.groups, is_selected, self.theme)
                })
                .collect();
            Paragraph::new(groups_text).render(groups_inner, buf);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filters::{FilterDef, FilterType};
    use crate::theme::Theme;
    use ratatui::{Terminal, backend::TestBackend};

    fn make_filter(pattern: &str, enabled: bool, filter_type: FilterType) -> FilterDef {
        FilterDef {
            id: 0,
            pattern: pattern.to_string(),
            enabled,
            filter_type,
            color_config: None,
            use_regex: false,
            ignore_case: false,
            group: None,
        }
    }

    #[test]
    fn test_sidebar_filter_row_positions_no_borders() {
        let theme = Theme::default();
        let filters = vec![
            make_filter("foo", true, FilterType::Include),
            make_filter("bar", true, FilterType::Include),
        ];
        let sidebar = Sidebar {
            filters: &filters,
            groups: &[],
            all_filters: &[],
            group_names: &[],
            selected_group: None,
            is_group_mode: false,
            show_groups: false,
            match_counts: &[1, 2],
            selected_filter_idx: 0,
            filter_enabled: true,
            show_marks_only: false,
            filter_progress: None,
            show_borders: false,
            is_filter_mode: false,
            scroll_offset: 0,
            highlight_mode: false,
            search: "",
            searching: false,
            group_search: "",
            group_searching: false,
            theme: &theme,
        };
        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
        let buf = terminal
            .draw(|f| f.render_widget(sidebar, f.area()))
            .unwrap();
        let row_text = |row: u16| -> String {
            (0..40u16)
                .map(|c| {
                    buf.buffer
                        .cell(ratatui::prelude::Position::new(c, row))
                        .unwrap()
                        .symbol()
                        .to_string()
                })
                .collect::<String>()
                .trim_end()
                .to_string()
        };
        // row 0 is the title; filter 0 starts at row 1, filter 1 at row 2
        assert!(row_text(0).contains("Filters"), "row 0 should be title");
        assert!(row_text(1).contains("foo"), "filter 0 should be at row 1");
        assert!(row_text(2).contains("bar"), "filter 1 should be at row 2");
    }

    #[test]
    fn test_sidebar_renders_highlight_mode_title() {
        let theme = Theme::default();
        let sidebar = Sidebar {
            filters: &[],
            groups: &[],
            all_filters: &[],
            group_names: &[],
            selected_group: None,
            is_group_mode: false,
            show_groups: true,
            match_counts: &[],
            selected_filter_idx: 0,
            filter_enabled: true,
            show_marks_only: false,
            filter_progress: None,
            show_borders: false,
            is_filter_mode: false,
            scroll_offset: 0,
            highlight_mode: true,
            search: "",
            searching: false,
            group_search: "",
            group_searching: false,
            theme: &theme,
        };
        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
        let buf = terminal
            .draw(|f| f.render_widget(sidebar, f.area()))
            .unwrap();
        let row_text = |row: u16| -> String {
            (0..40u16)
                .map(|c| {
                    buf.buffer
                        .cell(ratatui::prelude::Position::new(c, row))
                        .unwrap()
                        .symbol()
                        .to_string()
                })
                .collect::<String>()
                .trim_end()
                .to_string()
        };
        assert!(
            row_text(0).contains("HIGHLIGHT"),
            "row 0 should show the highlight-mode marker: {:?}",
            row_text(0)
        );
    }

    #[test]
    fn test_sidebar_scrolls_to_keep_selection_visible() {
        let theme = Theme::default();
        let filters: Vec<FilterDef> = (0..30)
            .map(|i| make_filter(&format!("pattern_{i}"), true, FilterType::Include))
            .collect();
        let match_counts = vec![0; filters.len()];
        // Small terminal: only a handful of rows fit, selection is near the end.
        let selected = 25;
        let (content_w, content_h) = sidebar_inner_dims(
            Rect {
                x: 0,
                y: 0,
                width: 40,
                height: 10,
            },
            false,
        );
        let scroll_offset =
            compute_scroll_offset(&filters, selected, &match_counts, content_w, content_h, 0);
        let sidebar = Sidebar {
            filters: &filters,
            groups: &[],
            all_filters: &[],
            group_names: &[],
            selected_group: None,
            is_group_mode: false,
            show_groups: false,
            match_counts: &match_counts,
            selected_filter_idx: selected,
            filter_enabled: true,
            show_marks_only: false,
            filter_progress: None,
            show_borders: false,
            is_filter_mode: false,
            scroll_offset,
            highlight_mode: false,
            search: "",
            searching: false,
            group_search: "",
            group_searching: false,
            theme: &theme,
        };
        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
        let buf = terminal
            .draw(|f| f.render_widget(sidebar, f.area()))
            .unwrap();
        let screen: String = (0..10u16)
            .map(|row| {
                (0..40u16)
                    .map(|c| {
                        buf.buffer
                            .cell(ratatui::prelude::Position::new(c, row))
                            .unwrap()
                            .symbol()
                            .to_string()
                    })
                    .collect::<String>()
            })
            .collect();
        assert!(
            screen.contains(&format!("pattern_{selected}")),
            "selected filter row must be scrolled into view: {screen:?}"
        );
    }

    #[test]
    fn test_sidebar_renders_without_filters() {
        let theme = Theme::default();
        let sidebar = Sidebar {
            filters: &[],
            groups: &[],
            all_filters: &[],
            group_names: &[],
            selected_group: None,
            is_group_mode: false,
            show_groups: true,
            match_counts: &[],
            selected_filter_idx: 0,
            filter_enabled: true,
            show_marks_only: false,
            filter_progress: None,
            show_borders: true,
            is_filter_mode: false,
            scroll_offset: 0,
            highlight_mode: false,
            search: "",
            searching: false,
            group_search: "",
            group_searching: false,
            theme: &theme,
        };
        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
        terminal
            .draw(|f| f.render_widget(sidebar, f.area()))
            .unwrap();
    }

    #[test]
    fn test_sidebar_renders_with_filters() {
        let theme = Theme::default();
        let filters = vec![
            make_filter("error", true, FilterType::Include),
            make_filter("debug", false, FilterType::Exclude),
        ];
        let match_counts = vec![5, 0];
        let sidebar = Sidebar {
            filters: &filters,
            groups: &[],
            all_filters: &[],
            group_names: &[],
            selected_group: None,
            is_group_mode: false,
            show_groups: true,
            match_counts: &match_counts,
            selected_filter_idx: 0,
            filter_enabled: true,
            show_marks_only: false,
            filter_progress: None,
            show_borders: false,
            is_filter_mode: false,
            scroll_offset: 0,
            highlight_mode: false,
            search: "",
            searching: false,
            group_search: "",
            group_searching: false,
            theme: &theme,
        };
        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
        terminal
            .draw(|f| f.render_widget(sidebar, f.area()))
            .unwrap();
    }

    #[test]
    fn test_build_sidebar_title_marks_only() {
        let title_line =
            build_sidebar_title(true, true, false, None, 2, 4, "", false, Style::default());
        let title: String = title_line
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(title.contains("MARKS ONLY"));
        assert!(title.contains("[2/4]"));
    }

    #[test]
    fn test_build_sidebar_title_disabled() {
        let title_line =
            build_sidebar_title(false, false, false, None, 0, 3, "", false, Style::default());
        let title: String = title_line
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(title.contains("[OFF]"));
        assert!(title.contains("[0/3]"));
    }

    #[test]
    fn test_build_sidebar_title_with_progress() {
        let title_line = build_sidebar_title(
            true,
            false,
            false,
            Some(50),
            1,
            2,
            "",
            false,
            Style::default(),
        );
        let title: String = title_line
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(title.contains("50%"));
    }

    #[test]
    fn test_build_sidebar_title_indexing_complete() {
        let title_line = build_sidebar_title(
            true,
            false,
            false,
            Some(100),
            1,
            2,
            "",
            false,
            Style::default(),
        );
        let title: String = title_line
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(title.contains("Indexing"));
    }

    #[test]
    fn test_build_sidebar_title_highlight_mode() {
        let title_line =
            build_sidebar_title(true, false, true, None, 1, 2, "", false, Style::default());
        let title: String = title_line
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(title.contains("[HIGHLIGHT]"));
    }

    #[test]
    fn test_build_sidebar_title_highlight_mode_with_marks_only() {
        let title_line =
            build_sidebar_title(true, true, true, None, 1, 2, "", false, Style::default());
        let title: String = title_line
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(title.contains("MARKS ONLY"));
        assert!(title.contains("[HIGHLIGHT]"));
    }

    #[test]
    fn test_build_sidebar_title_search_placeholder_is_dimmed() {
        let title_style = Style::default().fg(Color::Cyan);
        let line = build_sidebar_title(true, false, false, None, 1, 2, "", true, title_style);
        let placeholder_span = line
            .spans
            .iter()
            .find(|s| s.content.contains("type to search"))
            .expect("placeholder span should be present while searching with an empty query");
        assert!(placeholder_span.style.add_modifier.contains(Modifier::DIM));
    }

    #[test]
    fn test_build_sidebar_title_search_query_is_not_dimmed() {
        let title_style = Style::default().fg(Color::Cyan);
        let line = build_sidebar_title(true, false, false, None, 1, 2, "err", true, title_style);
        let query_span = line
            .spans
            .iter()
            .find(|s| s.content.contains("/err"))
            .expect("query span should be present while searching with a non-empty query");
        assert!(!query_span.style.add_modifier.contains(Modifier::DIM));
    }

    #[test]
    fn test_build_filter_row_include() {
        let theme = Theme::default();
        let filter = make_filter("hello", true, FilterType::Include);
        let line = build_filter_row(&filter, 0, 0, &[3], &theme, &[]);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains(">[x]"));
        assert!(text.contains("In"));
        assert!(text.contains("hello"));
        assert!(text.contains("(3)"));
    }

    #[test]
    fn test_build_filter_row_highlight() {
        let theme = Theme::default();
        let filter = make_filter("hello", true, FilterType::Highlight);
        let line = build_filter_row(&filter, 0, 0, &[3], &theme, &[]);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains(">[x] H:"));
        assert!(text.contains("hello"));
        assert!(text.contains("(3)"));
    }

    #[test]
    fn test_build_filter_row_color_applies_only_to_value() {
        use crate::filters::ColorConfig;
        let theme = Theme::default();
        let mut filter = make_filter("hello", true, FilterType::Include);
        filter.color_config = Some(ColorConfig {
            fg: Some(ratatui::style::Color::Red),
            bg: None,
            match_only: true,
        });
        let line = build_filter_row(&filter, 0, 0, &[3], &theme, &[]);

        let value_span = line
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "hello")
            .expect("value span present");
        assert_eq!(value_span.style.fg, Some(ratatui::style::Color::Red));

        for span in line.spans.iter().filter(|s| s.content.as_ref() != "hello") {
            assert_ne!(
                span.style.fg,
                Some(ratatui::style::Color::Red),
                "non-value span should not be colored: {:?}",
                span.content
            );
        }
    }

    #[test]
    fn test_build_filter_row_uses_group_style_when_filter_color_is_none() {
        use crate::filters::{ColorConfig, GroupDef};
        let theme = Theme::default();
        let mut filter = make_filter("hello", true, FilterType::Include);
        filter.group = Some("errs".to_string());
        let groups = vec![GroupDef {
            name: "errs".to_string(),
            color_config: Some(ColorConfig {
                fg: Some(ratatui::style::Color::Red),
                bg: None,
                match_only: true,
            }),
        }];
        let line = build_filter_row(&filter, 0, 0, &[3], &theme, &groups);

        let value_span = line
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "hello")
            .expect("value span present");
        assert_eq!(value_span.style.fg, Some(ratatui::style::Color::Red));
    }

    #[test]
    fn test_build_filter_row_filter_color_overrides_group_style() {
        use crate::filters::{ColorConfig, GroupDef};
        let theme = Theme::default();
        let mut filter = make_filter("hello", true, FilterType::Include);
        filter.group = Some("errs".to_string());
        filter.color_config = Some(ColorConfig {
            fg: None,
            bg: Some(ratatui::style::Color::Green),
            match_only: true,
        });
        let groups = vec![GroupDef {
            name: "errs".to_string(),
            color_config: Some(ColorConfig {
                fg: Some(ratatui::style::Color::Red),
                bg: Some(ratatui::style::Color::Black),
                match_only: true,
            }),
        }];
        let line = build_filter_row(&filter, 0, 0, &[3], &theme, &groups);

        let value_span = line
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "hello")
            .expect("value span present");
        // Filter's own (partial) color_config wins outright — no merge with
        // the group's fg, even though the filter didn't set its own fg.
        assert_eq!(value_span.style.bg, Some(ratatui::style::Color::Green));
        assert_ne!(value_span.style.fg, Some(ratatui::style::Color::Red));
    }

    #[test]
    fn test_build_filter_row_colors_group_tag_when_group_has_style() {
        use crate::filters::{ColorConfig, GroupDef};
        let theme = Theme::default();
        let mut filter = make_filter("hello", true, FilterType::Include);
        filter.group = Some("errs".to_string());
        let groups = vec![GroupDef {
            name: "errs".to_string(),
            color_config: Some(ColorConfig {
                fg: Some(ratatui::style::Color::Red),
                bg: Some(ratatui::style::Color::Black),
                match_only: true,
            }),
        }];
        let line = build_filter_row(&filter, 0, 0, &[3], &theme, &groups);

        let group_tag_span = line
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "[errs] ")
            .expect("group tag span present");
        assert_eq!(group_tag_span.style.fg, Some(ratatui::style::Color::Red));
        assert_eq!(group_tag_span.style.bg, Some(ratatui::style::Color::Black));
    }

    #[test]
    fn test_build_filter_row_group_tag_reflects_group_style_even_when_filter_overrides_value() {
        use crate::filters::{ColorConfig, GroupDef};
        let theme = Theme::default();
        let mut filter = make_filter("hello", true, FilterType::Include);
        filter.group = Some("errs".to_string());
        filter.color_config = Some(ColorConfig {
            fg: Some(ratatui::style::Color::Green),
            bg: None,
            match_only: true,
        });
        let groups = vec![GroupDef {
            name: "errs".to_string(),
            color_config: Some(ColorConfig {
                fg: Some(ratatui::style::Color::Red),
                bg: None,
                match_only: true,
            }),
        }];
        let line = build_filter_row(&filter, 0, 0, &[3], &theme, &groups);

        // The group tag always reflects the group's own style, even though
        // the filter's value uses its own overriding color instead.
        let group_tag_span = line
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "[errs] ")
            .expect("group tag span present");
        assert_eq!(group_tag_span.style.fg, Some(ratatui::style::Color::Red));

        let value_span = line
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "hello")
            .expect("value span present");
        assert_eq!(value_span.style.fg, Some(ratatui::style::Color::Green));
    }

    #[test]
    fn test_build_filter_row_group_tag_default_style_when_group_has_no_style() {
        let theme = Theme::default();
        let mut filter = make_filter("hello", true, FilterType::Include);
        filter.group = Some("errs".to_string());
        let line = build_filter_row(&filter, 0, 0, &[3], &theme, &[]);

        let group_tag_span = line
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "[errs] ")
            .expect("group tag span present");
        assert_eq!(group_tag_span.style.fg, Some(theme.text));
        assert_eq!(group_tag_span.style.bg, None);
    }

    #[test]
    fn test_build_filter_row_selected_is_bold_and_underlined() {
        let theme = Theme::default();
        let filter = make_filter("hello", true, FilterType::Include);
        let selected_line = build_filter_row(&filter, 0, 0, &[3], &theme, &[]);
        for span in &selected_line.spans {
            assert!(
                span.style.add_modifier.contains(Modifier::BOLD),
                "selected row span {:?} should be bold",
                span.content
            );
            assert!(
                span.style.add_modifier.contains(Modifier::UNDERLINED),
                "selected row span {:?} should be underlined",
                span.content
            );
        }

        let unselected_line = build_filter_row(&filter, 1, 0, &[3], &theme, &[]);
        for span in &unselected_line.spans {
            assert!(
                !span.style.add_modifier.contains(Modifier::BOLD),
                "unselected row span {:?} should not be bold",
                span.content
            );
            assert!(
                !span.style.add_modifier.contains(Modifier::UNDERLINED),
                "unselected row span {:?} should not be underlined",
                span.content
            );
        }
    }

    #[test]
    fn test_build_filter_row_exclude_disabled() {
        let theme = Theme::default();
        let filter = make_filter("noise", false, FilterType::Exclude);
        let line = build_filter_row(&filter, 1, 0, &[0, 0], &theme, &[]);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("[ ]"));
        assert!(text.contains("Out"));
        assert!(!text.contains("("));
    }

    #[test]
    fn test_build_filter_row_field_filter() {
        let theme = Theme::default();
        let pattern = format!("{}key:val", crate::filters::FIELD_PREFIX);
        let filter = make_filter(&pattern, true, FilterType::Include);
        let line = build_filter_row(&filter, 0, 0, &[1], &theme, &[]);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("key=val"));
        assert!(text.contains("[field]"));
    }

    #[test]
    fn test_build_filter_row_compound_field_filter_shows_all_conditions_and_text() {
        let theme = Theme::default();
        let pattern = crate::filters::encode_field_filter(
            &[
                ("level".to_string(), "INFO".to_string()),
                ("component".to_string(), "Draco".to_string()),
            ],
            Some("Power measuments:"),
        );
        let filter = make_filter(&pattern, true, FilterType::Include);
        let line = build_filter_row(&filter, 0, 0, &[1], &theme, &[]);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("level=INFO"), "got: {text:?}");
        assert!(text.contains("component=Draco"), "got: {text:?}");
        assert!(text.contains("Power measuments:"), "got: {text:?}");
        assert!(text.contains("[field]"));
    }

    #[test]
    fn test_filter_row_display_text_shows_group() {
        let mut filter = make_filter("ERROR", true, FilterType::Include);
        filter.group = Some("errors".to_string());
        let text = filter_row_display_text(&filter, 0, 0, &[3]);
        assert_eq!(text, ">[x] In: [errors] ERROR (3)");
    }

    #[test]
    fn test_filter_row_display_text_no_group_omits_tag() {
        let filter = make_filter("ERROR", true, FilterType::Include);
        let text = filter_row_display_text(&filter, 0, 0, &[3]);
        assert_eq!(text, ">[x] In: ERROR (3)");
    }

    #[test]
    fn test_filter_row_display_text_shows_ignore_case_tag() {
        let mut filter = make_filter("ERROR", true, FilterType::Include);
        filter.ignore_case = true;
        let text = filter_row_display_text(&filter, 0, 0, &[3]);
        assert_eq!(text, ">[x] In: ERROR [i] (3)");
    }

    #[test]
    fn test_filter_row_display_text_case_sensitive_omits_ignore_case_tag() {
        let filter = make_filter("ERROR", true, FilterType::Include);
        let text = filter_row_display_text(&filter, 0, 0, &[3]);
        assert!(!text.contains("[i]"));
    }

    #[test]
    fn test_narrowed_filter_indices_empty_search_returns_all() {
        let filters = vec![
            make_filter("ERROR", true, FilterType::Include),
            make_filter("timeout", true, FilterType::Exclude),
        ];
        let indices = narrowed_filter_indices(&filters, &[0, 0], "");
        assert_eq!(indices, vec![0, 1]);
    }

    #[test]
    fn test_narrowed_filter_indices_matches_pattern_text() {
        let filters = vec![
            make_filter("ERROR", true, FilterType::Include),
            make_filter("timeout", true, FilterType::Exclude),
            make_filter("errno", true, FilterType::Include),
        ];
        let indices = narrowed_filter_indices(&filters, &[0, 0, 0], "err");
        assert_eq!(indices, vec![0, 2]);
    }

    #[test]
    fn test_narrowed_filter_indices_no_match_returns_empty() {
        let filters = vec![make_filter("ERROR", true, FilterType::Include)];
        let indices = narrowed_filter_indices(&filters, &[0], "zzz");
        assert!(indices.is_empty());
    }

    #[test]
    fn test_narrowed_filter_indices_matches_group_tag() {
        let mut filter = make_filter("ERROR", true, FilterType::Include);
        filter.group = Some("critical".to_string());
        let filters = vec![filter, make_filter("timeout", true, FilterType::Exclude)];
        let indices = narrowed_filter_indices(&filters, &[0, 0], "critical");
        assert_eq!(indices, vec![0]);
    }

    #[test]
    fn test_narrowed_filter_indices_supports_regex_alternation() {
        let filters = vec![
            make_filter("ERROR", true, FilterType::Include),
            make_filter("timeout", true, FilterType::Exclude),
            make_filter("warn", true, FilterType::Include),
        ];
        let indices = narrowed_filter_indices(&filters, &[0, 0, 0], "ERROR|warn");
        assert_eq!(indices, vec![0, 2]);
    }

    #[test]
    fn test_narrowed_filter_indices_invalid_regex_falls_back_to_literal() {
        let filters = vec![
            make_filter("(err)", true, FilterType::Include),
            make_filter("timeout", true, FilterType::Exclude),
        ];
        // "(err" is an unclosed group — invalid regex mid-composition — must
        // fall back to a literal substring search rather than panicking or
        // matching nothing.
        let indices = narrowed_filter_indices(&filters, &[0, 0], "(err");
        assert_eq!(indices, vec![0]);
    }

    #[test]
    fn test_sidebar_filter_mode_active_bold_title_bordered() {
        let theme = Theme::default();
        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
        let buf = terminal
            .draw(|f| {
                f.render_widget(
                    Sidebar {
                        filters: &[],
                        groups: &[],
                        all_filters: &[],
                        group_names: &[],
                        selected_group: None,
                        is_group_mode: false,
                        show_groups: true,
                        match_counts: &[],
                        selected_filter_idx: 0,
                        filter_enabled: true,
                        show_marks_only: false,
                        filter_progress: None,
                        show_borders: true,
                        is_filter_mode: true,
                        scroll_offset: 0,
                        highlight_mode: false,
                        search: "",
                        searching: false,
                        group_search: "",
                        group_searching: false,
                        theme: &theme,
                    },
                    f.area(),
                )
            })
            .unwrap()
            .area;
        assert_eq!(buf, buf);
        let active_style = Style::default().fg(theme.text_highlight_fg);
        let inactive_style = Style::default().fg(theme.border_title);
        assert_ne!(active_style, inactive_style);
    }

    #[test]
    fn test_sidebar_filter_mode_inactive_normal_title_bordered() {
        let theme = Theme::default();
        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
        terminal
            .draw(|f| {
                f.render_widget(
                    Sidebar {
                        filters: &[],
                        groups: &[],
                        all_filters: &[],
                        group_names: &[],
                        selected_group: None,
                        is_group_mode: false,
                        show_groups: true,
                        match_counts: &[],
                        selected_filter_idx: 0,
                        filter_enabled: true,
                        show_marks_only: false,
                        filter_progress: None,
                        show_borders: true,
                        is_filter_mode: false,
                        scroll_offset: 0,
                        highlight_mode: false,
                        search: "",
                        searching: false,
                        group_search: "",
                        group_searching: false,
                        theme: &theme,
                    },
                    f.area(),
                )
            })
            .unwrap();
    }

    #[test]
    fn test_sidebar_filter_mode_active_borderless() {
        let theme = Theme::default();
        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
        terminal
            .draw(|f| {
                f.render_widget(
                    Sidebar {
                        filters: &[],
                        groups: &[],
                        all_filters: &[],
                        group_names: &[],
                        selected_group: None,
                        is_group_mode: false,
                        show_groups: true,
                        match_counts: &[],
                        selected_filter_idx: 0,
                        filter_enabled: true,
                        show_marks_only: false,
                        filter_progress: None,
                        show_borders: false,
                        is_filter_mode: true,
                        scroll_offset: 0,
                        highlight_mode: false,
                        search: "",
                        searching: false,
                        group_search: "",
                        group_searching: false,
                        theme: &theme,
                    },
                    f.area(),
                )
            })
            .unwrap();
    }

    #[test]
    fn test_filter_mode_active_uses_highlight_border_color() {
        let theme = Theme::default();
        let active_border = Style::default().fg(theme.text_highlight_fg);
        let inactive_border = Style::default().fg(theme.border);
        assert_ne!(
            active_border, inactive_border,
            "text_highlight_fg and border must differ for the visual cue to be visible"
        );
    }

    #[test]
    fn test_build_group_row_shows_name_and_count() {
        let theme = Theme::default();
        let mut f0 = make_filter("a", true, FilterType::Include);
        f0.group = Some("net".to_string());
        let mut f1 = make_filter("b", true, FilterType::Include);
        f1.group = Some("net".to_string());
        let filters = vec![f0, f1];
        let line = build_group_row("net", &filters, &[], false, &theme);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "[x] net (2)");
    }

    #[test]
    fn test_build_group_row_zero_filters_shows_zero_count() {
        let theme = Theme::default();
        let line = build_group_row("empty", &[], &[], false, &theme);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "[ ] empty (0)");
    }

    #[test]
    fn test_build_group_row_all_disabled_shows_unchecked_status() {
        let theme = Theme::default();
        let mut f0 = make_filter("a", false, FilterType::Include);
        f0.group = Some("net".to_string());
        let line = build_group_row("net", &[f0], &[], false, &theme);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "[ ] net (1)");
    }

    #[test]
    fn test_build_group_row_mixed_enabled_shows_mixed_status() {
        let theme = Theme::default();
        let mut f0 = make_filter("a", true, FilterType::Include);
        f0.group = Some("net".to_string());
        let mut f1 = make_filter("b", false, FilterType::Include);
        f1.group = Some("net".to_string());
        let line = build_group_row("net", &[f0, f1], &[], false, &theme);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "[-] net (2)");
    }

    #[test]
    fn test_build_group_row_uses_group_style() {
        use crate::filters::ColorConfig;
        let theme = Theme::default();
        let groups = vec![GroupDef {
            name: "net".to_string(),
            color_config: Some(ColorConfig {
                fg: Some(ratatui::style::Color::Red),
                bg: None,
                match_only: true,
            }),
        }];
        let line = build_group_row("net", &[], &groups, false, &theme);
        assert_eq!(line.spans[1].style.fg, Some(ratatui::style::Color::Red));
    }

    #[test]
    fn test_build_group_row_no_style_uses_theme_default() {
        let theme = Theme::default();
        let line = build_group_row("net", &[], &[], false, &theme);
        assert_eq!(line.spans[1].style.fg, Some(theme.text));
    }

    #[test]
    fn test_build_group_row_selected_is_bold_and_underlined() {
        let theme = Theme::default();
        let selected = build_group_row("net", &[], &[], true, &theme);
        assert!(
            selected.spans[0]
                .style
                .add_modifier
                .contains(Modifier::BOLD | Modifier::UNDERLINED)
        );
        let unselected = build_group_row("net", &[], &[], false, &theme);
        assert!(
            !unselected.spans[0]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
    }

    #[test]
    fn test_split_sidebar_heights_no_groups_still_reserves_minimum_height() {
        // 0 groups + 1 label row = 1, but the section never shrinks below
        // the 8-row minimum, even empty.
        assert_eq!(split_sidebar_heights(20, 0), (12, 8));
    }

    #[test]
    fn test_split_sidebar_heights_few_groups_still_reserves_minimum_height() {
        // 3 group rows + 1 label row = 4, still below the 8-row minimum.
        assert_eq!(split_sidebar_heights(20, 3), (12, 8));
    }

    #[test]
    fn test_split_sidebar_heights_grows_past_minimum_for_many_groups() {
        // 10 group rows + 1 label row = 11, above the 8-row minimum.
        assert_eq!(split_sidebar_heights(20, 10), (9, 11));
    }

    #[test]
    fn test_split_sidebar_heights_caps_groups_leaving_at_least_one_filter_row() {
        assert_eq!(split_sidebar_heights(5, 20), (1, 4));
    }

    #[test]
    fn test_split_sidebar_heights_zero_inner_height_does_not_panic() {
        assert_eq!(split_sidebar_heights(0, 5), (0, 0));
    }

    #[test]
    fn test_split_sidebar_heights_small_inner_height_caps_below_minimum() {
        assert_eq!(split_sidebar_heights(10, 0), (2, 8));
    }

    fn render_to_screen(sidebar: Sidebar, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let buf = terminal
            .draw(|f| f.render_widget(sidebar, f.area()))
            .unwrap();
        (0..height)
            .map(|row| {
                (0..width)
                    .map(|c| {
                        buf.buffer
                            .cell(ratatui::prelude::Position::new(c, row))
                            .unwrap()
                            .symbol()
                            .to_string()
                    })
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn test_sidebar_renders_groups_section_below_filters() {
        let theme = Theme::default();
        let filters = vec![make_filter("foo", true, FilterType::Include)];
        let group_names = vec!["net".to_string(), "sys".to_string()];
        let sidebar = Sidebar {
            filters: &filters,
            groups: &[],
            all_filters: &filters,
            group_names: &group_names,
            match_counts: &[1],
            selected_filter_idx: 0,
            selected_group: None,
            filter_enabled: true,
            show_marks_only: false,
            filter_progress: None,
            show_borders: false,
            is_filter_mode: false,
            is_group_mode: false,
            show_groups: true,
            scroll_offset: 0,
            highlight_mode: false,
            search: "",
            searching: false,
            group_search: "",
            group_searching: false,
            theme: &theme,
        };
        let screen = render_to_screen(sidebar, 40, 10);
        assert!(screen.contains("foo"), "filter row must render: {screen:?}");
        assert!(screen.contains("net"), "group row must render: {screen:?}");
        assert!(screen.contains("sys"), "group row must render: {screen:?}");
    }

    #[test]
    fn test_sidebar_groups_section_shows_label_with_count() {
        let theme = Theme::default();
        let group_names = vec!["net".to_string(), "sys".to_string()];
        let sidebar = Sidebar {
            filters: &[],
            groups: &[],
            all_filters: &[],
            group_names: &group_names,
            match_counts: &[],
            selected_filter_idx: 0,
            selected_group: None,
            filter_enabled: true,
            show_marks_only: false,
            filter_progress: None,
            show_borders: false,
            is_filter_mode: false,
            is_group_mode: false,
            show_groups: true,
            scroll_offset: 0,
            highlight_mode: false,
            search: "",
            searching: false,
            group_search: "",
            group_searching: false,
            theme: &theme,
        };
        let screen = render_to_screen(sidebar, 40, 10);
        assert!(
            screen.contains("Groups [2]"),
            "groups label with count must render: {screen:?}"
        );
    }

    #[test]
    fn test_build_groups_title_shows_count() {
        let line = build_groups_title(3, "", false, Style::default());
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "Groups [3]");
    }

    #[test]
    fn test_build_groups_title_search_placeholder_is_dimmed() {
        let title_style = Style::default().fg(Color::Cyan);
        let line = build_groups_title(2, "", true, title_style);
        let placeholder_span = line
            .spans
            .iter()
            .find(|s| s.content.contains("type to search"))
            .expect("placeholder span should be present while searching with an empty query");
        assert!(placeholder_span.style.add_modifier.contains(Modifier::DIM));
    }

    #[test]
    fn test_build_groups_title_search_query_shown() {
        let line = build_groups_title(1, "net", true, Style::default());
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("/net"));
    }

    #[test]
    fn test_narrowed_group_names_empty_search_returns_all() {
        let names = vec!["alpha".to_string(), "beta".to_string()];
        assert_eq!(narrowed_group_names(&names, ""), names);
    }

    #[test]
    fn test_narrowed_group_names_matches_substring() {
        let names = vec!["alpha".to_string(), "beta".to_string(), "candy".to_string()];
        assert_eq!(
            narrowed_group_names(&names, "a"),
            vec!["alpha".to_string(), "beta".to_string(), "candy".to_string()]
        );
        assert_eq!(
            narrowed_group_names(&names, "^a"),
            vec!["alpha".to_string()]
        );
    }

    #[test]
    fn test_narrowed_group_names_no_match_returns_empty() {
        let names = vec!["alpha".to_string()];
        assert!(narrowed_group_names(&names, "zzz").is_empty());
    }

    #[test]
    fn test_sidebar_groups_section_narrows_by_group_search() {
        let theme = Theme::default();
        let group_names = vec!["alpha".to_string(), "beta".to_string()];
        let sidebar = Sidebar {
            filters: &[],
            groups: &[],
            all_filters: &[],
            group_names: &group_names,
            match_counts: &[],
            selected_filter_idx: 0,
            selected_group: None,
            filter_enabled: true,
            show_marks_only: false,
            filter_progress: None,
            show_borders: false,
            is_filter_mode: false,
            is_group_mode: true,
            show_groups: true,
            scroll_offset: 0,
            highlight_mode: false,
            search: "",
            searching: false,
            group_search: "beta",
            group_searching: true,
            theme: &theme,
        };
        let screen = render_to_screen(sidebar, 40, 12);
        assert!(screen.contains("beta"), "matching group must render");
        assert!(
            !screen.contains("alpha"),
            "non-matching group must be narrowed out: {screen:?}"
        );
    }

    #[test]
    fn test_sidebar_groups_section_empty_when_no_groups() {
        let theme = Theme::default();
        let filters = vec![make_filter("foo", true, FilterType::Include)];
        let sidebar = Sidebar {
            filters: &filters,
            groups: &[],
            all_filters: &filters,
            group_names: &[],
            match_counts: &[1],
            selected_filter_idx: 0,
            selected_group: None,
            filter_enabled: true,
            show_marks_only: false,
            filter_progress: None,
            show_borders: false,
            is_filter_mode: false,
            is_group_mode: false,
            show_groups: true,
            scroll_offset: 0,
            highlight_mode: false,
            search: "",
            searching: false,
            group_search: "",
            group_searching: false,
            theme: &theme,
        };
        let screen = render_to_screen(sidebar, 40, 10);
        // Filter row still gets the full remaining height when no groups exist.
        assert!(screen.contains("foo"));
    }

    #[test]
    fn test_sidebar_groups_section_hidden_when_show_groups_false() {
        let theme = Theme::default();
        let group_names = vec!["net".to_string()];
        let sidebar = Sidebar {
            filters: &[],
            groups: &[],
            all_filters: &[],
            group_names: &group_names,
            match_counts: &[],
            selected_filter_idx: 0,
            selected_group: None,
            filter_enabled: true,
            show_marks_only: false,
            filter_progress: None,
            show_borders: false,
            is_filter_mode: false,
            is_group_mode: false,
            show_groups: false,
            scroll_offset: 0,
            highlight_mode: false,
            search: "",
            searching: false,
            group_search: "",
            group_searching: false,
            theme: &theme,
        };
        let screen = render_to_screen(sidebar, 40, 10);
        assert!(
            !screen.contains("net"),
            "group section must be fully hidden when show_groups is false: {screen:?}"
        );
    }

    #[test]
    fn test_sidebar_selected_group_is_highlighted() {
        let theme = Theme::default();
        let group_names = vec!["net".to_string(), "sys".to_string()];
        let sidebar = Sidebar {
            filters: &[],
            groups: &[],
            all_filters: &[],
            group_names: &group_names,
            match_counts: &[],
            selected_filter_idx: 0,
            selected_group: Some("sys"),
            filter_enabled: true,
            show_marks_only: false,
            filter_progress: None,
            show_borders: false,
            is_filter_mode: false,
            is_group_mode: true,
            show_groups: true,
            scroll_offset: 0,
            highlight_mode: false,
            search: "",
            searching: false,
            group_search: "",
            group_searching: false,
            theme: &theme,
        };
        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
        let buf = terminal
            .draw(|f| f.render_widget(sidebar, f.area()))
            .unwrap();
        let row_text = |row: u16| -> String {
            (0..40u16)
                .map(|c| {
                    buf.buffer
                        .cell(ratatui::prelude::Position::new(c, row))
                        .unwrap()
                        .symbol()
                        .to_string()
                })
                .collect()
        };
        let find_cell_style = |needle: &str| -> ratatui::style::Modifier {
            for row in 0..10u16 {
                if let Some(col) = row_text(row).find(needle) {
                    return buf
                        .buffer
                        .cell(ratatui::prelude::Position::new(col as u16, row))
                        .unwrap()
                        .style()
                        .add_modifier;
                }
            }
            panic!("{needle:?} row must render");
        };
        assert!(
            find_cell_style("sys").contains(Modifier::BOLD),
            "selected group row should be bold"
        );
        assert!(
            !find_cell_style("net").contains(Modifier::BOLD),
            "unselected group row should not be bold"
        );
    }
}
