use crossterm::event::{KeyCode, KeyModifiers};

use crate::{
    mode::{
        app_mode::Mode, command_mode::CommandMode, filter_mode::FilterManagementMode,
        search_mode::SearchMode,
    },
    ui::{KeyResult, TabState},
};

#[derive(Debug)]
pub struct NormalMode;

impl Mode for NormalMode {
    fn handle_key(
        self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        // Pass these to the global handler
        if key == KeyCode::Char('q') && modifiers.is_empty() {
            return (self, KeyResult::Ignored);
        }
        if matches!(key, KeyCode::Tab | KeyCode::BackTab) {
            return (self, KeyResult::Ignored);
        }
        if key == KeyCode::Char('w') && modifiers.contains(KeyModifiers::CONTROL) {
            return (self, KeyResult::Ignored);
        }
        if key == KeyCode::Char('t') && modifiers.contains(KeyModifiers::CONTROL) {
            return (self, KeyResult::Ignored);
        }

        // Ctrl+d: half page down
        if key == KeyCode::Char('d') && modifiers.contains(KeyModifiers::CONTROL) {
            let half = (tab.visible_height / 2).max(1);
            tab.scroll_offset = tab.scroll_offset.saturating_add(half);
            tab.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }
        // Ctrl+u: half page up
        if key == KeyCode::Char('u') && modifiers.contains(KeyModifiers::CONTROL) {
            let half = (tab.visible_height / 2).max(1);
            tab.scroll_offset = tab.scroll_offset.saturating_sub(half);
            tab.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }

        match key {
            KeyCode::PageDown => {
                let page = tab.visible_height.max(1);
                tab.scroll_offset = tab.scroll_offset.saturating_add(page);
                tab.g_key_pressed = false;
            }
            KeyCode::PageUp => {
                let page = tab.visible_height.max(1);
                tab.scroll_offset = tab.scroll_offset.saturating_sub(page);
                tab.g_key_pressed = false;
            }
            KeyCode::Char(':') => {
                let history = tab.command_history.clone();
                return (
                    Box::new(CommandMode::with_history(String::new(), 0, history)),
                    KeyResult::Handled,
                );
            }
            KeyCode::Char('f') => {
                return (
                    Box::new(FilterManagementMode {
                        selected_filter_index: 0,
                    }),
                    KeyResult::Handled,
                );
            }
            KeyCode::Char('s') => {
                tab.show_sidebar = !tab.show_sidebar;
                tab.g_key_pressed = false;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                tab.scroll_offset = tab.scroll_offset.saturating_add(1);
                tab.g_key_pressed = false;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                tab.scroll_offset = tab.scroll_offset.saturating_sub(1);
                tab.g_key_pressed = false;
            }
            KeyCode::Char('h') => {
                if !tab.wrap {
                    tab.horizontal_scroll = tab.horizontal_scroll.saturating_sub(1);
                }
                tab.g_key_pressed = false;
            }
            KeyCode::Char('l') => {
                if !tab.wrap {
                    tab.horizontal_scroll = tab.horizontal_scroll.saturating_add(1);
                }
                tab.g_key_pressed = false;
            }
            KeyCode::Char('w') => {
                tab.wrap = !tab.wrap;
                tab.g_key_pressed = false;
            }
            KeyCode::Char('G') => {
                let n = tab.visible_indices.len();
                if n > 0 {
                    tab.scroll_offset = n - 1;
                }
                tab.g_key_pressed = false;
            }
            KeyCode::Char('g') => {
                if tab.g_key_pressed {
                    tab.scroll_offset = 0;
                    tab.g_key_pressed = false;
                } else {
                    tab.g_key_pressed = true;
                }
            }
            KeyCode::Char('m') => {
                if let Some(&line_idx) = tab.visible_indices.get(tab.scroll_offset) {
                    tab.log_manager.toggle_mark(line_idx);
                }
                tab.g_key_pressed = false;
            }
            KeyCode::Char('/') => {
                tab.g_key_pressed = false;
                return (
                    Box::new(SearchMode {
                        input: String::new(),
                        forward: true,
                    }),
                    KeyResult::Handled,
                );
            }
            KeyCode::Char('?') => {
                tab.g_key_pressed = false;
                return (
                    Box::new(SearchMode {
                        input: String::new(),
                        forward: false,
                    }),
                    KeyResult::Handled,
                );
            }
            KeyCode::Char('n') => {
                if let Some(result) = tab.search.next_match() {
                    let line_idx = result.line_idx;
                    tab.scroll_to_line_idx(line_idx);
                }
                tab.g_key_pressed = false;
            }
            KeyCode::Char('N') => {
                if let Some(result) = tab.search.previous_match() {
                    let line_idx = result.line_idx;
                    tab.scroll_to_line_idx(line_idx);
                }
                tab.g_key_pressed = false;
            }
            _ => {
                tab.g_key_pressed = false;
            }
        }
        (self, KeyResult::Handled)
    }

    fn status_line(&self) -> &str {
        "[NORMAL] [q]uit | : => command Mode | [f]ilter mode | [s]idebar | [m]ark Line | / => search | ? => search backward | [n]ext match | N => prev match | PgDn/Ctrl+d PgUp/Ctrl+u | Tab/Shift+Tab => switch tab"
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

    fn make_tab(lines: &[&str]) -> TabState {
        let data = lines.join("\n").into_bytes();
        let file_reader = FileReader::from_bytes(data);
        let rt = Arc::new(tokio::runtime::Runtime::new().unwrap());
        let db = Arc::new(rt.block_on(Database::in_memory()).unwrap());
        let log_manager = LogManager::new(db, rt, None);
        TabState::new(file_reader, log_manager, "test".to_string())
    }

    fn press(tab: &mut TabState, code: KeyCode, modifiers: KeyModifiers) -> (Box<dyn Mode>, KeyResult) {
        Box::new(NormalMode).handle_key(tab, code, modifiers)
    }

    #[test]
    fn test_j_increments_scroll_offset() {
        let mut tab = make_tab(&["a", "b", "c"]);
        press(&mut tab, KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(tab.scroll_offset, 1);
    }

    #[test]
    fn test_down_increments_scroll_offset() {
        let mut tab = make_tab(&["a", "b"]);
        press(&mut tab, KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(tab.scroll_offset, 1);
    }

    #[test]
    fn test_k_saturates_at_zero() {
        let mut tab = make_tab(&["a"]);
        press(&mut tab, KeyCode::Char('k'), KeyModifiers::NONE);
        assert_eq!(tab.scroll_offset, 0);
    }

    #[test]
    fn test_up_saturates_at_zero() {
        let mut tab = make_tab(&["a"]);
        press(&mut tab, KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(tab.scroll_offset, 0);
    }

    #[test]
    fn test_capital_g_jumps_to_last_visible_line() {
        let mut tab = make_tab(&["a", "b", "c", "d", "e"]);
        press(&mut tab, KeyCode::Char('G'), KeyModifiers::NONE);
        assert_eq!(tab.scroll_offset, 4);
    }

    #[test]
    fn test_capital_g_on_empty_does_not_panic() {
        let mut tab = make_tab(&[]);
        press(&mut tab, KeyCode::Char('G'), KeyModifiers::NONE);
        assert_eq!(tab.scroll_offset, 0);
    }

    #[test]
    fn test_gg_jumps_to_top() {
        let mut tab = make_tab(&["a", "b", "c"]);
        tab.scroll_offset = 2;
        press(&mut tab, KeyCode::Char('g'), KeyModifiers::NONE);
        assert!(tab.g_key_pressed);
        assert_eq!(tab.scroll_offset, 2);
        press(&mut tab, KeyCode::Char('g'), KeyModifiers::NONE);
        assert_eq!(tab.scroll_offset, 0);
        assert!(!tab.g_key_pressed);
    }

    #[test]
    fn test_ctrl_d_half_page_down() {
        let mut tab = make_tab(&["a", "b", "c", "d", "e", "f"]);
        tab.visible_height = 4;
        press(&mut tab, KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert_eq!(tab.scroll_offset, 2);
    }

    #[test]
    fn test_ctrl_u_half_page_up() {
        let mut tab = make_tab(&["a", "b", "c", "d"]);
        tab.visible_height = 4;
        tab.scroll_offset = 3;
        press(&mut tab, KeyCode::Char('u'), KeyModifiers::CONTROL);
        assert_eq!(tab.scroll_offset, 1);
    }

    #[test]
    fn test_page_down() {
        let mut tab = make_tab(&["a", "b", "c", "d", "e"]);
        tab.visible_height = 3;
        press(&mut tab, KeyCode::PageDown, KeyModifiers::NONE);
        assert_eq!(tab.scroll_offset, 3);
    }

    #[test]
    fn test_page_up_saturates_at_zero() {
        let mut tab = make_tab(&["a"]);
        tab.visible_height = 5;
        press(&mut tab, KeyCode::PageUp, KeyModifiers::NONE);
        assert_eq!(tab.scroll_offset, 0);
    }

    #[test]
    fn test_w_toggles_wrap() {
        let mut tab = make_tab(&["line"]);
        assert!(tab.wrap);
        press(&mut tab, KeyCode::Char('w'), KeyModifiers::NONE);
        assert!(!tab.wrap);
        press(&mut tab, KeyCode::Char('w'), KeyModifiers::NONE);
        assert!(tab.wrap);
    }

    #[test]
    fn test_s_toggles_sidebar() {
        let mut tab = make_tab(&["line"]);
        let initial = tab.show_sidebar;
        press(&mut tab, KeyCode::Char('s'), KeyModifiers::NONE);
        assert_eq!(tab.show_sidebar, !initial);
        press(&mut tab, KeyCode::Char('s'), KeyModifiers::NONE);
        assert_eq!(tab.show_sidebar, initial);
    }

    #[test]
    fn test_colon_transitions_to_command_mode() {
        let mut tab = make_tab(&["line"]);
        let (mode, result) = press(&mut tab, KeyCode::Char(':'), KeyModifiers::NONE);
        assert!(matches!(result, KeyResult::Handled));
        assert!(mode.command_state().is_some());
        assert!(mode.needs_input_bar());
    }

    #[test]
    fn test_f_transitions_to_filter_mode() {
        let mut tab = make_tab(&["line"]);
        let (mode, result) = press(&mut tab, KeyCode::Char('f'), KeyModifiers::NONE);
        assert!(matches!(result, KeyResult::Handled));
        assert!(mode.selected_filter_index().is_some());
    }

    #[test]
    fn test_slash_transitions_to_forward_search() {
        let mut tab = make_tab(&["line"]);
        let (mode, _) = press(&mut tab, KeyCode::Char('/'), KeyModifiers::NONE);
        assert!(mode.needs_input_bar());
        let search = mode.search_state();
        assert!(search.is_some());
        assert!(search.unwrap().1);
    }

    #[test]
    fn test_question_mark_transitions_to_backward_search() {
        let mut tab = make_tab(&["line"]);
        let (mode, _) = press(&mut tab, KeyCode::Char('?'), KeyModifiers::NONE);
        let search = mode.search_state();
        assert!(search.is_some());
        assert!(!search.unwrap().1);
    }

    #[test]
    fn test_q_returns_ignored() {
        let mut tab = make_tab(&["line"]);
        let (_, result) = press(&mut tab, KeyCode::Char('q'), KeyModifiers::NONE);
        assert!(matches!(result, KeyResult::Ignored));
    }

    #[test]
    fn test_tab_returns_ignored() {
        let mut tab = make_tab(&["line"]);
        let (_, result) = press(&mut tab, KeyCode::Tab, KeyModifiers::NONE);
        assert!(matches!(result, KeyResult::Ignored));
    }

    #[test]
    fn test_backtab_returns_ignored() {
        let mut tab = make_tab(&["line"]);
        let (_, result) = press(&mut tab, KeyCode::BackTab, KeyModifiers::NONE);
        assert!(matches!(result, KeyResult::Ignored));
    }

    #[test]
    fn test_ctrl_w_returns_ignored() {
        let mut tab = make_tab(&["line"]);
        let (_, result) = press(&mut tab, KeyCode::Char('w'), KeyModifiers::CONTROL);
        assert!(matches!(result, KeyResult::Ignored));
    }

    #[test]
    fn test_ctrl_t_returns_ignored() {
        let mut tab = make_tab(&["line"]);
        let (_, result) = press(&mut tab, KeyCode::Char('t'), KeyModifiers::CONTROL);
        assert!(matches!(result, KeyResult::Ignored));
    }

    #[test]
    fn test_h_decrements_horizontal_scroll_when_not_wrapped() {
        let mut tab = make_tab(&["long line"]);
        tab.wrap = false;
        tab.horizontal_scroll = 5;
        press(&mut tab, KeyCode::Char('h'), KeyModifiers::NONE);
        assert_eq!(tab.horizontal_scroll, 4);
    }

    #[test]
    fn test_l_increments_horizontal_scroll_when_not_wrapped() {
        let mut tab = make_tab(&["long line"]);
        tab.wrap = false;
        press(&mut tab, KeyCode::Char('l'), KeyModifiers::NONE);
        assert_eq!(tab.horizontal_scroll, 1);
    }

    #[test]
    fn test_h_no_horizontal_scroll_when_wrapped() {
        let mut tab = make_tab(&["long line"]);
        tab.wrap = true;
        tab.horizontal_scroll = 5;
        press(&mut tab, KeyCode::Char('h'), KeyModifiers::NONE);
        assert_eq!(tab.horizontal_scroll, 5);
    }

    #[test]
    fn test_l_no_horizontal_scroll_when_wrapped() {
        let mut tab = make_tab(&["long line"]);
        tab.wrap = true;
        press(&mut tab, KeyCode::Char('l'), KeyModifiers::NONE);
        assert_eq!(tab.horizontal_scroll, 0);
    }

    #[test]
    fn test_m_marks_current_line() {
        let mut tab = make_tab(&["line0", "line1"]);
        tab.scroll_offset = 0;
        press(&mut tab, KeyCode::Char('m'), KeyModifiers::NONE);
        assert!(tab.log_manager.get_marked_indices().contains(&0));
    }

    #[test]
    fn test_m_unmarks_already_marked_line() {
        let mut tab = make_tab(&["line0"]);
        tab.scroll_offset = 0;
        press(&mut tab, KeyCode::Char('m'), KeyModifiers::NONE);
        press(&mut tab, KeyCode::Char('m'), KeyModifiers::NONE);
        assert!(!tab.log_manager.get_marked_indices().contains(&0));
    }

    #[test]
    fn test_g_key_resets_on_non_g_press() {
        let mut tab = make_tab(&["a"]);
        press(&mut tab, KeyCode::Char('g'), KeyModifiers::NONE);
        assert!(tab.g_key_pressed);
        press(&mut tab, KeyCode::Char('j'), KeyModifiers::NONE);
        assert!(!tab.g_key_pressed);
    }

    #[test]
    fn test_status_line_contains_normal() {
        assert!(NormalMode.status_line().contains("[NORMAL]"));
    }
}
