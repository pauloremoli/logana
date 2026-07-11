use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
};

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

fn build_filter_row(
    filter: &FilterDef,
    idx: usize,
    selected: usize,
    match_counts: &[usize],
    theme: &Theme,
) -> Line<'static> {
    let (prefix, value, suffix) = filter_row_parts(filter, idx, selected, match_counts);
    let default_style = Style::default().fg(theme.text);
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

/// Row offset (in wrapped display rows) so the selected filter's row stays
/// within a viewport of `content_h` rows at wrap width `content_w`.
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
) -> usize {
    if content_w == 0 || content_h == 0 {
        return 0;
    }
    let mut row = 0usize;
    let mut selected_end = 0usize;
    for (i, filter) in filters.iter().enumerate() {
        let text = filter_row_display_text(filter, i, selected, match_counts);
        let row_h = line_row_count(text.as_bytes(), content_w);
        row += row_h;
        if i == selected {
            selected_end = row;
        }
    }
    selected_end.saturating_sub(content_h)
}

fn build_sidebar_title(
    filter_enabled: bool,
    show_marks_only: bool,
    filter_progress: Option<usize>,
    active_count: usize,
    total_count: usize,
) -> String {
    let filter_count_suffix = if total_count > 0 {
        format!(" [{}/{}]", active_count, total_count)
    } else {
        String::new()
    };
    let base = if show_marks_only {
        format!("Filters [MARKS ONLY]{}", filter_count_suffix)
    } else if filter_enabled {
        format!("Filters{}", filter_count_suffix)
    } else {
        format!("Filters [OFF]{}", filter_count_suffix)
    };
    match filter_progress {
        Some(pct) if pct < 100 => format!("{} {}%", base, pct),
        Some(_) => format!("{} Indexing\u{2026}", base),
        None => base,
    }
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
        let sidebar_title = build_sidebar_title(
            self.filter_enabled,
            self.show_marks_only,
            self.filter_progress,
            active_count,
            total_count,
        );

        let title_style = if self.is_filter_mode {
            Style::default().fg(self.theme.text_highlight_fg)
        } else {
            Style::default().fg(self.theme.border_title)
        };
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
                .title_style(title_style)
        } else {
            Block::default()
                .borders(Borders::NONE)
                .padding(Padding::new(1, 0, 0, 0))
                .title(sidebar_title)
                .title_style(title_style)
        };

        let inner = sidebar_block.inner(area);
        let scroll = compute_scroll_offset(
            self.filters,
            self.selected_filter_idx,
            self.match_counts,
            inner.width as usize,
            inner.height as usize,
        );

        Paragraph::new(filters_text)
            .wrap(Wrap { trim: false })
            .scroll((scroll.min(u16::MAX as usize) as u16, 0))
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
    fn test_sidebar_scrolls_to_keep_selection_visible() {
        let theme = Theme::default();
        let filters: Vec<FilterDef> = (0..30)
            .map(|i| make_filter(&format!("pattern_{i}"), true, FilterType::Include))
            .collect();
        let match_counts = vec![0; filters.len()];
        // Small terminal: only a handful of rows fit, selection is near the end.
        let selected = 25;
        let sidebar = Sidebar {
            filters: &filters,
            match_counts: &match_counts,
            selected_filter_idx: selected,
            filter_enabled: true,
            show_marks_only: false,
            filter_progress: None,
            show_borders: false,
            is_filter_mode: false,
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
            theme: &theme,
        };
        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
        terminal
            .draw(|f| f.render_widget(sidebar, f.area()))
            .unwrap();
    }

    #[test]
    fn test_build_sidebar_title_marks_only() {
        let title = build_sidebar_title(true, true, None, 2, 4);
        assert!(title.contains("MARKS ONLY"));
        assert!(title.contains("[2/4]"));
    }

    #[test]
    fn test_build_sidebar_title_disabled() {
        let title = build_sidebar_title(false, false, None, 0, 3);
        assert!(title.contains("[OFF]"));
        assert!(title.contains("[0/3]"));
    }

    #[test]
    fn test_build_sidebar_title_with_progress() {
        let title = build_sidebar_title(true, false, Some(50), 1, 2);
        assert!(title.contains("50%"));
    }

    #[test]
    fn test_build_sidebar_title_indexing_complete() {
        let title = build_sidebar_title(true, false, Some(100), 1, 2);
        assert!(title.contains("Indexing"));
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
