//! Core mode trait, render state enum, and shared mode infrastructure.
//!
//! [`Mode`] is the central trait: `handle_key` consumes `Box<Self>` and
//! returns a new `(Box<dyn Mode>, KeyResult)`. [`ModeRenderState`] is an
//! ISP-compliant enum — each variant carries exactly the data its popup
//! renderer needs, avoiding optional trait methods.

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
/// Each variant carries exactly the data the rendering layer needs for that mode.
#[derive(Debug, Clone)]
pub enum ModeRenderState {
    Normal,
    Command {
        input: String,
        cursor: usize,
        completion_index: Option<usize>,
        /// Original typed text before Tab cycling; `None` when no completion session is active.
        completion_query: Option<String>,
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
    Visual {
        anchor_col: Option<usize>,
        cursor_col: usize,
        pending_motion: bool,
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
    LevelColors {
        groups: Vec<ValueColorGroup>,
        search: String,
        selected: usize,
    },
    ConfirmRestore,
    ConfirmRestoreSession {
        files: Vec<String>,
    },
    ConfirmOpenDir {
        dir: String,
        files: Vec<String>,
    },
    Ui,
}

impl ModeRenderState {
    /// Returns a short uppercase label for the current mode, used in the tab
    /// bar when the mode bar is hidden.
    pub fn mode_name(&self) -> &'static str {
        match self {
            ModeRenderState::Normal => "NORMAL",
            ModeRenderState::Ui => "UI",
            ModeRenderState::Command { .. } => "COMMAND",
            ModeRenderState::Search { forward: true, .. } => "SEARCH",
            ModeRenderState::Search { forward: false, .. } => "SEARCH↑",
            ModeRenderState::FilterManagement { .. } => "FILTER",
            ModeRenderState::FilterEdit => "FILTER EDIT",
            ModeRenderState::VisualLine { .. } => "VISUAL LINE",
            ModeRenderState::Visual { .. } => "VISUAL",
            ModeRenderState::Comment { .. } => "COMMENT",
            ModeRenderState::KeybindingsHelp { .. } => "HELP",
            ModeRenderState::SelectFields { .. } => "FIELDS",
            ModeRenderState::DockerSelect { .. } => "DOCKER",
            ModeRenderState::ValueColors { .. } => "VALUE COLORS",
            ModeRenderState::LevelColors { .. } => "LEVEL COLORS",
            ModeRenderState::ConfirmRestore
            | ModeRenderState::ConfirmRestoreSession { .. }
            | ModeRenderState::ConfirmOpenDir { .. } => "CONFIRM",
        }
    }
}

#[async_trait]
pub trait Mode: std::fmt::Debug + Send {
    async fn handle_key(
        self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult);

    /// Returns a styled mode bar `Line` with `<KEY> action` spans based on the active keybindings.
    fn mode_bar_content(&self, kb: &Keybindings, theme: &Theme) -> Line<'static>;

    /// Returns the rendering state for this mode.
    ///
    /// The rendering layer matches on the returned enum variant to decide which
    /// UI elements to draw. Each variant carries exactly the data its renderer
    /// needs — no more, no less.
    fn render_state(&self) -> ModeRenderState;
}

/// Like `status_entry` but accepts a runtime-computed action string.
pub fn status_entry_dyn(
    spans: &mut Vec<Span<'static>>,
    key: String,
    action: String,
    theme: &Theme,
) {
    spans.push(Span::styled("<", Style::default().fg(theme.text)));
    spans.push(Span::styled(
        key,
        Style::default()
            .fg(theme.text_highlight_fg)
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(">", Style::default().fg(theme.text)));
    spans.push(Span::styled(
        format!(" {}  ", action),
        Style::default().fg(theme.text),
    ));
}

/// Appends a styled `<key> action  ` entry to `spans`.
/// Used by mode implementations to build the mode bar line.
pub fn status_entry(
    spans: &mut Vec<Span<'static>>,
    key: String,
    action: &'static str,
    theme: &Theme,
) {
    spans.push(Span::styled("<", Style::default().fg(theme.text)));
    spans.push(Span::styled(
        key,
        Style::default()
            .fg(theme.text_highlight_fg)
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(">", Style::default().fg(theme.text)));
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
            (Box::new(NormalMode::default()), KeyResult::Handled)
        } else if kb.no.matches(key, modifiers) {
            tab.log_manager.clear_filters().await;
            tab.log_manager.set_comments(vec![]);
            tab.begin_filter_refresh();
            (Box::new(NormalMode::default()), KeyResult::Handled)
        } else if kb.always.matches(key, modifiers) {
            tab.apply_file_context(&self.context);
            (
                Box::new(NormalMode::default()),
                KeyResult::AlwaysRestoreFile(self.context),
            )
        } else if kb.never.matches(key, modifiers) {
            tab.log_manager.clear_filters().await;
            tab.log_manager.set_comments(vec![]);
            tab.begin_filter_refresh();
            (Box::new(NormalMode::default()), KeyResult::NeverRestoreFile)
        } else {
            (self, KeyResult::Handled)
        }
    }

    fn mode_bar_content(&self, kb: &Keybindings, theme: &Theme) -> Line<'static> {
        let mut spans: Vec<Span<'static>> = vec![Span::styled(
            "[RESTORE]  ",
            Style::default()
                .fg(theme.text_highlight_fg)
                .add_modifier(Modifier::BOLD),
        )];
        spans.push(Span::styled(
            "Restore previous session?  ",
            Style::default().fg(theme.text),
        ));
        status_entry(&mut spans, kb.confirm.yes.display(), "yes", theme);
        status_entry(&mut spans, kb.confirm.no.display(), "no", theme);
        status_entry(&mut spans, kb.confirm.always.display(), "always", theme);
        status_entry(&mut spans, kb.confirm.never.display(), "never", theme);
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
            (
                Box::new(NormalMode::default()),
                KeyResult::RestoreSession(self.files),
            )
        } else if kb.no.matches(key, modifiers) {
            (Box::new(NormalMode::default()), KeyResult::Handled)
        } else if kb.always.matches(key, modifiers) {
            (
                Box::new(NormalMode::default()),
                KeyResult::AlwaysRestoreSession(self.files),
            )
        } else if kb.never.matches(key, modifiers) {
            (
                Box::new(NormalMode::default()),
                KeyResult::NeverRestoreSession,
            )
        } else {
            (self, KeyResult::Handled)
        }
    }

    fn mode_bar_content(&self, kb: &Keybindings, theme: &Theme) -> Line<'static> {
        let mut spans: Vec<Span<'static>> = vec![Span::styled(
            "[RESTORE SESSION]  ",
            Style::default()
                .fg(theme.text_highlight_fg)
                .add_modifier(Modifier::BOLD),
        )];
        spans.push(Span::styled(
            "Restore last session?  ",
            Style::default().fg(theme.text),
        ));
        status_entry(&mut spans, kb.confirm.yes.display(), "yes", theme);
        status_entry(&mut spans, kb.confirm.no.display(), "no", theme);
        status_entry(&mut spans, kb.confirm.always.display(), "always", theme);
        status_entry(&mut spans, kb.confirm.never.display(), "never", theme);
        Line::from(spans)
    }

    fn render_state(&self) -> ModeRenderState {
        ModeRenderState::ConfirmRestoreSession {
            files: self.files.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// ConfirmOpenDirMode
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct ConfirmOpenDirMode {
    pub dir: String,
    pub files: Vec<String>,
}

#[async_trait]
impl Mode for ConfirmOpenDirMode {
    async fn handle_key(
        self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        let kb = &tab.keybindings.confirm;
        if kb.yes.matches(key, modifiers) {
            (
                Box::new(NormalMode::default()),
                KeyResult::OpenFiles(self.files),
            )
        } else if kb.no.matches(key, modifiers) {
            (Box::new(NormalMode::default()), KeyResult::Handled)
        } else {
            (self, KeyResult::Handled)
        }
    }

    fn mode_bar_content(&self, kb: &Keybindings, theme: &Theme) -> Line<'static> {
        let n = self.files.len();
        let dir = self.dir.clone();
        let mut spans: Vec<Span<'static>> = vec![Span::styled(
            "[OPEN DIR]  ",
            Style::default()
                .fg(theme.text_highlight_fg)
                .add_modifier(Modifier::BOLD),
        )];
        spans.push(Span::styled(
            format!(
                "Open {} file{} from {}?  ",
                n,
                if n == 1 { "" } else { "s" },
                dir
            ),
            Style::default().fg(theme.text),
        ));
        status_entry(&mut spans, kb.confirm.yes.display(), "yes", theme);
        status_entry(&mut spans, kb.confirm.no.display(), "no", theme);
        Line::from(spans)
    }

    fn render_state(&self) -> ModeRenderState {
        ModeRenderState::ConfirmOpenDir {
            dir: self.dir.clone(),
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
            level_colors_disabled: [
                "trace", "debug", "info", "notice", "warning", "error", "fatal",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            show_sidebar: false,
            horizontal_scroll: 3,
            marked_lines: vec![],
            file_hash: None,
            show_line_numbers: false,
            comments: vec![],
            show_mode_bar: true,
            show_borders: true,
            show_keys: false,
            raw_mode: false,
            sidebar_width: 30,
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
        assert!(!tab.level_colors_disabled.is_empty());
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
    async fn test_confirm_restore_n_preserves_preview_marks() {
        let mut tab = make_tab(&["line0", "line1", "line2"]).await;
        // Simulate user adding a mark during the preview phase.
        tab.log_manager.toggle_mark(1);
        assert_eq!(tab.log_manager.get_marked_indices(), vec![1]);

        let mode = ConfirmRestoreMode {
            context: default_context(),
        };
        press_restore(mode, &mut tab, KeyCode::Char('n')).await;

        // Mark added during preview must survive declining the restore.
        assert_eq!(
            tab.log_manager.get_marked_indices(),
            vec![1],
            "preview marks must not be erased on decline"
        );
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
    async fn test_confirm_restore_mode_bar_content() {
        let mode = ConfirmRestoreMode {
            context: default_context(),
        };
        assert!(matches!(
            mode.render_state(),
            ModeRenderState::ConfirmRestore
        ));
    }

    #[tokio::test]
    async fn test_confirm_restore_context_method() {
        let ctx = default_context();
        let mode = ConfirmRestoreMode {
            context: ctx.clone(),
        };
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
    async fn test_confirm_session_mode_bar_content() {
        let mode = ConfirmRestoreSessionMode {
            files: vec!["file.log".to_string()],
        };
        assert!(matches!(
            mode.render_state(),
            ModeRenderState::ConfirmRestoreSession { .. }
        ));
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

    // ── ConfirmOpenDirMode ───────────────────────────────────────────────────

    async fn press_open_dir(
        mode: ConfirmOpenDirMode,
        tab: &mut TabState,
        code: KeyCode,
    ) -> (Box<dyn Mode>, KeyResult) {
        Box::new(mode)
            .handle_key(tab, code, KeyModifiers::NONE)
            .await
    }

    #[tokio::test]
    async fn test_confirm_open_dir_y_returns_open_files() {
        let mut tab = make_tab(&["line"]).await;
        let files = vec!["/tmp/a.log".to_string(), "/tmp/b.log".to_string()];
        let mode = ConfirmOpenDirMode {
            dir: "/tmp".to_string(),
            files: files.clone(),
        };
        let (mode2, result) = press_open_dir(mode, &mut tab, KeyCode::Char('y')).await;
        assert!(matches!(result, KeyResult::OpenFiles(ref f) if *f == files));
        assert!(!matches!(
            mode2.render_state(),
            ModeRenderState::ConfirmOpenDir { .. }
        ));
    }

    #[tokio::test]
    async fn test_confirm_open_dir_n_returns_normal() {
        let mut tab = make_tab(&["line"]).await;
        let mode = ConfirmOpenDirMode {
            dir: "/tmp".to_string(),
            files: vec!["/tmp/a.log".to_string()],
        };
        let (mode2, result) = press_open_dir(mode, &mut tab, KeyCode::Char('n')).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(!matches!(
            mode2.render_state(),
            ModeRenderState::ConfirmOpenDir { .. }
        ));
    }

    #[tokio::test]
    async fn test_confirm_open_dir_esc_returns_normal() {
        let mut tab = make_tab(&["line"]).await;
        let mode = ConfirmOpenDirMode {
            dir: "/tmp".to_string(),
            files: vec!["/tmp/a.log".to_string()],
        };
        let (mode2, result) = press_open_dir(mode, &mut tab, KeyCode::Esc).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(!matches!(
            mode2.render_state(),
            ModeRenderState::ConfirmOpenDir { .. }
        ));
    }

    #[tokio::test]
    async fn test_confirm_open_dir_other_key_stays_in_mode() {
        let mut tab = make_tab(&["line"]).await;
        let mode = ConfirmOpenDirMode {
            dir: "/tmp".to_string(),
            files: vec!["/tmp/a.log".to_string()],
        };
        let (mode2, result) = press_open_dir(mode, &mut tab, KeyCode::Char('z')).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(matches!(
            mode2.render_state(),
            ModeRenderState::ConfirmOpenDir { .. }
        ));
    }

    #[tokio::test]
    async fn test_confirm_open_dir_mode_bar_content() {
        let mode = ConfirmOpenDirMode {
            dir: "/tmp".to_string(),
            files: vec!["/tmp/a.log".to_string()],
        };
        assert!(matches!(
            mode.render_state(),
            ModeRenderState::ConfirmOpenDir { .. }
        ));
    }

    #[tokio::test]
    async fn test_confirm_open_dir_render_state() {
        let files = vec!["/tmp/a.log".to_string()];
        let mode = ConfirmOpenDirMode {
            dir: "/tmp".to_string(),
            files: files.clone(),
        };
        match mode.render_state() {
            ModeRenderState::ConfirmOpenDir {
                dir,
                files: returned,
            } => {
                assert_eq!(dir, "/tmp");
                assert_eq!(returned, files);
            }
            other => panic!("expected ConfirmOpenDir, got {:?}", other),
        }
    }

    // ── ModeRenderState::mode_name ───────────────────────────────────────────

    #[test]
    fn mode_name_covers_all_variants() {
        assert_eq!(ModeRenderState::Normal.mode_name(), "NORMAL");
        assert_eq!(
            ModeRenderState::Command {
                input: String::new(),
                cursor: 0,
                completion_index: None,
                completion_query: None,
            }
            .mode_name(),
            "COMMAND"
        );
        assert_eq!(
            ModeRenderState::Search {
                query: String::new(),
                forward: true
            }
            .mode_name(),
            "SEARCH"
        );
        assert_eq!(
            ModeRenderState::Search {
                query: String::new(),
                forward: false
            }
            .mode_name(),
            "SEARCH↑"
        );
        assert_eq!(
            ModeRenderState::FilterManagement { selected_index: 0 }.mode_name(),
            "FILTER"
        );
        assert_eq!(ModeRenderState::FilterEdit.mode_name(), "FILTER EDIT");
        assert_eq!(
            ModeRenderState::VisualLine { anchor: 0 }.mode_name(),
            "VISUAL LINE"
        );
        assert_eq!(
            ModeRenderState::Visual {
                anchor_col: None,
                cursor_col: 0,
                pending_motion: false
            }
            .mode_name(),
            "VISUAL"
        );
        assert_eq!(
            ModeRenderState::Comment {
                lines: vec![],
                cursor_row: 0,
                cursor_col: 0,
                line_count: 0
            }
            .mode_name(),
            "COMMENT"
        );
        assert_eq!(
            ModeRenderState::KeybindingsHelp {
                scroll: 0,
                search: String::new()
            }
            .mode_name(),
            "HELP"
        );
        assert_eq!(
            ModeRenderState::SelectFields {
                fields: vec![],
                selected: 0
            }
            .mode_name(),
            "FIELDS"
        );
        assert_eq!(
            ModeRenderState::DockerSelect {
                containers: vec![],
                selected: 0,
                error: None
            }
            .mode_name(),
            "DOCKER"
        );
        assert_eq!(
            ModeRenderState::ValueColors {
                groups: vec![],
                search: String::new(),
                selected: 0
            }
            .mode_name(),
            "VALUE COLORS"
        );
        assert_eq!(
            ModeRenderState::LevelColors {
                groups: vec![],
                search: String::new(),
                selected: 0
            }
            .mode_name(),
            "LEVEL COLORS"
        );
        assert_eq!(ModeRenderState::ConfirmRestore.mode_name(), "CONFIRM");
        assert_eq!(
            ModeRenderState::ConfirmRestoreSession { files: vec![] }.mode_name(),
            "CONFIRM"
        );
        assert_eq!(
            ModeRenderState::ConfirmOpenDir {
                dir: String::new(),
                files: vec![]
            }
            .mode_name(),
            "CONFIRM"
        );
        assert_eq!(ModeRenderState::Ui.mode_name(), "UI");
    }
}
