use ratatui::{
    prelude::*,
    widgets::{Paragraph, Wrap},
};

use crate::theme::Theme;

pub struct InputBar<'a> {
    pub query: &'a str,
    pub forward: bool,
    pub is_active: bool,
    pub total_matches: usize,
    pub current_occurrence: usize,
    pub progress: Option<(String, usize)>,
    pub theme: &'a Theme,
}

impl<'a> InputBar<'a> {
    pub fn cursor_position(&self, input_area: Rect) -> Option<(u16, u16)> {
        if !self.is_active {
            return None;
        }
        let cursor_x = input_area.x + 1 + self.query.len() as u16;
        if cursor_x < input_area.x + input_area.width {
            Some((cursor_x, input_area.y))
        } else {
            None
        }
    }

    fn hint_text(&self) -> String {
        if !self.query.is_empty() {
            if self.is_active {
                format!("  {} matches", self.total_matches)
            } else if self.total_matches == 0 {
                "  no matches".to_string()
            } else {
                format!(
                    "  match {} / {}",
                    self.current_occurrence, self.total_matches
                )
            }
        } else {
            "  Type pattern and press Enter to search".to_string()
        }
    }
}

impl<'a> Widget for InputBar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(area);

        let prefix = if self.forward { "/" } else { "?" };
        let search_line = Paragraph::new(format!("{}{}", prefix, self.query))
            .style(
                Style::default()
                    .fg(self.theme.cursor_fg)
                    .bg(self.theme.cursor_bg),
            )
            .wrap(Wrap { trim: false });
        search_line.render(chunks[0], buf);

        let hint_text = self.hint_text();
        let hint = Paragraph::new(hint_text)
            .style(Style::default().fg(self.theme.text).bg(self.theme.root_bg));
        hint.render(chunks[1], buf);

        if let Some((bar_str, pct)) = self.progress {
            let text = format!(" {} {}% ", bar_str, pct);
            let text_width = text.chars().count() as u16;
            let x = chunks[1].x + (chunks[1].width.saturating_sub(text_width)) / 2;
            let w = chunks[1].width.min(text_width);
            let progress_rect = Rect::new(x, chunks[1].y, w, 1);
            Paragraph::new(text)
                .style(
                    Style::default()
                        .fg(self.theme.border)
                        .bg(self.theme.root_bg),
                )
                .render(progress_rect, buf);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;
    use ratatui::{Terminal, backend::TestBackend};

    fn make_bar<'a>(query: &'a str, is_active: bool, theme: &'a Theme) -> InputBar<'a> {
        InputBar {
            query,
            forward: true,
            is_active,
            total_matches: 5,
            current_occurrence: 2,
            progress: None,
            theme,
        }
    }

    #[test]
    fn test_input_bar_renders_active() {
        let theme = Theme::default();
        let bar = InputBar {
            query: "hello",
            forward: true,
            is_active: true,
            total_matches: 3,
            current_occurrence: 1,
            progress: None,
            theme: &theme,
        };
        let mut terminal = Terminal::new(TestBackend::new(80, 2)).unwrap();
        terminal.draw(|f| f.render_widget(bar, f.area())).unwrap();
    }

    #[test]
    fn test_input_bar_renders_inactive() {
        let theme = Theme::default();
        let bar = InputBar {
            query: "world",
            forward: false,
            is_active: false,
            total_matches: 2,
            current_occurrence: 1,
            progress: None,
            theme: &theme,
        };
        let mut terminal = Terminal::new(TestBackend::new(80, 2)).unwrap();
        terminal.draw(|f| f.render_widget(bar, f.area())).unwrap();
    }

    #[test]
    fn test_cursor_position_active() {
        let theme = Theme::default();
        let bar = InputBar {
            query: "abc",
            forward: true,
            is_active: true,
            total_matches: 1,
            current_occurrence: 1,
            progress: None,
            theme: &theme,
        };
        let area = Rect::new(0, 5, 80, 1);
        let pos = bar.cursor_position(area);
        assert_eq!(pos, Some((4, 5)));
    }

    #[test]
    fn test_cursor_position_inactive() {
        let theme = Theme::default();
        let bar = InputBar {
            query: "abc",
            forward: true,
            is_active: false,
            total_matches: 1,
            current_occurrence: 1,
            progress: None,
            theme: &theme,
        };
        let area = Rect::new(0, 5, 80, 1);
        assert_eq!(bar.cursor_position(area), None);
    }

    #[test]
    fn test_hint_text_active_with_matches() {
        let theme = Theme::default();
        let bar = InputBar {
            query: "x",
            forward: true,
            is_active: true,
            total_matches: 7,
            current_occurrence: 1,
            progress: None,
            theme: &theme,
        };
        assert_eq!(bar.hint_text(), "  7 matches");
    }

    #[test]
    fn test_hint_text_inactive_no_matches() {
        let theme = Theme::default();
        let bar = InputBar {
            query: "x",
            forward: true,
            is_active: false,
            total_matches: 0,
            current_occurrence: 0,
            progress: None,
            theme: &theme,
        };
        assert_eq!(bar.hint_text(), "  no matches");
    }

    #[test]
    fn test_hint_text_inactive_with_matches() {
        let theme = Theme::default();
        let bar = InputBar {
            query: "x",
            forward: true,
            is_active: false,
            total_matches: 10,
            current_occurrence: 3,
            progress: None,
            theme: &theme,
        };
        assert_eq!(bar.hint_text(), "  match 3 / 10");
    }

    #[test]
    fn test_hint_text_empty_query() {
        let theme = Theme::default();
        let bar = make_bar("", true, &theme);
        assert!(bar.hint_text().contains("Type pattern"));
    }
}
