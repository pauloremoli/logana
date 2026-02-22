use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{Terminal, prelude::*};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::Keybindings;
use crate::db::{FileContext, FileContextStore, SessionStore};
use crate::file_reader::FileReader;
use crate::log_manager::LogManager;
use crate::mode::app_mode::{ConfirmRestoreMode, ConfirmRestoreSessionMode};
use crate::mode::command_mode::CommandMode;
use crate::mode::filter_mode::FilterManagementMode;
use crate::mode::normal_mode::NormalMode;
use crate::theme::Theme;

use super::{FileLoadState, KeyResult, StdinLoadState, TabState};

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

pub struct App {
    pub tabs: Vec<TabState>,
    pub active_tab: usize,
    pub theme: Theme,
    pub db: Arc<crate::db::Database>,
    pub should_quit: bool,
    /// In-progress background file load (startup or session restore).
    pub file_load_state: Option<FileLoadState>,
    /// In-progress stdin read — separate slot so session-restore cannot overwrite it.
    pub stdin_load_state: Option<StdinLoadState>,
    /// Shared keybindings — propagated to every new tab.
    pub keybindings: Arc<Keybindings>,
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("active_tab", &self.active_tab)
            .field("num_tabs", &self.tabs.len())
            .finish()
    }
}

impl App {
    pub async fn new(
        log_manager: LogManager,
        file_reader: FileReader,
        theme: Theme,
        keybindings: Arc<Keybindings>,
    ) -> App {
        let db = log_manager.db.clone();

        let title = log_manager
            .source_file()
            .map(|s| {
                std::path::Path::new(s)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(s)
                    .to_string()
            })
            .unwrap_or_else(|| "stdin".to_string());

        let no_source = log_manager.source_file().is_none();
        let no_data = file_reader.line_count() == 0;

        let mut tab = TabState::new(file_reader, log_manager, title);
        tab.keybindings = keybindings.clone();

        // Check for saved context only when we have real data (not a placeholder
        // that will be replaced by a background load started after App::new).
        if let Some(source) = tab.log_manager.source_file() {
            if tab.file_reader.line_count() > 0 {
                let source = source.to_string();
                if let Ok(Some(ctx)) = db.load_file_context(&source).await {
                    tab.mode = Box::new(ConfirmRestoreMode { context: ctx });
                }
            }
        } else if no_source && no_data {
            // No file argument and no piped data — offer to restore last session.
            if let Ok(files) = db.load_session().await
                && !files.is_empty()
            {
                tab.mode = Box::new(ConfirmRestoreSessionMode { files });
            }
        }

        App {
            tabs: vec![tab],
            active_tab: 0,
            theme,
            db,
            should_quit: false,
            file_load_state: None,
            stdin_load_state: None,
            keybindings,
        }
    }

    pub fn tab(&self) -> &TabState {
        &self.tabs[self.active_tab]
    }

    pub fn tab_mut(&mut self) -> &mut TabState {
        &mut self.tabs[self.active_tab]
    }

    pub(super) async fn save_tab_context(&self, tab: &TabState) {
        if let Some(ctx) = tab.to_file_context() {
            let _ = self.db.save_file_context(&ctx).await;
        }
    }

    pub(super) async fn save_all_contexts(&self) {
        let source_files: Vec<String> = self
            .tabs
            .iter()
            .filter_map(|t| t.log_manager.source_file().map(|s| s.to_string()))
            .collect();

        let contexts: Vec<FileContext> = self
            .tabs
            .iter()
            .filter_map(|t| t.to_file_context())
            .collect();

        if !source_files.is_empty() {
            let _ = self.db.save_session(&source_files).await;
        }
        for ctx in &contexts {
            let _ = self.db.save_file_context(ctx).await;
        }
    }

    pub async fn close_tab(&mut self) -> bool {
        self.save_tab_context(&self.tabs[self.active_tab]).await;
        if self.tabs.len() <= 1 {
            return true; // signal to quit
        }
        self.tabs.remove(self.active_tab);
        if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        }
        false
    }

    pub(super) async fn handle_global_key(&mut self, key: KeyCode, modifiers: KeyModifiers) {
        let kb = self.keybindings.clone();
        if kb.global.quit.matches(key, modifiers) {
            self.save_all_contexts().await;
            self.should_quit = true;
        } else if kb.global.next_tab.matches(key, modifiers) {
            if self.tabs.len() > 1 {
                self.active_tab = (self.active_tab + 1) % self.tabs.len();
            }
        } else if kb.global.prev_tab.matches(key, modifiers) {
            if self.tabs.len() > 1 {
                self.active_tab = if self.active_tab == 0 {
                    self.tabs.len() - 1
                } else {
                    self.active_tab - 1
                };
            }
        } else if kb.global.close_tab.matches(key, modifiers) {
            if self.close_tab().await {
                self.save_all_contexts().await;
                self.should_quit = true;
            }
        } else if kb.global.new_tab.matches(key, modifiers) {
            let history = self.tabs[self.active_tab].command_history.clone();
            self.tabs[self.active_tab].mode =
                Box::new(CommandMode::with_history("open ".to_string(), 5, history));
        }
    }

    /// Execute a command string, transitioning mode on success/failure.
    pub async fn execute_command_str(&mut self, cmd: String) {
        let result = self.run_command(&cmd).await;
        let tab = &mut self.tabs[self.active_tab];
        match result {
            Ok(mode_was_set) => {
                if !cmd.trim().is_empty() {
                    tab.command_history.push(cmd.trim().to_string());
                }
                if !mode_was_set {
                    if let Some(idx) = tab.filter_context.take() {
                        tab.mode = Box::new(FilterManagementMode {
                            selected_filter_index: idx,
                        });
                    } else {
                        tab.mode = Box::new(NormalMode);
                    }
                }
            }
            Err(msg) => {
                tab.command_error = Some(msg);
                let history = tab.command_history.clone();
                let cmd_len = cmd.len();
                tab.mode = Box::new(CommandMode {
                    input: cmd,
                    cursor: cmd_len,
                    history,
                    history_index: None,
                    completion_index: None,
                });
            }
        }
    }

    pub async fn run(&mut self, terminal: &mut Terminal<impl Backend>) -> anyhow::Result<()> {
        let mut last_tick = Instant::now();
        let tick_rate = Duration::from_millis(250);

        loop {
            terminal.draw(|frame| self.ui(frame))?;

            // Poll for background load completion and file watch updates each frame.
            self.advance_file_load().await;
            self.advance_stdin_load().await;
            self.advance_file_watches();

            let poll_timeout = tick_rate
                .checked_sub(last_tick.elapsed())
                .unwrap_or_else(|| Duration::from_secs(0));

            if crossterm::event::poll(poll_timeout)?
                && let crossterm::event::Event::Key(key) = crossterm::event::read()?
                && key.kind == crossterm::event::KeyEventKind::Press
            {
                let tab = &mut self.tabs[self.active_tab];
                let mode = std::mem::replace(&mut tab.mode, Box::new(NormalMode));
                let (next_mode, result) = mode.handle_key(tab, key.code, key.modifiers).await;
                tab.mode = next_mode;
                match result {
                    KeyResult::Handled => {}
                    KeyResult::Ignored => self.handle_global_key(key.code, key.modifiers).await,
                    KeyResult::ExecuteCommand(cmd) => self.execute_command_str(cmd).await,
                    KeyResult::RestoreSession(files) => self.restore_session(files).await,
                    KeyResult::DockerAttach(id, name) => self.open_docker_logs(id, name).await,
                    KeyResult::ApplyValueColors(disabled) => {
                        self.theme.value_colors.disabled = disabled;
                    }
                }
            }

            if self.should_quit {
                return Ok(());
            }

            if last_tick.elapsed() >= tick_rate {
                last_tick = Instant::now();
            }
        }
    }

    pub async fn handle_key_event(&mut self, key_code: KeyCode) {
        self.handle_key_event_with_modifiers(key_code, KeyModifiers::NONE)
            .await;
    }

    pub async fn handle_key_event_with_modifiers(
        &mut self,
        key_code: KeyCode,
        modifiers: KeyModifiers,
    ) {
        let tab = &mut self.tabs[self.active_tab];
        let mode = std::mem::replace(&mut tab.mode, Box::new(NormalMode));
        let (next_mode, result) = mode.handle_key(tab, key_code, modifiers).await;
        tab.mode = next_mode;
        match result {
            KeyResult::Handled => {}
            KeyResult::Ignored => self.handle_global_key(key_code, modifiers).await,
            KeyResult::ExecuteCommand(cmd) => self.execute_command_str(cmd).await,
            KeyResult::RestoreSession(files) => self.restore_session(files).await,
            KeyResult::DockerAttach(id, name) => self.open_docker_logs(id, name).await,
            KeyResult::ApplyValueColors(disabled) => {
                self.theme.value_colors.disabled = disabled;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auto_complete::shell_split;
    use crate::config::Keybindings;
    use crate::db::Database;
    use crate::file_reader::FileReader;
    use crate::log_manager::LogManager;
    use crate::types::FilterType;
    use std::sync::Arc;

    async fn make_tab(lines: &[&str]) -> (FileReader, LogManager) {
        let data: Vec<u8> = lines.join("\n").into_bytes();
        let file_reader = FileReader::from_bytes(data);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        (file_reader, log_manager)
    }

    async fn make_app(lines: &[&str]) -> App {
        let (file_reader, log_manager) = make_tab(lines).await;
        App::new(
            log_manager,
            file_reader,
            Theme::default(),
            Arc::new(Keybindings::default()),
        )
        .await
    }

    #[tokio::test]
    async fn test_toggle_wrap_command() {
        let mut app = make_app(&["INFO something", "WARN warning", "ERROR error"]).await;
        app.execute_command_str("wrap".to_string()).await;
        assert!(!app.tab().wrap);
        app.execute_command_str("wrap".to_string()).await;
        assert!(app.tab().wrap);
    }

    #[tokio::test]
    async fn test_add_filter_command() {
        let mut app = make_app(&["INFO something", "WARN warning", "ERROR error"]).await;
        app.execute_command_str("filter foo".to_string()).await;
        let filters = app.tab().log_manager.get_filters();
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].filter_type, FilterType::Include);
        assert_eq!(filters[0].pattern, "foo");
    }

    #[tokio::test]
    async fn test_add_exclude_command() {
        let mut app = make_app(&["INFO something", "WARN warning", "ERROR error"]).await;
        app.execute_command_str("exclude bar".to_string()).await;
        let filters = app.tab().log_manager.get_filters();
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].filter_type, FilterType::Exclude);
        assert_eq!(filters[0].pattern, "bar");
    }

    #[tokio::test]
    async fn test_shell_split_basic() {
        assert_eq!(shell_split("filter foo"), vec!["filter", "foo"]);
        assert_eq!(shell_split("  filter  foo  "), vec!["filter", "foo"]);
        assert_eq!(shell_split(""), Vec::<String>::new());
    }

    #[tokio::test]
    async fn test_shell_split_quoted() {
        assert_eq!(
            shell_split(r#"filter "hello world""#),
            vec!["filter", "hello world"]
        );
        assert_eq!(
            shell_split(r#"exclude "foo bar baz""#),
            vec!["exclude", "foo bar baz"]
        );
    }

    #[tokio::test]
    async fn test_filter_command_with_quoted_pattern() {
        let mut app = make_app(&["INFO something", "WARN warning", "ERROR error"]).await;
        app.execute_command_str(r#"filter "hello world""#.to_string())
            .await;
        let filters = app.tab().log_manager.get_filters();
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].pattern, "hello world");
        assert_eq!(filters[0].filter_type, FilterType::Include);
    }

    #[tokio::test]
    async fn test_filter_reduces_visible() {
        let lines = vec!["INFO something", "WARN warning", "ERROR error"];
        let (file_reader, log_manager) = make_tab(&lines).await;
        let mut app = App::new(
            log_manager,
            file_reader,
            Theme::default(),
            Arc::new(Keybindings::default()),
        )
        .await;

        assert_eq!(app.tab().visible_indices.len(), 3);

        app.execute_command_str("filter INFO".to_string()).await;

        assert_eq!(app.tab().visible_indices.len(), 1);
    }

    #[tokio::test]
    async fn test_mark_toggle() {
        let lines = vec!["line0", "line1", "line2"];
        let (file_reader, log_manager) = make_tab(&lines).await;
        let mut app = App::new(
            log_manager,
            file_reader,
            Theme::default(),
            Arc::new(Keybindings::default()),
        )
        .await;

        app.tab_mut().scroll_offset = 0;
        app.handle_key_event_with_modifiers(KeyCode::Char('m'), KeyModifiers::NONE)
            .await;
        assert!(app.tab().log_manager.is_marked(0));

        app.handle_key_event_with_modifiers(KeyCode::Char('m'), KeyModifiers::NONE)
            .await;
        assert!(!app.tab().log_manager.is_marked(0));
    }

    #[tokio::test]
    async fn test_scroll_g_key() {
        let lines: Vec<&str> = (0..20).map(|_| "line").collect();
        let (file_reader, log_manager) = make_tab(&lines).await;
        let mut app = App::new(
            log_manager,
            file_reader,
            Theme::default(),
            Arc::new(Keybindings::default()),
        )
        .await;

        // 'G' goes to end
        app.handle_key_event_with_modifiers(KeyCode::Char('G'), KeyModifiers::NONE)
            .await;
        assert_eq!(app.tab().scroll_offset, 19);

        // 'gg' goes to top
        app.handle_key_event_with_modifiers(KeyCode::Char('g'), KeyModifiers::NONE)
            .await;
        app.handle_key_event_with_modifiers(KeyCode::Char('g'), KeyModifiers::NONE)
            .await;
        assert_eq!(app.tab().scroll_offset, 0);
    }

    #[tokio::test]
    async fn test_to_file_context_none_without_source() {
        let app = make_app(&["line"]).await;
        assert!(app.tab().to_file_context().is_none());
    }

    #[tokio::test]
    async fn test_clear_filters_command() {
        let mut app = make_app(&["INFO a", "WARN b", "ERROR c"]).await;
        app.execute_command_str("filter INFO".to_string()).await;
        assert_eq!(app.tab().log_manager.get_filters().len(), 1);
        app.execute_command_str("clear-filters".to_string()).await;
        assert!(app.tab().log_manager.get_filters().is_empty());
        assert_eq!(app.tab().visible_indices.len(), 3);
    }

    #[tokio::test]
    async fn test_disable_filters_command() {
        let mut app = make_app(&["INFO a", "WARN b", "ERROR c"]).await;
        app.execute_command_str("filter INFO".to_string()).await;
        assert_eq!(app.tab().visible_indices.len(), 1);

        app.execute_command_str("disable-filters".to_string()).await;
        assert!(!app.tab().log_manager.get_filters()[0].enabled);
        assert_eq!(app.tab().visible_indices.len(), 3);
    }

    #[tokio::test]
    async fn test_enable_filters_command() {
        let mut app = make_app(&["INFO a", "WARN b", "ERROR c"]).await;
        app.execute_command_str("filter INFO".to_string()).await;
        app.execute_command_str("disable-filters".to_string()).await;
        assert!(!app.tab().log_manager.get_filters()[0].enabled);

        app.execute_command_str("enable-filters".to_string()).await;
        assert!(app.tab().log_manager.get_filters()[0].enabled);
        assert_eq!(app.tab().visible_indices.len(), 1);
    }

    #[tokio::test]
    async fn test_filtering_command_toggles_bypass() {
        let mut app = make_app(&["INFO a", "WARN b", "ERROR c"]).await;
        app.execute_command_str("filter INFO".to_string()).await;
        assert_eq!(app.tab().visible_indices.len(), 1);
        assert!(app.tab().filtering_enabled);

        app.execute_command_str("filtering".to_string()).await;
        assert!(!app.tab().filtering_enabled);
        assert_eq!(app.tab().visible_indices.len(), 3);

        app.execute_command_str("filtering".to_string()).await;
        assert!(app.tab().filtering_enabled);
        assert_eq!(app.tab().visible_indices.len(), 1);
    }

    #[tokio::test]
    async fn test_hide_field_by_name() {
        let mut app = make_app(&[r#"{"level":"INFO","msg":"hello"}"#]).await;
        assert!(app.tab().hidden_fields.is_empty());
        app.execute_command_str("hide-field msg".to_string()).await;
        assert!(app.tab().hidden_fields.contains("msg"));
        assert!(!app.tab().hidden_fields.contains("level"));
    }

    #[tokio::test]
    async fn test_hide_field_by_name_stores_string() {
        let mut app = make_app(&[r#"{"level":"INFO","msg":"hello"}"#]).await;
        // Even numeric strings are stored as field names (no index-based hiding).
        app.execute_command_str("hide-field 0".to_string()).await;
        assert!(app.tab().hidden_fields.contains("0"));
    }

    #[tokio::test]
    async fn test_show_field_removes_hidden_name() {
        let mut app = make_app(&[r#"{"level":"INFO","msg":"hello"}"#]).await;
        app.execute_command_str("hide-field msg".to_string()).await;
        assert!(app.tab().hidden_fields.contains("msg"));
        app.execute_command_str("show-field msg".to_string()).await;
        assert!(!app.tab().hidden_fields.contains("msg"));
    }

    #[tokio::test]
    async fn test_show_field_removes_hidden_string() {
        let mut app = make_app(&[r#"{"level":"INFO","msg":"hello"}"#]).await;
        app.execute_command_str("hide-field level".to_string())
            .await;
        assert!(app.tab().hidden_fields.contains("level"));
        app.execute_command_str("show-field level".to_string())
            .await;
        assert!(!app.tab().hidden_fields.contains("level"));
    }

    #[tokio::test]
    async fn test_show_all_fields_clears_everything() {
        let mut app = make_app(&[r#"{"level":"INFO","msg":"hello"}"#]).await;
        app.execute_command_str("hide-field msg".to_string()).await;
        app.execute_command_str("hide-field level".to_string())
            .await;
        assert!(!app.tab().hidden_fields.is_empty());
        app.execute_command_str("show-all-fields".to_string()).await;
        assert!(app.tab().hidden_fields.is_empty());
    }
}
