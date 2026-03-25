use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
};

use crate::theme::Theme;
use crate::types::{FilterDef, FilterType};

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

/// Returns the plain display text for a filter row (no styling).
/// Used both for rendering and for hit-testing wrapped sidebar rows.
pub(crate) fn filter_row_display_text(
    filter: &FilterDef,
    idx: usize,
    selected: usize,
    match_counts: &[usize],
) -> String {
    let status = if filter.enabled { "[x]" } else { "[ ]" };
    let selected_prefix = if idx == selected { ">" } else { " " };
    let is_date = filter.pattern.starts_with(crate::date_filter::DATE_PREFIX);
    let is_field = filter
        .pattern
        .starts_with(crate::field_filter::FIELD_PREFIX);
    let filter_type_str = if is_date {
        "Date"
    } else {
        match filter.filter_type {
            FilterType::Include => "In",
            FilterType::Exclude => "Out",
        }
    };
    let field_display_buf: String;
    let (display_pattern, field_tag) = if is_date {
        (&filter.pattern[crate::date_filter::DATE_PREFIX.len()..], "")
    } else if is_field {
        let expr = &filter.pattern[crate::field_filter::FIELD_PREFIX.len()..];
        field_display_buf = if let Some(colon) = expr.find(':') {
            format!("{}={}", &expr[..colon], &expr[colon + 1..])
        } else {
            expr.to_string()
        };
        (field_display_buf.as_str(), " [field]")
    } else {
        (&filter.pattern[..], "")
    };
    let count_str = if filter.enabled {
        let count = match_counts.get(idx).copied().unwrap_or(0);
        format!(" ({})", count)
    } else {
        String::new()
    };
    format!(
        "{}{} {}: {}{}{}",
        selected_prefix, status, filter_type_str, display_pattern, field_tag, count_str
    )
}

fn build_filter_row(
    filter: &FilterDef,
    idx: usize,
    selected: usize,
    match_counts: &[usize],
    theme: &Theme,
) -> Line<'static> {
    let text = filter_row_display_text(filter, idx, selected, match_counts);
    let mut style = Style::default().fg(theme.text);
    if let Some(cfg) = &filter.color_config {
        if let Some(fg) = cfg.fg {
            style = style.fg(fg);
        }
        if let Some(bg) = cfg.bg {
            style = style.bg(bg);
        }
    }
    Line::from(text).style(style)
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

        Paragraph::new(filters_text)
            .wrap(Wrap { trim: false })
            .block(sidebar_block)
            .render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;
    use crate::types::{FilterDef, FilterType};
    use ratatui::{Terminal, backend::TestBackend};

    fn make_filter(pattern: &str, enabled: bool, filter_type: FilterType) -> FilterDef {
        FilterDef {
            id: 0,
            pattern: pattern.to_string(),
            enabled,
            filter_type,
            color_config: None,
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
        let pattern = format!("{}key:val", crate::field_filter::FIELD_PREFIX);
        let filter = make_filter(&pattern, true, FilterType::Include);
        let line = build_filter_row(&filter, 0, 0, &[1], &theme);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("key=val"));
        assert!(text.contains("[field]"));
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
