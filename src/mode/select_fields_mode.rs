use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::{
    config::Keybindings,
    mode::app_mode::{Mode, ModeRenderState, status_entry},
    mode::normal_mode::NormalMode,
    theme::Theme,
    types::FieldLayout,
    ui::{KeyResult, TabState},
};

#[derive(Debug)]
pub struct SelectFieldsMode {
    /// Field name + enabled toggle.
    pub fields: Vec<(String, bool)>,
    /// Cursor position in the fields list.
    pub selected: usize,
    /// Snapshot of the full `FieldLayout` on entry (restored on Esc cancel).
    original_layout: FieldLayout,
}

impl SelectFieldsMode {
    pub fn new(fields: Vec<(String, bool)>, original_layout: FieldLayout) -> Self {
        SelectFieldsMode {
            fields,
            selected: 0,
            original_layout,
        }
    }
}

#[async_trait]
impl Mode for SelectFieldsMode {
    async fn handle_key(
        mut self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        let kb = &tab.keybindings;
        if kb.select_fields.apply.matches(key, modifiers) {
            let enabled: Vec<String> = self
                .fields
                .iter()
                .filter(|(_, on)| *on)
                .map(|(name, _)| name.clone())
                .collect();
            let all_ordered: Vec<String> = self.fields.iter().map(|(n, _)| n.clone()).collect();
            tab.field_layout.columns = Some(enabled);
            tab.field_layout.columns_order = Some(all_ordered);
            tab.invalidate_parse_cache();
            return (Box::new(NormalMode::default()), KeyResult::Handled);
        }
        if kb.select_fields.cancel.matches(key, modifiers) {
            tab.field_layout = std::mem::take(&mut self.original_layout);
            tab.invalidate_parse_cache();
            return (Box::new(NormalMode::default()), KeyResult::Handled);
        }
        if kb.navigation.scroll_down.matches(key, modifiers) {
            if !self.fields.is_empty() {
                self.selected = (self.selected + 1).min(self.fields.len() - 1);
            }
        } else if kb.navigation.scroll_up.matches(key, modifiers) {
            self.selected = self.selected.saturating_sub(1);
        } else if kb.select_fields.toggle.matches(key, modifiers) {
            if let Some(f) = self.fields.get_mut(self.selected) {
                f.1 = !f.1;
            }
        } else if kb.select_fields.move_down.matches(key, modifiers) {
            if self.selected + 1 < self.fields.len() {
                self.fields.swap(self.selected, self.selected + 1);
                self.selected += 1;
            }
        } else if kb.select_fields.move_up.matches(key, modifiers) {
            if self.selected > 0 {
                self.fields.swap(self.selected, self.selected - 1);
                self.selected -= 1;
            }
        } else if kb.select_fields.all.matches(key, modifiers) {
            for f in &mut self.fields {
                f.1 = true;
            }
        } else if kb.select_fields.none.matches(key, modifiers) {
            for f in &mut self.fields {
                f.1 = false;
            }
        }
        (self, KeyResult::Handled)
    }

    fn mode_bar_content(&self, kb: &Keybindings, theme: &Theme) -> Line<'static> {
        let mut spans: Vec<Span<'static>> = vec![Span::styled(
            "[SELECT FIELDS]  ",
            Style::default()
                .fg(theme.text_highlight_fg)
                .add_modifier(Modifier::BOLD),
        )];
        status_entry(
            &mut spans,
            kb.select_fields.toggle.display(),
            "toggle",
            theme,
        );
        // Move up/down
        spans.push(Span::styled("<", Style::default().fg(theme.border)));
        spans.push(Span::styled(
            kb.select_fields.move_up.display(),
            Style::default()
                .fg(theme.text_highlight_fg)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled("/", Style::default().fg(theme.border)));
        spans.push(Span::styled(
            kb.select_fields.move_down.display(),
            Style::default()
                .fg(theme.text_highlight_fg)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled("> reorder  ", Style::default().fg(theme.text)));
        status_entry(&mut spans, kb.select_fields.apply.display(), "apply", theme);
        status_entry(
            &mut spans,
            kb.select_fields.cancel.display(),
            "cancel",
            theme,
        );
        status_entry(&mut spans, kb.select_fields.all.display(), "all", theme);
        status_entry(&mut spans, kb.select_fields.none.display(), "none", theme);
        Line::from(spans)
    }

    fn render_state(&self) -> ModeRenderState {
        ModeRenderState::SelectFields {
            fields: self.fields.clone(),
            selected: self.selected,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::file_reader::FileReader;
    use crate::log_manager::LogManager;
    use crate::mode::app_mode::ModeRenderState;
    use std::sync::Arc;

    async fn make_tab() -> TabState {
        let file_reader = FileReader::from_bytes(b"line1\nline2\n".to_vec());
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        TabState::new(file_reader, log_manager, "test".to_string())
    }

    fn sample_fields() -> Vec<(String, bool)> {
        vec![
            ("timestamp".to_string(), true),
            ("level".to_string(), true),
            ("message".to_string(), true),
            ("request_id".to_string(), false),
        ]
    }

    async fn press(
        mode: SelectFieldsMode,
        tab: &mut TabState,
        code: KeyCode,
    ) -> (Box<dyn Mode>, KeyResult) {
        Box::new(mode)
            .handle_key(tab, code, KeyModifiers::NONE)
            .await
    }

    #[tokio::test]
    async fn test_j_moves_cursor_down() {
        let mut tab = make_tab().await;
        let mode = SelectFieldsMode::new(sample_fields(), FieldLayout::default());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('j')).await;
        match mode2.render_state() {
            ModeRenderState::SelectFields { selected, .. } => assert_eq!(selected, 1),
            other => panic!("expected SelectFields, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_k_moves_cursor_up() {
        let mut tab = make_tab().await;
        let mut mode = SelectFieldsMode::new(sample_fields(), FieldLayout::default());
        mode.selected = 2;
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('k')).await;
        match mode2.render_state() {
            ModeRenderState::SelectFields { selected, .. } => assert_eq!(selected, 1),
            other => panic!("expected SelectFields, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_k_at_zero_stays() {
        let mut tab = make_tab().await;
        let mode = SelectFieldsMode::new(sample_fields(), FieldLayout::default());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('k')).await;
        match mode2.render_state() {
            ModeRenderState::SelectFields { selected, .. } => assert_eq!(selected, 0),
            other => panic!("expected SelectFields, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_j_at_end_stays() {
        let mut tab = make_tab().await;
        let mut mode = SelectFieldsMode::new(sample_fields(), FieldLayout::default());
        mode.selected = 3;
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('j')).await;
        match mode2.render_state() {
            ModeRenderState::SelectFields { selected, .. } => assert_eq!(selected, 3),
            other => panic!("expected SelectFields, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_space_toggles_field() {
        let mut tab = make_tab().await;
        let mode = SelectFieldsMode::new(sample_fields(), FieldLayout::default());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char(' ')).await;
        match mode2.render_state() {
            ModeRenderState::SelectFields { fields, .. } => {
                assert!(!fields[0].1); // was true, now false
            }
            other => panic!("expected SelectFields, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_a_enables_all() {
        let mut tab = make_tab().await;
        let mode = SelectFieldsMode::new(sample_fields(), FieldLayout::default());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('a')).await;
        match mode2.render_state() {
            ModeRenderState::SelectFields { fields, .. } => {
                assert!(fields.iter().all(|(_, on)| *on));
            }
            other => panic!("expected SelectFields, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_n_disables_all() {
        let mut tab = make_tab().await;
        let mode = SelectFieldsMode::new(sample_fields(), FieldLayout::default());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('n')).await;
        match mode2.render_state() {
            ModeRenderState::SelectFields { fields, .. } => {
                assert!(fields.iter().all(|(_, on)| !*on));
            }
            other => panic!("expected SelectFields, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_enter_applies_enabled_fields() {
        let mut tab = make_tab().await;
        // Only timestamp and message enabled
        let fields = vec![
            ("timestamp".to_string(), true),
            ("level".to_string(), false),
            ("message".to_string(), true),
        ];
        let mode = SelectFieldsMode::new(fields, FieldLayout::default());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Enter).await;
        assert!(!matches!(
            mode2.render_state(),
            ModeRenderState::SelectFields { .. }
        )); // transitioned to NormalMode
        assert_eq!(
            tab.field_layout.columns,
            Some(vec!["timestamp".to_string(), "message".to_string()])
        );
        assert_eq!(
            tab.field_layout.columns_order,
            Some(vec![
                "timestamp".to_string(),
                "level".to_string(),
                "message".to_string()
            ])
        );
    }

    #[tokio::test]
    async fn test_enter_all_enabled_saves_columns() {
        let mut tab = make_tab().await;
        let fields = vec![("timestamp".to_string(), true), ("level".to_string(), true)];
        let mode = SelectFieldsMode::new(fields, FieldLayout::default());
        let (_, _) = press(mode, &mut tab, KeyCode::Enter).await;
        assert_eq!(
            tab.field_layout.columns,
            Some(vec!["timestamp".to_string(), "level".to_string()])
        );
        assert_eq!(
            tab.field_layout.columns_order,
            Some(vec!["timestamp".to_string(), "level".to_string()])
        );
    }

    #[tokio::test]
    async fn test_esc_restores_original_layout() {
        let mut tab = make_tab().await;
        let original = FieldLayout {
            columns: Some(vec!["level".to_string()]),
            columns_order: Some(vec!["level".to_string(), "timestamp".to_string()]),
        };
        let mode = SelectFieldsMode::new(sample_fields(), original.clone());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Esc).await;
        assert!(!matches!(
            mode2.render_state(),
            ModeRenderState::SelectFields { .. }
        )); // NormalMode
        assert_eq!(tab.field_layout.columns, original.columns);
        assert_eq!(tab.field_layout.columns_order, original.columns_order);
    }

    #[tokio::test]
    async fn test_mode_bar_content() {
        let mode = SelectFieldsMode::new(sample_fields(), FieldLayout::default());
        assert!(matches!(
            mode.render_state(),
            ModeRenderState::SelectFields { .. }
        ));
    }

    #[tokio::test]
    async fn test_down_arrow_moves_cursor() {
        let mut tab = make_tab().await;
        let mode = SelectFieldsMode::new(sample_fields(), FieldLayout::default());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Down).await;
        match mode2.render_state() {
            ModeRenderState::SelectFields { selected, .. } => assert_eq!(selected, 1),
            other => panic!("expected SelectFields, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_up_arrow_moves_cursor() {
        let mut tab = make_tab().await;
        let mut mode = SelectFieldsMode::new(sample_fields(), FieldLayout::default());
        mode.selected = 2;
        let (mode2, _) = press(mode, &mut tab, KeyCode::Up).await;
        match mode2.render_state() {
            ModeRenderState::SelectFields { selected, .. } => assert_eq!(selected, 1),
            other => panic!("expected SelectFields, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_shift_j_moves_field_down() {
        let mut tab = make_tab().await;
        let mut mode = SelectFieldsMode::new(sample_fields(), FieldLayout::default());
        mode.selected = 0; // "timestamp"
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('J')).await;
        match mode2.render_state() {
            ModeRenderState::SelectFields { fields, selected } => {
                assert_eq!(selected, 1);
                assert_eq!(fields[0].0, "level");
                assert_eq!(fields[1].0, "timestamp");
            }
            other => panic!("expected SelectFields, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_shift_k_moves_field_up() {
        let mut tab = make_tab().await;
        let mut mode = SelectFieldsMode::new(sample_fields(), FieldLayout::default());
        mode.selected = 2; // "message"
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('K')).await;
        match mode2.render_state() {
            ModeRenderState::SelectFields { fields, selected } => {
                assert_eq!(selected, 1);
                assert_eq!(fields[1].0, "message");
                assert_eq!(fields[2].0, "level");
            }
            other => panic!("expected SelectFields, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_shift_j_at_end_stays() {
        let mut tab = make_tab().await;
        let mut mode = SelectFieldsMode::new(sample_fields(), FieldLayout::default());
        mode.selected = 3; // last item
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('J')).await;
        match mode2.render_state() {
            ModeRenderState::SelectFields { fields, selected } => {
                assert_eq!(selected, 3);
                assert_eq!(fields[3].0, "request_id"); // unchanged
            }
            other => panic!("expected SelectFields, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_shift_k_at_zero_stays() {
        let mut tab = make_tab().await;
        let mode = SelectFieldsMode::new(sample_fields(), FieldLayout::default());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('K')).await;
        match mode2.render_state() {
            ModeRenderState::SelectFields { fields, selected } => {
                assert_eq!(selected, 0);
                assert_eq!(fields[0].0, "timestamp"); // unchanged
            }
            other => panic!("expected SelectFields, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_enter_preserves_reordered_fields() {
        let mut tab = make_tab().await;
        let fields = vec![
            ("level".to_string(), true),
            ("timestamp".to_string(), true),
            ("message".to_string(), false),
        ];
        let mode = SelectFieldsMode::new(fields, FieldLayout::default());
        let (_, _) = press(mode, &mut tab, KeyCode::Enter).await;
        assert_eq!(
            tab.field_layout.columns,
            Some(vec!["level".to_string(), "timestamp".to_string()])
        );
    }
}
