use ratatui::{
    prelude::*,
    style::Modifier,
    widgets::{Block, Borders, Padding, Paragraph},
};

use crate::config::Keybindings;
use crate::theme::Theme;

use super::popup_entry;

pub struct ConfirmRestoreModal<'a> {
    pub theme: &'a Theme,
    pub keybindings: &'a Keybindings,
}

impl<'a> Widget for ConfirmRestoreModal<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let modal_width = 44_u16;
        let modal_height = 5_u16;
        let x = area.x + (area.width.saturating_sub(modal_width)) / 2;
        let y = area.y + (area.height.saturating_sub(modal_height)) / 2;
        let modal_area = Rect::new(x, y, modal_width, modal_height);

        let kb = &self.keybindings;
        let key_style = Style::default()
            .fg(self.theme.text_highlight_fg)
            .add_modifier(Modifier::BOLD);
        let txt_style = Style::default().fg(self.theme.text);
        let br_style = Style::default().fg(self.theme.text);

        let mut spans: Vec<Span<'static>> = vec![Span::styled(" ", txt_style)];
        popup_entry(
            &mut spans,
            kb.confirm.yes.display(),
            "yes",
            key_style,
            txt_style,
            br_style,
        );
        popup_entry(
            &mut spans,
            kb.confirm.no.display(),
            "no",
            key_style,
            txt_style,
            br_style,
        );

        ratatui::widgets::Clear.render(modal_area, buf);
        Paragraph::new(Line::from(spans))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(self.theme.border_title))
                    .title(" Restore previous session? ")
                    .title_style(
                        Style::default()
                            .fg(self.theme.text_highlight_fg)
                            .add_modifier(Modifier::BOLD),
                    )
                    .title_alignment(Alignment::Center)
                    .padding(Padding::new(0, 0, 1, 0)),
            )
            .style(Style::default().bg(self.theme.root_bg))
            .render(modal_area, buf);
    }
}

pub struct ConfirmRestoreSessionModal<'a> {
    pub theme: &'a Theme,
    pub keybindings: &'a Keybindings,
    pub files: &'a [String],
}

impl<'a> Widget for ConfirmRestoreSessionModal<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let file_names: Vec<&str> = self
            .files
            .iter()
            .map(|f| {
                std::path::Path::new(f)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(f.as_str())
            })
            .collect();

        let modal_width = 50_u16;
        let modal_height = (file_names.len() as u16 + 6).min(area.height);
        let x = area.x + (area.width.saturating_sub(modal_width)) / 2;
        let y = area.y + (area.height.saturating_sub(modal_height)) / 2;
        let modal_area = Rect::new(x, y, modal_width, modal_height);

        ratatui::widgets::Clear.render(modal_area, buf);

        let mut lines: Vec<Line> = vec![Line::from(Span::styled(
            " Files:",
            Style::default().fg(self.theme.text),
        ))];
        for name in &file_names {
            lines.push(Line::from(Span::styled(
                format!("  \u{2022} {}", name),
                Style::default().fg(self.theme.text),
            )));
        }
        lines.push(Line::from(""));

        let kb = &self.keybindings.confirm;
        let key_style = Style::default()
            .fg(self.theme.text_highlight_fg)
            .add_modifier(Modifier::BOLD);
        let txt_style = Style::default().fg(self.theme.text);
        let br_style = Style::default().fg(self.theme.text);
        let mut yn_spans: Vec<Span<'static>> = vec![Span::styled(" ", txt_style)];
        popup_entry(
            &mut yn_spans,
            kb.yes.display(),
            "yes",
            key_style,
            txt_style,
            br_style,
        );
        popup_entry(
            &mut yn_spans,
            kb.no.display(),
            "no",
            key_style,
            txt_style,
            br_style,
        );
        lines.push(Line::from(yn_spans));

        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(self.theme.border_title))
                    .title(" Restore last session? ")
                    .title_style(
                        Style::default()
                            .fg(self.theme.text_highlight_fg)
                            .add_modifier(Modifier::BOLD),
                    )
                    .title_alignment(Alignment::Center)
                    .padding(Padding::new(0, 0, 1, 0)),
            )
            .style(Style::default().bg(self.theme.root_bg))
            .render(modal_area, buf);
    }
}

pub struct ConfirmOpenDirModal<'a> {
    pub theme: &'a Theme,
    pub keybindings: &'a Keybindings,
    pub files: &'a [String],
}

impl<'a> Widget for ConfirmOpenDirModal<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        const MAX_DISPLAY: usize = 10;
        let display_count = self.files.len().min(MAX_DISPLAY);
        let extra = self.files.len().saturating_sub(MAX_DISPLAY);
        let extra_line: u16 = if extra > 0 { 1 } else { 0 };
        let modal_width = 60_u16;
        let modal_height = (display_count as u16 + 4 + extra_line).min(area.height);
        let x = area.x + (area.width.saturating_sub(modal_width)) / 2;
        let y = area.y + (area.height.saturating_sub(modal_height)) / 2;
        let modal_area = Rect::new(x, y, modal_width, modal_height);

        ratatui::widgets::Clear.render(modal_area, buf);

        let txt_style = Style::default().fg(self.theme.text);
        let br_style = Style::default().fg(self.theme.text);
        let key_style = Style::default()
            .fg(self.theme.text_highlight_fg)
            .add_modifier(Modifier::BOLD);

        let mut lines_out: Vec<Line> = Vec::new();
        for path in self.files.iter().take(MAX_DISPLAY) {
            let name = std::path::Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(path.as_str());
            lines_out.push(Line::from(Span::styled(
                format!("  \u{2022} {}", name),
                txt_style,
            )));
        }
        if extra > 0 {
            lines_out.push(Line::from(Span::styled(
                format!("  \u{2026} and {} more", extra),
                br_style,
            )));
        }
        lines_out.push(Line::from(""));

        let kb = &self.keybindings.confirm;
        let mut yn_spans: Vec<Span<'static>> = vec![Span::styled(" ", txt_style)];
        popup_entry(
            &mut yn_spans,
            kb.yes.display(),
            "yes",
            key_style,
            txt_style,
            br_style,
        );
        popup_entry(
            &mut yn_spans,
            kb.no.display(),
            "no",
            key_style,
            txt_style,
            br_style,
        );
        lines_out.push(Line::from(yn_spans));

        let title = format!(
            " Open directory? ({} file{}) ",
            self.files.len(),
            if self.files.len() == 1 { "" } else { "s" }
        );
        Paragraph::new(lines_out)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(self.theme.border_title))
                    .title(title)
                    .title_style(
                        Style::default()
                            .fg(self.theme.text_highlight_fg)
                            .add_modifier(Modifier::BOLD),
                    )
                    .title_alignment(Alignment::Center)
                    .padding(Padding::new(0, 0, 1, 0)),
            )
            .style(Style::default().bg(self.theme.root_bg))
            .render(modal_area, buf);
    }
}
