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

/// The single mark glyph for a row — `m` says "this file will be merged"
/// on its own, no need to also show the extraction checkbox alongside it,
/// so a merge mark takes priority over a plain extraction tick when a row
/// (rarely) has both. A pure function so every case can be unit-tested
/// without rendering anything.
pub fn mark_glyph(check_state: CheckState, merge_check_state: CheckState) -> &'static str {
    match merge_check_state {
        CheckState::Checked => "[m] ",
        CheckState::Partial => "[~] ",
        CheckState::Unchecked => match check_state {
            CheckState::Checked => "[x] ",
            CheckState::Partial => "[~] ",
            CheckState::Unchecked => "[ ] ",
        },
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
    /// so the search row can show a "type to search..." placeholder the
    /// instant the search key is pressed, before any character is typed.
    pub searching: bool,
}

/// The number of rows actually visible in the popup's content area for a
/// given frame `area_height` and row count — i.e. the "page size" for
/// paging/scrolling through `rows`. Pulled out of `render()` so the mode
/// layer can ask "how many rows fit on screen right now" (for `PageUp`/
/// `PageDown`/half-page navigation) using the exact same sizing math the
/// widget itself uses to lay out the popup, instead of a second,
/// potentially-drifting calculation.
pub fn popup_content_height(area_height: u16, num_rows: usize, searching: bool) -> usize {
    let content_rows = num_rows as u16;
    let extra = if searching { 6 } else { 5 };
    let popup_height = (content_rows + extra)
        .min(area_height * 4 / 5)
        .max(9)
        .min(area_height.saturating_sub(2));
    // Mirrors `Block::bordered().inner(..)`: `Borders::ALL` consumes exactly
    // one row top and bottom.
    let inner_h = popup_height.saturating_sub(2) as usize;
    let footer_lines = 3usize;
    let search_rows = if searching { 1usize } else { 0 };
    inner_h.saturating_sub(footer_lines + search_rows)
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
        let content_h = popup_content_height(area.height, self.rows.len(), self.searching);

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
            let mark = mark_glyph(row.check_state, row.merge_check_state);
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
                format!("{prefix}{indent}{mark}{}", row.name),
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

        let kb = &self.keybindings.archive_picker;
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
            kb.merge_toggle.display(),
            "merge",
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
    fn test_popup_content_height_matches_rendered_row_count() {
        // Cross-check the extracted pure function against what render()
        // actually draws, rather than hand-computing the arithmetic (which
        // would just duplicate — and risk silently drifting from — the
        // formula under test).
        let theme = Theme::default();
        let kb = Keybindings::default();
        let rows: Vec<ArchiveRow> = (0..50).map(|i| row(&format!("file{i}.log"))).collect();
        let popup = ArchivePickerPopup {
            theme: &theme,
            keybindings: &kb,
            rows: &rows,
            selected: 0,
            source_path: "archive.zip",
            search: "",
            searching: false,
        };
        let area_height = 24u16;
        let mut terminal = Terminal::new(TestBackend::new(80, area_height)).unwrap();
        let buf = terminal.draw(|f| f.render_widget(popup, f.area())).unwrap();
        let rendered_row_count = (0..area_height)
            .filter(|&y| row_text(buf.buffer, y).contains(".log"))
            .count();
        let expected = popup_content_height(area_height, rows.len(), false);
        assert_eq!(rendered_row_count, expected);
    }

    #[test]
    fn test_popup_content_height_shrinks_when_searching() {
        // The search input row and extra footer line both eat into the
        // content area when searching, so the content height must shrink
        // (not grow) relative to the non-searching case for the same area.
        let not_searching = popup_content_height(24, 50, false);
        let searching = popup_content_height(24, 50, true);
        assert!(searching < not_searching);
    }

    #[test]
    fn test_mark_glyph_neither() {
        assert_eq!(
            mark_glyph(CheckState::Unchecked, CheckState::Unchecked),
            "[ ] "
        );
    }

    #[test]
    fn test_mark_glyph_extraction_only() {
        assert_eq!(
            mark_glyph(CheckState::Checked, CheckState::Unchecked),
            "[x] "
        );
    }

    #[test]
    fn test_mark_glyph_merge_only() {
        assert_eq!(
            mark_glyph(CheckState::Unchecked, CheckState::Checked),
            "[m] "
        );
    }

    #[test]
    fn test_mark_glyph_both_prefers_merge() {
        // A row rarely has both, but when it does, `m` alone is enough to
        // say what will happen to the file — no need to also show `x`.
        assert_eq!(mark_glyph(CheckState::Checked, CheckState::Checked), "[m] ");
    }

    #[test]
    fn test_mark_glyph_partial_extraction() {
        assert_eq!(
            mark_glyph(CheckState::Partial, CheckState::Unchecked),
            "[~] "
        );
    }

    #[test]
    fn test_mark_glyph_partial_merge() {
        assert_eq!(
            mark_glyph(CheckState::Unchecked, CheckState::Partial),
            "[~] "
        );
    }

    #[test]
    fn test_mark_glyph_partial_merge_prefers_over_checked_extraction() {
        assert_eq!(mark_glyph(CheckState::Checked, CheckState::Partial), "[~] ");
    }

    fn row(name: &str) -> ArchiveRow {
        ArchiveRow {
            name: name.to_string(),
            depth: 0,
            is_container: false,
            check_state: CheckState::Unchecked,
            merge_check_state: CheckState::Unchecked,
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
    fn test_row_renders_mark_glyph_independently() {
        let theme = Theme::default();
        let kb = Keybindings::default();
        let mut checked_row = row("a.log");
        checked_row.check_state = CheckState::Checked;
        let mut merged_row = row("b.log");
        merged_row.merge_check_state = CheckState::Checked;
        let mut both_row = row("c.log");
        both_row.check_state = CheckState::Checked;
        both_row.merge_check_state = CheckState::Checked;
        let rows = vec![checked_row, merged_row, both_row];
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
        assert!(
            text.contains("[x] a.log"),
            "extraction-checked-only row must show [x]: {text}"
        );
        assert!(
            text.contains("[m] b.log"),
            "merge-marked-only row must show [m]: {text}"
        );
        assert!(
            text.contains("[m] c.log"),
            "row marked both ways must show [m], not both: {text}"
        );
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
        // with no placeholder/`/query` line pushed in above it.
        assert!(text.contains("a.log"));
        assert!(!text.contains("type to search"));
    }

    #[test]
    fn test_search_placeholder_shown_immediately_on_empty_query() {
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
        assert!(text.contains("type to search..."));
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
