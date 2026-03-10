//! Popup and modal renderers: confirm dialogs, field selector, value colors,
//! Docker container picker, keybindings help, and comment editor.
//!
//! Each popup is rendered as a floating overlay using ratatui's [`Clear`]
//! widget. Popup state is carried by the corresponding [`crate::mode`] variant
//! and exposed to the renderer via [`crate::mode::app_mode::ModeRenderState`].

use ratatui::{
    Frame,
    prelude::*,
    style::Modifier,
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
};

use crate::config::Keybindings;

use super::App;

/// Append `<KEY> action  ` spans to a footer line (popup style).
fn popup_entry(
    spans: &mut Vec<Span<'static>>,
    key: String,
    label: &str,
    key_style: Style,
    txt_style: Style,
    br_style: Style,
) {
    spans.push(Span::styled("<", br_style));
    spans.push(Span::styled(key, key_style));
    spans.push(Span::styled(format!("> {}  ", label), txt_style));
}

impl App {
    pub(super) fn render_confirm_restore_modal(&mut self, frame: &mut Frame<'_>) {
        let modal_width = 44_u16;
        let modal_height = 5_u16;
        let area = frame.area();
        let x = area.x + (area.width.saturating_sub(modal_width)) / 2;
        let y = area.y + (area.height.saturating_sub(modal_height)) / 2;
        let modal_area = ratatui::layout::Rect::new(x, y, modal_width, modal_height);

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

        frame.render_widget(ratatui::widgets::Clear, modal_area);
        let modal = Paragraph::new(Line::from(spans))
            .alignment(ratatui::layout::Alignment::Center)
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
                    .title_alignment(ratatui::layout::Alignment::Center)
                    .padding(ratatui::widgets::Padding::new(0, 0, 1, 0)),
            )
            .style(Style::default().bg(self.theme.root_bg));
        frame.render_widget(modal, modal_area);
    }

    pub(super) fn render_select_fields_popup(
        &mut self,
        frame: &mut Frame<'_>,
        fields: &[(String, bool)],
        selected: usize,
    ) {
        let area = frame.area();
        let popup_width = (area.width.saturating_sub(4)).clamp(40, 60);
        // 2 border rows + 1 separator + 2 footer lines + fields list
        let content_rows = fields.len() as u16;
        let popup_height = (content_rows + 5)
            .min(area.height * 4 / 5)
            .max(9)
            .min(area.height.saturating_sub(2));
        let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
        let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
        let popup_area = Rect::new(x, y, popup_width, popup_height);

        frame.render_widget(ratatui::widgets::Clear, popup_area);

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
        frame.render_widget(block, popup_area);

        // Split inner into content + separator + footer (2 lines)
        let inner_h = inner.height as usize;
        let footer_lines = 3usize; // separator + 2 hint lines
        let content_h = inner_h.saturating_sub(footer_lines);

        let vsplit = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),    // content
                Constraint::Length(1), // separator
                Constraint::Length(2), // footer
            ])
            .split(inner);

        // Scroll so selected is visible
        let scroll = if selected >= content_h {
            selected - content_h + 1
        } else {
            0
        };

        let mut lines: Vec<Line> = Vec::new();
        for (i, (name, enabled)) in fields.iter().enumerate().skip(scroll).take(content_h) {
            let is_selected = i == selected;
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

        // Pad remaining lines
        while lines.len() < content_h {
            lines.push(Line::from(""));
        }

        frame.render_widget(
            Paragraph::new(lines).style(Style::default().bg(self.theme.root_bg)),
            vsplit[0],
        );

        // Separator
        let sep = "─".repeat(vsplit[1].width as usize);
        frame.render_widget(
            Paragraph::new(sep).style(Style::default().fg(self.theme.text)),
            vsplit[1],
        );

        // Footer (two lines)
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
        // Move up/down combined display
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
        frame.render_widget(
            Paragraph::new(footer_lines).style(Style::default().bg(self.theme.root_bg)),
            vsplit[2],
        );

        // Scrollbar if needed
        let total = fields.len();
        if total > content_h {
            let mut sb_state =
                ScrollbarState::new(total.saturating_sub(content_h)).position(scroll);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .style(Style::default().fg(self.theme.border)),
                vsplit[0],
                &mut sb_state,
            );
        }
    }

    pub(super) fn render_value_colors_popup(
        &mut self,
        frame: &mut Frame<'_>,
        groups: &[crate::mode::value_colors_mode::ValueColorGroup],
        search: &str,
        selected: usize,
        title: &str,
    ) {
        use crate::auto_complete::fuzzy_match as fmatch;
        use crate::mode::value_colors_mode::ValueColorRow;

        // Build the flat visible-row list (same logic as ValueColorsMode::visible_rows).
        let mut vis_rows: Vec<ValueColorRow> = Vec::new();
        for (gi, group) in groups.iter().enumerate() {
            if search.is_empty() {
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
                        fmatch(search, &haystack)
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

        let area = frame.area();
        let popup_width = (area.width.saturating_sub(4)).clamp(40, 60);
        let row_count = vis_rows.len() as u16;
        // +5 = 2 border + 1 separator + 2 footer; +1 for search bar when active
        let extra = if search.is_empty() { 5 } else { 6 };
        let popup_height = (row_count + extra)
            .min(area.height * 4 / 5)
            .max(9)
            .min(area.height.saturating_sub(2));
        let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
        let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
        let popup_area = Rect::new(x, y, popup_width, popup_height);

        frame.render_widget(ratatui::widgets::Clear, popup_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.theme.border_title))
            .title(format!(" {} ", title))
            .title_style(
                Style::default()
                    .fg(self.theme.text_highlight_fg)
                    .add_modifier(Modifier::BOLD),
            )
            .title_alignment(Alignment::Center)
            .style(Style::default().bg(self.theme.root_bg));

        let inner = block.inner(popup_area);
        frame.render_widget(block, popup_area);

        // Layout: optional search bar + content + separator + footer
        let has_search = !search.is_empty();
        let footer_lines = 3usize; // separator + 2 hint lines
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

        // Search bar
        if let Some(sa) = search_area {
            let search_line = Line::from(vec![
                Span::styled(
                    " /",
                    Style::default()
                        .fg(self.theme.text_highlight_fg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    search.to_string(),
                    Style::default().fg(self.theme.text_highlight_fg),
                ),
            ]);
            frame.render_widget(
                Paragraph::new(search_line).style(Style::default().bg(self.theme.root_bg)),
                sa,
            );
        }

        // Scroll
        let scroll = if selected >= content_h {
            selected - content_h + 1
        } else {
            0
        };

        // Compute group tri-state for each group
        let group_state: Vec<Option<bool>> = groups
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
            let is_sel = i == selected;
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
                        format!("{}{}{}", prefix, check, groups[*gi].label),
                        header_style,
                    )));
                }
                ValueColorRow::Entry(gi, ei) => {
                    let entry = &groups[*gi].children[*ei];
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

        frame.render_widget(
            Paragraph::new(lines).style(Style::default().bg(self.theme.root_bg)),
            content_area,
        );

        // Separator
        let sep = "\u{2500}".repeat(sep_area.width as usize);
        frame.render_widget(
            Paragraph::new(sep).style(Style::default().fg(self.theme.text)),
            sep_area,
        );

        // Footer
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
        frame.render_widget(
            Paragraph::new(footer).style(Style::default().bg(self.theme.root_bg)),
            footer_area,
        );

        // Scrollbar
        let total = vis_rows.len();
        if total > content_h {
            let mut sb_state =
                ScrollbarState::new(total.saturating_sub(content_h)).position(scroll);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .style(Style::default().fg(self.theme.border)),
                content_area,
                &mut sb_state,
            );
        }
    }

    pub(super) fn render_docker_select_popup(
        &mut self,
        frame: &mut Frame<'_>,
        containers: &[crate::types::DockerContainer],
        selected: usize,
        error: Option<&str>,
    ) {
        let area = frame.area();
        let popup_width = (area.width.saturating_sub(4)).clamp(50, 80);
        let content_rows = if error.is_some() {
            3u16
        } else {
            containers.len() as u16
        };
        // 2 border rows + 1 separator + 1 footer + content
        let popup_height = (content_rows + 4)
            .min(area.height * 4 / 5)
            .max(8)
            .min(area.height.saturating_sub(2));
        let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
        let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
        let popup_area = Rect::new(x, y, popup_width, popup_height);

        frame.render_widget(ratatui::widgets::Clear, popup_area);

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
        frame.render_widget(block, popup_area);

        let vsplit = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),    // content
                Constraint::Length(1), // separator
                Constraint::Length(1), // footer
            ])
            .split(inner);

        let inner_h = inner.height as usize;
        let footer_lines = 2usize; // separator + footer
        let content_h = inner_h.saturating_sub(footer_lines);

        if let Some(err) = error {
            let err_line = Line::from(Span::styled(
                err.to_string(),
                Style::default().fg(self.theme.error_fg),
            ));
            frame.render_widget(
                Paragraph::new(vec![Line::from(""), err_line])
                    .alignment(Alignment::Center)
                    .style(Style::default().bg(self.theme.root_bg)),
                vsplit[0],
            );
        } else {
            // Scroll so selected is visible
            let scroll = if selected >= content_h {
                selected - content_h + 1
            } else {
                0
            };

            // Compute column widths from available space
            let total_w = vsplit[0].width as usize;
            // Layout: "> NAME          IMAGE            STATUS"
            let name_w = total_w * 35 / 100;
            let image_w = total_w * 35 / 100;
            let status_w = total_w.saturating_sub(name_w + image_w + 2); // 2 for prefix

            let mut lines: Vec<Line> = Vec::new();
            for (i, c) in containers.iter().enumerate().skip(scroll).take(content_h) {
                let is_selected = i == selected;
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

            frame.render_widget(
                Paragraph::new(lines).style(Style::default().bg(self.theme.root_bg)),
                vsplit[0],
            );

            // Scrollbar
            let total = containers.len();
            if total > content_h {
                let mut sb_state =
                    ScrollbarState::new(total.saturating_sub(content_h)).position(scroll);
                frame.render_stateful_widget(
                    Scrollbar::new(ScrollbarOrientation::VerticalRight)
                        .style(Style::default().fg(self.theme.border)),
                    vsplit[0],
                    &mut sb_state,
                );
            }
        }

        // Separator
        let sep = "─".repeat(vsplit[1].width as usize);
        frame.render_widget(
            Paragraph::new(sep).style(Style::default().fg(self.theme.text)),
            vsplit[1],
        );

        // Footer
        let kb = &self.keybindings.docker_select;
        let nav = &self.keybindings.navigation;
        let key_style = Style::default()
            .fg(self.theme.text_highlight_fg)
            .add_modifier(Modifier::BOLD);
        let txt_style = Style::default().fg(self.theme.text);
        let br_style = Style::default().fg(self.theme.text);
        let mut spans: Vec<Span<'static>> = Vec::new();
        // Navigate up/down combined display
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
        frame.render_widget(
            Paragraph::new(footer).style(Style::default().bg(self.theme.root_bg)),
            vsplit[2],
        );
    }

    pub(super) fn render_keybindings_help_popup(
        &mut self,
        frame: &mut Frame<'_>,
        keybindings: &Keybindings,
        scroll: usize,
        search: &str,
    ) {
        use crate::mode::keybindings_help_mode::{HelpRow, build_help_rows, filter_rows};

        let area = frame.area();
        let popup_width = (area.width.saturating_sub(4)).clamp(40, 72);
        // height: up to 80% of screen, min 10
        let popup_height = (area.height * 4 / 5)
            .max(10)
            .min(area.height.saturating_sub(2));
        let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
        let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
        let popup_area = Rect::new(x, y, popup_width, popup_height);

        frame.render_widget(ratatui::widgets::Clear, popup_area);

        // Inner area: popup minus borders (2) minus search bar (1) minus search separator (1)
        let inner_h = popup_height.saturating_sub(4) as usize; // 2 borders + 1 search + 1 sep
        let col_w = (popup_width.saturating_sub(2)) as usize; // inside left/right borders

        // Build and filter rows
        let all_rows = build_help_rows(keybindings);
        let rows = filter_rows(&all_rows, search);

        // Clamp scroll
        let total = rows.len();
        let scroll = scroll.min(total.saturating_sub(inner_h));

        let visible: Vec<&HelpRow> = rows.iter().skip(scroll).take(inner_h).collect();

        // Render each row as a ratatui Line
        // Layout: " <KEY>  ACTION"
        //   1 space + "<" + key (up to key_col) + ">" + gap + action
        let key_col = 14usize;
        let action_col = col_w.saturating_sub(key_col + 5);

        let mut lines: Vec<Line> = Vec::new();
        for row in &visible {
            match row {
                HelpRow::Header(title) => {
                    let bar = "─".repeat(col_w.saturating_sub(title.len() + 3));
                    lines.push(Line::from(vec![Span::styled(
                        format!("── {} {}", title, bar),
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

        // Pad remaining lines
        while lines.len() < inner_h {
            lines.push(Line::from(""));
        }

        // Build the outer block (with scrollbar if needed)
        let close_keys = keybindings.help.close.display();
        let title = if search.is_empty() {
            format!(" Keybindings Help ({} to close) ", close_keys)
        } else {
            format!(" Keybindings Help  /{}█ ", search)
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
        frame.render_widget(block, popup_area);

        // Split inner into search bar, separator, content
        let vsplit = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // search bar
                Constraint::Length(1), // separator
                Constraint::Min(1),    // content
            ])
            .split(inner);

        // Search bar
        let search_display = if search.is_empty() {
            Span::styled("  type to filter…", Style::default().fg(self.theme.text))
        } else {
            Span::styled(
                format!("  /{}", search),
                Style::default().fg(self.theme.text),
            )
        };
        frame.render_widget(Paragraph::new(Line::from(search_display)), vsplit[0]);

        // Separator
        let sep = "─".repeat(vsplit[1].width as usize);
        frame.render_widget(
            Paragraph::new(sep).style(Style::default().fg(self.theme.text)),
            vsplit[1],
        );

        // Content paragraph + scrollbar
        let content_area = vsplit[2];
        frame.render_widget(
            Paragraph::new(lines).style(Style::default().bg(self.theme.root_bg)),
            content_area,
        );

        if total > inner_h {
            let mut sb_state = ScrollbarState::new(total.saturating_sub(inner_h)).position(scroll);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .style(Style::default().fg(self.theme.border)),
                content_area,
                &mut sb_state,
            );
        }
    }

    pub(super) fn render_confirm_restore_session_modal(
        &mut self,
        frame: &mut Frame<'_>,
        files: &[String],
    ) {
        let file_names: Vec<&str> = files
            .iter()
            .map(|f| {
                std::path::Path::new(f)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(f.as_str())
            })
            .collect();

        let modal_width = 50_u16;
        // borders(2) + blank(1) + header(1) + files + blank(1) + y/n(1)
        let modal_height = (file_names.len() as u16 + 6).min(frame.area().height);
        let area = frame.area();
        let x = area.x + (area.width.saturating_sub(modal_width)) / 2;
        let y = area.y + (area.height.saturating_sub(modal_height)) / 2;
        let modal_area = ratatui::layout::Rect::new(x, y, modal_width, modal_height);

        frame.render_widget(ratatui::widgets::Clear, modal_area);

        let mut lines: Vec<Line> = vec![Line::from(Span::styled(
            " Files:",
            Style::default().fg(self.theme.text),
        ))];
        for name in &file_names {
            lines.push(Line::from(Span::styled(
                format!("  • {}", name),
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

        let modal = Paragraph::new(lines)
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
                    .title_alignment(ratatui::layout::Alignment::Center)
                    .padding(ratatui::widgets::Padding::new(0, 0, 1, 0)),
            )
            .style(Style::default().bg(self.theme.root_bg));
        frame.render_widget(modal, modal_area);
    }

    pub(super) fn render_confirm_open_dir_modal(
        &mut self,
        frame: &mut Frame<'_>,
        dir: &str,
        files: &[String],
    ) {
        const MAX_DISPLAY: usize = 10;
        let display_count = files.len().min(MAX_DISPLAY);
        let extra = files.len().saturating_sub(MAX_DISPLAY);
        let extra_line: u16 = if extra > 0 { 1 } else { 0 };
        let modal_width = 60_u16;
        // borders(2) + top-padding(1) + files + extra_line + blank(1) + y/n(1)
        let modal_height = (display_count as u16 + 4 + extra_line).min(frame.area().height);
        let area = frame.area();
        let x = area.x + (area.width.saturating_sub(modal_width)) / 2;
        let y = area.y + (area.height.saturating_sub(modal_height)) / 2;
        let modal_area = ratatui::layout::Rect::new(x, y, modal_width, modal_height);

        frame.render_widget(ratatui::widgets::Clear, modal_area);

        let txt_style = Style::default().fg(self.theme.text);
        let br_style = Style::default().fg(self.theme.text);
        let key_style = Style::default()
            .fg(self.theme.text_highlight_fg)
            .add_modifier(Modifier::BOLD);

        let mut lines_out: Vec<Line> = Vec::new();
        for path in files.iter().take(MAX_DISPLAY) {
            let name = std::path::Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(path.as_str());
            lines_out.push(Line::from(Span::styled(format!("  • {}", name), txt_style)));
        }
        if extra > 0 {
            lines_out.push(Line::from(Span::styled(
                format!("  … and {} more", extra),
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
            files.len(),
            if files.len() == 1 { "" } else { "s" }
        );
        let modal = Paragraph::new(lines_out)
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
                    .title_alignment(ratatui::layout::Alignment::Center)
                    .padding(ratatui::widgets::Padding::new(0, 0, 1, 0)),
            )
            .style(Style::default().bg(self.theme.root_bg));
        frame.render_widget(modal, modal_area);

        let _ = dir; // used in title via `files.len()`, dir shown in mode bar
    }

    pub(super) fn render_comment_popup(
        &mut self,
        frame: &mut Frame<'_>,
        lines: &[String],
        cursor_row: usize,
        cursor_col: usize,
        line_count: usize,
        kb: &Keybindings,
    ) {
        let area = frame.area();
        let popup_width = area.width.saturating_sub(8).clamp(40, 70);
        let text_rows = lines.len().max(1) as u16;
        // borders(2) + text editor rows + separator(1) + footer(1)
        let popup_height = (text_rows + 4).min(area.height.saturating_sub(4)).max(6);
        let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
        let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
        let popup_area = Rect::new(x, y, popup_width, popup_height);

        frame.render_widget(ratatui::widgets::Clear, popup_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.theme.border_title))
            .title(format!(" Comment ({} lines) ", line_count))
            .title_style(
                Style::default()
                    .fg(self.theme.text_highlight_fg)
                    .add_modifier(Modifier::BOLD),
            )
            .title_alignment(Alignment::Center)
            .style(Style::default().bg(self.theme.root_bg));

        let inner = block.inner(popup_area);
        frame.render_widget(block, popup_area);

        // Split inner into: [text editor (growable), separator (1 row), footer (1 row)]
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(inner);

        // Text editor
        let text_lines: Vec<Line> = lines.iter().map(|l| Line::from(l.as_str())).collect();
        frame.render_widget(
            Paragraph::new(text_lines)
                .style(Style::default().fg(self.theme.text).bg(self.theme.root_bg)),
            chunks[0],
        );

        // Separator
        let sep_text = "─".repeat(chunks[1].width as usize);
        frame.render_widget(
            Paragraph::new(sep_text)
                .style(Style::default().fg(self.theme.text).bg(self.theme.root_bg)),
            chunks[1],
        );

        // Footer hints
        let key_style = Style::default()
            .fg(self.theme.text_highlight_fg)
            .add_modifier(Modifier::BOLD);
        let txt_style = Style::default().fg(self.theme.text);
        let br_style = Style::default().fg(self.theme.text);
        let mut footer_spans: Vec<Span<'static>> = Vec::new();
        popup_entry(
            &mut footer_spans,
            kb.comment.newline.display(),
            "newline",
            key_style,
            txt_style,
            br_style,
        );
        popup_entry(
            &mut footer_spans,
            kb.comment.save.display(),
            "save",
            key_style,
            txt_style,
            br_style,
        );
        popup_entry(
            &mut footer_spans,
            kb.comment.cancel.display(),
            "cancel",
            key_style,
            txt_style,
            br_style,
        );
        frame.render_widget(
            Paragraph::new(Line::from(footer_spans)).style(Style::default().bg(self.theme.root_bg)),
            chunks[2],
        );

        // Position cursor inside the text editor area
        let text_area = chunks[0];
        let cur_x = text_area.x + cursor_col as u16;
        let cur_y = text_area.y + cursor_row as u16;
        if cur_x < text_area.x + text_area.width && cur_y < text_area.y + text_area.height {
            frame.set_cursor_position((cur_x, cur_y));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Keybindings, RestoreSessionPolicy};
    use crate::db::{Database, FileContext};
    use crate::file_reader::FileReader;
    use crate::log_manager::LogManager;
    use crate::mode::app_mode::{ConfirmRestoreMode, ConfirmRestoreSessionMode};
    use crate::mode::comment_mode::CommentMode;
    use crate::mode::docker_select_mode::DockerSelectMode;
    use crate::mode::keybindings_help_mode::KeybindingsHelpMode;
    use crate::mode::select_fields_mode::SelectFieldsMode;
    use crate::mode::value_colors_mode::{ValueColorEntry, ValueColorGroup, ValueColorsMode};
    use crate::theme::Theme;
    use crate::types::{DockerContainer, FieldLayout};
    use ratatui::{Terminal, backend::TestBackend};
    use std::collections::HashSet;
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
            RestoreSessionPolicy::default(),
        )
        .await
    }

    fn make_terminal() -> Terminal<TestBackend> {
        Terminal::new(TestBackend::new(80, 24)).unwrap()
    }

    #[tokio::test]
    async fn test_confirm_restore_modal() {
        let mut app = make_app(&["line one", "line two"]).await;
        let context = FileContext {
            source_file: "/tmp/test.log".to_string(),
            scroll_offset: 0,
            search_query: String::new(),
            wrap: true,
            level_colors_disabled: std::collections::HashSet::new(),
            show_sidebar: true,
            horizontal_scroll: 0,
            marked_lines: vec![],
            file_hash: None,
            show_line_numbers: true,
            comments: vec![],
            show_mode_bar: true,
            show_borders: true,
            show_keys: false,
            raw_mode: false,
            sidebar_width: 30,
        };
        app.tabs[0].mode = Box::new(ConfirmRestoreMode { context });
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_select_fields_basic() {
        let mut app = make_app(&["line one", "line two"]).await;
        let fields = vec![
            ("timestamp".to_string(), true),
            ("level".to_string(), true),
            ("message".to_string(), false),
        ];
        app.tabs[0].mode = Box::new(SelectFieldsMode::new(fields, FieldLayout::default()));
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_select_fields_with_scroll() {
        let mut app = make_app(&["line one", "line two"]).await;
        let fields: Vec<(String, bool)> = (0..35)
            .map(|i| (format!("field_{}", i), i % 2 == 0))
            .collect();
        app.tabs[0].mode = Box::new(SelectFieldsMode::new(fields, FieldLayout::default()));
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
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
        app.tabs[0].mode = Box::new(mode);
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
        app.tabs[0].mode = Box::new(mode);
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
        app.tabs[0].mode = Box::new(mode);
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
        app.tabs[0].mode = Box::new(mode);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
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
        app.tabs[0].mode = Box::new(DockerSelectMode::new(containers));
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_docker_select_error() {
        let mut app = make_app(&["line one", "line two"]).await;
        app.tabs[0].mode = Box::new(DockerSelectMode::with_error("Docker not found".to_string()));
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
        app.tabs[0].mode = Box::new(DockerSelectMode::new(containers));
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_keybindings_help_basic() {
        let mut app = make_app(&["line one", "line two"]).await;
        app.tabs[0].mode = Box::new(KeybindingsHelpMode::new());
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_keybindings_help_with_search() {
        let mut app = make_app(&["line one", "line two"]).await;
        let mut mode = KeybindingsHelpMode::new();
        mode.search = "scroll".to_string();
        app.tabs[0].mode = Box::new(mode);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_keybindings_help_scroll() {
        let mut app = make_app(&["line one", "line two"]).await;
        let mut mode = KeybindingsHelpMode::new();
        mode.scroll = 5;
        app.tabs[0].mode = Box::new(mode);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_confirm_restore_session() {
        let mut app = make_app(&["line one", "line two"]).await;
        app.tabs[0].mode = Box::new(ConfirmRestoreSessionMode {
            files: vec!["file1.log".to_string(), "file2.log".to_string()],
        });
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_comment_popup_basic() {
        let mut app = make_app(&["line one", "line two"]).await;
        app.tabs[0].mode = Box::new(CommentMode::new(vec![0, 1]));
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_comment_popup_multiline() {
        let mut app = make_app(&["line one", "line two", "line three"]).await;
        let mut mode = CommentMode::new(vec![0, 1]);
        mode.lines = vec!["line 1".to_string(), "line 2".to_string()];
        mode.cursor_row = 1;
        app.tabs[0].mode = Box::new(mode);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_comment_popup_cursor_boundary() {
        let mut app = make_app(&["line one", "line two"]).await;
        let mut mode = CommentMode::new(vec![0]);
        mode.lines = vec!["short".to_string()];
        mode.cursor_col = 100;
        app.tabs[0].mode = Box::new(mode);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }
}
