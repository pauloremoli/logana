use crate::{
    config::Keybindings,
    mode::app_mode::{Mode, ModeRenderState, status_entry},
    mode::normal_mode::NormalMode,
    theme::Theme,
    ui::{KeyResult, TabState},
};
use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

#[derive(Debug)]
pub struct MergeSelectMode {
    /// Tab title + selected toggle (for display).
    pub tabs: Vec<(String, bool)>,
    /// Actual `App::tabs` index for each entry in `tabs`.
    pub tab_indices: Vec<usize>,
    /// Cursor position in the list.
    pub selected: usize,
}

impl MergeSelectMode {
    pub fn new(tabs: Vec<(String, bool)>, tab_indices: Vec<usize>) -> Self {
        MergeSelectMode {
            tabs,
            tab_indices,
            selected: 0,
        }
    }
}

#[async_trait]
impl Mode for MergeSelectMode {
    async fn handle_key(
        mut self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        let kb = &tab.interaction.keybindings;

        if kb.select_fields.apply.matches(key, modifiers) {
            let selected: Vec<usize> = self
                .tabs
                .iter()
                .enumerate()
                .filter(|(_, (_, on))| *on)
                .map(|(i, _)| self.tab_indices[i])
                .collect();
            if selected.len() < 2 {
                tab.interaction.command_error = Some("Select at least 2 tabs to merge".to_string());
                return (self, KeyResult::Handled);
            }
            return (
                Box::new(NormalMode::default()),
                KeyResult::OpenMergedView {
                    source_tab_indices: selected,
                },
            );
        }

        if kb.select_fields.cancel.matches(key, modifiers) {
            return (Box::new(NormalMode::default()), KeyResult::Handled);
        }

        if kb.navigation.scroll_down.matches(key, modifiers) {
            if !self.tabs.is_empty() {
                self.selected = (self.selected + 1).min(self.tabs.len() - 1);
            }
        } else if kb.navigation.scroll_up.matches(key, modifiers) {
            self.selected = self.selected.saturating_sub(1);
        } else if kb.select_fields.toggle.matches(key, modifiers) {
            if let Some(t) = self.tabs.get_mut(self.selected) {
                t.1 = !t.1;
            }
        } else if kb.select_fields.all.matches(key, modifiers) {
            for t in &mut self.tabs {
                t.1 = true;
            }
        } else if kb.select_fields.none.matches(key, modifiers) {
            for t in &mut self.tabs {
                t.1 = false;
            }
        }

        (self, KeyResult::Ignored)
    }

    fn mode_bar_content(&self, kb: &Keybindings, theme: &Theme) -> Line<'static> {
        let mut spans: Vec<Span<'static>> = vec![Span::styled(
            "[MERGE SELECT]  ",
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
        status_entry(&mut spans, kb.select_fields.apply.display(), "merge", theme);
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
        ModeRenderState::MergeSelect {
            tabs: self.tabs.clone(),
            selected: self.selected,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Database, LogManager};
    use crate::ingestion::FileReader;
    use crate::mode::app_mode::ModeRenderState;
    use crossterm::event::KeyModifiers;
    use std::sync::Arc;

    async fn make_tab() -> TabState {
        let reader = FileReader::from_bytes(b"line1\nline2\n".to_vec());
        let db = Arc::new(Database::in_memory().await.unwrap());
        let lm = LogManager::new(db, None).await;
        TabState::new(reader, lm, "test".to_string())
    }

    fn tabs(n: usize) -> Vec<(String, bool)> {
        (0..n).map(|i| (format!("tab{i}"), false)).collect()
    }

    fn indices(n: usize) -> Vec<usize> {
        (0..n).collect()
    }

    async fn press(
        mode: MergeSelectMode,
        tab: &mut TabState,
        code: KeyCode,
    ) -> (Box<dyn Mode>, KeyResult) {
        Box::new(mode)
            .handle_key(tab, code, KeyModifiers::NONE)
            .await
    }

    fn extract_merge_state(state: ModeRenderState) -> (Vec<(String, bool)>, usize) {
        match state {
            ModeRenderState::MergeSelect { tabs, selected } => (tabs, selected),
            other => panic!("expected MergeSelect, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_render_state_returns_merge_select() {
        let mode = MergeSelectMode::new(tabs(3), indices(3));
        assert!(matches!(
            mode.render_state(),
            ModeRenderState::MergeSelect { .. }
        ));
    }

    #[tokio::test]
    async fn test_scroll_down_moves_cursor() {
        let mut tab = make_tab().await;
        let mode = MergeSelectMode::new(tabs(3), indices(3));
        let (mode2, result) = press(mode, &mut tab, KeyCode::Char('j')).await;
        assert!(matches!(result, KeyResult::Ignored));
        let (_, selected) = extract_merge_state(mode2.render_state());
        assert_eq!(selected, 1);
    }

    #[tokio::test]
    async fn test_scroll_down_clamped_at_last() {
        let mut tab = make_tab().await;
        let mut mode = MergeSelectMode::new(tabs(2), indices(2));
        mode.selected = 1;
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('j')).await;
        let (_, selected) = extract_merge_state(mode2.render_state());
        assert_eq!(selected, 1);
    }

    #[tokio::test]
    async fn test_scroll_down_empty_list_is_noop() {
        let mut tab = make_tab().await;
        let mode = MergeSelectMode::new(vec![], vec![]);
        let (mode2, result) = press(mode, &mut tab, KeyCode::Char('j')).await;
        assert!(matches!(result, KeyResult::Ignored));
        let (_, selected) = extract_merge_state(mode2.render_state());
        assert_eq!(selected, 0);
    }

    #[tokio::test]
    async fn test_scroll_up_moves_cursor() {
        let mut tab = make_tab().await;
        let mut mode = MergeSelectMode::new(tabs(3), indices(3));
        mode.selected = 2;
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('k')).await;
        let (_, selected) = extract_merge_state(mode2.render_state());
        assert_eq!(selected, 1);
    }

    #[tokio::test]
    async fn test_scroll_up_clamped_at_zero() {
        let mut tab = make_tab().await;
        let mode = MergeSelectMode::new(tabs(3), indices(3));
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('k')).await;
        let (_, selected) = extract_merge_state(mode2.render_state());
        assert_eq!(selected, 0);
    }

    #[tokio::test]
    async fn test_toggle_enables_selected_tab() {
        let mut tab = make_tab().await;
        let mode = MergeSelectMode::new(tabs(3), indices(3));
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char(' ')).await;
        let (tabs, _) = extract_merge_state(mode2.render_state());
        assert!(tabs[0].1);
        assert!(!tabs[1].1);
    }

    #[tokio::test]
    async fn test_toggle_disables_enabled_tab() {
        let mut tab = make_tab().await;
        let mut mode = MergeSelectMode::new(tabs(3), indices(3));
        mode.tabs[0].1 = true;
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char(' ')).await;
        let (tabs, _) = extract_merge_state(mode2.render_state());
        assert!(!tabs[0].1);
    }

    #[tokio::test]
    async fn test_select_all() {
        let mut tab = make_tab().await;
        let mode = MergeSelectMode::new(tabs(3), indices(3));
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('a')).await;
        let (tabs, _) = extract_merge_state(mode2.render_state());
        assert!(tabs.iter().all(|(_, on)| *on));
    }

    #[tokio::test]
    async fn test_select_none() {
        let mut tab = make_tab().await;
        let mut mode = MergeSelectMode::new(tabs(3), indices(3));
        for t in &mut mode.tabs {
            t.1 = true;
        }
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('n')).await;
        let (tabs, _) = extract_merge_state(mode2.render_state());
        assert!(tabs.iter().all(|(_, on)| !on));
    }

    #[tokio::test]
    async fn test_apply_with_fewer_than_two_selected_shows_error() {
        let mut tab = make_tab().await;
        let mut mode = MergeSelectMode::new(tabs(3), indices(3));
        mode.tabs[0].1 = true;
        let (_, result) = press(mode, &mut tab, KeyCode::Enter).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(tab.interaction.command_error.is_some());
    }

    #[tokio::test]
    async fn test_apply_with_zero_selected_shows_error() {
        let mut tab = make_tab().await;
        let mode = MergeSelectMode::new(tabs(3), indices(3));
        let (_, result) = press(mode, &mut tab, KeyCode::Enter).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(tab.interaction.command_error.is_some());
    }

    #[tokio::test]
    async fn test_apply_with_two_selected_opens_merged_view() {
        let mut tab = make_tab().await;
        let mut mode = MergeSelectMode::new(tabs(3), vec![10, 20, 30]);
        mode.tabs[0].1 = true;
        mode.tabs[2].1 = true;
        let (_, result) = press(mode, &mut tab, KeyCode::Enter).await;
        assert!(matches!(
            result,
            KeyResult::OpenMergedView { source_tab_indices }
                if source_tab_indices == vec![10, 30]
        ));
    }

    #[tokio::test]
    async fn test_cancel_returns_to_normal_mode() {
        let mut tab = make_tab().await;
        let mode = MergeSelectMode::new(tabs(3), indices(3));
        let (_, result) = press(mode, &mut tab, KeyCode::Esc).await;
        assert!(matches!(result, KeyResult::Handled));
    }

    #[tokio::test]
    async fn test_unknown_key_returns_ignored() {
        let mut tab = make_tab().await;
        let mode = MergeSelectMode::new(tabs(3), indices(3));
        let (_, result) = press(mode, &mut tab, KeyCode::F(5)).await;
        assert!(matches!(result, KeyResult::Ignored));
    }

    #[test]
    fn test_mode_bar_content_contains_merge_select_label() {
        let mode = MergeSelectMode::new(tabs(2), indices(2));
        let kb = Keybindings::default();
        let theme = crate::theme::Theme::default();
        let line = mode.mode_bar_content(&kb, &theme);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("MERGE SELECT"));
    }

    #[test]
    fn test_new_initializes_selected_at_zero() {
        let mode = MergeSelectMode::new(tabs(5), indices(5));
        assert_eq!(mode.selected, 0);
        assert_eq!(mode.tabs.len(), 5);
        assert_eq!(mode.tab_indices.len(), 5);
    }
}
