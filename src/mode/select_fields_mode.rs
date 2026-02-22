use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyModifiers};

use crate::{
    mode::{app_mode::Mode, normal_mode::NormalMode},
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
        _modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        match key {
            KeyCode::Char('j') | KeyCode::Down => {
                if !self.fields.is_empty() {
                    self.selected = (self.selected + 1).min(self.fields.len() - 1);
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
            }
            KeyCode::Char(' ') => {
                if let Some(f) = self.fields.get_mut(self.selected) {
                    f.1 = !f.1;
                }
            }
            KeyCode::Char('J') => {
                // Move selected field down
                if self.selected + 1 < self.fields.len() {
                    self.fields.swap(self.selected, self.selected + 1);
                    self.selected += 1;
                }
            }
            KeyCode::Char('K') => {
                // Move selected field up
                if self.selected > 0 {
                    self.fields.swap(self.selected, self.selected - 1);
                    self.selected -= 1;
                }
            }
            KeyCode::Char('a') => {
                for f in &mut self.fields {
                    f.1 = true;
                }
            }
            KeyCode::Char('n') => {
                for f in &mut self.fields {
                    f.1 = false;
                }
            }
            KeyCode::Enter => {
                let enabled: Vec<String> = self
                    .fields
                    .iter()
                    .filter(|(_, on)| *on)
                    .map(|(name, _)| name.clone())
                    .collect();
                // Always store the full ordered list so user's ordering is
                // preserved.  Also store disabled names so the order is
                // restored when the modal is reopened.
                let all_ordered: Vec<String> =
                    self.fields.iter().map(|(n, _)| n.clone()).collect();
                tab.field_layout.json_columns = Some(enabled);
                tab.field_layout.json_columns_order = Some(all_ordered);
                return (Box::new(NormalMode), KeyResult::Handled);
            }
            KeyCode::Esc => {
                tab.field_layout = std::mem::take(&mut self.original_layout);
                return (Box::new(NormalMode), KeyResult::Handled);
            }
            _ => {}
        }
        (self, KeyResult::Handled)
    }

    fn status_line(&self) -> &str {
        "[SELECT FIELDS] Space=toggle | J/K=reorder | Enter=apply | Esc=cancel | a=all | n=none"
    }

    fn select_fields_state(&self) -> Option<(&[(String, bool)], usize)> {
        Some((&self.fields, self.selected))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::file_reader::FileReader;
    use crate::log_manager::LogManager;
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
        assert_eq!(mode2.select_fields_state().unwrap().1, 1);
    }

    #[tokio::test]
    async fn test_k_moves_cursor_up() {
        let mut tab = make_tab().await;
        let mut mode = SelectFieldsMode::new(sample_fields(), FieldLayout::default());
        mode.selected = 2;
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('k')).await;
        assert_eq!(mode2.select_fields_state().unwrap().1, 1);
    }

    #[tokio::test]
    async fn test_k_at_zero_stays() {
        let mut tab = make_tab().await;
        let mode = SelectFieldsMode::new(sample_fields(), FieldLayout::default());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('k')).await;
        assert_eq!(mode2.select_fields_state().unwrap().1, 0);
    }

    #[tokio::test]
    async fn test_j_at_end_stays() {
        let mut tab = make_tab().await;
        let mut mode = SelectFieldsMode::new(sample_fields(), FieldLayout::default());
        mode.selected = 3;
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('j')).await;
        assert_eq!(mode2.select_fields_state().unwrap().1, 3);
    }

    #[tokio::test]
    async fn test_space_toggles_field() {
        let mut tab = make_tab().await;
        let mode = SelectFieldsMode::new(sample_fields(), FieldLayout::default());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char(' ')).await;
        let (fields, _) = mode2.select_fields_state().unwrap();
        assert!(!fields[0].1); // was true, now false
    }

    #[tokio::test]
    async fn test_a_enables_all() {
        let mut tab = make_tab().await;
        let mode = SelectFieldsMode::new(sample_fields(), FieldLayout::default());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('a')).await;
        let (fields, _) = mode2.select_fields_state().unwrap();
        assert!(fields.iter().all(|(_, on)| *on));
    }

    #[tokio::test]
    async fn test_n_disables_all() {
        let mut tab = make_tab().await;
        let mode = SelectFieldsMode::new(sample_fields(), FieldLayout::default());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('n')).await;
        let (fields, _) = mode2.select_fields_state().unwrap();
        assert!(fields.iter().all(|(_, on)| !*on));
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
        assert!(mode2.select_fields_state().is_none()); // transitioned to NormalMode
        assert_eq!(
            tab.field_layout.json_columns,
            Some(vec!["timestamp".to_string(), "message".to_string()])
        );
        assert_eq!(
            tab.field_layout.json_columns_order,
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
        let fields = vec![
            ("timestamp".to_string(), true),
            ("level".to_string(), true),
        ];
        let mode = SelectFieldsMode::new(fields, FieldLayout::default());
        let (_, _) = press(mode, &mut tab, KeyCode::Enter).await;
        assert_eq!(
            tab.field_layout.json_columns,
            Some(vec!["timestamp".to_string(), "level".to_string()])
        );
        assert_eq!(
            tab.field_layout.json_columns_order,
            Some(vec!["timestamp".to_string(), "level".to_string()])
        );
    }

    #[tokio::test]
    async fn test_esc_restores_original_layout() {
        let mut tab = make_tab().await;
        let original = FieldLayout {
            json_columns: Some(vec!["level".to_string()]),
            json_columns_order: Some(vec!["level".to_string(), "timestamp".to_string()]),
        };
        let mode = SelectFieldsMode::new(sample_fields(), original.clone());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Esc).await;
        assert!(mode2.select_fields_state().is_none()); // NormalMode
        assert_eq!(tab.field_layout.json_columns, original.json_columns);
        assert_eq!(
            tab.field_layout.json_columns_order,
            original.json_columns_order
        );
    }

    #[tokio::test]
    async fn test_status_line() {
        let mode = SelectFieldsMode::new(sample_fields(), FieldLayout::default());
        assert!(mode.status_line().contains("[SELECT FIELDS]"));
    }

    #[tokio::test]
    async fn test_down_arrow_moves_cursor() {
        let mut tab = make_tab().await;
        let mode = SelectFieldsMode::new(sample_fields(), FieldLayout::default());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Down).await;
        assert_eq!(mode2.select_fields_state().unwrap().1, 1);
    }

    #[tokio::test]
    async fn test_up_arrow_moves_cursor() {
        let mut tab = make_tab().await;
        let mut mode = SelectFieldsMode::new(sample_fields(), FieldLayout::default());
        mode.selected = 2;
        let (mode2, _) = press(mode, &mut tab, KeyCode::Up).await;
        assert_eq!(mode2.select_fields_state().unwrap().1, 1);
    }

    #[tokio::test]
    async fn test_shift_j_moves_field_down() {
        let mut tab = make_tab().await;
        let mut mode = SelectFieldsMode::new(sample_fields(), FieldLayout::default());
        mode.selected = 0; // "timestamp"
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('J')).await;
        let (fields, sel) = mode2.select_fields_state().unwrap();
        assert_eq!(sel, 1);
        assert_eq!(fields[0].0, "level");
        assert_eq!(fields[1].0, "timestamp");
    }

    #[tokio::test]
    async fn test_shift_k_moves_field_up() {
        let mut tab = make_tab().await;
        let mut mode = SelectFieldsMode::new(sample_fields(), FieldLayout::default());
        mode.selected = 2; // "message"
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('K')).await;
        let (fields, sel) = mode2.select_fields_state().unwrap();
        assert_eq!(sel, 1);
        assert_eq!(fields[1].0, "message");
        assert_eq!(fields[2].0, "level");
    }

    #[tokio::test]
    async fn test_shift_j_at_end_stays() {
        let mut tab = make_tab().await;
        let mut mode = SelectFieldsMode::new(sample_fields(), FieldLayout::default());
        mode.selected = 3; // last item
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('J')).await;
        let (fields, sel) = mode2.select_fields_state().unwrap();
        assert_eq!(sel, 3);
        assert_eq!(fields[3].0, "request_id"); // unchanged
    }

    #[tokio::test]
    async fn test_shift_k_at_zero_stays() {
        let mut tab = make_tab().await;
        let mode = SelectFieldsMode::new(sample_fields(), FieldLayout::default());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('K')).await;
        let (fields, sel) = mode2.select_fields_state().unwrap();
        assert_eq!(sel, 0);
        assert_eq!(fields[0].0, "timestamp"); // unchanged
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
            tab.field_layout.json_columns,
            Some(vec!["level".to_string(), "timestamp".to_string()])
        );
    }
}
