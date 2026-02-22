use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::{
    config::Keybindings,
    db::FileContext,
    mode::{normal_mode::NormalMode, value_colors_mode::ValueColorGroup},
    theme::Theme,
    types::DockerContainer,
    ui::{KeyResult, TabState},
};

/// Rendering state returned by each mode via `Mode::render_state()`.
///
/// Each variant carries exactly the data the rendering layer needs for that mode,
/// satisfying the Interface Segregation Principle: modes only expose what they own.
#[derive(Debug, Clone)]
pub enum ModeRenderState {
    Normal,
    Command {
        input: String,
        cursor: usize,
        completion_index: Option<usize>,
    },
    Search {
        query: String,
        forward: bool,
    },
    FilterManagement {
        selected_index: usize,
    },
    FilterEdit,
    VisualLine {
        anchor: usize,
    },
    Comment {
        lines: Vec<String>,
        cursor_row: usize,
        cursor_col: usize,
        line_count: usize,
    },
    KeybindingsHelp {
        scroll: usize,
        search: String,
    },
    SelectFields {
        fields: Vec<(String, bool)>,
        selected: usize,
    },
    DockerSelect {
        containers: Vec<DockerContainer>,
        selected: usize,
        error: Option<String>,
    },
    ValueColors {
        groups: Vec<ValueColorGroup>,
        search: String,
        selected: usize,
    },
    ConfirmRestore,
    ConfirmRestoreSession {
        files: Vec<String>,
    },
}

#[async_trait]
pub trait Mode: std::fmt::Debug + Send {
    async fn handle_key(
        self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult);

    fn status_line(&self) -> &str;

    /// Like `status_line` but returns a styled `Line` with `<KEY> action` spans.
    /// Default implementation wraps the static status string in theme text color.
    fn dynamic_status_line(&self, _kb: &Keybindings, theme: &Theme) -> Line<'static> {
        Line::from(Span::styled(
            self.status_line().to_string(),
            Style::default().fg(theme.text),
        ))
    }

    /// Returns the rendering state for this mode.
    ///
    /// The rendering layer matches on the returned enum variant to decide which
    /// UI elements to draw. Each variant carries exactly the data its renderer
    /// needs — no more, no less.
    fn render_state(&self) -> ModeRenderState;
}

/// Appends a styled `<key> action  ` entry to `spans`.
/// Used by mode implementations to build the status bar line.
pub fn status_entry(
    spans: &mut Vec<Span<'static>>,
    key: String,
    action: &'static str,
    theme: &Theme,
) {
    spans.push(Span::styled("<", Style::default().fg(theme.border)));
    spans.push(Span::styled(
        key,
        Style::default()
            .fg(theme.text_highlight)
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(">", Style::default().fg(theme.border)));
    spans.push(Span::styled(
        format!(" {}  ", action),
        Style::default().fg(theme.text),
    ));
}

#[derive(Debug)]
pub struct ConfirmRestoreMode {
    pub context: FileContext,
}

#[async_trait]
impl Mode for ConfirmRestoreMode {
    async fn handle_key(
        self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        let kb = &tab.keybindings.confirm;
        if kb.yes.matches(key, modifiers) {
            tab.apply_file_context(&self.context);
            (Box::new(NormalMode), KeyResult::Handled)
        } else if kb.no.matches(key, modifiers) {
            tab.log_manager.clear_filters().await;
            tab.log_manager.set_marks(vec![]);
            tab.log_manager.set_comments(vec![]);
            tab.refresh_visible();
            (Box::new(NormalMode), KeyResult::Handled)
        } else {
            (self, KeyResult::Handled)
        }
    }

    fn status_line(&self) -> &str {
        "[RESTORE] Restore previous session?  <y> yes  <n> no"
    }

    fn dynamic_status_line(&self, kb: &Keybindings, theme: &Theme) -> Line<'static> {
        let mut spans: Vec<Span<'static>> = vec![Span::styled(
            "[RESTORE]  ",
            Style::default()
                .fg(theme.text_highlight)
                .add_modifier(Modifier::BOLD),
        )];
        spans.push(Span::styled(
            "Restore previous session?  ",
            Style::default().fg(theme.text),
        ));
        status_entry(&mut spans, kb.confirm.yes.display(), "yes", theme);
        status_entry(&mut spans, kb.confirm.no.display(), "no", theme);
        Line::from(spans)
    }

    fn render_state(&self) -> ModeRenderState {
        ModeRenderState::ConfirmRestore
    }
}

// ---------------------------------------------------------------------------
// ConfirmRestoreSessionMode
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct ConfirmRestoreSessionMode {
    pub files: Vec<String>,
}

#[async_trait]
impl Mode for ConfirmRestoreSessionMode {
    async fn handle_key(
        self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        let kb = &tab.keybindings.confirm;
        if kb.yes.matches(key, modifiers) {
            (Box::new(NormalMode), KeyResult::RestoreSession(self.files))
        } else if kb.no.matches(key, modifiers) {
            (Box::new(NormalMode), KeyResult::Handled)
        } else {
            (self, KeyResult::Handled)
        }
    }

    fn status_line(&self) -> &str {
        "[RESTORE SESSION] Restore last session?  <y> yes  <n> no"
    }

    fn dynamic_status_line(&self, kb: &Keybindings, theme: &Theme) -> Line<'static> {
        let mut spans: Vec<Span<'static>> = vec![Span::styled(
            "[RESTORE SESSION]  ",
            Style::default()
                .fg(theme.text_highlight)
                .add_modifier(Modifier::BOLD),
        )];
        spans.push(Span::styled(
            "Restore last session?  ",
            Style::default().fg(theme.text),
        ));
        status_entry(&mut spans, kb.confirm.yes.display(), "yes", theme);
        status_entry(&mut spans, kb.confirm.no.display(), "no", theme);
        Line::from(spans)
    }

    fn render_state(&self) -> ModeRenderState {
        ModeRenderState::ConfirmRestoreSession {
            files: self.files.clone(),
        }
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

    async fn make_tab(lines: &[&str]) -> TabState {
        let data = lines.join("\n").into_bytes();
        let file_reader = FileReader::from_bytes(data);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
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
            comments: vec![],
        }
    }

    async fn press_restore(
        mode: ConfirmRestoreMode,
        tab: &mut TabState,
        code: KeyCode,
    ) -> (Box<dyn Mode>, KeyResult) {
        Box::new(mode)
            .handle_key(tab, code, KeyModifiers::NONE)
            .await
    }

    async fn press_session(
        mode: ConfirmRestoreSessionMode,
        tab: &mut TabState,
        code: KeyCode,
    ) -> (Box<dyn Mode>, KeyResult) {
        Box::new(mode)
            .handle_key(tab, code, KeyModifiers::NONE)
            .await
    }

    // ── ConfirmRestoreMode ───────────────────────────────────────────────────

    #[tokio::test]
    async fn test_confirm_restore_y_applies_context() {
        let mut tab = make_tab(&["line0", "line1"]).await;
        let ctx = default_context();
        let mode = ConfirmRestoreMode { context: ctx };
        let (mode2, result) = press_restore(mode, &mut tab, KeyCode::Char('y')).await;
        assert!(matches!(result, KeyResult::Handled));
        // Should transition to NormalMode
        assert!(!matches!(
            mode2.render_state(),
            ModeRenderState::ConfirmRestore
        ));
        // Context should have been applied
        assert_eq!(tab.scroll_offset, 5);
        assert!(!tab.wrap);
        assert!(!tab.show_sidebar);
        assert!(!tab.level_colors);
        assert!(!tab.show_line_numbers);
        assert_eq!(tab.horizontal_scroll, 3);
    }

    #[tokio::test]
    async fn test_confirm_restore_n_clears_filters_and_returns_normal() {
        let mut tab = make_tab(&["error", "warn"]).await;
        tab.log_manager
            .add_filter_with_color("error".to_string(), FilterType::Include, None, None, false)
            .await;
        tab.refresh_visible();
        assert_eq!(tab.log_manager.get_filters().len(), 1);

        let mode = ConfirmRestoreMode {
            context: default_context(),
        };
        let (mode2, result) = press_restore(mode, &mut tab, KeyCode::Char('n')).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(!matches!(
            mode2.render_state(),
            ModeRenderState::ConfirmRestore
        ));
        assert_eq!(tab.log_manager.get_filters().len(), 0);
    }

    #[tokio::test]
    async fn test_confirm_restore_esc_clears_filters_and_returns_normal() {
        let mut tab = make_tab(&["line"]).await;
        tab.log_manager
            .add_filter_with_color("line".to_string(), FilterType::Include, None, None, false)
            .await;
        tab.refresh_visible();

        let mode = ConfirmRestoreMode {
            context: default_context(),
        };
        let (mode2, result) = press_restore(mode, &mut tab, KeyCode::Esc).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(!matches!(
            mode2.render_state(),
            ModeRenderState::ConfirmRestore
        ));
        assert_eq!(tab.log_manager.get_filters().len(), 0);
    }

    #[tokio::test]
    async fn test_confirm_restore_other_key_stays_in_mode() {
        let mut tab = make_tab(&["line"]).await;
        let ctx = default_context();
        let mode = ConfirmRestoreMode { context: ctx };
        let (mode2, result) = press_restore(mode, &mut tab, KeyCode::Char('x')).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(matches!(
            mode2.render_state(),
            ModeRenderState::ConfirmRestore
        ));
    }

    #[tokio::test]
    async fn test_confirm_restore_status_line() {
        let mode = ConfirmRestoreMode {
            context: default_context(),
        };
        assert!(mode.status_line().contains("[RESTORE]"));
    }

    #[tokio::test]
    async fn test_confirm_restore_context_method() {
        let ctx = default_context();
        let mode = ConfirmRestoreMode {
            context: ctx.clone(),
        };
        assert!(mode.status_line().contains("[RESTORE]"));
        assert!(matches!(
            mode.render_state(),
            ModeRenderState::ConfirmRestore
        ));
    }

    // ── ConfirmRestoreSessionMode ────────────────────────────────────────────

    #[tokio::test]
    async fn test_confirm_session_y_returns_restore_session() {
        let mut tab = make_tab(&["line"]).await;
        let files = vec!["/var/log/a.log".to_string(), "/var/log/b.log".to_string()];
        let mode = ConfirmRestoreSessionMode {
            files: files.clone(),
        };
        let (mode2, result) = press_session(mode, &mut tab, KeyCode::Char('y')).await;
        assert!(matches!(result, KeyResult::RestoreSession(ref f) if *f == files));
        assert!(!matches!(
            mode2.render_state(),
            ModeRenderState::ConfirmRestoreSession { .. }
        ));
    }

    #[tokio::test]
    async fn test_confirm_session_n_returns_normal_mode() {
        let mut tab = make_tab(&["line"]).await;
        let mode = ConfirmRestoreSessionMode {
            files: vec!["file.log".to_string()],
        };
        let (mode2, result) = press_session(mode, &mut tab, KeyCode::Char('n')).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(!matches!(
            mode2.render_state(),
            ModeRenderState::ConfirmRestoreSession { .. }
        ));
    }

    #[tokio::test]
    async fn test_confirm_session_esc_returns_normal_mode() {
        let mut tab = make_tab(&["line"]).await;
        let mode = ConfirmRestoreSessionMode {
            files: vec!["file.log".to_string()],
        };
        let (mode2, result) = press_session(mode, &mut tab, KeyCode::Esc).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(!matches!(
            mode2.render_state(),
            ModeRenderState::ConfirmRestoreSession { .. }
        ));
    }

    #[tokio::test]
    async fn test_confirm_session_other_key_stays_in_mode() {
        let mut tab = make_tab(&["line"]).await;
        let files = vec!["file.log".to_string()];
        let mode = ConfirmRestoreSessionMode { files };
        let (mode2, result) = press_session(mode, &mut tab, KeyCode::Char('z')).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(matches!(
            mode2.render_state(),
            ModeRenderState::ConfirmRestoreSession { .. }
        ));
    }

    #[tokio::test]
    async fn test_confirm_session_status_line() {
        let mode = ConfirmRestoreSessionMode {
            files: vec!["file.log".to_string()],
        };
        assert!(mode.status_line().contains("[RESTORE SESSION]"));
    }

    #[tokio::test]
    async fn test_confirm_session_files_method() {
        let files = vec!["a.log".to_string(), "b.log".to_string()];
        let mode = ConfirmRestoreSessionMode {
            files: files.clone(),
        };
        match mode.render_state() {
            ModeRenderState::ConfirmRestoreSession { files: returned } => {
                assert_eq!(returned, files);
            }
            other => panic!("expected ConfirmRestoreSession, got {:?}", other),
        }
    }

    // ── Mode trait defaults ──────────────────────────────────────────────────

    #[tokio::test]
    async fn test_confirm_restore_mode_default_methods() {
        let mode = ConfirmRestoreMode {
            context: default_context(),
        };
        assert!(!matches!(
            mode.render_state(),
            ModeRenderState::FilterManagement { .. }
        ));
        assert!(!matches!(
            mode.render_state(),
            ModeRenderState::Command { .. }
        ));
        assert!(!matches!(
            mode.render_state(),
            ModeRenderState::Search { .. }
        ));
        assert!(!matches!(
            mode.render_state(),
            ModeRenderState::Command { .. } | ModeRenderState::Search { .. }
        ));
        assert!(!matches!(
            mode.render_state(),
            ModeRenderState::ConfirmRestoreSession { .. }
        ));
    }

    #[tokio::test]
    async fn test_confirm_session_mode_default_methods() {
        let mode = ConfirmRestoreSessionMode { files: vec![] };
        assert!(!matches!(
            mode.render_state(),
            ModeRenderState::FilterManagement { .. }
        ));
        assert!(!matches!(
            mode.render_state(),
            ModeRenderState::Command { .. }
        ));
        assert!(!matches!(
            mode.render_state(),
            ModeRenderState::Search { .. }
        ));
        assert!(!matches!(
            mode.render_state(),
            ModeRenderState::Command { .. } | ModeRenderState::Search { .. }
        ));
        assert!(!matches!(
            mode.render_state(),
            ModeRenderState::ConfirmRestore
        ));
    }
}
