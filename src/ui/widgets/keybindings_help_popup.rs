use ratatui::{
    prelude::*,
    style::Modifier,
    widgets::{
        Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, StatefulWidget,
    },
};

use crate::config::Keybindings;
use crate::theme::Theme;

pub struct KeybindingsHelpPopup<'a> {
    pub theme: &'a Theme,
    pub keybindings: &'a Keybindings,
    pub scroll: usize,
    pub search: &'a str,
}

impl<'a> Widget for KeybindingsHelpPopup<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        use crate::mode::keybindings_help_mode::{HelpRow, build_help_rows, filter_rows};

        let popup_width = (area.width.saturating_sub(4)).clamp(40, 72);
        let popup_height = (area.height * 4 / 5)
            .max(10)
            .min(area.height.saturating_sub(2));
        let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
        let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
        let popup_area = Rect::new(x, y, popup_width, popup_height);

        ratatui::widgets::Clear.render(popup_area, buf);

        let inner_h = popup_height.saturating_sub(4) as usize;
        let col_w = (popup_width.saturating_sub(2)) as usize;

        let all_rows = build_help_rows(self.keybindings);
        let rows = filter_rows(&all_rows, self.search);

        let total = rows.len();
        let scroll = self.scroll.min(total.saturating_sub(inner_h));

        let visible: Vec<&HelpRow> = rows.iter().skip(scroll).take(inner_h).collect();

        let key_col = 14usize;
        let action_col = col_w.saturating_sub(key_col + 5);

        let mut lines: Vec<Line> = Vec::new();
        for row in &visible {
            match row {
                HelpRow::Header(title) => {
                    let bar = "\u{2500}".repeat(col_w.saturating_sub(title.len() + 3));
                    lines.push(Line::from(vec![Span::styled(
                        format!("\u{2500}\u{2500} {} {}", title, bar),
                        Style::default()
                            .fg(self.theme.text_highlight_fg)
                            .add_modifier(Modifier::BOLD),
                    )]));
                }
                HelpRow::Entry { action, keys } => {
                    let keys_str = if keys.len() > key_col {
                        &keys[..key_col]
                    } else {
                        keys.as_str()
                    };
                    let action_str = if action.len() > action_col {
                        &action[..action_col]
                    } else {
                        action.as_str()
                    };
                    let gap = " ".repeat(key_col.saturating_sub(keys_str.len()));
                    lines.push(Line::from(vec![
                        Span::raw(" "),
                        Span::styled("<", Style::default().fg(self.theme.text)),
                        Span::styled(
                            keys_str.to_string(),
                            Style::default()
                                .fg(self.theme.text_highlight_fg)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(">", Style::default().fg(self.theme.text)),
                        Span::raw(format!("{}  ", gap)),
                        Span::styled(action_str.to_string(), Style::default().fg(self.theme.text)),
                    ]));
                }
            }
        }

        while lines.len() < inner_h {
            lines.push(Line::from(""));
        }

        let close_keys = self.keybindings.help.close.display();
        let title = if self.search.is_empty() {
            format!(" Keybindings Help ({} to close) ", close_keys)
        } else {
            format!(" Keybindings Help  /{}█ ", self.search)
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.theme.border_title))
            .title(title)
            .title_style(
                Style::default()
                    .fg(self.theme.text_highlight_fg)
                    .add_modifier(Modifier::BOLD),
            )
            .title_alignment(Alignment::Center)
            .style(Style::default().bg(self.theme.root_bg));

        let inner = block.inner(popup_area);
        block.render(popup_area, buf);

        let vsplit = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(1),
            ])
            .split(inner);

        let search_display = if self.search.is_empty() {
            Span::styled(
                "  type to filter\u{2026}",
                Style::default().fg(self.theme.text),
            )
        } else {
            Span::styled(
                format!("  /{}", self.search),
                Style::default().fg(self.theme.text),
            )
        };
        Paragraph::new(Line::from(search_display)).render(vsplit[0], buf);

        let sep = "\u{2500}".repeat(vsplit[1].width as usize);
        Paragraph::new(sep)
            .style(Style::default().fg(self.theme.text))
            .render(vsplit[1], buf);

        let content_area = vsplit[2];
        Paragraph::new(lines)
            .style(Style::default().bg(self.theme.root_bg))
            .render(content_area, buf);

        if total > inner_h {
            let mut sb_state = ScrollbarState::new(total.saturating_sub(inner_h)).position(scroll);
            StatefulWidget::render(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .style(Style::default().fg(self.theme.border)),
                content_area,
                buf,
                &mut sb_state,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::config::Keybindings;
    use crate::db::Database;
    use crate::file_reader::FileReader;
    use crate::log_manager::LogManager;
    use crate::mode::keybindings_help_mode::KeybindingsHelpMode;
    use crate::theme::Theme;
    use crate::ui::App;
    use ratatui::{Terminal, backend::TestBackend};
    use std::sync::Arc;

    async fn make_app(lines: &[&str]) -> App {
        let data: Vec<u8> = lines.join("\n").into_bytes();
        let file_reader = FileReader::from_bytes(data);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        App::new(
            log_manager,
            file_reader,
            Theme::default(),
            Arc::new(Keybindings::default()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
    }

    fn make_terminal() -> Terminal<TestBackend> {
        Terminal::new(TestBackend::new(80, 24)).unwrap()
    }

    #[tokio::test]
    async fn test_keybindings_help_basic() {
        let mut app = make_app(&["line one", "line two"]).await;
        app.tabs[0].interaction.mode = Box::new(KeybindingsHelpMode::new());
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_keybindings_help_with_search() {
        let mut app = make_app(&["line one", "line two"]).await;
        let mut mode = KeybindingsHelpMode::new();
        mode.search = "scroll".to_string();
        app.tabs[0].interaction.mode = Box::new(mode);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_keybindings_help_scroll() {
        let mut app = make_app(&["line one", "line two"]).await;
        let mut mode = KeybindingsHelpMode::new();
        mode.scroll = 5;
        app.tabs[0].interaction.mode = Box::new(mode);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }
}
