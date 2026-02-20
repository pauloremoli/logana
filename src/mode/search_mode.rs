use crossterm::event::{KeyCode, KeyModifiers};

use crate::{
    mode::{app_mode::Mode, normal_mode::NormalMode},
    ui::{KeyResult, TabState},
};

#[derive(Debug)]
pub struct SearchMode {
    pub input: String,
    pub forward: bool,
}

impl Mode for SearchMode {
    fn handle_key(
        mut self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        _modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        if matches!(key, KeyCode::Tab | KeyCode::BackTab) {
            return (self, KeyResult::Ignored);
        }
        match key {
            KeyCode::Enter => {
                let visible = tab.visible_indices.clone();
                let _ = tab.search.search(&self.input, &visible, &tab.file_reader);
                let result = if self.forward {
                    tab.search.next_match()
                } else {
                    tab.search.previous_match()
                };
                if let Some(r) = result {
                    let line_idx = r.line_idx;
                    tab.scroll_to_line_idx(line_idx);
                }
                (Box::new(NormalMode), KeyResult::Handled)
            }
            KeyCode::Esc => {
                self.input.clear();
                (Box::new(NormalMode), KeyResult::Handled)
            }
            KeyCode::Backspace => {
                self.input.pop();
                (self, KeyResult::Handled)
            }
            KeyCode::Char(c) => {
                self.input.push(c);
                (self, KeyResult::Handled)
            }
            _ => (self, KeyResult::Handled),
        }
    }

    fn status_line(&self) -> &str {
        "[SEARCH] Esc => cancel | Enter => search"
    }

    fn search_state(&self) -> Option<(&str, bool)> {
        Some((&self.input, self.forward))
    }

    fn needs_input_bar(&self) -> bool {
        true
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

    fn press(mode: SearchMode, tab: &mut TabState, code: KeyCode) -> (Box<dyn Mode>, KeyResult) {
        Box::new(mode).handle_key(tab, code, KeyModifiers::NONE)
    }

    #[test]
    fn test_char_appends_to_input() {
        let mut tab = make_tab(&["line"]);
        let (mode, result) = press(forward_mode(""), &mut tab, KeyCode::Char('e'));
        assert!(matches!(result, KeyResult::Handled));
        let state = mode.search_state().unwrap();
        assert_eq!(state.0, "e");
    }

    #[test]
    fn test_multiple_chars_build_query() {
        let mut tab = make_tab(&["line"]);
        let mode = forward_mode("err");
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('o'));
        let state = mode2.search_state().unwrap();
        assert_eq!(state.0, "erro");
    }

    #[test]
    fn test_backspace_removes_last_char() {
        let mut tab = make_tab(&["line"]);
        let (mode2, result) = press(forward_mode("error"), &mut tab, KeyCode::Backspace);
        assert!(matches!(result, KeyResult::Handled));
        let state = mode2.search_state().unwrap();
        assert_eq!(state.0, "erro");
    }

    #[test]
    fn test_backspace_on_empty_no_panic() {
        let mut tab = make_tab(&["line"]);
        let (mode2, _) = press(forward_mode(""), &mut tab, KeyCode::Backspace);
        let state = mode2.search_state().unwrap();
        assert_eq!(state.0, "");
    }

    #[test]
    fn test_esc_clears_input_and_returns_normal_mode() {
        let mut tab = make_tab(&["line"]);
        let (mode2, result) = press(forward_mode("error"), &mut tab, KeyCode::Esc);
        assert!(matches!(result, KeyResult::Handled));
        assert!(mode2.search_state().is_none());
        assert!(mode2.command_state().is_none());
    }

    #[test]
    fn test_tab_returns_ignored() {
        let mut tab = make_tab(&["line"]);
        let (_, result) = press(forward_mode(""), &mut tab, KeyCode::Tab);
        assert!(matches!(result, KeyResult::Ignored));
    }

    #[test]
    fn test_backtab_returns_ignored() {
        let mut tab = make_tab(&["line"]);
        let (_, result) = press(forward_mode(""), &mut tab, KeyCode::BackTab);
        assert!(matches!(result, KeyResult::Ignored));
    }

    #[test]
    fn test_enter_executes_forward_search_and_returns_normal_mode() {
        let mut tab = make_tab(&["error: file not found", "warn: low memory", "error: timeout"]);
        let (mode2, result) = press(forward_mode("error"), &mut tab, KeyCode::Enter);
        assert!(matches!(result, KeyResult::Handled));
        assert!(mode2.search_state().is_none());
    }

    #[test]
    fn test_enter_with_no_match_still_returns_normal_mode() {
        let mut tab = make_tab(&["info: all good", "warn: minor issue"]);
        let (mode2, result) = press(forward_mode("critical"), &mut tab, KeyCode::Enter);
        assert!(matches!(result, KeyResult::Handled));
        assert!(mode2.search_state().is_none());
    }

    #[test]
    fn test_enter_scrolls_to_matching_line() {
        let mut tab = make_tab(&["line0", "line1", "error here", "line3"]);
        tab.visible_height = 10;
        press(forward_mode("error"), &mut tab, KeyCode::Enter);
        assert_eq!(tab.scroll_offset, 2);
    }

    #[test]
    fn test_search_state_forward_true() {
        let mode = forward_mode("test");
        let state = mode.search_state().unwrap();
        assert_eq!(state.0, "test");
        assert!(state.1);
    }

    #[test]
    fn test_search_state_forward_false() {
        let mode = backward_mode("warn");
        let state = mode.search_state().unwrap();
        assert_eq!(state.0, "warn");
        assert!(!state.1);
    }

    #[test]
    fn test_needs_input_bar() {
        assert!(forward_mode("").needs_input_bar());
    }

    #[test]
    fn test_status_line_contains_search() {
        assert!(forward_mode("").status_line().contains("[SEARCH]"));
    }
}
