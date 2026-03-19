use ratatui::{
    prelude::*,
    style::Modifier,
    widgets::{
        Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, StatefulWidget,
    },
};

use crate::config::Keybindings;
use crate::theme::Theme;

use super::popup_entry;

pub struct SelectFieldsPopup<'a> {
    pub theme: &'a Theme,
    pub keybindings: &'a Keybindings,
    pub fields: &'a [(String, bool)],
    pub selected: usize,
}

impl<'a> Widget for SelectFieldsPopup<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let popup_width = (area.width.saturating_sub(4)).clamp(40, 60);
        let content_rows = self.fields.len() as u16;
        let popup_height = (content_rows + 5)
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
            .title(" Select Fields ")
            .title_style(
                Style::default()
                    .fg(self.theme.text_highlight_fg)
                    .add_modifier(Modifier::BOLD),
            )
            .title_alignment(Alignment::Center)
            .style(Style::default().bg(self.theme.root_bg));

        let inner = block.inner(popup_area);
        block.render(popup_area, buf);

        let inner_h = inner.height as usize;
        let footer_lines = 3usize;
        let content_h = inner_h.saturating_sub(footer_lines);

        let vsplit = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),
                Constraint::Length(1),
                Constraint::Length(2),
            ])
            .split(inner);

        let scroll = if self.selected >= content_h {
            self.selected - content_h + 1
        } else {
            0
        };

        let mut lines: Vec<Line> = Vec::new();
        for (i, (name, enabled)) in self.fields.iter().enumerate().skip(scroll).take(content_h) {
            let is_selected = i == self.selected;
            let prefix = if is_selected { "> " } else { "  " };
            let check = if *enabled { "[x] " } else { "[ ] " };
            let style = if is_selected {
                Style::default()
                    .fg(self.theme.text_highlight_fg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(self.theme.text)
            };
            lines.push(Line::from(Span::styled(
                format!("{}{}{}", prefix, check, name),
                style,
            )));
        }

        while lines.len() < content_h {
            lines.push(Line::from(""));
        }

        Paragraph::new(lines)
            .style(Style::default().bg(self.theme.root_bg))
            .render(vsplit[0], buf);

        let sep = "\u{2500}".repeat(vsplit[1].width as usize);
        Paragraph::new(sep)
            .style(Style::default().fg(self.theme.text))
            .render(vsplit[1], buf);

        let kb = &self.keybindings.select_fields;
        let key_style = Style::default()
            .fg(self.theme.text_highlight_fg)
            .add_modifier(Modifier::BOLD);
        let txt_style = Style::default().fg(self.theme.text);
        let br_style = Style::default().fg(self.theme.text);
        let mut line1: Vec<Span<'static>> = Vec::new();
        popup_entry(
            &mut line1,
            kb.toggle.display(),
            "toggle",
            key_style,
            txt_style,
            br_style,
        );
        line1.push(Span::styled("<", br_style));
        line1.push(Span::styled(
            format!("{}/{}", kb.move_up.display(), kb.move_down.display()),
            key_style,
        ));
        line1.push(Span::styled("> reorder  ", txt_style));
        popup_entry(
            &mut line1,
            kb.all.display(),
            "all",
            key_style,
            txt_style,
            br_style,
        );
        popup_entry(
            &mut line1,
            kb.none.display(),
            "none",
            key_style,
            txt_style,
            br_style,
        );
        let mut line2: Vec<Span<'static>> = Vec::new();
        popup_entry(
            &mut line2,
            kb.apply.display(),
            "apply",
            key_style,
            txt_style,
            br_style,
        );
        popup_entry(
            &mut line2,
            kb.cancel.display(),
            "cancel",
            key_style,
            txt_style,
            br_style,
        );
        let footer_lines = vec![Line::from(line1), Line::from(line2)];
        Paragraph::new(footer_lines)
            .style(Style::default().bg(self.theme.root_bg))
            .render(vsplit[2], buf);

        let total = self.fields.len();
        if total > content_h {
            let mut sb_state =
                ScrollbarState::new(total.saturating_sub(content_h)).position(scroll);
            StatefulWidget::render(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .style(Style::default().fg(self.theme.border)),
                vsplit[0],
                buf,
                &mut sb_state,
            );
        }
    }
}
