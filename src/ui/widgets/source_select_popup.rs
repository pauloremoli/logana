use ratatui::{
    prelude::*,
    style::Modifier,
    widgets::{
        Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, StatefulWidget,
    },
};

use crate::config::{DEFAULT_DLT_PORT, Keybindings};
use crate::theme::Theme;

use super::popup_entry;

pub struct DockerSelectPopup<'a> {
    pub theme: &'a Theme,
    pub keybindings: &'a Keybindings,
    pub containers: &'a [crate::mode::docker_select_mode::DockerContainer],
    pub selected: usize,
    pub error: Option<&'a str>,
}

impl<'a> Widget for DockerSelectPopup<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let popup_width = (area.width.saturating_sub(4)).clamp(50, 80);
        let content_rows = if self.error.is_some() {
            3u16
        } else {
            let inner_w = popup_width.saturating_sub(2) as usize;
            let name_w = inner_w * 35 / 100;
            self.containers
                .iter()
                .map(|c| {
                    if name_w == 0 || c.name.is_empty() {
                        1u16
                    } else {
                        c.name.chars().count().div_ceil(name_w) as u16
                    }
                })
                .sum::<u16>()
                .max(1)
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
            let total_w = vsplit[0].width as usize;
            let name_w = total_w * 35 / 100;
            let image_w = total_w * 35 / 100;
            let status_w = total_w.saturating_sub(name_w + image_w + 2);

            // Compute how many visual rows each container occupies (name may wrap)
            let name_row_counts: Vec<usize> = self
                .containers
                .iter()
                .map(|c| {
                    if name_w == 0 || c.name.is_empty() {
                        1
                    } else {
                        c.name.chars().count().div_ceil(name_w)
                    }
                })
                .collect();

            let row_starts: Vec<usize> = {
                let mut v = vec![0usize; self.containers.len() + 1];
                for i in 0..self.containers.len() {
                    v[i + 1] = v[i] + name_row_counts[i];
                }
                v
            };
            let total_visual_rows = *row_starts.last().unwrap_or(&0);

            let selected_visual_row = if self.selected < self.containers.len() {
                row_starts[self.selected]
            } else {
                0
            };
            let scroll = if selected_visual_row >= content_h {
                selected_visual_row - content_h + 1
            } else {
                0
            };

            let mut lines: Vec<Line> = Vec::new();
            for (i, c) in self.containers.iter().enumerate() {
                if lines.len() >= content_h {
                    break;
                }
                let container_start = row_starts[i];
                if container_start + name_row_counts[i] <= scroll {
                    continue;
                }
                if container_start >= scroll + content_h {
                    break;
                }

                let is_selected = i == self.selected;
                let prefix = if is_selected { "> " } else { "  " };
                let style = if is_selected {
                    Style::default()
                        .fg(self.theme.text_highlight_fg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(self.theme.text)
                };

                let name_chars: Vec<char> = c.name.chars().collect();
                let name_chunks: Vec<String> = if name_w == 0 || name_chars.is_empty() {
                    vec![c.name.clone()]
                } else {
                    name_chars
                        .chunks(name_w)
                        .map(|chunk| chunk.iter().collect::<String>())
                        .collect()
                };

                let image: String = c.image.chars().take(image_w).collect();
                let status: String = c.status.chars().take(status_w).collect();

                let first_visible_chunk = scroll.saturating_sub(container_start);

                for (chunk_idx, name_chunk) in
                    name_chunks.iter().enumerate().skip(first_visible_chunk)
                {
                    if lines.len() >= content_h {
                        break;
                    }
                    if chunk_idx == 0 {
                        lines.push(Line::from(Span::styled(
                            format!(
                                "{}{:<nw$} {:<iw$} {}",
                                prefix,
                                name_chunk,
                                image,
                                status,
                                nw = name_w,
                                iw = image_w
                            ),
                            style,
                        )));
                    } else {
                        lines.push(Line::from(Span::styled(
                            format!("  {:<nw$}", name_chunk, nw = name_w),
                            style,
                        )));
                    }
                }
            }

            while lines.len() < content_h {
                lines.push(Line::from(""));
            }

            Paragraph::new(lines)
                .style(Style::default().bg(self.theme.root_bg))
                .render(vsplit[0], buf);

            if total_visual_rows > content_h {
                let mut sb_state = ScrollbarState::new(total_visual_rows.saturating_sub(content_h))
                    .position(scroll);
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
                let host_port = format!("{}:{}", dev.host, dev.port.unwrap_or(DEFAULT_DLT_PORT));
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

#[cfg(test)]
mod tests {
    use crate::config::Keybindings;
    use crate::db::Database;
    use crate::db::LogManager;
    use crate::ingestion::FileReader;
    use crate::mode::docker_select_mode::DockerContainer;
    use crate::mode::docker_select_mode::DockerSelectMode;
    use crate::theme::Theme;
    use crate::ui::App;
    use ratatui::{Terminal, backend::TestBackend};
    use std::sync::Arc;

    async fn make_app(lines: &[&str]) -> App {
        let data: Vec<u8> = lines.join("\n").into_bytes();
        let file_reader = FileReader::from_bytes(data);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
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
    async fn test_docker_select_basic() {
        let mut app = make_app(&["line one", "line two"]).await;
        let containers = vec![
            DockerContainer {
                id: "abc123".to_string(),
                name: "web-app".to_string(),
                image: "nginx:latest".to_string(),
                status: "Up 2 hours".to_string(),
            },
            DockerContainer {
                id: "def456".to_string(),
                name: "db".to_string(),
                image: "postgres:15".to_string(),
                status: "Up 3 hours".to_string(),
            },
            DockerContainer {
                id: "ghi789".to_string(),
                name: "cache".to_string(),
                image: "redis:7".to_string(),
                status: "Up 1 hour".to_string(),
            },
        ];
        app.tabs[0].interaction.mode = Box::new(DockerSelectMode::new(containers));
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_docker_select_error() {
        let mut app = make_app(&["line one", "line two"]).await;
        app.tabs[0].interaction.mode =
            Box::new(DockerSelectMode::with_error("Docker not found".to_string()));
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_docker_select_scrollbar() {
        let mut app = make_app(&["line one", "line two"]).await;
        let containers: Vec<DockerContainer> = (0..25)
            .map(|i| DockerContainer {
                id: format!("id_{}", i),
                name: format!("container_{}", i),
                image: format!("image_{}:latest", i),
                status: format!("Up {} hours", i),
            })
            .collect();
        app.tabs[0].interaction.mode = Box::new(DockerSelectMode::new(containers));
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_docker_select_selected_not_first() {
        let mut app = make_app(&["line one"]).await;
        let containers = vec![
            DockerContainer {
                id: "a".to_string(),
                name: "first".to_string(),
                image: "img:1".to_string(),
                status: "running".to_string(),
            },
            DockerContainer {
                id: "b".to_string(),
                name: "second".to_string(),
                image: "img:2".to_string(),
                status: "running".to_string(),
            },
        ];
        let mut mode = DockerSelectMode::new(containers);
        mode.selected = 1;
        app.tabs[0].interaction.mode = Box::new(mode);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_docker_select_long_names_truncated() {
        let mut app = make_app(&["line one"]).await;
        let containers = vec![DockerContainer {
            id: "abc".to_string(),
            name: "a".repeat(100),
            image: "b".repeat(100),
            status: "c".repeat(100),
        }];
        app.tabs[0].interaction.mode = Box::new(DockerSelectMode::new(containers));
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_docker_select_scroll_past_viewport() {
        let mut app = make_app(&["line one"]).await;
        let containers: Vec<DockerContainer> = (0..30)
            .map(|i| DockerContainer {
                id: format!("id_{i}"),
                name: format!("container_{i}"),
                image: format!("img:{i}"),
                status: "Up".to_string(),
            })
            .collect();
        let mut mode = DockerSelectMode::new(containers);
        mode.selected = 28;
        app.tabs[0].interaction.mode = Box::new(mode);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_dlt_select_empty_devices() {
        use crate::mode::dlt_select_mode::DltSelectMode;
        let mut app = make_app(&["line one"]).await;
        app.tabs[0].interaction.mode = Box::new(DltSelectMode::new(vec![]));
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_dlt_select_with_devices() {
        use crate::config::DltDevice;
        use crate::mode::dlt_select_mode::DltSelectMode;
        let mut app = make_app(&["line one"]).await;
        let devices = vec![
            DltDevice {
                name: "ecu1".to_string(),
                host: "192.168.1.1".to_string(),
                port: Some(3490),
            },
            DltDevice {
                name: "ecu2".to_string(),
                host: "192.168.1.2".to_string(),
                port: Some(3491),
            },
        ];
        app.tabs[0].interaction.mode = Box::new(DltSelectMode::new(devices));
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_dlt_select_error() {
        use crate::mode::dlt_select_mode::DltSelectMode;
        let mut app = make_app(&["line one"]).await;
        let mut mode = DltSelectMode::new(vec![]);
        mode.error = Some("Connection refused".to_string());
        app.tabs[0].interaction.mode = Box::new(mode);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_dlt_select_add_device_form() {
        use crate::mode::dlt_select_mode::{AddDeviceState, DltSelectMode};
        let mut app = make_app(&["line one"]).await;
        let mut mode = DltSelectMode::new(vec![]);
        mode.adding = Some(AddDeviceState {
            fields: [String::new(), String::new(), "3490".to_string()],
            active_field: 0,
            cursor: 0,
        });
        app.tabs[0].interaction.mode = Box::new(mode);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_dlt_select_scrollbar_many_devices() {
        use crate::config::DltDevice;
        use crate::mode::dlt_select_mode::DltSelectMode;
        let mut app = make_app(&["line one"]).await;
        let devices: Vec<DltDevice> = (0..30)
            .map(|i| DltDevice {
                name: format!("ecu{i}"),
                host: "127.0.0.1".to_string(),
                port: Some(3490 + i as u16),
            })
            .collect();
        app.tabs[0].interaction.mode = Box::new(DltSelectMode::new(devices));
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_dlt_select_selected_not_first() {
        use crate::config::DltDevice;
        use crate::mode::dlt_select_mode::DltSelectMode;
        let mut app = make_app(&["line one"]).await;
        let devices = vec![
            DltDevice {
                name: "a".to_string(),
                host: "127.0.0.1".to_string(),
                port: Some(3490),
            },
            DltDevice {
                name: "b".to_string(),
                host: "127.0.0.2".to_string(),
                port: Some(3491),
            },
        ];
        let mut mode = DltSelectMode::new(devices);
        mode.selected = 1;
        app.tabs[0].interaction.mode = Box::new(mode);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_dlt_select_add_selected() {
        use crate::config::DltDevice;
        use crate::mode::dlt_select_mode::DltSelectMode;
        let mut app = make_app(&["line one"]).await;
        let devices = vec![DltDevice {
            name: "dev1".to_string(),
            host: "127.0.0.1".to_string(),
            port: Some(3490),
        }];
        let mut mode = DltSelectMode::new(devices);
        mode.selected = 1;
        app.tabs[0].interaction.mode = Box::new(mode);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_dlt_select_add_form_with_error() {
        use crate::mode::dlt_select_mode::{AddDeviceState, DltSelectMode};
        let mut app = make_app(&["line one"]).await;
        let mut mode = DltSelectMode::new(vec![]);
        mode.adding = Some(AddDeviceState {
            fields: [String::new(), String::new(), "3490".to_string()],
            active_field: 0,
            cursor: 0,
        });
        mode.error = Some("Port must be a number".to_string());
        app.tabs[0].interaction.mode = Box::new(mode);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_dlt_select_add_form_second_field_active() {
        use crate::mode::dlt_select_mode::{AddDeviceState, DltSelectMode};
        let mut app = make_app(&["line one"]).await;
        let mut mode = DltSelectMode::new(vec![]);
        mode.adding = Some(AddDeviceState {
            fields: ["mydevice".to_string(), String::new(), "3490".to_string()],
            active_field: 1,
            cursor: 0,
        });
        app.tabs[0].interaction.mode = Box::new(mode);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_dlt_select_long_host_truncated() {
        use crate::config::DltDevice;
        use crate::mode::dlt_select_mode::DltSelectMode;
        let mut app = make_app(&["line one"]).await;
        let devices = vec![DltDevice {
            name: "a".repeat(60),
            host: "b".repeat(60),
            port: Some(9999),
        }];
        app.tabs[0].interaction.mode = Box::new(DltSelectMode::new(devices));
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }
}
