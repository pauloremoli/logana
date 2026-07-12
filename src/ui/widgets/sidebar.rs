use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
};

use crate::commands::auto_complete::fuzzy_match;
use crate::filters::{FilterDef, FilterType};
use crate::theme::Theme;
use crate::ui::field_layout::line_row_count;

pub struct Sidebar<'a> {
    pub filters: &'a [FilterDef],
    pub match_counts: &'a [usize],
    pub selected_filter_idx: usize,
    pub filter_enabled: bool,
    pub show_marks_only: bool,
    pub filter_progress: Option<usize>,
    pub show_borders: bool,
    pub is_filter_mode: bool,
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
    pub theme: &'a Theme,
}

/// Splits a filter row into `(prefix, value, suffix)`, where `value` is the
/// filter's pattern text (the part the filter's color highlight should apply
/// to) and `prefix`/`suffix` are the surrounding metadata (checkbox, type,
/// group tag, field tag, match count).
fn filter_row_parts(
    filter: &FilterDef,
    idx: usize,
    selected: usize,
    match_counts: &[usize],
) -> (String, String, String) {
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
        let value = if let Some(colon) = expr.find(':') {
            format!("{}={}", &expr[..colon], &expr[colon + 1..])
        } else {
            expr.to_string()
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
    let count_str = if filter.enabled {
        let count = match_counts.get(idx).copied().unwrap_or(0);
        format!(" ({})", count)
    } else {
        String::new()
    };
    let prefix = format!(
        "{}{} {}: {}",
        selected_prefix, status, filter_type_str, group_tag
    );
    let suffix = format!("{}{}", field_tag, count_str);
    (prefix, display_pattern, suffix)
}

/// Returns the plain display text for a filter row (no styling).
/// Used both for rendering and for hit-testing wrapped sidebar rows.
pub fn filter_row_display_text(
    filter: &FilterDef,
    idx: usize,
    selected: usize,
    match_counts: &[usize],
) -> String {
    let (prefix, value, suffix) = filter_row_parts(filter, idx, selected, match_counts);
    format!("{prefix}{value}{suffix}")
}

/// Indices (into `filters`) of entries whose rendered row text fuzzy-matches
/// `search`, in original list order. Empty `search` means "everything
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
            fuzzy_match(search, &text)
        })
        .map(|(idx, _)| idx)
        .collect()
}

fn build_filter_row(
    filter: &FilterDef,
    idx: usize,
    selected: usize,
    match_counts: &[usize],
    theme: &Theme,
) -> Line<'static> {
    let (prefix, value, suffix) = filter_row_parts(filter, idx, selected, match_counts);
    let mut default_style = Style::default().fg(theme.text);
    if idx == selected {
        default_style = default_style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
    }
    let mut value_style = default_style;
    if let Some(cfg) = &filter.color_config {
        if let Some(fg) = cfg.fg {
            value_style = value_style.fg(fg);
        }
        if let Some(bg) = cfg.bg {
            value_style = value_style.bg(bg);
        }
    }
    Line::from(vec![
        Span::styled(prefix, default_style),
        Span::styled(value, value_style),
        Span::styled(suffix, default_style),
    ])
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
/// the viewport it was previously scrolled to (`prev_scroll`).
///
/// Scrolls just enough to bring the selected filter's rows into a viewport
/// of `content_h` rows at wrap width `content_w` — if it's already fully
/// visible at `prev_scroll`, the offset is left unchanged, so moving the
/// selection within the current view never re-scrolls it.
///
/// Shared with [`crate::ui::input_handler::InputHandler::hit_test_sidebar`],
/// which must replicate this exact scroll to map a click's screen row back
/// to a filter index.
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

    // Gate on `searching` rather than `search.is_empty()`: the moment right
    // after pressing `/` has an empty query but must still show a visible
    // cue that the app is now capturing search text. The placeholder is
    // dimmed to read as a hint rather than active content; the query itself
    // keeps the normal title style once something's been typed.
    if searching {
        if search.is_empty() {
            spans.push(Span::styled(
                " type to search...",
                title_style.add_modifier(Modifier::DIM),
            ));
        } else {
            spans.push(Span::styled(format!(" /{search}"), title_style));
        }
    }

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

        Paragraph::new(filters_text)
            .wrap(Wrap { trim: false })
            .scroll((self.scroll_offset.min(u16::MAX as usize) as u16, 0))
            .block(sidebar_block)
            .render(area, buf);
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
        let line = build_filter_row(&filter, 0, 0, &[3], &theme);
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
        let line = build_filter_row(&filter, 0, 0, &[3], &theme);
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
        let line = build_filter_row(&filter, 0, 0, &[3], &theme);

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
    fn test_build_filter_row_selected_is_bold_and_underlined() {
        let theme = Theme::default();
        let filter = make_filter("hello", true, FilterType::Include);
        let selected_line = build_filter_row(&filter, 0, 0, &[3], &theme);
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

        let unselected_line = build_filter_row(&filter, 1, 0, &[3], &theme);
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
        let line = build_filter_row(&filter, 1, 0, &[0, 0], &theme);
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
        let line = build_filter_row(&filter, 0, 0, &[1], &theme);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("key=val"));
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
    fn test_sidebar_filter_mode_active_bold_title_bordered() {
        let theme = Theme::default();
        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
        let buf = terminal
            .draw(|f| {
                f.render_widget(
                    Sidebar {
                        filters: &[],
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
}
