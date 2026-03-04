use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::{
    config::Keybindings,
    mode::app_mode::{Mode, ModeRenderState, status_entry},
    mode::normal_mode::NormalMode,
    theme::Theme,
    ui::{KeyResult, TabState},
};

#[derive(Debug)]
pub struct SearchMode {
    pub input: String,
    pub forward: bool,
}

#[async_trait]
impl Mode for SearchMode {
    async fn handle_key(
        mut self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        if matches!(key, KeyCode::Tab | KeyCode::BackTab) {
            return (self, KeyResult::Ignored);
        }
        let kb = tab.keybindings.search.clone();
        if kb.confirm.matches(key, modifiers) {
            let visible = tab.visible_indices.clone();
            let texts = tab.collect_display_texts(&visible);
            let _ = tab.search.search(&self.input, &visible, |li| texts.get(&li).cloned());
            tab.search.set_forward(self.forward);
            // Navigate to the first match relative to the current line.
            let current_line_idx = tab
                .visible_indices
                .get(tab.scroll_offset)
                .copied()
                .unwrap_or(0);
            tab.search
                .set_position_for_search(current_line_idx, self.forward);
            let result = if self.forward {
                tab.search.next_match()
            } else {
                tab.search.previous_match()
            };
            if let Some(r) = result {
                let line_idx = r.line_idx;
                tab.scroll_to_line_idx(line_idx);
            }
            (Box::new(NormalMode::default()), KeyResult::Handled)
        } else if kb.cancel.matches(key, modifiers) {
            tab.search.clear();
            (Box::new(NormalMode::default()), KeyResult::Handled)
        } else {
            match key {
                KeyCode::Backspace => {
                    self.input.pop();
                    if self.input.is_empty() {
                        tab.search.clear();
                    } else {
                        let visible = tab.visible_indices.clone();
                        let texts = tab.collect_display_texts(&visible);
                        let _ = tab.search.search(&self.input, &visible, |li| texts.get(&li).cloned());
                        self.seed_incremental_position(tab);
                    }
                    (self, KeyResult::Handled)
                }
                KeyCode::Char(c) => {
                    self.input.push(c);
                    let visible = tab.visible_indices.clone();
                    let texts = tab.collect_display_texts(&visible);
                    let _ = tab.search.search(&self.input, &visible, |li| texts.get(&li).cloned());
                    self.seed_incremental_position(tab);
                    (self, KeyResult::Handled)
                }
                _ => (self, KeyResult::Handled),
            }
        }
    }

    fn mode_bar_content(&self, kb: &Keybindings, theme: &Theme) -> Line<'static> {
        let mut spans: Vec<Span<'static>> = vec![Span::styled(
            "[SEARCH]  ",
            Style::default()
                .fg(theme.text_highlight_fg)
                .add_modifier(Modifier::BOLD),
        )];
        status_entry(&mut spans, kb.search.cancel.display(), "cancel", theme);
        status_entry(&mut spans, kb.search.confirm.display(), "search", theme);
        Line::from(spans)
    }

    fn render_state(&self) -> ModeRenderState {
        ModeRenderState::Search {
            query: self.input.clone(),
            forward: self.forward,
        }
    }
}

impl SearchMode {
    /// After an incremental search, position the "current" occurrence highlight
    /// at the match closest to the cursor line (without scrolling).
    fn seed_incremental_position(&self, tab: &mut TabState) {
        let current_line_idx = tab
            .visible_indices
            .get(tab.scroll_offset)
            .copied()
            .unwrap_or(0);
        tab.search
            .set_position_for_search(current_line_idx, self.forward);
        if self.forward {
            tab.search.next_match();
        } else {
            tab.search.previous_match();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::file_reader::FileReader;
    use crate::log_manager::LogManager;
    use crate::ui::{KeyResult, TabState};
    use std::sync::Arc;

    async fn make_tab(lines: &[&str]) -> TabState {
        let data = lines.join("\n").into_bytes();
        let file_reader = FileReader::from_bytes(data);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        TabState::new(file_reader, log_manager, "test".to_string())
    }

    fn forward_mode(input: &str) -> SearchMode {
        SearchMode {
            input: input.to_string(),
            forward: true,
        }
    }

    fn backward_mode(input: &str) -> SearchMode {
        SearchMode {
            input: input.to_string(),
            forward: false,
        }
    }

    async fn press(
        mode: SearchMode,
        tab: &mut TabState,
        code: KeyCode,
    ) -> (Box<dyn Mode>, KeyResult) {
        Box::new(mode)
            .handle_key(tab, code, KeyModifiers::NONE)
            .await
    }

    #[tokio::test]
    async fn test_char_appends_to_input() {
        let mut tab = make_tab(&["line"]).await;
        let (mode, result) = press(forward_mode(""), &mut tab, KeyCode::Char('e')).await;
        assert!(matches!(result, KeyResult::Handled));
        match mode.render_state() {
            ModeRenderState::Search { query, .. } => assert_eq!(query, "e"),
            other => panic!("expected Search, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_multiple_chars_build_query() {
        let mut tab = make_tab(&["line"]).await;
        let mode = forward_mode("err");
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('o')).await;
        match mode2.render_state() {
            ModeRenderState::Search { query, .. } => assert_eq!(query, "erro"),
            other => panic!("expected Search, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_backspace_removes_last_char() {
        let mut tab = make_tab(&["line"]).await;
        let (mode2, result) = press(forward_mode("error"), &mut tab, KeyCode::Backspace).await;
        assert!(matches!(result, KeyResult::Handled));
        match mode2.render_state() {
            ModeRenderState::Search { query, .. } => assert_eq!(query, "erro"),
            other => panic!("expected Search, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_backspace_on_empty_no_panic() {
        let mut tab = make_tab(&["line"]).await;
        let (mode2, _) = press(forward_mode(""), &mut tab, KeyCode::Backspace).await;
        match mode2.render_state() {
            ModeRenderState::Search { query, .. } => assert_eq!(query, ""),
            other => panic!("expected Search, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_esc_returns_normal_mode_and_clears_search() {
        let mut tab = make_tab(&["error line"]).await;
        tab.visible_indices = vec![0];
        // Simulate incremental search having run
        let visible = tab.visible_indices.clone();
        let texts = tab.collect_display_texts(&visible);
        tab.search
            .search("error", &visible, |li| texts.get(&li).cloned())
            .unwrap();
        assert!(tab.search.get_pattern().is_some());
        let (mode2, result) = press(forward_mode("error"), &mut tab, KeyCode::Esc).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(!matches!(mode2.render_state(), ModeRenderState::Search { .. }));
        assert!(tab.search.get_pattern().is_none());
        assert!(tab.search.get_results().is_empty());
    }

    #[tokio::test]
    async fn test_tab_returns_ignored() {
        let mut tab = make_tab(&["line"]).await;
        let (_, result) = press(forward_mode(""), &mut tab, KeyCode::Tab).await;
        assert!(matches!(result, KeyResult::Ignored));
    }

    #[tokio::test]
    async fn test_backtab_returns_ignored() {
        let mut tab = make_tab(&["line"]).await;
        let (_, result) = press(forward_mode(""), &mut tab, KeyCode::BackTab).await;
        assert!(matches!(result, KeyResult::Ignored));
    }

    #[tokio::test]
    async fn test_enter_executes_forward_search_and_returns_normal_mode() {
        let mut tab = make_tab(&[
            "error: file not found",
            "warn: low memory",
            "error: timeout",
        ])
        .await;
        let (mode2, result) = press(forward_mode("error"), &mut tab, KeyCode::Enter).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(!matches!(
            mode2.render_state(),
            ModeRenderState::Search { .. }
        ));
    }

    #[tokio::test]
    async fn test_enter_with_no_match_still_returns_normal_mode() {
        let mut tab = make_tab(&["info: all good", "warn: minor issue"]).await;
        let (mode2, result) = press(forward_mode("critical"), &mut tab, KeyCode::Enter).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(!matches!(
            mode2.render_state(),
            ModeRenderState::Search { .. }
        ));
    }

    #[tokio::test]
    async fn test_enter_scrolls_to_matching_line() {
        let mut tab = make_tab(&["line0", "line1", "error here", "line3"]).await;
        tab.visible_height = 10;
        press(forward_mode("error"), &mut tab, KeyCode::Enter).await;
        assert_eq!(tab.scroll_offset, 2);
    }

    #[test]
    fn test_search_state_forward_true() {
        let mode = forward_mode("test");
        match mode.render_state() {
            ModeRenderState::Search { query, forward } => {
                assert_eq!(query, "test");
                assert!(forward);
            }
            other => panic!("expected Search, got {:?}", other),
        }
    }

    #[test]
    fn test_search_state_forward_false() {
        let mode = backward_mode("warn");
        match mode.render_state() {
            ModeRenderState::Search { query, forward } => {
                assert_eq!(query, "warn");
                assert!(!forward);
            }
            other => panic!("expected Search, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_typing_char_updates_search_results() {
        // Use plain text lines that won't trigger the structured-log format parser.
        let mut tab = make_tab(&["needle in haystack", "nothing here", "needle again"]).await;
        tab.visible_indices = vec![0, 1, 2];
        press(forward_mode("needl"), &mut tab, KeyCode::Char('e')).await;
        assert_eq!(tab.search.get_results().len(), 2);
    }

    #[tokio::test]
    async fn test_backspace_updates_search_results() {
        // Use plain text lines that won't trigger the structured-log format parser.
        let mut tab = make_tab(&["needle in haystack", "nothing here", "needle again"]).await;
        tab.visible_indices = vec![0, 1, 2];
        // Start with "needles" (no match), backspace to "needle" (2 matches)
        press(forward_mode("needles"), &mut tab, KeyCode::Backspace).await;
        assert_eq!(tab.search.get_results().len(), 2);
    }

    #[tokio::test]
    async fn test_backspace_to_empty_clears_results() {
        let mut tab = make_tab(&["error: disk full"]).await;
        tab.visible_indices = vec![0];
        press(forward_mode("e"), &mut tab, KeyCode::Backspace).await;
        assert!(tab.search.get_results().is_empty());
        assert!(tab.search.get_pattern().is_none());
    }

    #[test]
    fn test_needs_input_bar() {
        assert!(matches!(
            forward_mode("").render_state(),
            ModeRenderState::Command { .. } | ModeRenderState::Search { .. }
        ));
    }

    #[test]
    fn test_mode_bar_content_contains_search() {
        assert!(matches!(
            forward_mode("").render_state(),
            ModeRenderState::Search { .. }
        ));
    }
}
