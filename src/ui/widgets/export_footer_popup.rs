use ratatui::{
    prelude::*,
    style::Modifier,
    widgets::{Block, Borders, Paragraph},
};

use crate::config::Keybindings;
use crate::mode::export_footer_mode::placeholder_to_label;
use crate::theme::Theme;

use super::popup_entry;

pub struct ExportFooterPopup<'a> {
    pub theme: &'a Theme,
    pub keybindings: &'a Keybindings,
    pub path: &'a str,
    pub fields: &'a [(String, Vec<String>)],
    pub active_idx: usize,
    pub cursor_row: usize,
    pub cursor_col: usize,
}

struct LayoutParams {
    popup_x: u16,
    popup_y: u16,
    popup_w: u16,
    popup_h: u16,
    field_h: u16,
    n_fields: u16,
}

impl LayoutParams {
    const LABEL_H: u16 = 1;
    const SEP_H: u16 = 1;
    const FOOTER_H: u16 = 1;

    fn fixed_rows(n: u16) -> u16 {
        n * Self::LABEL_H + n.saturating_sub(1) * Self::SEP_H + Self::SEP_H + Self::FOOTER_H
    }

    fn field_text_y(&self, idx: usize) -> u16 {
        1 + idx as u16 * (Self::LABEL_H + self.field_h + Self::SEP_H) + Self::LABEL_H
    }
}

impl<'a> ExportFooterPopup<'a> {
    fn layout(&self, area: Rect) -> LayoutParams {
        let n = self.fields.len() as u16;
        let popup_w = area.width.saturating_sub(8).clamp(50, 80);
        let min_h = 8 + n * 2;
        let popup_h = area.height.saturating_sub(4).min(24).max(min_h);
        let x = area.x + (area.width.saturating_sub(popup_w)) / 2;
        let y = area.y + (area.height.saturating_sub(popup_h)) / 2;
        let inner_h = popup_h.saturating_sub(2);
        let fixed = LayoutParams::fixed_rows(n);
        let field_h = inner_h.saturating_sub(fixed).checked_div(n).unwrap_or(0);
        LayoutParams {
            popup_x: x,
            popup_y: y,
            popup_w,
            popup_h,
            field_h,
            n_fields: n,
        }
    }

    fn scroll_for(cursor_row: usize, field_h: u16) -> u16 {
        cursor_row.saturating_sub(field_h.saturating_sub(1) as usize) as u16
    }

    pub fn cursor_position(&self, area: Rect) -> Option<(u16, u16)> {
        let p = self.layout(area);
        let text_y_offset = p.field_text_y(self.active_idx);
        let scroll = Self::scroll_for(self.cursor_row, p.field_h);
        let text_area_y = p.popup_y + text_y_offset;
        let text_area_x = p.popup_x + 1;
        let inner_w = p.popup_w.saturating_sub(2);
        let cur_x = text_area_x + self.cursor_col as u16;
        let cur_y = text_area_y + self.cursor_row as u16 - scroll;
        if cur_x < text_area_x + inner_w && cur_y < text_area_y + p.field_h {
            Some((cur_x, cur_y))
        } else {
            None
        }
    }
}

fn truncated_path(path: &str, popup_w: u16) -> String {
    if path.len() > popup_w as usize - 12 {
        format!("…{}", &path[path.len() - (popup_w as usize - 13)..])
    } else {
        path.to_string()
    }
}

fn label_style(active: bool, theme: &Theme) -> Style {
    if active {
        Style::default()
            .fg(theme.text_highlight_fg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.text)
    }
}

fn render_field_label(text: &str, active: bool, theme: &Theme, area: Rect, buf: &mut Buffer) {
    Paragraph::new(text)
        .style(label_style(active, theme))
        .render(area, buf);
}

fn render_text_field(lines: &[String], scroll: u16, theme: &Theme, area: Rect, buf: &mut Buffer) {
    let content: Vec<Line> = lines.iter().map(|l| Line::from(l.as_str())).collect();
    Paragraph::new(content)
        .scroll((scroll, 0))
        .style(Style::default().fg(theme.text).bg(theme.root_bg))
        .render(area, buf);
}

fn render_section_separator(theme: &Theme, area: Rect, buf: &mut Buffer) {
    let sep = "\u{2500}".repeat(area.width as usize);
    Paragraph::new(sep)
        .style(Style::default().fg(theme.border_title).bg(theme.root_bg))
        .render(area, buf);
}

fn render_footer_separator(theme: &Theme, area: Rect, buf: &mut Buffer) {
    let sep = "\u{2500}".repeat(area.width as usize);
    Paragraph::new(sep)
        .style(Style::default().fg(theme.text).bg(theme.root_bg))
        .render(area, buf);
}

fn render_keybindings(theme: &Theme, area: Rect, buf: &mut Buffer) {
    let key_style = Style::default()
        .fg(theme.text_highlight_fg)
        .add_modifier(Modifier::BOLD);
    let txt_style = Style::default().fg(theme.text);
    let br_style = Style::default().fg(theme.text);
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (key, label) in [
        ("Tab", "switch"),
        ("Enter", "newline"),
        ("Ctrl-S", "export"),
        ("Esc", "cancel"),
    ] {
        popup_entry(
            &mut spans,
            key.to_string(),
            label,
            key_style,
            txt_style,
            br_style,
        );
    }
    Paragraph::new(Line::from(spans))
        .style(Style::default().bg(theme.root_bg))
        .render(area, buf);
}

impl<'a> Widget for ExportFooterPopup<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let p = self.layout(area);
        let popup_area = Rect::new(p.popup_x, p.popup_y, p.popup_w, p.popup_h);
        ratatui::widgets::Clear.render(popup_area, buf);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.theme.border_title))
            .title(format!(
                " Export: {} ",
                truncated_path(self.path, p.popup_w)
            ))
            .title_style(
                Style::default()
                    .fg(self.theme.text_highlight_fg)
                    .add_modifier(Modifier::BOLD),
            )
            .title_alignment(Alignment::Center)
            .style(Style::default().bg(self.theme.root_bg));

        let inner = block.inner(popup_area);
        block.render(popup_area, buf);

        let mut constraints: Vec<Constraint> = Vec::new();
        for i in 0..p.n_fields as usize {
            constraints.push(Constraint::Length(LayoutParams::LABEL_H));
            constraints.push(Constraint::Length(p.field_h));
            if i + 1 < p.n_fields as usize {
                constraints.push(Constraint::Length(LayoutParams::SEP_H));
            }
        }
        constraints.push(Constraint::Length(LayoutParams::SEP_H));
        constraints.push(Constraint::Length(LayoutParams::FOOTER_H));

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);

        let mut chunk_idx = 0;
        for (i, (name, lines)) in self.fields.iter().enumerate() {
            let active = i == self.active_idx;
            let label = placeholder_to_label(name);
            render_field_label(&label, active, self.theme, chunks[chunk_idx], buf);
            chunk_idx += 1;
            let scroll = if active {
                Self::scroll_for(self.cursor_row, p.field_h)
            } else {
                0
            };
            render_text_field(lines, scroll, self.theme, chunks[chunk_idx], buf);
            chunk_idx += 1;
            if i + 1 < self.fields.len() {
                render_section_separator(self.theme, chunks[chunk_idx], buf);
                chunk_idx += 1;
            }
        }
        render_footer_separator(self.theme, chunks[chunk_idx], buf);
        chunk_idx += 1;
        render_keybindings(self.theme, chunks[chunk_idx], buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Keybindings;
    use crate::db::{Database, LogManager};
    use crate::ingestion::FileReader;
    use crate::mode::export_footer_mode::ExportFooterMode;
    use crate::theme::Theme;
    use crate::ui::App;
    use ratatui::{Terminal, backend::TestBackend};
    use std::sync::Arc;

    async fn make_app() -> App {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let lm = LogManager::new(db, None).await;
        App::builder(
            lm,
            FileReader::from_bytes(b"line\n".to_vec()),
            Theme::default(),
            Arc::new(Keybindings::default()),
        )
        .build()
        .await
    }

    fn make_popup<'a>(
        theme: &'a Theme,
        kb: &'a Keybindings,
        fields: &'a [(String, Vec<String>)],
        active_idx: usize,
    ) -> ExportFooterPopup<'a> {
        ExportFooterPopup {
            theme,
            keybindings: kb,
            path: "/tmp/out.md",
            fields,
            active_idx,
            cursor_row: 0,
            cursor_col: 0,
        }
    }

    #[test]
    fn test_cursor_first_field_row0_col0() {
        let theme = Theme::default();
        let kb = Keybindings::default();
        let fields: Vec<(String, Vec<String>)> = vec![
            ("conclusion".to_string(), vec!["hello".to_string()]),
            ("next_steps".to_string(), vec![String::new()]),
        ];
        let popup = make_popup(&theme, &kb, &fields, 0);
        let area = Rect::new(0, 0, 80, 24);
        let pos = popup.cursor_position(area);
        assert!(pos.is_some(), "cursor must be visible");
        let (cx, cy) = pos.unwrap();
        let p = popup.layout(area);
        assert_eq!(cy, p.popup_y + p.field_text_y(0));
        assert_eq!(cx, p.popup_x + 1);
    }

    #[test]
    fn test_cursor_second_field_row0_col0() {
        let theme = Theme::default();
        let kb = Keybindings::default();
        let fields: Vec<(String, Vec<String>)> = vec![
            ("conclusion".to_string(), vec![String::new()]),
            ("next_steps".to_string(), vec!["step".to_string()]),
        ];
        let popup = make_popup(&theme, &kb, &fields, 1);
        let area = Rect::new(0, 0, 80, 24);
        let pos = popup.cursor_position(area);
        assert!(pos.is_some(), "cursor must be visible");
        let (_, cy) = pos.unwrap();
        let p = popup.layout(area);
        assert_eq!(cy, p.popup_y + p.field_text_y(1));
    }

    #[test]
    fn test_cursor_position_dynamic_fields() {
        let theme = Theme::default();
        let kb = Keybindings::default();
        let fields: Vec<(String, Vec<String>)> = vec![
            ("conclusion".to_string(), vec![String::new()]),
            ("next_steps".to_string(), vec![String::new()]),
        ];
        let popup = make_popup(&theme, &kb, &fields, 0);
        let area = Rect::new(0, 0, 80, 24);
        let p = popup.layout(area);
        let y0 = p.field_text_y(0);
        let y1 = p.field_text_y(1);
        let gap = y1 - y0;
        assert_eq!(
            gap,
            p.field_h + LayoutParams::SEP_H + LayoutParams::LABEL_H,
            "field_text_y formula: idx=1 should be idx=0 + field_h + sep + label"
        );
    }

    #[tokio::test]
    async fn test_export_footer_popup_renders_without_panic() {
        let mut app = make_app().await;
        app.tabs[0].interaction.mode = Box::new(ExportFooterMode::new(
            "/tmp/out.md".into(),
            "markdown".into(),
            vec!["conclusion".into(), "next_steps".into()],
        ));
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_export_footer_popup_with_content() {
        let mut app = make_app().await;
        let mut mode = ExportFooterMode::new(
            "/tmp/out.md".into(),
            "markdown".into(),
            vec!["conclusion".into(), "next_steps".into()],
        );
        mode.fields[0].lines = vec!["all good".to_string()];
        mode.fields[1].lines = vec!["fix now".to_string()];
        app.tabs[0].interaction.mode = Box::new(mode);
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_export_footer_popup_second_field_active() {
        let mut app = make_app().await;
        let mut mode = ExportFooterMode::new(
            "/tmp/out.md".into(),
            "markdown".into(),
            vec!["conclusion".into(), "next_steps".into()],
        );
        mode.active_idx = 1;
        mode.fields[1].lines = vec!["do this".to_string()];
        app.tabs[0].interaction.mode = Box::new(mode);
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();
    }
}
