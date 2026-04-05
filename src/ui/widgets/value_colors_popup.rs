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

pub struct ValueColorsPopup<'a> {
    pub theme: &'a Theme,
    pub keybindings: &'a Keybindings,
    pub groups: &'a [crate::mode::value_colors_mode::ValueColorGroup],
    pub search: &'a str,
    pub selected: usize,
    pub title: &'a str,
}

impl<'a> Widget for ValueColorsPopup<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        use crate::commands::auto_complete::fuzzy_match as fmatch;
        use crate::mode::value_colors_mode::ValueColorRow;

        let mut vis_rows: Vec<ValueColorRow> = Vec::new();
        for (gi, group) in self.groups.iter().enumerate() {
            if self.search.is_empty() {
                vis_rows.push(ValueColorRow::Group(gi));
                for (ei, _) in group.children.iter().enumerate() {
                    vis_rows.push(ValueColorRow::Entry(gi, ei));
                }
            } else {
                let matching: Vec<usize> = group
                    .children
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| {
                        let haystack = format!("{} {}", group.label, e.label);
                        fmatch(self.search, &haystack)
                    })
                    .map(|(i, _)| i)
                    .collect();
                if !matching.is_empty() {
                    vis_rows.push(ValueColorRow::Group(gi));
                    for ei in matching {
                        vis_rows.push(ValueColorRow::Entry(gi, ei));
                    }
                }
            }
        }

        let popup_width = (area.width.saturating_sub(4)).clamp(40, 60);
        let row_count = vis_rows.len() as u16;
        let extra = if self.search.is_empty() { 5 } else { 6 };
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
            .title(format!(" {} ", self.title))
            .title_style(
                Style::default()
                    .fg(self.theme.text_highlight_fg)
                    .add_modifier(Modifier::BOLD),
            )
            .title_alignment(Alignment::Center)
            .style(Style::default().bg(self.theme.root_bg));

        let inner = block.inner(popup_area);
        block.render(popup_area, buf);

        let has_search = !self.search.is_empty();
        let footer_lines = 3usize;
        let search_rows = if has_search { 1usize } else { 0 };
        let content_h = inner
            .height
            .saturating_sub((footer_lines + search_rows) as u16) as usize;

        let mut constraints = vec![];
        if has_search {
            constraints.push(Constraint::Length(1));
        }
        constraints.push(Constraint::Min(1));
        constraints.push(Constraint::Length(1));
        constraints.push(Constraint::Length(2));

        let vsplit = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);

        let (search_area, content_area, sep_area, footer_area) = if has_search {
            (Some(vsplit[0]), vsplit[1], vsplit[2], vsplit[3])
        } else {
            (None, vsplit[0], vsplit[1], vsplit[2])
        };

        if let Some(sa) = search_area {
            let search_line = Line::from(vec![
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
            ]);
            Paragraph::new(search_line)
                .style(Style::default().bg(self.theme.root_bg))
                .render(sa, buf);
        }

        let scroll = if self.selected >= content_h {
            self.selected - content_h + 1
        } else {
            0
        };

        let group_state: Vec<Option<bool>> = self
            .groups
            .iter()
            .map(|g| {
                let all = g.children.iter().all(|c| c.enabled);
                let none = g.children.iter().all(|c| !c.enabled);
                if all {
                    Some(true)
                } else if none {
                    Some(false)
                } else {
                    None
                }
            })
            .collect();

        let mut lines: Vec<Line> = Vec::new();
        for (i, row) in vis_rows.iter().enumerate().skip(scroll).take(content_h) {
            let is_sel = i == self.selected;
            let prefix = if is_sel { "> " } else { "  " };
            let sel_style = if is_sel {
                Style::default()
                    .fg(self.theme.text_highlight_fg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(self.theme.text)
            };

            match row {
                ValueColorRow::Group(gi) => {
                    let check = match group_state[*gi] {
                        Some(true) => "[x] ",
                        Some(false) => "[ ] ",
                        None => "[-] ",
                    };
                    let header_style = if is_sel {
                        sel_style
                    } else {
                        Style::default()
                            .fg(self.theme.text)
                            .add_modifier(Modifier::BOLD)
                    };
                    lines.push(Line::from(Span::styled(
                        format!("{}{}{}", prefix, check, self.groups[*gi].label),
                        header_style,
                    )));
                }
                ValueColorRow::Entry(gi, ei) => {
                    let entry = &self.groups[*gi].children[*ei];
                    let check = if entry.enabled { "[x] " } else { "[ ] " };
                    let swatch_style = Style::default().fg(entry.color);
                    lines.push(Line::from(vec![
                        Span::styled(format!("  {}{}", prefix, check), sel_style),
                        Span::styled("\u{2588}\u{2588}", swatch_style),
                        Span::styled(format!(" {}", entry.label), sel_style),
                    ]));
                }
            }
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

        let kb = &self.keybindings.value_colors;
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
        line1.push(Span::styled(
            "type to search",
            Style::default().fg(self.theme.text),
        ));
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
        let footer = vec![Line::from(line1), Line::from(line2)];
        Paragraph::new(footer)
            .style(Style::default().bg(self.theme.root_bg))
            .render(footer_area, buf);

        let total = vis_rows.len();
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
    use crate::config::Keybindings;
    use crate::db::Database;
    use crate::db::LogManager;
    use crate::ingestion::FileReader;
    use crate::mode::value_colors_mode::{ValueColorEntry, ValueColorGroup, ValueColorsMode};
    use crate::theme::Theme;
    use crate::ui::App;
    use ratatui::{Terminal, backend::TestBackend};
    use std::collections::HashSet;
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
    async fn test_value_colors_basic() {
        let mut app = make_app(&["line one", "line two"]).await;
        let groups = vec![ValueColorGroup {
            label: "HTTP Methods".to_string(),
            children: vec![
                ValueColorEntry {
                    key: "http_get".to_string(),
                    label: "GET".to_string(),
                    color: ratatui::style::Color::Green,
                    enabled: true,
                },
                ValueColorEntry {
                    key: "http_post".to_string(),
                    label: "POST".to_string(),
                    color: ratatui::style::Color::Yellow,
                    enabled: true,
                },
            ],
        }];
        let mode = ValueColorsMode::new(groups, HashSet::new());
        app.tabs[0].interaction.mode = Box::new(mode);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_value_colors_with_search() {
        let mut app = make_app(&["line one", "line two"]).await;
        let groups = vec![ValueColorGroup {
            label: "HTTP Methods".to_string(),
            children: vec![
                ValueColorEntry {
                    key: "http_get".to_string(),
                    label: "GET".to_string(),
                    color: ratatui::style::Color::Green,
                    enabled: true,
                },
                ValueColorEntry {
                    key: "http_post".to_string(),
                    label: "POST".to_string(),
                    color: ratatui::style::Color::Yellow,
                    enabled: true,
                },
            ],
        }];
        let mut mode = ValueColorsMode::new(groups, HashSet::new());
        mode.search = "http".to_string();
        app.tabs[0].interaction.mode = Box::new(mode);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_value_colors_partial_enabled() {
        let mut app = make_app(&["line one", "line two"]).await;
        let groups = vec![ValueColorGroup {
            label: "Status Codes".to_string(),
            children: vec![
                ValueColorEntry {
                    key: "status_2xx".to_string(),
                    label: "2xx".to_string(),
                    color: ratatui::style::Color::Green,
                    enabled: true,
                },
                ValueColorEntry {
                    key: "status_4xx".to_string(),
                    label: "4xx".to_string(),
                    color: ratatui::style::Color::Red,
                    enabled: false,
                },
                ValueColorEntry {
                    key: "status_5xx".to_string(),
                    label: "5xx".to_string(),
                    color: ratatui::style::Color::Magenta,
                    enabled: true,
                },
            ],
        }];
        let mut disabled = HashSet::new();
        disabled.insert("status_4xx".to_string());
        let mode = ValueColorsMode::new(groups, disabled);
        app.tabs[0].interaction.mode = Box::new(mode);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_value_colors_scrollbar() {
        let mut app = make_app(&["line one", "line two"]).await;
        let children: Vec<ValueColorEntry> = (0..30)
            .map(|i| ValueColorEntry {
                key: format!("key_{}", i),
                label: format!("Entry {}", i),
                color: ratatui::style::Color::Cyan,
                enabled: true,
            })
            .collect();
        let groups = vec![ValueColorGroup {
            label: "Many Entries".to_string(),
            children,
        }];
        let mode = ValueColorsMode::new(groups, HashSet::new());
        app.tabs[0].interaction.mode = Box::new(mode);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }
}
