use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyModifiers};

use crate::{
    config::Keybindings,
    mode::{
        app_mode::Mode, command_mode::CommandMode, filter_mode::FilterManagementMode,
        keybindings_help_mode::KeybindingsHelpMode, search_mode::SearchMode,
        visual_mode::VisualLineMode,
    },
    ui::{KeyResult, TabState},
};

#[derive(Debug)]
pub struct NormalMode;

#[async_trait]
impl Mode for NormalMode {
    async fn handle_key(
        self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        // Clone the Arc so we can mutate `tab` freely in each branch.
        let kb = tab.keybindings.clone();

        // ── Global key passthrough ──────────────────────────────────────────
        if kb.global.quit.matches(key, modifiers) {
            return (self, KeyResult::Ignored);
        }
        if kb.global.next_tab.matches(key, modifiers)
            || kb.global.prev_tab.matches(key, modifiers)
        {
            return (self, KeyResult::Ignored);
        }
        if kb.global.close_tab.matches(key, modifiers) {
            return (self, KeyResult::Ignored);
        }
        if kb.global.new_tab.matches(key, modifiers) {
            return (self, KeyResult::Ignored);
        }

        // ── Normal-mode actions ─────────────────────────────────────────────

        if kb.normal.half_page_down.matches(key, modifiers) {
            let half = (tab.visible_height / 2).max(1);
            tab.scroll_offset = tab.scroll_offset.saturating_add(half);
            tab.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }

        if kb.normal.half_page_up.matches(key, modifiers) {
            let half = (tab.visible_height / 2).max(1);
            tab.scroll_offset = tab.scroll_offset.saturating_sub(half);
            tab.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }

        if kb.normal.page_down.matches(key, modifiers) {
            let page = tab.visible_height.max(1);
            tab.scroll_offset = tab.scroll_offset.saturating_add(page);
            tab.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }

        if kb.normal.page_up.matches(key, modifiers) {
            let page = tab.visible_height.max(1);
            tab.scroll_offset = tab.scroll_offset.saturating_sub(page);
            tab.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }

        if kb.normal.command_mode.matches(key, modifiers) {
            let history = tab.command_history.clone();
            tab.g_key_pressed = false;
            return (
                Box::new(CommandMode::with_history(String::new(), 0, history)),
                KeyResult::Handled,
            );
        }

        if kb.normal.filter_mode.matches(key, modifiers) {
            tab.g_key_pressed = false;
            return (
                Box::new(FilterManagementMode {
                    selected_filter_index: 0,
                }),
                KeyResult::Handled,
            );
        }

        if kb.normal.toggle_filtering.matches(key, modifiers) {
            tab.filtering_enabled = !tab.filtering_enabled;
            tab.refresh_visible();
            tab.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }

        if kb.normal.toggle_sidebar.matches(key, modifiers) {
            tab.show_sidebar = !tab.show_sidebar;
            tab.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }

        if kb.normal.scroll_down.matches(key, modifiers) {
            tab.scroll_offset = tab.scroll_offset.saturating_add(1);
            tab.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }

        if kb.normal.scroll_up.matches(key, modifiers) {
            tab.scroll_offset = tab.scroll_offset.saturating_sub(1);
            tab.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }

        if kb.normal.scroll_left.matches(key, modifiers) {
            if !tab.wrap {
                tab.horizontal_scroll = tab.horizontal_scroll.saturating_sub(1);
            }
            tab.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }

        if kb.normal.scroll_right.matches(key, modifiers) {
            if !tab.wrap {
                tab.horizontal_scroll = tab.horizontal_scroll.saturating_add(1);
            }
            tab.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }

        if kb.normal.toggle_wrap.matches(key, modifiers) {
            tab.wrap = !tab.wrap;
            tab.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }

        if kb.normal.go_to_bottom.matches(key, modifiers) {
            let n = tab.visible_indices.len();
            if n > 0 {
                tab.scroll_offset = n - 1;
            }
            tab.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }

        // gg chord: first press sets the flag; second press jumps to top.
        if kb.normal.go_to_top_chord.matches(key, modifiers) {
            if tab.g_key_pressed {
                tab.scroll_offset = 0;
                tab.g_key_pressed = false;
            } else {
                tab.g_key_pressed = true;
            }
            return (self, KeyResult::Handled);
        }

        if kb.normal.mark_line.matches(key, modifiers) {
            if let Some(&line_idx) = tab.visible_indices.get(tab.scroll_offset) {
                tab.log_manager.toggle_mark(line_idx);
            }
            tab.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }

        if kb.normal.toggle_marks_only.matches(key, modifiers) {
            tab.show_marks_only = !tab.show_marks_only;
            tab.refresh_visible();
            tab.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }

        if kb.normal.visual_mode.matches(key, modifiers) {
            let anchor = tab.scroll_offset;
            tab.g_key_pressed = false;
            return (Box::new(VisualLineMode { anchor }), KeyResult::Handled);
        }

        if kb.normal.search_forward.matches(key, modifiers) {
            tab.g_key_pressed = false;
            return (
                Box::new(SearchMode {
                    input: String::new(),
                    forward: true,
                }),
                KeyResult::Handled,
            );
        }

        if kb.normal.search_backward.matches(key, modifiers) {
            tab.g_key_pressed = false;
            return (
                Box::new(SearchMode {
                    input: String::new(),
                    forward: false,
                }),
                KeyResult::Handled,
            );
        }

        if kb.normal.next_match.matches(key, modifiers) {
            if let Some(result) = tab.search.next_match() {
                let line_idx = result.line_idx;
                tab.scroll_to_line_idx(line_idx);
            }
            tab.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }

        if kb.normal.prev_match.matches(key, modifiers) {
            if let Some(result) = tab.search.previous_match() {
                let line_idx = result.line_idx;
                tab.scroll_to_line_idx(line_idx);
            }
            tab.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }

        if kb.normal.show_keybindings.matches(key, modifiers) {
            tab.g_key_pressed = false;
            return (
                Box::new(KeybindingsHelpMode::new()),
                KeyResult::Handled,
            );
        }

        // Unrecognised key — consume it, reset the gg-chord state.
        tab.g_key_pressed = false;
        (self, KeyResult::Handled)
    }

    fn status_line(&self) -> &str {
        "[NORMAL] [q]uit | [f]ilter mode | [F] toggle filtering | [m]ark | [M] marks only | [s]idebar | [V]isual select | Tab/Shift+Tab switch tab | [F1] help"
    }

    fn dynamic_status_line(&self, kb: &Keybindings) -> String {
        format!(
            "[NORMAL] [{}]uit | [{}]ilter | [{}] tog.filter | [{}]ark | [{}] marks only | [{}]idebar | [{}]isual | [{}] help | {}/{} tabs",
            kb.global.quit.display(),
            kb.normal.filter_mode.display(),
            kb.normal.toggle_filtering.display(),
            kb.normal.mark_line.display(),
            kb.normal.toggle_marks_only.display(),
            kb.normal.toggle_sidebar.display(),
            kb.normal.visual_mode.display(),
            kb.normal.show_keybindings.display(),
            kb.global.next_tab.display(),
            kb.global.prev_tab.display(),
        )
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

    async fn press(
        tab: &mut TabState,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        Box::new(NormalMode)
            .handle_key(tab, code, modifiers)
            .await
    }

    #[tokio::test]
    async fn test_j_increments_scroll_offset() {
        let mut tab = make_tab(&["a", "b", "c"]).await;
        press(&mut tab, KeyCode::Char('j'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll_offset, 1);
    }

    #[tokio::test]
    async fn test_down_increments_scroll_offset() {
        let mut tab = make_tab(&["a", "b"]).await;
        press(&mut tab, KeyCode::Down, KeyModifiers::NONE).await;
        assert_eq!(tab.scroll_offset, 1);
    }

    #[tokio::test]
    async fn test_k_saturates_at_zero() {
        let mut tab = make_tab(&["a"]).await;
        press(&mut tab, KeyCode::Char('k'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll_offset, 0);
    }

    #[tokio::test]
    async fn test_up_saturates_at_zero() {
        let mut tab = make_tab(&["a"]).await;
        press(&mut tab, KeyCode::Up, KeyModifiers::NONE).await;
        assert_eq!(tab.scroll_offset, 0);
    }

    #[tokio::test]
    async fn test_capital_g_jumps_to_last_visible_line() {
        let mut tab = make_tab(&["a", "b", "c", "d", "e"]).await;
        press(&mut tab, KeyCode::Char('G'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll_offset, 4);
    }

    #[tokio::test]
    async fn test_capital_g_on_empty_does_not_panic() {
        let mut tab = make_tab(&[]).await;
        press(&mut tab, KeyCode::Char('G'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll_offset, 0);
    }

    #[tokio::test]
    async fn test_gg_jumps_to_top() {
        let mut tab = make_tab(&["a", "b", "c"]).await;
        tab.scroll_offset = 2;
        press(&mut tab, KeyCode::Char('g'), KeyModifiers::NONE).await;
        assert!(tab.g_key_pressed);
        assert_eq!(tab.scroll_offset, 2);
        press(&mut tab, KeyCode::Char('g'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll_offset, 0);
        assert!(!tab.g_key_pressed);
    }

    #[tokio::test]
    async fn test_ctrl_d_half_page_down() {
        let mut tab = make_tab(&["a", "b", "c", "d", "e", "f"]).await;
        tab.visible_height = 4;
        press(&mut tab, KeyCode::Char('d'), KeyModifiers::CONTROL).await;
        assert_eq!(tab.scroll_offset, 2);
    }

    #[tokio::test]
    async fn test_ctrl_u_half_page_up() {
        let mut tab = make_tab(&["a", "b", "c", "d"]).await;
        tab.visible_height = 4;
        tab.scroll_offset = 3;
        press(&mut tab, KeyCode::Char('u'), KeyModifiers::CONTROL).await;
        assert_eq!(tab.scroll_offset, 1);
    }

    #[tokio::test]
    async fn test_page_down() {
        let mut tab = make_tab(&["a", "b", "c", "d", "e"]).await;
        tab.visible_height = 3;
        press(&mut tab, KeyCode::PageDown, KeyModifiers::NONE).await;
        assert_eq!(tab.scroll_offset, 3);
    }

    #[tokio::test]
    async fn test_page_up_saturates_at_zero() {
        let mut tab = make_tab(&["a"]).await;
        tab.visible_height = 5;
        press(&mut tab, KeyCode::PageUp, KeyModifiers::NONE).await;
        assert_eq!(tab.scroll_offset, 0);
    }

    #[tokio::test]
    async fn test_w_toggles_wrap() {
        let mut tab = make_tab(&["line"]).await;
        assert!(tab.wrap);
        press(&mut tab, KeyCode::Char('w'), KeyModifiers::NONE).await;
        assert!(!tab.wrap);
        press(&mut tab, KeyCode::Char('w'), KeyModifiers::NONE).await;
        assert!(tab.wrap);
    }

    #[tokio::test]
    async fn test_s_toggles_sidebar() {
        let mut tab = make_tab(&["line"]).await;
        let initial = tab.show_sidebar;
        press(&mut tab, KeyCode::Char('s'), KeyModifiers::NONE).await;
        assert_eq!(tab.show_sidebar, !initial);
        press(&mut tab, KeyCode::Char('s'), KeyModifiers::NONE).await;
        assert_eq!(tab.show_sidebar, initial);
    }

    #[tokio::test]
    async fn test_colon_transitions_to_command_mode() {
        let mut tab = make_tab(&["line"]).await;
        let (mode, result) = press(&mut tab, KeyCode::Char(':'), KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(mode.command_state().is_some());
        assert!(mode.needs_input_bar());
    }

    #[tokio::test]
    async fn test_f_transitions_to_filter_mode() {
        let mut tab = make_tab(&["line"]).await;
        let (mode, result) = press(&mut tab, KeyCode::Char('f'), KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(mode.selected_filter_index().is_some());
    }

    #[tokio::test]
    async fn test_slash_transitions_to_forward_search() {
        let mut tab = make_tab(&["line"]).await;
        let (mode, _) = press(&mut tab, KeyCode::Char('/'), KeyModifiers::NONE).await;
        assert!(mode.needs_input_bar());
        let search = mode.search_state();
        assert!(search.is_some());
        assert!(search.unwrap().1);
    }

    #[tokio::test]
    async fn test_question_mark_transitions_to_backward_search() {
        let mut tab = make_tab(&["line"]).await;
        let (mode, _) = press(&mut tab, KeyCode::Char('?'), KeyModifiers::NONE).await;
        let search = mode.search_state();
        assert!(search.is_some());
        assert!(!search.unwrap().1);
    }

    #[tokio::test]
    async fn test_q_returns_ignored() {
        let mut tab = make_tab(&["line"]).await;
        let (_, result) = press(&mut tab, KeyCode::Char('q'), KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::Ignored));
    }

    #[tokio::test]
    async fn test_tab_returns_ignored() {
        let mut tab = make_tab(&["line"]).await;
        let (_, result) = press(&mut tab, KeyCode::Tab, KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::Ignored));
    }

    #[tokio::test]
    async fn test_backtab_returns_ignored() {
        let mut tab = make_tab(&["line"]).await;
        let (_, result) = press(&mut tab, KeyCode::BackTab, KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::Ignored));
    }

    #[tokio::test]
    async fn test_ctrl_w_returns_ignored() {
        let mut tab = make_tab(&["line"]).await;
        let (_, result) = press(&mut tab, KeyCode::Char('w'), KeyModifiers::CONTROL).await;
        assert!(matches!(result, KeyResult::Ignored));
    }

    #[tokio::test]
    async fn test_ctrl_t_returns_ignored() {
        let mut tab = make_tab(&["line"]).await;
        let (_, result) = press(&mut tab, KeyCode::Char('t'), KeyModifiers::CONTROL).await;
        assert!(matches!(result, KeyResult::Ignored));
    }

    #[tokio::test]
    async fn test_h_decrements_horizontal_scroll_when_not_wrapped() {
        let mut tab = make_tab(&["long line"]).await;
        tab.wrap = false;
        tab.horizontal_scroll = 5;
        press(&mut tab, KeyCode::Char('h'), KeyModifiers::NONE).await;
        assert_eq!(tab.horizontal_scroll, 4);
    }

    #[tokio::test]
    async fn test_l_increments_horizontal_scroll_when_not_wrapped() {
        let mut tab = make_tab(&["long line"]).await;
        tab.wrap = false;
        press(&mut tab, KeyCode::Char('l'), KeyModifiers::NONE).await;
        assert_eq!(tab.horizontal_scroll, 1);
    }

    #[tokio::test]
    async fn test_h_no_horizontal_scroll_when_wrapped() {
        let mut tab = make_tab(&["long line"]).await;
        tab.wrap = true;
        tab.horizontal_scroll = 5;
        press(&mut tab, KeyCode::Char('h'), KeyModifiers::NONE).await;
        assert_eq!(tab.horizontal_scroll, 5);
    }

    #[tokio::test]
    async fn test_l_no_horizontal_scroll_when_wrapped() {
        let mut tab = make_tab(&["long line"]).await;
        tab.wrap = true;
        press(&mut tab, KeyCode::Char('l'), KeyModifiers::NONE).await;
        assert_eq!(tab.horizontal_scroll, 0);
    }

    #[tokio::test]
    async fn test_m_marks_current_line() {
        let mut tab = make_tab(&["line0", "line1"]).await;
        tab.scroll_offset = 0;
        press(&mut tab, KeyCode::Char('m'), KeyModifiers::NONE).await;
        assert!(tab.log_manager.get_marked_indices().contains(&0));
    }

    #[tokio::test]
    async fn test_m_unmarks_already_marked_line() {
        let mut tab = make_tab(&["line0"]).await;
        tab.scroll_offset = 0;
        press(&mut tab, KeyCode::Char('m'), KeyModifiers::NONE).await;
        press(&mut tab, KeyCode::Char('m'), KeyModifiers::NONE).await;
        assert!(!tab.log_manager.get_marked_indices().contains(&0));
    }

    #[tokio::test]
    async fn test_g_key_resets_on_non_g_press() {
        let mut tab = make_tab(&["a"]).await;
        press(&mut tab, KeyCode::Char('g'), KeyModifiers::NONE).await;
        assert!(tab.g_key_pressed);
        press(&mut tab, KeyCode::Char('j'), KeyModifiers::NONE).await;
        assert!(!tab.g_key_pressed);
    }

    #[tokio::test]
    async fn test_capital_f_toggles_filtering_enabled() {
        let mut tab = make_tab(&["a", "b", "c"]).await;
        assert!(tab.filtering_enabled);
        press(&mut tab, KeyCode::Char('F'), KeyModifiers::NONE).await;
        assert!(!tab.filtering_enabled);
        press(&mut tab, KeyCode::Char('F'), KeyModifiers::NONE).await;
        assert!(tab.filtering_enabled);
    }

    #[tokio::test]
    async fn test_filtering_disabled_shows_all_lines() {
        let mut tab = make_tab(&["error", "warn", "info"]).await;
        tab.log_manager
            .add_filter_with_color(
                "error".to_string(),
                crate::types::FilterType::Include,
                None,
                None,
                false,
            )
            .await;
        tab.refresh_visible();
        // With filtering on, only "error" line is visible
        assert_eq!(tab.visible_indices.len(), 1);

        press(&mut tab, KeyCode::Char('F'), KeyModifiers::NONE).await;
        // With filtering off, all 3 lines are visible
        assert_eq!(tab.visible_indices.len(), 3);
    }

    #[tokio::test]
    async fn test_capital_m_toggles_marks_only() {
        let mut tab = make_tab(&["a", "b", "c"]).await;
        assert!(!tab.show_marks_only);
        press(&mut tab, KeyCode::Char('M'), KeyModifiers::NONE).await;
        assert!(tab.show_marks_only);
        press(&mut tab, KeyCode::Char('M'), KeyModifiers::NONE).await;
        assert!(!tab.show_marks_only);
    }

    #[tokio::test]
    async fn test_marks_only_shows_only_marked_lines() {
        let mut tab = make_tab(&["line0", "line1", "line2"]).await;
        // Mark lines 0 and 2
        tab.log_manager.toggle_mark(0);
        tab.log_manager.toggle_mark(2);

        press(&mut tab, KeyCode::Char('M'), KeyModifiers::NONE).await;

        assert_eq!(tab.visible_indices, vec![0, 2]);
    }

    #[tokio::test]
    async fn test_marks_only_off_restores_all_lines() {
        let mut tab = make_tab(&["line0", "line1", "line2"]).await;
        tab.log_manager.toggle_mark(1);
        press(&mut tab, KeyCode::Char('M'), KeyModifiers::NONE).await;
        assert_eq!(tab.visible_indices.len(), 1);

        press(&mut tab, KeyCode::Char('M'), KeyModifiers::NONE).await;
        assert_eq!(tab.visible_indices.len(), 3);
    }

    #[tokio::test]
    async fn test_marks_only_empty_when_no_marks() {
        let mut tab = make_tab(&["a", "b"]).await;
        press(&mut tab, KeyCode::Char('M'), KeyModifiers::NONE).await;
        assert!(tab.visible_indices.is_empty());
    }

    #[test]
    fn test_status_line_contains_normal() {
        assert!(NormalMode.status_line().contains("[NORMAL]"));
    }

    #[test]
    fn test_status_line_contains_marks_only_hint() {
        assert!(NormalMode.status_line().contains("[M] marks only"));
    }
}
