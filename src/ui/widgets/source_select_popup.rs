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

pub struct DockerSelectPopup<'a> {
    pub theme: &'a Theme,
    pub keybindings: &'a Keybindings,
    pub containers: &'a [crate::types::DockerContainer],
    pub selected: usize,
    pub error: Option<&'a str>,
}

impl<'a> Widget for DockerSelectPopup<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let popup_width = (area.width.saturating_sub(4)).clamp(50, 80);
        let content_rows = if self.error.is_some() {
            3u16
        } else {
            self.containers.len() as u16
        };
        let popup_height = (content_rows + 4)
            .min(area.height * 4 / 5)
            .max(8)
            .min(area.height.saturating_sub(2));
        let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
        let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
        let popup_area = Rect::new(x, y, popup_width, popup_height);

        ratatui::widgets::Clear.render(popup_area, buf);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.theme.border_title))
            .title(" Docker Containers ")
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
                Constraint::Min(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(inner);

        let inner_h = inner.height as usize;
        let footer_lines = 2usize;
        let content_h = inner_h.saturating_sub(footer_lines);

        if let Some(err) = self.error {
            let err_line = Line::from(Span::styled(
                err.to_string(),
                Style::default().fg(self.theme.error_fg),
            ));
            Paragraph::new(vec![Line::from(""), err_line])
                .alignment(Alignment::Center)
                .style(Style::default().bg(self.theme.root_bg))
                .render(vsplit[0], buf);
        } else {
            let scroll = if self.selected >= content_h {
                self.selected - content_h + 1
            } else {
                0
            };

            let total_w = vsplit[0].width as usize;
            let name_w = total_w * 35 / 100;
            let image_w = total_w * 35 / 100;
            let status_w = total_w.saturating_sub(name_w + image_w + 2);

            let mut lines: Vec<Line> = Vec::new();
            for (i, c) in self
                .containers
                .iter()
                .enumerate()
                .skip(scroll)
                .take(content_h)
            {
                let is_selected = i == self.selected;
                let prefix = if is_selected { "> " } else { "  " };
                let name = if c.name.len() > name_w {
                    &c.name[..name_w]
                } else {
                    &c.name
                };
                let image = if c.image.len() > image_w {
                    &c.image[..image_w]
                } else {
                    &c.image
                };
                let status = if c.status.len() > status_w {
                    &c.status[..status_w]
                } else {
                    &c.status
                };
                let style = if is_selected {
                    Style::default()
                        .fg(self.theme.text_highlight_fg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(self.theme.text)
                };
                lines.push(Line::from(Span::styled(
                    format!(
                        "{}{:<nw$} {:<iw$} {}",
                        prefix,
                        name,
                        image,
                        status,
                        nw = name_w,
                        iw = image_w
                    ),
                    style,
                )));
            }

            while lines.len() < content_h {
                lines.push(Line::from(""));
            }

            Paragraph::new(lines)
                .style(Style::default().bg(self.theme.root_bg))
                .render(vsplit[0], buf);

            let total = self.containers.len();
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

        let sep = "\u{2500}".repeat(vsplit[1].width as usize);
        Paragraph::new(sep)
            .style(Style::default().fg(self.theme.text))
            .render(vsplit[1], buf);

        let kb = &self.keybindings.docker_select;
        let nav = &self.keybindings.navigation;
        let key_style = Style::default()
            .fg(self.theme.text_highlight_fg)
            .add_modifier(Modifier::BOLD);
        let txt_style = Style::default().fg(self.theme.text);
        let br_style = Style::default().fg(self.theme.text);
        let mut spans: Vec<Span<'static>> = Vec::new();
        spans.push(Span::styled("<", br_style));
        spans.push(Span::styled(
            format!("{}/{}", nav.scroll_up.display(), nav.scroll_down.display()),
            key_style,
        ));
        spans.push(Span::styled("> navigate  ", txt_style));
        popup_entry(
            &mut spans,
            kb.confirm.display(),
            "attach",
            key_style,
            txt_style,
            br_style,
        );
        popup_entry(
            &mut spans,
            kb.cancel.display(),
            "cancel",
            key_style,
            txt_style,
            br_style,
        );
        let footer = Line::from(spans);
        Paragraph::new(footer)
            .style(Style::default().bg(self.theme.root_bg))
            .render(vsplit[2], buf);
    }
}

pub struct DltSelectPopup<'a> {
    pub theme: &'a Theme,
    pub keybindings: &'a Keybindings,
    pub devices: &'a [crate::config::DltDevice],
    pub selected: usize,
    pub error: Option<&'a str>,
    pub adding: Option<&'a crate::mode::dlt_select_mode::AddDeviceRenderState>,
}

impl<'a> Widget for DltSelectPopup<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let popup_width = (area.width.saturating_sub(4)).clamp(50, 80);

        if let Some(add_state) = self.adding {
            let popup_height = 12u16.min(area.height.saturating_sub(2));
            let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
            let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
            let popup_area = Rect::new(x, y, popup_width, popup_height);

            ratatui::widgets::Clear.render(popup_area, buf);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(self.theme.border_title))
                .title(" Add DLT Device ")
                .title_style(
                    Style::default()
                        .fg(self.theme.text_highlight_fg)
                        .add_modifier(Modifier::BOLD),
                )
                .title_alignment(Alignment::Center)
                .style(Style::default().bg(self.theme.root_bg));

            let inner = block.inner(popup_area);
            block.render(popup_area, buf);

            let labels = ["Name", "Host", "Port"];
            let vsplit = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(2),
                    Constraint::Length(2),
                    Constraint::Length(2),
                    Constraint::Length(1),
                    Constraint::Min(0),
                    Constraint::Length(1),
                ])
                .split(inner);

            let txt_style = Style::default().fg(self.theme.text);
            let active_style = Style::default()
                .fg(self.theme.text_highlight_fg)
                .add_modifier(Modifier::BOLD);

            for (i, label) in labels.iter().enumerate() {
                let is_active = i == add_state.active_field;
                let label_style = if is_active { active_style } else { txt_style };
                let value = &add_state.fields[i];
                let display = if is_active {
                    let cursor_pos = add_state.cursor;
                    let before: String = value.chars().take(cursor_pos).collect();
                    let cursor_ch: String = value.chars().skip(cursor_pos).take(1).collect();
                    let after: String = value.chars().skip(cursor_pos + 1).collect();
                    let cursor_display = if cursor_ch.is_empty() {
                        " ".to_string()
                    } else {
                        cursor_ch
                    };
                    vec![
                        Span::styled(format!("  {}: ", label), label_style),
                        Span::styled(before, txt_style),
                        Span::styled(
                            cursor_display,
                            Style::default()
                                .fg(self.theme.root_bg)
                                .bg(self.theme.text_highlight_fg),
                        ),
                        Span::styled(after, txt_style),
                    ]
                } else {
                    vec![
                        Span::styled(format!("  {}: ", label), label_style),
                        Span::styled(value.clone(), txt_style),
                    ]
                };
                Paragraph::new(Line::from(display))
                    .style(Style::default().bg(self.theme.root_bg))
                    .render(vsplit[i], buf);
            }

            if let Some(err) = self.error {
                Paragraph::new(Span::styled(
                    format!("  {}", err),
                    Style::default().fg(self.theme.error_fg),
                ))
                .style(Style::default().bg(self.theme.root_bg))
                .render(vsplit[3], buf);
            }

            let kb = &self.keybindings.dlt_select;
            let key_style = Style::default()
                .fg(self.theme.text_highlight_fg)
                .add_modifier(Modifier::BOLD);
            let txt_style = Style::default().fg(self.theme.text);
            let br_style = Style::default().fg(self.theme.text);
            let mut footer_spans: Vec<Span<'static>> = Vec::new();
            popup_entry(
                &mut footer_spans,
                self.keybindings.dlt_select.next_field.display(),
                "next field",
                key_style,
                txt_style,
                br_style,
            );
            popup_entry(
                &mut footer_spans,
                kb.confirm.display(),
                "save",
                key_style,
                txt_style,
                br_style,
            );
            popup_entry(
                &mut footer_spans,
                kb.cancel.display(),
                "cancel",
                key_style,
                txt_style,
                br_style,
            );
            Paragraph::new(Line::from(footer_spans))
                .style(Style::default().bg(self.theme.root_bg))
                .render(vsplit[5], buf);
            return;
        }

        let total_entries = self.devices.len() + 1;
        let content_rows = if self.error.is_some() {
            3u16
        } else {
            total_entries as u16
        };
        let popup_height = (content_rows + 4)
            .min(area.height * 4 / 5)
            .max(8)
            .min(area.height.saturating_sub(2));
        let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
        let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
        let popup_area = Rect::new(x, y, popup_width, popup_height);

        ratatui::widgets::Clear.render(popup_area, buf);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.theme.border_title))
            .title(" DLT Devices ")
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
                Constraint::Min(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(inner);

        let inner_h = inner.height as usize;
        let footer_lines = 2usize;
        let content_h = inner_h.saturating_sub(footer_lines);

        if let Some(err) = self.error {
            let err_line = Line::from(Span::styled(
                err.to_string(),
                Style::default().fg(self.theme.error_fg),
            ));
            Paragraph::new(vec![Line::from(""), err_line])
                .alignment(Alignment::Center)
                .style(Style::default().bg(self.theme.root_bg))
                .render(vsplit[0], buf);
        } else {
            let scroll = if self.selected >= content_h {
                self.selected - content_h + 1
            } else {
                0
            };

            let total_w = vsplit[0].width as usize;
            let name_w = total_w * 40 / 100;
            let host_w = total_w.saturating_sub(name_w + 2);

            let mut lines: Vec<Line> = Vec::new();
            for (i, dev) in self.devices.iter().enumerate().skip(scroll).take(content_h) {
                let is_selected = i == self.selected;
                let prefix = if is_selected { "> " } else { "  " };
                let name = if dev.name.len() > name_w {
                    &dev.name[..name_w]
                } else {
                    &dev.name
                };
                let host_port = format!("{}:{}", dev.host, dev.port);
                let hp_display = if host_port.len() > host_w {
                    &host_port[..host_w]
                } else {
                    &host_port
                };
                let style = if is_selected {
                    Style::default()
                        .fg(self.theme.text_highlight_fg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(self.theme.text)
                };
                lines.push(Line::from(Span::styled(
                    format!("{}{:<nw$} {}", prefix, name, hp_display, nw = name_w),
                    style,
                )));
            }

            let add_idx = self.devices.len();
            if add_idx >= scroll && lines.len() < content_h {
                let is_selected = self.selected == add_idx;
                let prefix = if is_selected { "> " } else { "  " };
                let style = if is_selected {
                    Style::default()
                        .fg(self.theme.text_highlight_fg)
                        .add_modifier(Modifier::BOLD | Modifier::DIM)
                } else {
                    Style::default()
                        .fg(self.theme.text)
                        .add_modifier(Modifier::DIM)
                };
                lines.push(Line::from(Span::styled(
                    format!("{}+ Add new device...", prefix),
                    style,
                )));
            }

            while lines.len() < content_h {
                lines.push(Line::from(""));
            }

            Paragraph::new(lines)
                .style(Style::default().bg(self.theme.root_bg))
                .render(vsplit[0], buf);

            if total_entries > content_h {
                let mut sb_state =
                    ScrollbarState::new(total_entries.saturating_sub(content_h)).position(scroll);
                StatefulWidget::render(
                    Scrollbar::new(ScrollbarOrientation::VerticalRight)
                        .style(Style::default().fg(self.theme.border)),
                    vsplit[0],
                    buf,
                    &mut sb_state,
                );
            }
        }

        let sep = "\u{2500}".repeat(vsplit[1].width as usize);
        Paragraph::new(sep)
            .style(Style::default().fg(self.theme.text))
            .render(vsplit[1], buf);

        let kb = &self.keybindings.dlt_select;
        let nav = &self.keybindings.navigation;
        let key_style = Style::default()
            .fg(self.theme.text_highlight_fg)
            .add_modifier(Modifier::BOLD);
        let txt_style = Style::default().fg(self.theme.text);
        let br_style = Style::default().fg(self.theme.text);
        let mut spans: Vec<Span<'static>> = Vec::new();
        spans.push(Span::styled("<", br_style));
        spans.push(Span::styled(
            format!("{}/{}", nav.scroll_up.display(), nav.scroll_down.display()),
            key_style,
        ));
        spans.push(Span::styled("> navigate  ", txt_style));
        popup_entry(
            &mut spans,
            kb.confirm.display(),
            "connect",
            key_style,
            txt_style,
            br_style,
        );
        popup_entry(
            &mut spans,
            kb.delete.display(),
            "delete",
            key_style,
            txt_style,
            br_style,
        );
        popup_entry(
            &mut spans,
            kb.cancel.display(),
            "cancel",
            key_style,
            txt_style,
            br_style,
        );
        let footer = Line::from(spans);
        Paragraph::new(footer)
            .style(Style::default().bg(self.theme.root_bg))
            .render(vsplit[2], buf);
    }
}
