use ratatui::{
    prelude::*,
    widgets::{Paragraph, Wrap},
};

use crate::mode::app_mode::status_entry;
use crate::theme::Theme;

pub struct InputBar<'a> {
    pub query: &'a str,
    pub cursor_pos: usize,
    pub forward: bool,
    pub is_active: bool,
    pub total_matches: usize,
    pub current_occurrence: usize,
    pub progress: Option<(String, usize)>,
    /// Configured `next_match`/`prev_match` keys, shown next to the match
    /// count once a search is committed so the navigation keys are
    /// discoverable without opening the keybindings help overlay.
    pub next_match_key: &'a str,
    pub prev_match_key: &'a str,
    pub theme: &'a Theme,
}

impl<'a> InputBar<'a> {
    pub fn cursor_position(&self, input_area: Rect) -> Option<(u16, u16)> {
        if !self.is_active {
            return None;
        }
        let cursor_x = input_area.x + 1 + self.cursor_pos as u16;
        if cursor_x < input_area.x + input_area.width {
            Some((cursor_x, input_area.y))
        } else {
            None
        }
    }

    /// Builds the hint line shown below the search input. While typing, the
    /// confirm key isn't repeated here — `SearchMode`'s mode bar already
    /// shows `<Enter> search` for as long as the search box is open. Once a
    /// search is committed, the navigation keys are styled the same way as
    /// a mode bar's `<key> action` entries (see `status_entry`) so they
    /// read consistently with the rest of the UI.
    fn hint_line(&self) -> Line<'static> {
        let text_style = Style::default().fg(self.theme.text);
        if self.query.is_empty() {
            return Line::from(Span::styled("  Type a pattern to search", text_style));
        }
        if self.is_active {
            return Line::from(Span::styled(
                format!("  {} matches", self.total_matches),
                text_style,
            ));
        }
        if self.total_matches == 0 {
            return Line::from(Span::styled("  no matches", text_style));
        }
        let mut spans = vec![Span::styled(
            format!(
                "  match {} / {}   ",
                self.current_occurrence, self.total_matches
            ),
            text_style,
        )];
        status_entry(
            &mut spans,
            self.next_match_key.to_string(),
            "next",
            self.theme,
        );
        status_entry(
            &mut spans,
            self.prev_match_key.to_string(),
            "prev",
            self.theme,
        );
        Line::from(spans)
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

        let hint_line = self.hint_line();
        let hint = Paragraph::new(hint_line).style(Style::default().bg(self.theme.root_bg));
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

    fn line_text(line: &Line<'static>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn make_bar<'a>(query: &'a str, is_active: bool, theme: &'a Theme) -> InputBar<'a> {
        InputBar {
            query,
            cursor_pos: query.len(),
            forward: true,
            is_active,
            total_matches: 5,
            current_occurrence: 2,
            progress: None,
            next_match_key: "n",
            prev_match_key: "N",
            theme,
        }
    }

    #[test]
    fn test_input_bar_renders_active() {
        let theme = Theme::default();
        let bar = InputBar {
            query: "hello",
            cursor_pos: 5,
            forward: true,
            is_active: true,
            total_matches: 3,
            current_occurrence: 1,
            progress: None,
            next_match_key: "n",
            prev_match_key: "N",
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
            cursor_pos: 5,
            forward: false,
            is_active: false,
            total_matches: 2,
            current_occurrence: 1,
            progress: None,
            next_match_key: "n",
            prev_match_key: "N",
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
            cursor_pos: 3,
            forward: true,
            is_active: true,
            total_matches: 1,
            current_occurrence: 1,
            progress: None,
            next_match_key: "n",
            prev_match_key: "N",
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
            cursor_pos: 3,
            forward: true,
            is_active: false,
            total_matches: 1,
            current_occurrence: 1,
            progress: None,
            next_match_key: "n",
            prev_match_key: "N",
            theme: &theme,
        };
        let area = Rect::new(0, 5, 80, 1);
        assert_eq!(bar.cursor_position(area), None);
    }

    #[test]
    fn test_hint_active_with_matches_does_not_repeat_confirm_key() {
        // SearchMode's mode bar already shows <Enter> search while the
        // search box is open, so the hint line shouldn't repeat it.
        let theme = Theme::default();
        let bar = InputBar {
            query: "x",
            cursor_pos: 1,
            forward: true,
            is_active: true,
            total_matches: 7,
            current_occurrence: 1,
            progress: None,
            next_match_key: "n",
            prev_match_key: "N",
            theme: &theme,
        };
        assert_eq!(line_text(&bar.hint_line()), "  7 matches");
    }

    #[test]
    fn test_hint_inactive_no_matches() {
        let theme = Theme::default();
        let bar = InputBar {
            query: "x",
            cursor_pos: 1,
            forward: true,
            is_active: false,
            total_matches: 0,
            current_occurrence: 0,
            progress: None,
            next_match_key: "n",
            prev_match_key: "N",
            theme: &theme,
        };
        assert_eq!(line_text(&bar.hint_line()), "  no matches");
    }

    #[test]
    fn test_hint_inactive_with_matches_mentions_navigation_keys() {
        let theme = Theme::default();
        let bar = InputBar {
            query: "x",
            cursor_pos: 1,
            forward: true,
            is_active: false,
            total_matches: 10,
            current_occurrence: 3,
            progress: None,
            next_match_key: "n",
            prev_match_key: "N",
            theme: &theme,
        };
        let text = line_text(&bar.hint_line());
        assert!(text.contains("match 3 / 10"), "{text}");
        assert!(text.contains('n'), "{text}");
        assert!(text.contains("next"), "{text}");
        assert!(text.contains('N'), "{text}");
        assert!(text.contains("prev"), "{text}");
    }

    #[test]
    fn test_hint_inactive_with_matches_uses_configured_keys() {
        let theme = Theme::default();
        let bar = InputBar {
            query: "x",
            cursor_pos: 1,
            forward: true,
            is_active: false,
            total_matches: 10,
            current_occurrence: 3,
            progress: None,
            next_match_key: "Ctrl+n",
            prev_match_key: "Ctrl+p",
            theme: &theme,
        };
        let text = line_text(&bar.hint_line());
        assert!(text.contains("Ctrl+n"), "{text}");
        assert!(text.contains("Ctrl+p"), "{text}");
    }

    #[test]
    fn test_hint_empty_query() {
        let theme = Theme::default();
        let bar = make_bar("", true, &theme);
        assert_eq!(line_text(&bar.hint_line()), "  Type a pattern to search");
    }
}
