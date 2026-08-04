use ratatui::{
    prelude::*,
    style::Modifier,
    widgets::{
        Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, StatefulWidget,
    },
};

use crate::theme::Theme;

use super::popup_entry;

pub struct ThemePickerPopup<'a> {
    pub theme: &'a Theme,
    /// Available theme names.
    pub entries: &'a [String],
    /// Index into the *visible* (filtered) entries.
    pub selected: usize,
    pub search: &'a str,
}

impl<'a> Widget for ThemePickerPopup<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        use crate::commands::auto_complete::fuzzy_match;

        let vis: Vec<usize> = if self.search.is_empty() {
            (0..self.entries.len()).collect()
        } else {
            self.entries
                .iter()
                .enumerate()
                .filter(|(_, name)| fuzzy_match(self.search, name))
                .map(|(i, _)| i)
                .collect()
        };

        let popup_width = (area.width.saturating_sub(4)).clamp(40, 70);
        let row_count = vis.len() as u16;
        let extra = 6u16;
        let popup_height = (row_count + extra)
            .min(area.height * 4 / 5)
            .max(9)
            .min(area.height.saturating_sub(2));
        let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
        let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
        let popup_area = Rect::new(x, y, popup_width, popup_height);

        ratatui::widgets::Clear.render(popup_area, buf);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.theme.border_title))
            .title(" Theme ")
            .title_style(
                Style::default()
                    .fg(self.theme.text_highlight_fg)
                    .add_modifier(Modifier::BOLD),
            )
            .title_alignment(Alignment::Center)
            .style(Style::default().bg(self.theme.root_bg));

        let inner = block.inner(popup_area);
        block.render(popup_area, buf);

        let footer_lines = 1usize;
        let content_h = inner.height.saturating_sub((footer_lines + 2) as u16) as usize;

        let vsplit = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
                Constraint::Length(footer_lines as u16),
            ])
            .split(inner);
        let search_area = vsplit[0];
        let content_area = vsplit[1];
        let sep_area = vsplit[2];
        let footer_area = vsplit[3];

        let search_line = if self.search.is_empty() {
            Line::from(Span::styled(
                " type to search...",
                Style::default()
                    .fg(self.theme.text)
                    .add_modifier(Modifier::DIM),
            ))
        } else {
            Line::from(vec![
                Span::styled(
                    " /",
                    Style::default()
                        .fg(self.theme.text_highlight_fg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    self.search.to_string(),
                    Style::default().fg(self.theme.text_highlight_fg),
                ),
            ])
        };
        Paragraph::new(search_line)
            .style(Style::default().bg(self.theme.root_bg))
            .render(search_area, buf);

        let scroll = if self.selected >= content_h {
            self.selected - content_h + 1
        } else {
            0
        };

        let mut lines: Vec<Line> = Vec::new();
        for (i, &entry_idx) in vis.iter().enumerate().skip(scroll).take(content_h) {
            let is_selected = i == self.selected;
            let name = &self.entries[entry_idx];
            let prefix = if is_selected { "> " } else { "  " };
            let style = if is_selected {
                Style::default()
                    .fg(self.theme.text_highlight_fg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(self.theme.text)
            };
            lines.push(Line::from(Span::styled(format!("{prefix}{name}"), style)));
        }

        while lines.len() < content_h {
            lines.push(Line::from(""));
        }

        Paragraph::new(lines)
            .style(Style::default().bg(self.theme.root_bg))
            .render(content_area, buf);

        let sep = "\u{2500}".repeat(sep_area.width as usize);
        Paragraph::new(sep)
            .style(Style::default().fg(self.theme.text))
            .render(sep_area, buf);

        let key_style = Style::default()
            .fg(self.theme.text_highlight_fg)
            .add_modifier(Modifier::BOLD);
        let txt_style = Style::default().fg(self.theme.text);
        let br_style = Style::default().fg(self.theme.text);
        let mut line1: Vec<Span<'static>> = Vec::new();
        popup_entry(
            &mut line1,
            "\u{2191}\u{2193}".to_string(),
            "preview",
            key_style,
            txt_style,
            br_style,
        );
        popup_entry(
            &mut line1,
            "Enter".to_string(),
            "apply",
            key_style,
            txt_style,
            br_style,
        );
        popup_entry(
            &mut line1,
            "Esc".to_string(),
            "cancel",
            key_style,
            txt_style,
            br_style,
        );
        Paragraph::new(vec![Line::from(line1)])
            .style(Style::default().bg(self.theme.root_bg))
            .render(footer_area, buf);

        let total = vis.len();
        if total > content_h {
            let mut sb_state =
                ScrollbarState::new(total.saturating_sub(content_h)).position(scroll);
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
    use crate::mode::theme_picker_mode::ThemePickerMode;
    use crate::theme::Theme;
    use crate::ui::App;
    use crate::{config::Keybindings, ingestion::FileReader};
    use ratatui::{Terminal, backend::TestBackend};
    use std::sync::Arc;

    async fn make_app(lines: &[&str]) -> App {
        let data: Vec<u8> = lines.join("\n").into_bytes();
        let file_reader = FileReader::from_bytes(data);
        let db = Arc::new(crate::db::Database::in_memory().await.unwrap());
        let log_manager = crate::db::LogManager::new(db, None).await;
        App::builder(
            log_manager,
            file_reader,
            Theme::default(),
            Arc::new(Keybindings::default()),
        )
        .build()
        .await
    }

    fn make_terminal() -> Terminal<TestBackend> {
        Terminal::new(TestBackend::new(80, 24)).unwrap()
    }

    #[tokio::test]
    async fn test_theme_picker_popup_basic() {
        let mut app = make_app(&["line one"]).await;
        let entries = vec!["dracula".to_string(), "nord".to_string()];
        app.tabs[0].interaction.mode = Box::new(ThemePickerMode::new(entries, Theme::default()));
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("dracula"));
        assert!(content.contains("nord"));
        assert!(content.contains("Theme"));
    }

    #[tokio::test]
    async fn test_theme_picker_popup_shows_search_placeholder_when_empty() {
        let mut app = make_app(&["line one"]).await;
        let entries = vec!["dracula".to_string()];
        app.tabs[0].interaction.mode = Box::new(ThemePickerMode::new(entries, Theme::default()));
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("type to search"));
    }

    #[tokio::test]
    async fn test_theme_picker_popup_many_entries_shows_scrollbar() {
        let mut app = make_app(&["line one"]).await;
        let entries: Vec<String> = (0..30).map(|i| format!("theme{i}")).collect();
        app.tabs[0].interaction.mode = Box::new(ThemePickerMode::new(entries, Theme::default()));
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }
}
