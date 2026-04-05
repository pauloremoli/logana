use crate::{
    config::Keybindings,
    mode::{
        app_mode::{Mode, ModeRenderState, status_entry},
        normal_mode::NormalMode,
    },
    theme::Theme,
    ui::{KeyResult, TabState},
};
use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

#[derive(Debug)]
pub struct FooterField {
    pub name: String,
    pub lines: Vec<String>,
}

#[derive(Debug)]
pub struct ExportFooterMode {
    pub path: String,
    pub template_name: String,
    pub fields: Vec<FooterField>,
    pub active_idx: usize,
    pub cursor_row: usize,
    pub cursor_col: usize,
}

pub fn placeholder_to_label(name: &str) -> String {
    name.split('_')
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

impl ExportFooterMode {
    pub fn new(path: String, template_name: String, field_names: Vec<String>) -> Self {
        let fields = field_names
            .into_iter()
            .map(|name| FooterField {
                name,
                lines: vec![String::new()],
            })
            .collect();
        Self {
            path,
            template_name,
            fields,
            active_idx: 0,
            cursor_row: 0,
            cursor_col: 0,
        }
    }

    fn active_lines(&self) -> &Vec<String> {
        &self.fields[self.active_idx].lines
    }

    fn switch_to(&mut self, idx: usize) {
        self.active_idx = idx;
        let (row, col) = {
            let lines = self.active_lines();
            let row = lines.len().saturating_sub(1);
            let col = lines.last().map_or(0, |l| l.len());
            (row, col)
        };
        self.cursor_row = row;
        self.cursor_col = col;
    }

    fn insert_newline(&mut self) {
        let (row, col) = (self.cursor_row, self.cursor_col);
        let lines = &mut self.fields[self.active_idx].lines;
        let rest = lines[row][col..].to_string();
        lines[row].truncate(col);
        lines.insert(row + 1, rest);
        self.cursor_row += 1;
        self.cursor_col = 0;
    }

    fn insert_char(&mut self, c: char) {
        let (row, col) = (self.cursor_row, self.cursor_col);
        let lines = &mut self.fields[self.active_idx].lines;
        lines[row].insert(col, c);
        self.cursor_col += 1;
    }

    fn backspace(&mut self) {
        let (row, col) = (self.cursor_row, self.cursor_col);
        if col > 0 {
            let lines = &mut self.fields[self.active_idx].lines;
            lines[row].remove(col - 1);
            self.cursor_col -= 1;
        } else if row > 0 {
            let prev_len = {
                let lines = &mut self.fields[self.active_idx].lines;
                let current = lines.remove(row);
                let prev_len = lines[row - 1].len();
                lines[row - 1].push_str(&current);
                prev_len
            };
            self.cursor_row -= 1;
            self.cursor_col = prev_len;
        }
    }

    fn delete_forward(&mut self) {
        let (row, col) = (self.cursor_row, self.cursor_col);
        let lines = &mut self.fields[self.active_idx].lines;
        if col < lines[row].len() {
            lines[row].remove(col);
        } else if row + 1 < lines.len() {
            let next = lines.remove(row + 1);
            lines[row].push_str(&next);
        }
    }

    fn move_left(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
        } else if self.cursor_row > 0 {
            self.cursor_row -= 1;
            self.cursor_col = self.active_lines()[self.cursor_row].len();
        }
    }

    fn move_right(&mut self) {
        let (row, col) = (self.cursor_row, self.cursor_col);
        let lines = self.active_lines();
        if col < lines[row].len() {
            self.cursor_col += 1;
        } else if row + 1 < lines.len() {
            self.cursor_row += 1;
            self.cursor_col = 0;
        }
    }

    fn move_up(&mut self) {
        if self.cursor_row > 0 {
            self.cursor_row -= 1;
            let max_col = self.active_lines()[self.cursor_row].len();
            self.cursor_col = self.cursor_col.min(max_col);
        }
    }

    fn move_down(&mut self) {
        let row = self.cursor_row;
        if row + 1 < self.active_lines().len() {
            self.cursor_row += 1;
            let max_col = self.active_lines()[self.cursor_row].len();
            self.cursor_col = self.cursor_col.min(max_col);
        }
    }
}

#[async_trait]
impl Mode for ExportFooterMode {
    async fn handle_key(
        mut self: Box<Self>,
        _tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        let ctrl = modifiers.contains(KeyModifiers::CONTROL);
        let shift = modifiers.contains(KeyModifiers::SHIFT);

        if key == KeyCode::Esc {
            return (Box::new(NormalMode::default()), KeyResult::Handled);
        }

        if ctrl && key == KeyCode::Char('s') {
            let footer_fields = self
                .fields
                .iter()
                .map(|f| (f.name.clone(), f.lines.join("\n")))
                .collect();
            return (
                Box::new(NormalMode::default()),
                KeyResult::ExportWithFooter {
                    path: self.path,
                    template_name: self.template_name,
                    footer_fields,
                },
            );
        }

        if key == KeyCode::Tab {
            let n = self.fields.len();
            let next = if shift {
                (self.active_idx + n - 1) % n
            } else {
                (self.active_idx + 1) % n
            };
            self.switch_to(next);
            return (self, KeyResult::Handled);
        }

        match key {
            KeyCode::Enter => self.insert_newline(),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete_forward(),
            KeyCode::Char(c) if !ctrl => self.insert_char(c),
            KeyCode::Left => self.move_left(),
            KeyCode::Right => self.move_right(),
            KeyCode::Up => self.move_up(),
            KeyCode::Down => self.move_down(),
            _ => {}
        }

        (self, KeyResult::Handled)
    }

    fn mode_bar_content(&self, _kb: &Keybindings, theme: &Theme) -> Line<'static> {
        let active_name = self
            .fields
            .get(self.active_idx)
            .map(|f| placeholder_to_label(&f.name))
            .unwrap_or_default();
        let label = format!("[EXPORT — {}]  ", active_name);
        let mut spans: Vec<Span<'static>> = vec![Span::styled(
            label,
            Style::default()
                .fg(theme.text_highlight_fg)
                .add_modifier(Modifier::BOLD),
        )];
        status_entry(&mut spans, "Tab".to_string(), "switch field", theme);
        status_entry(&mut spans, "Ctrl-S".to_string(), "export", theme);
        status_entry(&mut spans, "Esc".to_string(), "cancel", theme);
        Line::from(spans)
    }

    fn render_state(&self) -> ModeRenderState {
        ModeRenderState::ExportFooter {
            path: self.path.clone(),
            template_name: self.template_name.clone(),
            fields: self
                .fields
                .iter()
                .map(|f| (f.name.clone(), f.lines.clone()))
                .collect(),
            active_idx: self.active_idx,
            cursor_row: self.cursor_row,
            cursor_col: self.cursor_col,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Database, LogManager};
    use crate::ingestion::FileReader;
    use crate::ui::TabState;
    use std::sync::Arc;

    async fn make_tab() -> TabState {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let lm = LogManager::new(db, None).await;
        TabState::new(FileReader::from_bytes(b"a\nb\n".to_vec()), lm, "t".into())
    }

    async fn press(
        mode: ExportFooterMode,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        Box::new(mode).handle_key(tab, key, modifiers).await
    }

    fn make_mode() -> ExportFooterMode {
        ExportFooterMode::new(
            "/tmp/out.md".into(),
            "markdown".into(),
            vec!["conclusion".into(), "next_steps".into()],
        )
    }

    #[test]
    fn test_placeholder_to_label() {
        assert_eq!(placeholder_to_label("conclusion"), "Conclusion");
        assert_eq!(placeholder_to_label("next_steps"), "Next Steps");
        assert_eq!(
            placeholder_to_label("my_long_field_name"),
            "My Long Field Name"
        );
        assert_eq!(placeholder_to_label(""), "");
    }

    #[tokio::test]
    async fn test_tab_switches_from_conclusion_to_next_steps() {
        let mut tab = make_tab().await;
        let mode = make_mode();
        assert_eq!(mode.active_idx, 0);
        let (mode2, result) = press(mode, &mut tab, KeyCode::Tab, KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::Handled));
        match mode2.render_state() {
            ModeRenderState::ExportFooter { active_idx, .. } => {
                assert_eq!(active_idx, 1);
            }
            other => panic!("expected ExportFooter, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_tab_wraps_back_to_first() {
        let mut tab = make_tab().await;
        let mut mode = make_mode();
        mode.active_idx = 1;
        let (mode2, _) = press(mode, &mut tab, KeyCode::Tab, KeyModifiers::NONE).await;
        match mode2.render_state() {
            ModeRenderState::ExportFooter { active_idx, .. } => {
                assert_eq!(active_idx, 0);
            }
            other => panic!("expected ExportFooter, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_enter_inserts_newline_in_active_field() {
        let mut tab = make_tab().await;
        let mut mode = make_mode();
        mode.fields[0].lines[0] = "hello world".to_string();
        mode.cursor_col = 5;
        let (mode2, _) = press(mode, &mut tab, KeyCode::Enter, KeyModifiers::NONE).await;
        match mode2.render_state() {
            ModeRenderState::ExportFooter {
                fields,
                cursor_row,
                cursor_col,
                ..
            } => {
                assert_eq!(fields[0].1[0], "hello");
                assert_eq!(fields[0].1[1], " world");
                assert_eq!(cursor_row, 1);
                assert_eq!(cursor_col, 0);
            }
            other => panic!("expected ExportFooter, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_ctrl_s_returns_export_with_footer() {
        let mut tab = make_tab().await;
        let mut mode = make_mode();
        mode.fields[0].lines = vec!["my conclusion".to_string()];
        mode.fields[1].lines = vec!["step one".to_string(), "step two".to_string()];
        let (_, result) = press(mode, &mut tab, KeyCode::Char('s'), KeyModifiers::CONTROL).await;
        match result {
            KeyResult::ExportWithFooter {
                path,
                template_name,
                footer_fields,
            } => {
                assert_eq!(path, "/tmp/out.md");
                assert_eq!(template_name, "markdown");
                assert_eq!(
                    footer_fields,
                    vec![
                        ("conclusion".to_string(), "my conclusion".to_string()),
                        ("next_steps".to_string(), "step one\nstep two".to_string()),
                    ]
                );
            }
            other => panic!("expected ExportWithFooter, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_esc_returns_to_normal_mode() {
        let mut tab = make_tab().await;
        let mode = make_mode();
        let (mode2, result) = press(mode, &mut tab, KeyCode::Esc, KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(!matches!(
            mode2.render_state(),
            ModeRenderState::ExportFooter { .. }
        ));
    }

    #[tokio::test]
    async fn test_char_inserts_into_active_field() {
        let mut tab = make_tab().await;
        let mode = make_mode();
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('x'), KeyModifiers::NONE).await;
        match mode2.render_state() {
            ModeRenderState::ExportFooter { fields, .. } => {
                assert_eq!(fields[0].1[0], "x");
            }
            other => panic!("expected ExportFooter, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_char_inserts_into_second_field_when_active() {
        let mut tab = make_tab().await;
        let mut mode = make_mode();
        mode.active_idx = 1;
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('y'), KeyModifiers::NONE).await;
        match mode2.render_state() {
            ModeRenderState::ExportFooter { fields, .. } => {
                assert_eq!(fields[1].1[0], "y");
            }
            other => panic!("expected ExportFooter, got {:?}", other),
        }
    }

    #[test]
    fn test_render_state_reflects_all_fields() {
        let mut mode = make_mode();
        mode.fields[0].lines = vec!["done".to_string()];
        mode.fields[1].lines = vec!["todo".to_string()];
        match mode.render_state() {
            ModeRenderState::ExportFooter {
                path,
                template_name,
                fields,
                active_idx,
                ..
            } => {
                assert_eq!(path, "/tmp/out.md");
                assert_eq!(template_name, "markdown");
                assert_eq!(active_idx, 0);
                assert_eq!(fields[0].1, vec!["done"]);
                assert_eq!(fields[1].1, vec!["todo"]);
            }
            other => panic!("expected ExportFooter, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_delete_removes_char_at_cursor() {
        let mut tab = make_tab().await;
        let mut mode = make_mode();
        mode.fields[0].lines[0] = "hello".to_string();
        mode.cursor_col = 2;
        let (mode2, _) = press(mode, &mut tab, KeyCode::Delete, KeyModifiers::NONE).await;
        match mode2.render_state() {
            ModeRenderState::ExportFooter {
                fields, cursor_col, ..
            } => {
                assert_eq!(fields[0].1[0], "helo");
                assert_eq!(cursor_col, 2);
            }
            other => panic!("expected ExportFooter, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_delete_at_end_of_line_merges_next_line() {
        let mut tab = make_tab().await;
        let mut mode = make_mode();
        mode.fields[0].lines = vec!["first".to_string(), "second".to_string()];
        mode.cursor_row = 0;
        mode.cursor_col = 5;
        let (mode2, _) = press(mode, &mut tab, KeyCode::Delete, KeyModifiers::NONE).await;
        match mode2.render_state() {
            ModeRenderState::ExportFooter {
                fields,
                cursor_row,
                cursor_col,
                ..
            } => {
                assert_eq!(fields[0].1.len(), 1);
                assert_eq!(fields[0].1[0], "firstsecond");
                assert_eq!(cursor_row, 0);
                assert_eq!(cursor_col, 5);
            }
            other => panic!("expected ExportFooter, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_delete_at_last_char_of_last_line_is_noop() {
        let mut tab = make_tab().await;
        let mut mode = make_mode();
        mode.fields[0].lines[0] = "hi".to_string();
        mode.cursor_col = 2;
        let (mode2, _) = press(mode, &mut tab, KeyCode::Delete, KeyModifiers::NONE).await;
        match mode2.render_state() {
            ModeRenderState::ExportFooter { fields, .. } => {
                assert_eq!(fields[0].1[0], "hi");
            }
            other => panic!("expected ExportFooter, got {:?}", other),
        }
    }
}
