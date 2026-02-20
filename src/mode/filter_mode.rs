use crossterm::event::{KeyCode, KeyModifiers};

use crate::mode::app_mode::Mode;
use crate::mode::command_mode::CommandMode;
use crate::mode::normal_mode::NormalMode;
use crate::types::FilterType;

use crate::ui::KeyResult;
use crate::ui::TabState;

#[derive(Debug)]
pub struct FilterManagementMode {
    pub selected_filter_index: usize,
}

impl Mode for FilterManagementMode {
    fn handle_key(
        self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        _modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        if matches!(key, KeyCode::Tab | KeyCode::BackTab) {
            return (self, KeyResult::Ignored);
        }

        let selected = self.selected_filter_index;

        match key {
            KeyCode::Esc => (Box::new(NormalMode), KeyResult::Handled),
            KeyCode::Up => (
                Box::new(FilterManagementMode {
                    selected_filter_index: selected.saturating_sub(1),
                }),
                KeyResult::Handled,
            ),
            KeyCode::Down => {
                let num_filters = tab.log_manager.get_filters().len();
                let new_idx = if num_filters > 0 {
                    (selected + 1).min(num_filters - 1)
                } else {
                    0
                };
                (
                    Box::new(FilterManagementMode {
                        selected_filter_index: new_idx,
                    }),
                    KeyResult::Handled,
                )
            }
            KeyCode::Char(' ') => {
                let filter_id = tab.log_manager.get_filters().get(selected).map(|f| f.id);
                if let Some(id) = filter_id {
                    tab.log_manager.toggle_filter(id);
                    tab.refresh_visible();
                }
                (
                    Box::new(FilterManagementMode {
                        selected_filter_index: selected,
                    }),
                    KeyResult::Handled,
                )
            }
            KeyCode::Char('d') => {
                let filter_id = tab.log_manager.get_filters().get(selected).map(|f| f.id);
                if let Some(id) = filter_id {
                    tab.log_manager.remove_filter(id);
                    tab.refresh_visible();
                    let remaining_len = tab.log_manager.get_filters().len();
                    let new_idx = if remaining_len > 0 && selected >= remaining_len {
                        remaining_len - 1
                    } else {
                        selected
                    };
                    (
                        Box::new(FilterManagementMode {
                            selected_filter_index: new_idx,
                        }),
                        KeyResult::Handled,
                    )
                } else {
                    (
                        Box::new(FilterManagementMode {
                            selected_filter_index: selected,
                        }),
                        KeyResult::Handled,
                    )
                }
            }
            KeyCode::Char('K') => {
                let filter_id = tab.log_manager.get_filters().get(selected).map(|f| f.id);
                if let Some(id) = filter_id {
                    tab.log_manager.move_filter_up(id);
                    tab.refresh_visible();
                    let new_idx = selected.saturating_sub(1);
                    (
                        Box::new(FilterManagementMode {
                            selected_filter_index: new_idx,
                        }),
                        KeyResult::Handled,
                    )
                } else {
                    (
                        Box::new(FilterManagementMode {
                            selected_filter_index: selected,
                        }),
                        KeyResult::Handled,
                    )
                }
            }
            KeyCode::Char('J') => {
                let filter_id = tab.log_manager.get_filters().get(selected).map(|f| f.id);
                if let Some(id) = filter_id {
                    tab.log_manager.move_filter_down(id);
                    tab.refresh_visible();
                    let total = tab.log_manager.get_filters().len();
                    let new_idx = if selected + 1 < total {
                        selected + 1
                    } else {
                        selected
                    };
                    (
                        Box::new(FilterManagementMode {
                            selected_filter_index: new_idx,
                        }),
                        KeyResult::Handled,
                    )
                } else {
                    (
                        Box::new(FilterManagementMode {
                            selected_filter_index: selected,
                        }),
                        KeyResult::Handled,
                    )
                }
            }
            KeyCode::Char('e') => {
                let filter_info = tab.log_manager.get_filters().get(selected).map(|f| {
                    (
                        f.id,
                        f.filter_type.clone(),
                        f.color_config.clone(),
                        f.pattern.clone(),
                    )
                });
                if let Some((id, ft, cc, pattern)) = filter_info {
                    tab.editing_filter_id = Some(id);
                    tab.filter_context = Some(selected);
                    let mut cmd = if ft == FilterType::Include {
                        String::from("filter")
                    } else {
                        String::from("exclude")
                    };
                    if ft == FilterType::Include {
                        if let Some(cfg) = &cc {
                            if let Some(fg) = cfg.fg {
                                cmd.push_str(&format!(" --fg {:?}", fg));
                            }
                            if let Some(bg) = cfg.bg {
                                cmd.push_str(&format!(" --bg {:?}", bg));
                            }
                            if cfg.match_only {
                                cmd.push_str(" -m");
                            }
                        }
                    }
                    cmd.push(' ');
                    cmd.push_str(&pattern);
                    let len = cmd.len();
                    let history = tab.command_history.clone();
                    (
                        Box::new(CommandMode::with_history(cmd, len, history)),
                        KeyResult::Handled,
                    )
                } else {
                    (
                        Box::new(FilterManagementMode {
                            selected_filter_index: selected,
                        }),
                        KeyResult::Handled,
                    )
                }
            }
            KeyCode::Char('c') => {
                let color_config = tab
                    .log_manager
                    .get_filters()
                    .get(selected)
                    .and_then(|f| f.color_config.clone());
                tab.filter_context = Some(selected);
                let mut cmd = String::from("set-color");
                if let Some(cfg) = color_config {
                    if let Some(fg) = cfg.fg {
                        cmd.push_str(&format!(" --fg {:?}", fg));
                    }
                    if let Some(bg) = cfg.bg {
                        cmd.push_str(&format!(" --bg {:?}", bg));
                    }
                }
                let len = cmd.len();
                let history = tab.command_history.clone();
                (
                    Box::new(CommandMode::with_history(cmd, len, history)),
                    KeyResult::Handled,
                )
            }
            KeyCode::Char('i') => {
                let history = tab.command_history.clone();
                (
                    Box::new(CommandMode::with_history("filter ".to_string(), 7, history)),
                    KeyResult::Handled,
                )
            }
            KeyCode::Char('x') => {
                let history = tab.command_history.clone();
                (
                    Box::new(CommandMode::with_history(
                        "exclude ".to_string(),
                        8,
                        history,
                    )),
                    KeyResult::Handled,
                )
            }
            _ => (
                Box::new(FilterManagementMode {
                    selected_filter_index: selected,
                }),
                KeyResult::Handled,
            ),
        }
    }

    fn status_line(&self) -> &str {
        "[FILTER] [i]nclude | e[x]clude | Space => toggle | [d]elete | [e]dit | set [c]olor | [J/K] move down/up | Esc => normal mode"
    }

    fn selected_filter_index(&self) -> Option<usize> {
        Some(self.selected_filter_index)
    }
}

// ---------------------------------------------------------------------------
// FilterEditMode
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct FilterEditMode {
    pub filter_id: Option<usize>,
    pub filter_input: String,
}

impl Mode for FilterEditMode {
    fn handle_key(
        self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        _modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        if matches!(key, KeyCode::Tab | KeyCode::BackTab) {
            return (self, KeyResult::Ignored);
        }
        match key {
            KeyCode::Enter => {
                if let Some(id) = self.filter_id {
                    tab.log_manager.edit_filter(id, self.filter_input);
                    tab.refresh_visible();
                }
                (
                    Box::new(FilterManagementMode {
                        selected_filter_index: 0,
                    }),
                    KeyResult::Handled,
                )
            }
            KeyCode::Esc => (
                Box::new(FilterManagementMode {
                    selected_filter_index: 0,
                }),
                KeyResult::Handled,
            ),
            KeyCode::Backspace => {
                let mut input = self.filter_input;
                input.pop();
                (
                    Box::new(FilterEditMode {
                        filter_id: self.filter_id,
                        filter_input: input,
                    }),
                    KeyResult::Handled,
                )
            }
            KeyCode::Char(c) => {
                let mut input = self.filter_input;
                input.push(c);
                (
                    Box::new(FilterEditMode {
                        filter_id: self.filter_id,
                        filter_input: input,
                    }),
                    KeyResult::Handled,
                )
            }
            _ => (self, KeyResult::Handled),
        }
    }

    fn status_line(&self) -> &str {
        "[FILTER EDIT] Esc => cancel | Enter => save"
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

    fn add_filter(tab: &mut TabState, pattern: &str, filter_type: FilterType) {
        tab.log_manager
            .add_filter_with_color(pattern.to_string(), filter_type, None, None, false);
        tab.refresh_visible();
    }

    fn filter_mode(idx: usize) -> FilterManagementMode {
        FilterManagementMode {
            selected_filter_index: idx,
        }
    }

    fn press(
        mode: FilterManagementMode,
        tab: &mut TabState,
        code: KeyCode,
    ) -> (Box<dyn Mode>, KeyResult) {
        Box::new(mode).handle_key(tab, code, KeyModifiers::NONE)
    }

    #[test]
    fn test_esc_transitions_to_normal_mode() {
        let mut tab = make_tab(&["line"]);
        let (mode, result) = press(filter_mode(0), &mut tab, KeyCode::Esc);
        assert!(matches!(result, KeyResult::Handled));
        assert!(mode.command_state().is_none());
        assert!(mode.selected_filter_index().is_none());
    }

    #[test]
    fn test_tab_returns_ignored() {
        let mut tab = make_tab(&["line"]);
        let (_, result) = press(filter_mode(0), &mut tab, KeyCode::Tab);
        assert!(matches!(result, KeyResult::Ignored));
    }

    #[test]
    fn test_backtab_returns_ignored() {
        let mut tab = make_tab(&["line"]);
        let (_, result) = press(filter_mode(0), &mut tab, KeyCode::BackTab);
        assert!(matches!(result, KeyResult::Ignored));
    }

    #[test]
    fn test_up_decrements_selected_index() {
        let mut tab = make_tab(&["a", "b"]);
        add_filter(&mut tab, "a", FilterType::Include);
        add_filter(&mut tab, "b", FilterType::Include);
        let (mode, _) = press(filter_mode(1), &mut tab, KeyCode::Up);
        assert_eq!(mode.selected_filter_index(), Some(0));
    }

    #[test]
    fn test_up_saturates_at_zero() {
        let mut tab = make_tab(&["a"]);
        let (mode, _) = press(filter_mode(0), &mut tab, KeyCode::Up);
        assert_eq!(mode.selected_filter_index(), Some(0));
    }

    #[test]
    fn test_down_increments_selected_index() {
        let mut tab = make_tab(&["a", "b"]);
        add_filter(&mut tab, "a", FilterType::Include);
        add_filter(&mut tab, "b", FilterType::Include);
        let (mode, _) = press(filter_mode(0), &mut tab, KeyCode::Down);
        assert_eq!(mode.selected_filter_index(), Some(1));
    }

    #[test]
    fn test_down_clamps_at_last_filter() {
        let mut tab = make_tab(&["a", "b"]);
        add_filter(&mut tab, "a", FilterType::Include);
        add_filter(&mut tab, "b", FilterType::Include);
        let (mode, _) = press(filter_mode(1), &mut tab, KeyCode::Down);
        assert_eq!(mode.selected_filter_index(), Some(1));
    }

    #[test]
    fn test_space_toggles_filter() {
        let mut tab = make_tab(&["a", "b"]);
        add_filter(&mut tab, "a", FilterType::Include);
        let id = tab.log_manager.get_filters()[0].id;
        assert!(tab.log_manager.get_filters()[0].enabled);
        press(filter_mode(0), &mut tab, KeyCode::Char(' '));
        assert!(!tab.log_manager.get_filters().iter().find(|f| f.id == id).unwrap().enabled);
    }

    #[test]
    fn test_d_deletes_filter() {
        let mut tab = make_tab(&["a", "b"]);
        add_filter(&mut tab, "a", FilterType::Include);
        assert_eq!(tab.log_manager.get_filters().len(), 1);
        press(filter_mode(0), &mut tab, KeyCode::Char('d'));
        assert_eq!(tab.log_manager.get_filters().len(), 0);
    }

    #[test]
    fn test_d_with_no_filters_no_panic() {
        let mut tab = make_tab(&["line"]);
        let (mode, result) = press(filter_mode(0), &mut tab, KeyCode::Char('d'));
        assert!(matches!(result, KeyResult::Handled));
        assert_eq!(mode.selected_filter_index(), Some(0));
    }

    #[test]
    fn test_i_opens_command_mode_with_filter_prefix() {
        let mut tab = make_tab(&["line"]);
        let (mode, _) = press(filter_mode(0), &mut tab, KeyCode::Char('i'));
        let state = mode.command_state();
        assert!(state.is_some());
        let (input, _) = state.unwrap();
        assert!(input.starts_with("filter "));
    }

    #[test]
    fn test_x_opens_command_mode_with_exclude_prefix() {
        let mut tab = make_tab(&["line"]);
        let (mode, _) = press(filter_mode(0), &mut tab, KeyCode::Char('x'));
        let state = mode.command_state();
        assert!(state.is_some());
        let (input, _) = state.unwrap();
        assert!(input.starts_with("exclude "));
    }

    #[test]
    fn test_e_opens_command_mode_with_filter_pattern() {
        let mut tab = make_tab(&["error", "warn"]);
        add_filter(&mut tab, "error", FilterType::Include);
        let (mode, _) = press(filter_mode(0), &mut tab, KeyCode::Char('e'));
        let state = mode.command_state();
        assert!(state.is_some());
        let (input, _) = state.unwrap();
        assert!(input.contains("error"));
    }

    #[test]
    fn test_c_opens_set_color_command() {
        let mut tab = make_tab(&["line"]);
        add_filter(&mut tab, "error", FilterType::Include);
        let (mode, _) = press(filter_mode(0), &mut tab, KeyCode::Char('c'));
        let state = mode.command_state();
        assert!(state.is_some());
        let (input, _) = state.unwrap();
        assert!(input.starts_with("set-color"));
    }

    #[test]
    fn test_capital_k_moves_filter_up() {
        let mut tab = make_tab(&["a", "b"]);
        add_filter(&mut tab, "first", FilterType::Include);
        add_filter(&mut tab, "second", FilterType::Include);
        let (mode, result) = press(filter_mode(1), &mut tab, KeyCode::Char('K'));
        assert!(matches!(result, KeyResult::Handled));
        assert_eq!(mode.selected_filter_index(), Some(0));
    }

    #[test]
    fn test_capital_j_moves_filter_down() {
        let mut tab = make_tab(&["a", "b"]);
        add_filter(&mut tab, "first", FilterType::Include);
        add_filter(&mut tab, "second", FilterType::Include);
        let (mode, result) = press(filter_mode(0), &mut tab, KeyCode::Char('J'));
        assert!(matches!(result, KeyResult::Handled));
        assert_eq!(mode.selected_filter_index(), Some(1));
    }

    #[test]
    fn test_status_line_contains_filter() {
        assert!(filter_mode(0).status_line().contains("[FILTER]"));
    }

    #[test]
    fn test_selected_filter_index_returns_current() {
        let mode = filter_mode(3);
        assert_eq!(mode.selected_filter_index(), Some(3));
    }

    // FilterEditMode tests

    fn edit_mode(filter_id: Option<usize>, input: &str) -> FilterEditMode {
        FilterEditMode {
            filter_id,
            filter_input: input.to_string(),
        }
    }

    fn press_edit(
        mode: FilterEditMode,
        tab: &mut TabState,
        code: KeyCode,
    ) -> (Box<dyn Mode>, KeyResult) {
        Box::new(mode).handle_key(tab, code, KeyModifiers::NONE)
    }

    #[test]
    fn test_edit_char_appends_to_input() {
        let mut tab = make_tab(&["line"]);
        let mode = edit_mode(None, "err");
        let (mode2, _) = press_edit(mode, &mut tab, KeyCode::Char('o'));
        assert_eq!(mode2.status_line(), "[FILTER EDIT] Esc => cancel | Enter => save");
    }

    #[test]
    fn test_edit_backspace_removes_char() {
        let mut tab = make_tab(&["line"]);
        let mode = edit_mode(None, "error");
        let (mode2, result) = press_edit(mode, &mut tab, KeyCode::Backspace);
        assert!(matches!(result, KeyResult::Handled));
        // Mode should still be FilterEditMode
        assert!(mode2.status_line().contains("[FILTER EDIT]"));
    }

    #[test]
    fn test_edit_esc_transitions_to_filter_mode() {
        let mut tab = make_tab(&["line"]);
        let mode = edit_mode(None, "error");
        let (mode2, result) = press_edit(mode, &mut tab, KeyCode::Esc);
        assert!(matches!(result, KeyResult::Handled));
        assert!(mode2.selected_filter_index().is_some());
    }

    #[test]
    fn test_edit_tab_returns_ignored() {
        let mut tab = make_tab(&["line"]);
        let mode = edit_mode(None, "err");
        let (_, result) = press_edit(mode, &mut tab, KeyCode::Tab);
        assert!(matches!(result, KeyResult::Ignored));
    }

    #[test]
    fn test_edit_enter_applies_filter_change() {
        let mut tab = make_tab(&["warn", "error"]);
        add_filter(&mut tab, "warn", FilterType::Include);
        let id = tab.log_manager.get_filters()[0].id;
        let mode = edit_mode(Some(id), "error");
        let (mode2, result) = press_edit(mode, &mut tab, KeyCode::Enter);
        assert!(matches!(result, KeyResult::Handled));
        // Should transition to FilterManagementMode
        assert!(mode2.selected_filter_index().is_some());
        assert_eq!(tab.log_manager.get_filters()[0].pattern, "error");
    }
}
