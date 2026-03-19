use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
};

pub struct ModeBar<'a> {
    pub content: Line<'a>,
    pub show_borders: bool,
    pub theme: &'a crate::theme::Theme,
}

impl<'a> Widget for ModeBar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = if self.show_borders {
            Block::default()
                .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
                .border_style(Style::default().fg(self.theme.border))
        } else {
            Block::default()
                .borders(Borders::NONE)
                .padding(Padding::new(1, 0, 0, 0))
        };
        Paragraph::new(self.content)
            .block(block)
            .wrap(Wrap { trim: true })
            .style(Style::default().bg(self.theme.root_bg))
            .render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;
    use ratatui::{Terminal, backend::TestBackend};

    fn render_mode_bar(content: Line<'static>, show_borders: bool) -> String {
        let theme = Theme::default();
        let mut terminal = Terminal::new(TestBackend::new(40, 3)).unwrap();
        terminal
            .draw(|f| {
                f.render_widget(
                    ModeBar {
                        content,
                        show_borders,
                        theme: &theme,
                    },
                    f.area(),
                );
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn test_mode_bar_no_borders_renders_content() {
        let content = Line::from("normal mode hints here");
        let output = render_mode_bar(content, false);
        assert!(output.contains("normal mode hints here"));
    }

    #[test]
    fn test_mode_bar_with_borders_renders_content() {
        let content = Line::from("command hints");
        let output = render_mode_bar(content, true);
        assert!(output.contains("command hints"));
        assert!(output.contains('│') || output.contains('┘') || output.contains('└'));
    }

    #[test]
    fn test_mode_bar_empty_content() {
        let output = render_mode_bar(Line::from(""), false);
        assert!(!output.is_empty());
    }
}
