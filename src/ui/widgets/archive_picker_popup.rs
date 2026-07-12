use ratatui::{
    prelude::*,
    style::Modifier,
    widgets::{
        Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, StatefulWidget,
    },
};

use crate::config::Keybindings;
use crate::ingestion::CheckState;
use crate::mode::archive_picker_mode::ArchiveRow;
use crate::theme::Theme;

use super::popup_entry;

/// The checkbox glyph shown for a row's [`CheckState`] — a pure function so
/// its three cases can be unit-tested without rendering anything.
pub fn checkbox_glyph(state: CheckState) -> &'static str {
    match state {
        CheckState::Checked => "[x] ",
        CheckState::Unchecked => "[ ] ",
        CheckState::Partial => "[~] ",
    }
}

pub struct ArchivePickerPopup<'a> {
    pub theme: &'a Theme,
    pub keybindings: &'a Keybindings,
    pub rows: &'a [ArchiveRow],
    pub selected: usize,
    pub source_path: &'a str,
    /// Live typeahead query narrowing `rows`; empty when not searching or
    /// when search is active but nothing has been typed yet.
    pub search: &'a str,
    /// True while capturing search input. Distinct from `!search.is_empty()`
    /// so the search row can show a `[SEARCH]` marker the instant the search
    /// key is pressed, before any character is typed.
    pub searching: bool,
}

impl<'a> Widget for ArchivePickerPopup<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let popup_width = (area.width.saturating_sub(4)).clamp(40, 80);
        let content_rows = self.rows.len() as u16;
        let extra = if self.searching { 6 } else { 5 };
        let popup_height = (content_rows + extra)
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
            .title(format!(" Archive Contents: {} ", self.source_path))
            .title_style(
                Style::default()
                    .fg(self.theme.text_highlight_fg)
                    .add_modifier(Modifier::BOLD),
            )
            .title_alignment(Alignment::Center)
            .style(Style::default().bg(self.theme.root_bg));

        let inner = block.inner(popup_area);
        block.render(popup_area, buf);

        let has_search = self.searching;
        let inner_h = inner.height as usize;
        let footer_lines = 3usize;
        let search_rows = if has_search { 1usize } else { 0 };
        let content_h = inner_h.saturating_sub(footer_lines + search_rows);

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
            let search_line = if self.search.is_empty() {
                Line::from(Span::styled(
                    " [SEARCH]",
                    Style::default()
                        .fg(self.theme.text_highlight_fg)
                        .add_modifier(Modifier::BOLD),
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
                .render(sa, buf);
        }

        let scroll = if self.selected >= content_h {
            self.selected - content_h + 1
        } else {
            0
        };

        let mut lines: Vec<Line> = Vec::new();
        for (i, row) in self.rows.iter().enumerate().skip(scroll).take(content_h) {
            let is_selected = i == self.selected;
            let prefix = if is_selected { "> " } else { "  " };
            let indent = "  ".repeat(row.depth);
            let checkbox = checkbox_glyph(row.check_state);
            let style = if row.is_error {
                Style::default().fg(self.theme.error_fg)
            } else if is_selected {
                Style::default()
                    .fg(self.theme.text_highlight_fg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(self.theme.text)
            };
            lines.push(Line::from(Span::styled(
                format!("{prefix}{indent}{checkbox}{}", row.name),
                style,
            )));
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
        popup_entry(
            &mut line1,
            kb.search.display(),
            "search",
            key_style,
            txt_style,
            br_style,
        );
        let mut line2: Vec<Span<'static>> = Vec::new();
        popup_entry(
            &mut line2,
            kb.apply.display(),
            "extract",
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

        let total = self.rows.len();
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
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn test_checkbox_glyph_checked() {
        assert_eq!(checkbox_glyph(CheckState::Checked), "[x] ");
    }

    #[test]
    fn test_checkbox_glyph_unchecked() {
        assert_eq!(checkbox_glyph(CheckState::Unchecked), "[ ] ");
    }

    #[test]
    fn test_checkbox_glyph_partial() {
        assert_eq!(checkbox_glyph(CheckState::Partial), "[~] ");
    }

    fn row(name: &str) -> ArchiveRow {
        ArchiveRow {
            name: name.to_string(),
            depth: 0,
            is_container: false,
            check_state: CheckState::Unchecked,
            is_error: false,
        }
    }

    fn row_text(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width)
            .map(|c| {
                buf.cell(ratatui::prelude::Position::new(c, y))
                    .unwrap()
                    .symbol()
                    .to_string()
            })
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    #[test]
    fn test_no_search_row_when_search_empty() {
        let theme = Theme::default();
        let kb = Keybindings::default();
        let rows = vec![row("a.log"), row("b.log")];
        let popup = ArchivePickerPopup {
            theme: &theme,
            keybindings: &kb,
            rows: &rows,
            selected: 0,
            source_path: "archive.zip",
            search: "",
            searching: false,
        };
        let mut terminal = Terminal::new(TestBackend::new(60, 15)).unwrap();
        let buf = terminal.draw(|f| f.render_widget(popup, f.area())).unwrap();
        let text: String = (0..15)
            .map(|y| row_text(buf.buffer, y))
            .collect::<Vec<_>>()
            .join("\n");
        // No search row means the first file row is right under the title,
        // with no `[SEARCH]`/`/query` line pushed in above it.
        assert!(text.contains("a.log"));
        assert!(!text.contains("SEARCH"));
    }

    #[test]
    fn test_search_marker_shown_immediately_on_empty_query() {
        let theme = Theme::default();
        let kb = Keybindings::default();
        let rows = vec![row("a.log")];
        let popup = ArchivePickerPopup {
            theme: &theme,
            keybindings: &kb,
            rows: &rows,
            selected: 0,
            source_path: "archive.zip",
            search: "",
            searching: true,
        };
        let mut terminal = Terminal::new(TestBackend::new(60, 15)).unwrap();
        let buf = terminal.draw(|f| f.render_widget(popup, f.area())).unwrap();
        let text: String = (0..15)
            .map(|y| row_text(buf.buffer, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("SEARCH"));
    }

    #[test]
    fn test_search_row_shown_with_query_when_searching() {
        let theme = Theme::default();
        let kb = Keybindings::default();
        let rows = vec![row("a.log")];
        let popup = ArchivePickerPopup {
            theme: &theme,
            keybindings: &kb,
            rows: &rows,
            selected: 0,
            source_path: "archive.zip",
            search: "err",
            searching: true,
        };
        let mut terminal = Terminal::new(TestBackend::new(60, 15)).unwrap();
        let buf = terminal.draw(|f| f.render_widget(popup, f.area())).unwrap();
        let text: String = (0..15)
            .map(|y| row_text(buf.buffer, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("/err"));
    }
}
