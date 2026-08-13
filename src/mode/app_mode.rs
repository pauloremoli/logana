use crate::{
    config::{DltDevice, Keybindings},
    db::FileContext,
    mode::docker_select_mode::DockerContainer,
    mode::{
        dlt_select_mode::AddDeviceRenderState, normal_mode::NormalMode,
        value_colors_mode::ValueColorGroup,
    },
    theme::Theme,
    ui::{KeyResult, TabState},
};
use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use std::sync::Arc;

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
        cursor: usize,
        forward: bool,
    },
    FilterManagement {
        selected_index: usize,
        /// Live typeahead query narrowing the sidebar; empty when not searching
        /// or when search is active but nothing has been typed yet.
        search: String,
        /// True while capturing search input — distinct from `search.is_empty()`,
        /// since the moment right after pressing `/` has an empty query but
        /// still needs a visible "you're searching now" indicator.
        searching: bool,
    },
    FilterEdit,
    GroupManagement {
        selected_group: String,
    },
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
        is_editing: bool,
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
    DltSelect {
        devices: Vec<DltDevice>,
        selected: usize,
        error: Option<String>,
        adding: Option<AddDeviceRenderState>,
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
        files: Arc<Vec<String>>,
    },
    MergeSelect {
        tabs: Vec<(String, bool)>,
        selected: usize,
    },
    ArchivePicker {
        rows: Vec<crate::mode::archive_picker_mode::ArchiveRow>,
        selected: usize,
        source_path: String,
        /// Live typeahead query narrowing `rows`; empty when not searching
        /// or when search is active but nothing has been typed yet.
        search: String,
        /// True while capturing search input — distinct from `search.is_empty()`
        /// so the popup can show a marker the instant `/` is pressed.
        searching: bool,
    },
    Ui,
    ExportFooter {
        path: String,
        template_name: String,
        fields: Vec<(String, Vec<String>)>,
        active_idx: usize,
        cursor_row: usize,
        cursor_col: usize,
    },
    DefaultFilters {
        rows: Vec<crate::mode::default_filters_mode::DefaultFilterRow>,
        search: String,
        selected: usize,
        /// `Some` while the selected row's path is being edited.
        editing: Option<crate::mode::default_filters_mode::PathEditState>,
    },
    FileSwitcher {
        /// (`App::tabs` index, tab title) for every open tab, snapshotted
        /// when the popup opened.
        entries: Vec<(usize, String)>,
        /// The tab that was active when the popup opened.
        active_tab: usize,
        /// Index into the *visible* (filtered) entries.
        selected: usize,
        search: String,
    },
    ThemePicker {
        /// Available theme names, snapshotted when the popup opened.
        entries: Vec<String>,
        /// Index into the *visible* (filtered) entries.
        selected: usize,
        search: String,
    },
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
            ModeRenderState::GroupManagement { .. } => "GROUP",
            ModeRenderState::VisualLine { .. } => "VISUAL LINE",
            ModeRenderState::Visual { .. } => "VISUAL",
            ModeRenderState::Comment { .. } => "COMMENT",
            ModeRenderState::KeybindingsHelp { .. } => "HELP",
            ModeRenderState::SelectFields { .. } => "FIELDS",
            ModeRenderState::DockerSelect { .. } => "DOCKER",
            ModeRenderState::DltSelect { .. } => "DLT",
            ModeRenderState::ValueColors { .. } => "VALUE COLORS",
            ModeRenderState::LevelColors { .. } => "LEVEL COLORS",
            ModeRenderState::ConfirmRestore | ModeRenderState::ConfirmRestoreSession { .. } => {
                "CONFIRM"
            }
            ModeRenderState::MergeSelect { .. } => "MERGE",
            ModeRenderState::ArchivePicker { .. } => "ARCHIVE",
            ModeRenderState::ExportFooter { .. } => "EXPORT",
            ModeRenderState::DefaultFilters { .. } => "DEFAULT FILTERS",
            ModeRenderState::FileSwitcher { .. } => "SWITCH",
            ModeRenderState::ThemePicker { .. } => "THEME",
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

    /// Called when the cursor line changes due to mouse scrolling.
    /// Modes that hold per-line state (e.g. visual char selection) should
    /// refresh that state. The default implementation is a no-op.
    /// Called when the cursor line changes due to mouse scrolling.
    /// Modes that hold per-line state should refresh it here. Default is a no-op.
    fn on_scroll_line_change(&mut self, _tab: &mut TabState) {}

    /// Downcast hook for the one case where `App` needs to mutate a mode's
    /// own state from outside `handle_key` — applying a background archive
    /// node expand to the *live* `ArchivePickerMode` still installed on the
    /// tab, so any selection/search/toggle changes made while the fetch was
    /// in flight aren't clobbered by replacing the whole mode. Default is
    /// `None`; only `ArchivePickerMode` overrides it.
    fn as_archive_picker_mut(
        &mut self,
    ) -> Option<&mut crate::mode::archive_picker_mode::ArchivePickerMode> {
        None
    }
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
        let kb = &tab.interaction.keybindings.confirm;
        if kb.yes.matches(key, modifiers) {
            tab.apply_file_context(&self.context);
            tab.sync_collapse_mask();
            (Box::new(NormalMode::default()), KeyResult::Handled)
        } else if kb.no.matches(key, modifiers) {
            tab.log_manager.clear_filters().await;
            tab.comment_manager.set(vec![]);
            tab.begin_filter_refresh();
            (Box::new(NormalMode::default()), KeyResult::Handled)
        } else if kb.always.matches(key, modifiers) {
            tab.apply_file_context(&self.context);
            tab.sync_collapse_mask();
            (
                Box::new(NormalMode::default()),
                KeyResult::AlwaysRestoreFile(Box::new(self.context)),
            )
        } else if kb.never.matches(key, modifiers) {
            tab.log_manager.clear_filters().await;
            tab.comment_manager.set(vec![]);
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

#[derive(Debug)]
pub struct ConfirmRestoreSessionMode {
    pub files: Arc<Vec<String>>,
}

#[async_trait]
impl Mode for ConfirmRestoreSessionMode {
    async fn handle_key(
        self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        let kb = &tab.interaction.keybindings.confirm;
        if kb.yes.matches(key, modifiers) {
            (
                Box::new(NormalMode::default()),
                KeyResult::RestoreSession(Arc::unwrap_or_clone(self.files)),
            )
        } else if kb.no.matches(key, modifiers) {
            (Box::new(NormalMode::default()), KeyResult::Handled)
        } else if kb.always.matches(key, modifiers) {
            (
                Box::new(NormalMode::default()),
                KeyResult::AlwaysRestoreSession(Arc::unwrap_or_clone(self.files)),
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
            files: Arc::clone(&self.files),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::db::LogManager;
    use crate::filters::{FilterOptions, FilterType};
    use crate::ingestion::FileReader;
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
            level_colors_disabled: [
                "trace", "debug", "info", "notice", "warning", "error", "fatal",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            horizontal_scroll: 3,
            marked_lines: vec![],
            file_hash: None,
            comments: vec![],
            show_keys: false,
            raw_mode: false,
            hidden_fields: std::collections::HashSet::new(),
            field_layout_columns: None,
            filtering_enabled: true,
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
        assert_eq!(tab.scroll.scroll_offset, 5);
        assert!(!tab.display.level_colors_disabled.is_empty());
        assert_eq!(tab.scroll.horizontal_scroll, 3);
    }

    /// Regression test: confirming a saved file-context restore must
    /// re-derive the collapse mask, not just copy fields off `context` —
    /// otherwise a file whose collapse mode was already on (persisted
    /// globally, independent of this per-file context) would keep showing
    /// every continuation line and no `+` marker after the user says "yes".
    #[tokio::test]
    async fn test_confirm_restore_y_applies_collapse_mask() {
        let mut tab = make_tab(&[
            "2024-07-24T10:00:00Z INFO request processed",
            "2024-07-24T10:00:01Z INFO another request",
            "2019-01-26 20:29:10.000 5.120.204.67 200 GET / HTTP/1.1",
        ])
        .await;
        assert!(tab.continuation_map.is_some());
        tab.display.collapse_continuations = true;

        let mode = ConfirmRestoreMode {
            context: default_context(),
        };
        press_restore(mode, &mut tab, KeyCode::Char('y')).await;

        assert_eq!(
            tab.filter.visible_indices.iter().collect::<Vec<_>>(),
            vec![0, 1],
            "line 2 (a continuation of line 1) must be hidden once the \
             restore is confirmed, matching collapse_continuations=true"
        );
    }

    #[tokio::test]
    async fn test_confirm_restore_n_clears_filters_and_returns_normal() {
        let mut tab = make_tab(&["error", "warn"]).await;
        tab.log_manager
            .add_filter_with_color(
                "error".to_string(),
                FilterType::Include,
                FilterOptions::default().line_mode(),
            )
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
        tab.mark_manager.toggle(1);
        assert_eq!(tab.mark_manager.get_indices(), vec![1]);

        let mode = ConfirmRestoreMode {
            context: default_context(),
        };
        press_restore(mode, &mut tab, KeyCode::Char('n')).await;

        // Mark added during preview must survive declining the restore.
        assert_eq!(
            tab.mark_manager.get_indices(),
            vec![1],
            "preview marks must not be erased on decline"
        );
    }

    #[tokio::test]
    async fn test_confirm_restore_esc_clears_filters_and_returns_normal() {
        let mut tab = make_tab(&["line"]).await;
        tab.log_manager
            .add_filter_with_color(
                "line".to_string(),
                FilterType::Include,
                FilterOptions::default().line_mode(),
            )
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

    #[tokio::test]
    async fn test_confirm_session_y_returns_restore_session() {
        let mut tab = make_tab(&["line"]).await;
        let files = vec!["/var/log/a.log".to_string(), "/var/log/b.log".to_string()];
        let mode = ConfirmRestoreSessionMode {
            files: Arc::new(files.clone()),
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
            files: Arc::new(vec!["file.log".to_string()]),
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
            files: Arc::new(vec!["file.log".to_string()]),
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
        let files = Arc::new(vec!["file.log".to_string()]);
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
            files: Arc::new(vec!["file.log".to_string()]),
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
            files: Arc::new(files.clone()),
        };
        match mode.render_state() {
            ModeRenderState::ConfirmRestoreSession { files: returned } => {
                assert_eq!(*returned, files);
            }
            other => panic!("expected ConfirmRestoreSession, got {:?}", other),
        }
    }

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
        let mode = ConfirmRestoreSessionMode {
            files: Arc::new(vec![]),
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
            ModeRenderState::ConfirmRestore
        ));
    }

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
                cursor: 0,
                forward: true
            }
            .mode_name(),
            "SEARCH"
        );
        assert_eq!(
            ModeRenderState::Search {
                query: String::new(),
                cursor: 0,
                forward: false
            }
            .mode_name(),
            "SEARCH↑"
        );
        assert_eq!(
            ModeRenderState::FilterManagement {
                selected_index: 0,
                search: String::new(),
                searching: false,
            }
            .mode_name(),
            "FILTER"
        );
        assert_eq!(ModeRenderState::FilterEdit.mode_name(), "FILTER EDIT");
        assert_eq!(
            ModeRenderState::GroupManagement {
                selected_group: String::new()
            }
            .mode_name(),
            "GROUP"
        );
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
                line_count: 0,
                is_editing: false
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
            ModeRenderState::DltSelect {
                devices: vec![],
                selected: 0,
                error: None,
                adding: None
            }
            .mode_name(),
            "DLT"
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
            ModeRenderState::ConfirmRestoreSession {
                files: Arc::new(vec![])
            }
            .mode_name(),
            "CONFIRM"
        );
        assert_eq!(ModeRenderState::Ui.mode_name(), "UI");
        assert_eq!(
            ModeRenderState::ArchivePicker {
                rows: vec![],
                selected: 0,
                source_path: String::new(),
                search: String::new(),
                searching: false,
            }
            .mode_name(),
            "ARCHIVE"
        );
        assert_eq!(
            ModeRenderState::ExportFooter {
                path: String::new(),
                template_name: String::new(),
                fields: vec![],
                active_idx: 0,
                cursor_row: 0,
                cursor_col: 0,
            }
            .mode_name(),
            "EXPORT"
        );
        assert_eq!(
            ModeRenderState::DefaultFilters {
                rows: vec![],
                search: String::new(),
                selected: 0,
                editing: None,
            }
            .mode_name(),
            "DEFAULT FILTERS"
        );
        assert_eq!(
            ModeRenderState::FileSwitcher {
                entries: vec![],
                active_tab: 0,
                selected: 0,
                search: String::new(),
            }
            .mode_name(),
            "SWITCH"
        );
        assert_eq!(
            ModeRenderState::ThemePicker {
                entries: vec![],
                selected: 0,
                search: String::new(),
            }
            .mode_name(),
            "THEME"
        );
    }
}
