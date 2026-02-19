use crossterm::event::{KeyCode, KeyModifiers};

use crate::{
    db::FileContext,
    mode::normal_mode::NormalMode,
    ui::{KeyResult, TabState},
};

pub trait Mode: std::fmt::Debug {
    fn handle_key(
        self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult);

    fn status_line(&self) -> &str;

    fn selected_filter_index(&self) -> Option<usize> {
        None
    }
    fn command_state(&self) -> Option<(&str, usize)> {
        None
    }
    fn search_state(&self) -> Option<(&str, bool)> {
        None
    }
    fn needs_input_bar(&self) -> bool {
        false
    }
    fn confirm_restore_context(&self) -> Option<&FileContext> {
        None
    }
}

#[derive(Debug)]
pub struct ConfirmRestoreMode {
    pub context: FileContext,
}

impl Mode for ConfirmRestoreMode {
    fn handle_key(
        self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        _modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        match key {
            KeyCode::Char('y') => {
                tab.apply_file_context(&self.context);
                (Box::new(NormalMode), KeyResult::Handled)
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                tab.log_manager.clear_filters();
                tab.log_manager.set_marks(vec![]);
                tab.refresh_visible();
                (Box::new(NormalMode), KeyResult::Handled)
            }
            _ => (self, KeyResult::Handled),
        }
    }

    fn status_line(&self) -> &str {
        "[RESTORE] Restore previous session? [y]es / [n]o"
    }

    fn confirm_restore_context(&self) -> Option<&FileContext> {
        Some(&self.context)
    }
}
