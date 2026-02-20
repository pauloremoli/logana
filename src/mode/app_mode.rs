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
    fn confirm_restore_session_files(&self) -> Option<&[String]> {
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

// ---------------------------------------------------------------------------
// ConfirmRestoreSessionMode
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct ConfirmRestoreSessionMode {
    pub files: Vec<String>,
}

impl Mode for ConfirmRestoreSessionMode {
    fn handle_key(
        self: Box<Self>,
        _tab: &mut TabState,
        key: KeyCode,
        _modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        match key {
            KeyCode::Char('y') => (Box::new(NormalMode), KeyResult::RestoreSession(self.files)),
            KeyCode::Char('n') | KeyCode::Esc => (Box::new(NormalMode), KeyResult::Handled),
            _ => (self, KeyResult::Handled),
        }
    }

    fn status_line(&self) -> &str {
        "[RESTORE SESSION] Restore last session? [y]es / [n]o"
    }

    fn confirm_restore_session_files(&self) -> Option<&[String]> {
        Some(&self.files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::file_reader::FileReader;
    use crate::log_manager::LogManager;
    use crate::types::FilterType;
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

    fn default_context() -> FileContext {
        FileContext {
            source_file: "/tmp/test.log".to_string(),
            scroll_offset: 5,
            search_query: String::new(),
            wrap: false,
            level_colors: false,
            show_sidebar: false,
            horizontal_scroll: 3,
            marked_lines: vec![],
            file_hash: None,
            show_line_numbers: false,
        }
    }

    fn press_restore(
        mode: ConfirmRestoreMode,
        tab: &mut TabState,
        code: KeyCode,
    ) -> (Box<dyn Mode>, KeyResult) {
        Box::new(mode).handle_key(tab, code, KeyModifiers::NONE)
    }

    fn press_session(
        mode: ConfirmRestoreSessionMode,
        tab: &mut TabState,
        code: KeyCode,
    ) -> (Box<dyn Mode>, KeyResult) {
        Box::new(mode).handle_key(tab, code, KeyModifiers::NONE)
    }

    // ── ConfirmRestoreMode ───────────────────────────────────────────────────

    #[test]
    fn test_confirm_restore_y_applies_context() {
        let mut tab = make_tab(&["line0", "line1"]);
        let ctx = default_context();
        let mode = ConfirmRestoreMode { context: ctx };
        let (mode2, result) = press_restore(mode, &mut tab, KeyCode::Char('y'));
        assert!(matches!(result, KeyResult::Handled));
        // Should transition to NormalMode
        assert!(mode2.confirm_restore_context().is_none());
        // Context should have been applied
        assert_eq!(tab.scroll_offset, 5);
        assert!(!tab.wrap);
        assert!(!tab.show_sidebar);
        assert!(!tab.level_colors);
        assert!(!tab.show_line_numbers);
        assert_eq!(tab.horizontal_scroll, 3);
    }

    #[test]
    fn test_confirm_restore_n_clears_filters_and_returns_normal() {
        let mut tab = make_tab(&["error", "warn"]);
        tab.log_manager
            .add_filter_with_color("error".to_string(), FilterType::Include, None, None, false);
        tab.refresh_visible();
        assert_eq!(tab.log_manager.get_filters().len(), 1);

        let mode = ConfirmRestoreMode {
            context: default_context(),
        };
        let (mode2, result) = press_restore(mode, &mut tab, KeyCode::Char('n'));
        assert!(matches!(result, KeyResult::Handled));
        assert!(mode2.confirm_restore_context().is_none());
        assert_eq!(tab.log_manager.get_filters().len(), 0);
    }

    #[test]
    fn test_confirm_restore_esc_clears_filters_and_returns_normal() {
        let mut tab = make_tab(&["line"]);
        tab.log_manager
            .add_filter_with_color("line".to_string(), FilterType::Include, None, None, false);
        tab.refresh_visible();

        let mode = ConfirmRestoreMode {
            context: default_context(),
        };
        let (mode2, result) = press_restore(mode, &mut tab, KeyCode::Esc);
        assert!(matches!(result, KeyResult::Handled));
        assert!(mode2.confirm_restore_context().is_none());
        assert_eq!(tab.log_manager.get_filters().len(), 0);
    }

    #[test]
    fn test_confirm_restore_other_key_stays_in_mode() {
        let mut tab = make_tab(&["line"]);
        let ctx = default_context();
        let mode = ConfirmRestoreMode { context: ctx };
        let (mode2, result) = press_restore(mode, &mut tab, KeyCode::Char('x'));
        assert!(matches!(result, KeyResult::Handled));
        assert!(mode2.confirm_restore_context().is_some());
    }

    #[test]
    fn test_confirm_restore_status_line() {
        let mode = ConfirmRestoreMode {
            context: default_context(),
        };
        assert!(mode.status_line().contains("[RESTORE]"));
    }

    #[test]
    fn test_confirm_restore_context_method() {
        let ctx = default_context();
        let mode = ConfirmRestoreMode { context: ctx.clone() };
        let returned = mode.confirm_restore_context().unwrap();
        assert_eq!(returned.source_file, ctx.source_file);
        assert_eq!(returned.scroll_offset, ctx.scroll_offset);
    }

    // ── ConfirmRestoreSessionMode ────────────────────────────────────────────

    #[test]
    fn test_confirm_session_y_returns_restore_session() {
        let mut tab = make_tab(&["line"]);
        let files = vec!["/var/log/a.log".to_string(), "/var/log/b.log".to_string()];
        let mode = ConfirmRestoreSessionMode { files: files.clone() };
        let (mode2, result) = press_session(mode, &mut tab, KeyCode::Char('y'));
        assert!(matches!(result, KeyResult::RestoreSession(ref f) if *f == files));
        assert!(mode2.confirm_restore_session_files().is_none());
    }

    #[test]
    fn test_confirm_session_n_returns_normal_mode() {
        let mut tab = make_tab(&["line"]);
        let mode = ConfirmRestoreSessionMode {
            files: vec!["file.log".to_string()],
        };
        let (mode2, result) = press_session(mode, &mut tab, KeyCode::Char('n'));
        assert!(matches!(result, KeyResult::Handled));
        assert!(mode2.confirm_restore_session_files().is_none());
    }

    #[test]
    fn test_confirm_session_esc_returns_normal_mode() {
        let mut tab = make_tab(&["line"]);
        let mode = ConfirmRestoreSessionMode {
            files: vec!["file.log".to_string()],
        };
        let (mode2, result) = press_session(mode, &mut tab, KeyCode::Esc);
        assert!(matches!(result, KeyResult::Handled));
        assert!(mode2.confirm_restore_session_files().is_none());
    }

    #[test]
    fn test_confirm_session_other_key_stays_in_mode() {
        let mut tab = make_tab(&["line"]);
        let files = vec!["file.log".to_string()];
        let mode = ConfirmRestoreSessionMode { files };
        let (mode2, result) = press_session(mode, &mut tab, KeyCode::Char('z'));
        assert!(matches!(result, KeyResult::Handled));
        assert!(mode2.confirm_restore_session_files().is_some());
    }

    #[test]
    fn test_confirm_session_status_line() {
        let mode = ConfirmRestoreSessionMode {
            files: vec!["file.log".to_string()],
        };
        assert!(mode.status_line().contains("[RESTORE SESSION]"));
    }

    #[test]
    fn test_confirm_session_files_method() {
        let files = vec!["a.log".to_string(), "b.log".to_string()];
        let mode = ConfirmRestoreSessionMode { files: files.clone() };
        assert_eq!(mode.confirm_restore_session_files(), Some(files.as_slice()));
    }

    // ── Mode trait defaults ──────────────────────────────────────────────────

    #[test]
    fn test_confirm_restore_mode_default_methods() {
        let mode = ConfirmRestoreMode {
            context: default_context(),
        };
        assert!(mode.selected_filter_index().is_none());
        assert!(mode.command_state().is_none());
        assert!(mode.search_state().is_none());
        assert!(!mode.needs_input_bar());
        assert!(mode.confirm_restore_session_files().is_none());
    }

    #[test]
    fn test_confirm_session_mode_default_methods() {
        let mode = ConfirmRestoreSessionMode {
            files: vec![],
        };
        assert!(mode.selected_filter_index().is_none());
        assert!(mode.command_state().is_none());
        assert!(mode.search_state().is_none());
        assert!(!mode.needs_input_bar());
        assert!(mode.confirm_restore_context().is_none());
    }
}
