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
