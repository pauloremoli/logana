use ratatui::{
    prelude::*,
    style::Modifier,
    widgets::{
        Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, StatefulWidget,
    },
};

use crate::mode::default_filters_mode::{DefaultFilterRow, PathEditState};
use crate::theme::Theme;

use super::popup_entry;

pub struct DefaultFiltersPopup<'a> {
    pub theme: &'a Theme,
    pub rows: &'a [DefaultFilterRow],
    pub search: &'a str,
    pub selected: usize,
    /// `Some` while the selected row's path is being edited.
    pub editing: Option<&'a PathEditState>,
}

impl<'a> Widget for DefaultFiltersPopup<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        use crate::commands::auto_complete::{complete_file_path, fuzzy_match};

        let vis_rows: Vec<usize> = if self.search.is_empty() {
            (0..self.rows.len()).collect()
        } else {
            self.rows
                .iter()
                .enumerate()
                .filter(|(_, r)| fuzzy_match(self.search, &r.name))
                .map(|(i, _)| i)
                .collect()
        };

        // Same Tab-cycling autocomplete UX as the command line's file-path
        // completion: computed from `query` (the text as typed before the
        // first Tab press), not `input` (which Tab may have already replaced
        // with a full candidate) — so the hint list stays stable while cycling.
        let file_completions: Vec<String> = self
            .editing
            .map(|e| complete_file_path(e.query.as_deref().unwrap_or(&e.input)))
            .unwrap_or_default();

        let popup_width = (area.width.saturating_sub(4)).clamp(40, 70);
        let row_count = vis_rows.len() as u16;
        let has_search = !self.search.is_empty();
        let has_edit = self.editing.is_some();
        let has_hints = !file_completions.is_empty();
        let extra = 5
            + if has_search { 1 } else { 0 }
            + if has_edit { 1 } else { 0 }
            + if has_hints { 1 } else { 0 };
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
            .title(" Default Filters ")
            .title_style(
                Style::default()
                    .fg(self.theme.text_highlight_fg)
                    .add_modifier(Modifier::BOLD),
            )
            .title_alignment(Alignment::Center)
            .style(Style::default().bg(self.theme.root_bg));

        let inner = block.inner(popup_area);
        block.render(popup_area, buf);

        let footer_lines = 2usize;
        let search_rows = if has_search { 1usize } else { 0 };
        let edit_rows = if has_edit { 1usize } else { 0 };
        let hint_rows = if has_hints { 1usize } else { 0 };
        let content_h = inner
            .height
            .saturating_sub((footer_lines + search_rows + edit_rows + hint_rows + 1) as u16)
            as usize;

        let mut constraints = vec![];
        if has_search {
            constraints.push(Constraint::Length(1));
        }
        constraints.push(Constraint::Min(1));
        if has_edit {
            constraints.push(Constraint::Length(1));
        }
        if has_hints {
            constraints.push(Constraint::Length(1));
        }
        constraints.push(Constraint::Length(1));
        constraints.push(Constraint::Length(footer_lines as u16));

        let vsplit = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);

        let mut idx = 0;
        let search_area = if has_search {
            let a = vsplit[idx];
            idx += 1;
            Some(a)
        } else {
            None
        };
        let content_area = vsplit[idx];
        idx += 1;
        let edit_area = if has_edit {
            let a = vsplit[idx];
            idx += 1;
            Some(a)
        } else {
            None
        };
        let hints_area = if has_hints {
            let a = vsplit[idx];
            idx += 1;
            Some(a)
        } else {
            None
        };
        let sep_area = vsplit[idx];
        idx += 1;
        let footer_area = vsplit[idx];

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

        let custom_style = Style::default().fg(self.theme.text_highlight_fg);
        // `cursor_fg` is only guaranteed readable when paired with `cursor_bg`
        // (some themes set it equal to `root_bg`, meant to sit on a bright
        // cursor highlight) — `inactive_tab_fg` stays legible against
        // `root_bg` directly across every shipped theme.
        let builtin_style = Style::default().fg(self.theme.inactive_tab_fg);
        let path_style = Style::default().fg(self.theme.text);

        let mut lines: Vec<Line> = Vec::new();
        for (i, &row_idx) in vis_rows.iter().enumerate().skip(scroll).take(content_h) {
            let is_sel = i == self.selected;
            let prefix = if is_sel { "> " } else { "  " };
            let row = &self.rows[row_idx];
            let name_style = if row.is_custom {
                custom_style
            } else {
                builtin_style
            };
            let name_style = if is_sel {
                name_style.add_modifier(Modifier::BOLD)
            } else {
                name_style
            };
            let path_text = row.path.as_deref().unwrap_or("none");
            lines.push(Line::from(vec![
                Span::styled(format!("{}{:<16}", prefix, row.name), name_style),
                Span::styled(format!(" {}", path_text), path_style),
            ]));
        }

        while lines.len() < content_h {
            lines.push(Line::from(""));
        }

        Paragraph::new(lines)
            .style(Style::default().bg(self.theme.root_bg))
            .render(content_area, buf);

        if let (Some(ea), Some(editing)) = (edit_area, self.editing) {
            let edit_line = Line::from(vec![
                Span::styled(
                    " path: ",
                    Style::default()
                        .fg(self.theme.text_highlight_fg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(editing.input.clone(), Style::default().fg(self.theme.text)),
            ]);
            Paragraph::new(edit_line)
                .style(Style::default().bg(self.theme.root_bg))
                .render(ea, buf);
        }

        if let Some(ha) = hints_area {
            let normal_style = Style::default().fg(self.theme.text).bg(self.theme.root_bg);
            let highlight_style = Style::default()
                .fg(self.theme.cursor_fg)
                .bg(self.theme.cursor_bg);
            let completion_index = self.editing.and_then(|e| e.completion_index);
            let hint_spans: Vec<Span> = file_completions
                .iter()
                .enumerate()
                .flat_map(|(i, name)| {
                    let style = if completion_index == Some(i) {
                        highlight_style
                    } else {
                        normal_style
                    };
                    vec![
                        Span::styled(format!(" {} ", super::file_display_name(name)), style),
                        Span::raw(" "),
                    ]
                })
                .collect();
            Paragraph::new(Line::from(hint_spans))
                .style(Style::default().bg(self.theme.root_bg))
                .render(ha, buf);
        }

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
        if has_edit {
            popup_entry(
                &mut line1,
                "Tab".to_string(),
                "complete",
                key_style,
                txt_style,
                br_style,
            );
            popup_entry(
                &mut line1,
                "Enter".to_string(),
                "save",
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
        } else {
            popup_entry(
                &mut line1,
                "Enter".to_string(),
                "edit",
                key_style,
                txt_style,
                br_style,
            );
            popup_entry(
                &mut line1,
                "d".to_string(),
                "clear",
                key_style,
                txt_style,
                br_style,
            );
            popup_entry(
                &mut line1,
                "Esc".to_string(),
                "close",
                key_style,
                txt_style,
                br_style,
            );
        }
        let footer = vec![Line::from(line1)];
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
    use crate::db::Database;
    use crate::db::LogManager;
    use crate::ingestion::FileReader;
    use crate::mode::default_filters_mode::DefaultFiltersMode;
    use crate::theme::Theme;
    use crate::ui::App;
    use crate::{config::Keybindings, mode::default_filters_mode::PathEditState};
    use ratatui::{Terminal, backend::TestBackend};
    use std::collections::HashMap;
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
    async fn test_default_filters_popup_basic() {
        let mut app = make_app(&["line one"]).await;
        let mut current = HashMap::new();
        current.insert("acme".to_string(), "/tmp/acme.json".to_string());
        let mode = DefaultFiltersMode::new(&["acme".to_string()], &current);
        app.tabs[0].interaction.mode = Box::new(mode);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_default_filters_popup_with_search() {
        let mut app = make_app(&["line one"]).await;
        let mut mode = DefaultFiltersMode::new(&["acme".to_string()], &HashMap::new());
        mode.search = "acme".to_string();
        app.tabs[0].interaction.mode = Box::new(mode);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_default_filters_popup_editing() {
        let mut app = make_app(&["line one"]).await;
        let mut mode = DefaultFiltersMode::new(&["acme".to_string()], &HashMap::new());
        mode.editing = Some(PathEditState {
            input: "/tmp/a.json".to_string(),
            cursor: 5,
            query: None,
            completion_index: None,
        });
        app.tabs[0].interaction.mode = Box::new(mode);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_default_filters_popup_editing_shows_file_completion_hints() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("acme-a.json"), "[]").unwrap();
        std::fs::write(dir.path().join("acme-b.json"), "[]").unwrap();
        let mut app = make_app(&["line one"]).await;
        let mut mode = DefaultFiltersMode::new(&["acme".to_string()], &HashMap::new());
        let input = dir.path().to_str().unwrap().to_string() + "/";
        mode.editing = Some(PathEditState {
            cursor: input.chars().count(),
            input,
            query: None,
            completion_index: Some(0),
        });
        app.tabs[0].interaction.mode = Box::new(mode);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("acme-a.json"));
        assert!(content.contains("acme-b.json"));
    }

    #[tokio::test]
    async fn test_default_filters_popup_many_rows_scrollbar() {
        let mut app = make_app(&["line one"]).await;
        let custom_names: Vec<String> = (0..30).map(|i| format!("schema_{i}")).collect();
        let mode = DefaultFiltersMode::new(&custom_names, &HashMap::new());
        app.tabs[0].interaction.mode = Box::new(mode);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    /// Regression guard: `atomic`, `jandedobbeleer`, `nord`, and `paradox`
    /// all set `cursor_fg` equal (or near-equal) to `root_bg`, since it's
    /// only meant to be readable when paired with `cursor_bg` (a bright
    /// highlight), not used directly against `root_bg`. Built-in format rows
    /// and the edit-line input text must use a color that's actually
    /// distinct from `root_bg` on every shipped theme, not `cursor_fg`.
    #[test]
    fn test_builtin_row_and_edit_colors_are_never_invisible_on_shipped_themes() {
        for name in ["atomic", "jandedobbeleer", "nord", "paradox"] {
            let theme = Theme::from_file(format!("{name}.json")).unwrap();
            assert_ne!(
                theme.inactive_tab_fg, theme.root_bg,
                "{name}: built-in row text would be invisible against root_bg"
            );
            assert_ne!(
                theme.text, theme.root_bg,
                "{name}: edit-line input text would be invisible against root_bg"
            );
        }
    }
}
