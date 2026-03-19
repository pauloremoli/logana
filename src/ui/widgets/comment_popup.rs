use ratatui::{
    prelude::*,
    style::Modifier,
    widgets::{Block, Borders, Paragraph},
};

use crate::config::Keybindings;
use crate::theme::Theme;

use super::popup_entry;

pub struct CommentPopup<'a> {
    pub theme: &'a Theme,
    pub keybindings: &'a Keybindings,
    pub lines: &'a [String],
    pub cursor_row: usize,
    pub cursor_col: usize,
    pub line_count: usize,
}

impl<'a> CommentPopup<'a> {
    pub fn cursor_position(&self, area: Rect) -> Option<(u16, u16)> {
        let popup_width = area.width.saturating_sub(8).clamp(40, 70);
        let text_rows = self.lines.len().max(1) as u16;
        let popup_height = (text_rows + 4).min(area.height.saturating_sub(4)).max(6);
        let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
        let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
        let text_area_x = x + 1;
        let text_area_y = y + 1;
        let text_area_width = popup_width.saturating_sub(2);
        let text_area_height = popup_height.saturating_sub(4);
        let cur_x = text_area_x + self.cursor_col as u16;
        let cur_y = text_area_y + self.cursor_row as u16;
        if cur_x < text_area_x + text_area_width && cur_y < text_area_y + text_area_height {
            Some((cur_x, cur_y))
        } else {
            None
        }
    }
}

impl<'a> Widget for CommentPopup<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let popup_width = area.width.saturating_sub(8).clamp(40, 70);
        let text_rows = self.lines.len().max(1) as u16;
        let popup_height = (text_rows + 4).min(area.height.saturating_sub(4)).max(6);
        let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
        let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
        let popup_area = Rect::new(x, y, popup_width, popup_height);

        ratatui::widgets::Clear.render(popup_area, buf);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.theme.border_title))
            .title(format!(" Comment ({} lines) ", self.line_count))
            .title_style(
                Style::default()
                    .fg(self.theme.text_highlight_fg)
                    .add_modifier(Modifier::BOLD),
            )
            .title_alignment(Alignment::Center)
            .style(Style::default().bg(self.theme.root_bg));

        let inner = block.inner(popup_area);
        block.render(popup_area, buf);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(inner);

        let text_lines: Vec<Line> = self.lines.iter().map(|l| Line::from(l.as_str())).collect();
        Paragraph::new(text_lines)
            .style(Style::default().fg(self.theme.text).bg(self.theme.root_bg))
            .render(chunks[0], buf);

        let sep_text = "\u{2500}".repeat(chunks[1].width as usize);
        Paragraph::new(sep_text)
            .style(Style::default().fg(self.theme.text).bg(self.theme.root_bg))
            .render(chunks[1], buf);

        let key_style = Style::default()
            .fg(self.theme.text_highlight_fg)
            .add_modifier(Modifier::BOLD);
        let txt_style = Style::default().fg(self.theme.text);
        let br_style = Style::default().fg(self.theme.text);
        let mut footer_spans: Vec<Span<'static>> = Vec::new();
        popup_entry(
            &mut footer_spans,
            self.keybindings.comment.newline.display(),
            "newline",
            key_style,
            txt_style,
            br_style,
        );
        popup_entry(
            &mut footer_spans,
            self.keybindings.comment.save.display(),
            "save",
            key_style,
            txt_style,
            br_style,
        );
        popup_entry(
            &mut footer_spans,
            self.keybindings.comment.cancel.display(),
            "cancel",
            key_style,
            txt_style,
            br_style,
        );
        Paragraph::new(Line::from(footer_spans))
            .style(Style::default().bg(self.theme.root_bg))
            .render(chunks[2], buf);
    }
}
