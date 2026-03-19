use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{Terminal, prelude::*};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::{Keybindings, RestoreSessionPolicy};
use crate::db::{AppSettingsStore, FileContext, FileContextStore, SessionStore};
use crate::file_reader::FileReader;
use crate::log_manager::LogManager;
use crate::mode::app_mode::{ConfirmRestoreMode, ConfirmRestoreSessionMode};
use crate::mode::command_mode::CommandMode;
use crate::mode::filter_mode::FilterManagementMode;
use crate::mode::normal_mode::NormalMode;
use crate::theme::Theme;

use super::{FileLoadState, KeyResult, StdinLoadState, TabState};

async fn resolve_bool_setting(
    db: &crate::db::Database,
    key: &str,
    config_override: Option<bool>,
    default: bool,
) -> bool {
    if let Some(v) = config_override {
        return v;
    }
    if let Ok(Some(val)) = db.load_app_setting(key).await {
        return val == "true";
    }
    default
}

async fn resolve_policy_setting(
    db: &crate::db::Database,
    key: &str,
    config_override: Option<RestoreSessionPolicy>,
) -> RestoreSessionPolicy {
    if let Some(v) = config_override {
        return v;
    }
    if let Ok(Some(val)) = db.load_app_setting(key).await {
        return match val.as_str() {
            "always" => RestoreSessionPolicy::Always,
            "never" => RestoreSessionPolicy::Never,
            _ => RestoreSessionPolicy::Ask,
        };
    }
    RestoreSessionPolicy::Ask
}

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
    /// Persistent clipboard instance — kept alive so clipboard managers can read the contents.
    pub clipboard: Option<arboard::Clipboard>,
    /// Global mode bar visibility — applies to all tabs uniformly.
    pub show_mode_bar: bool,
    /// Default show_borders value applied to new tabs.
    pub show_borders_default: bool,
    /// Default show_line_numbers value applied to new tabs.
    pub show_line_numbers: bool,
    /// Default show_sidebar value applied to new tabs.
    pub show_sidebar: bool,
    /// Default wrap value applied to new tabs.
    pub wrap: bool,
    /// When true, the initial file tab starts in tail mode (set by `--tail`).
    pub startup_tail: bool,
    /// When true, filters were supplied via `--filters` and the previous-session
    /// restore prompt must be suppressed so it cannot overwrite them.
    pub startup_filters: bool,
    /// Number of bytes to read for the instant preview (from config `preview_bytes`).
    pub preview_bytes: u64,
    /// Restore session policy from config — controls whether to skip the whole-session prompt.
    pub restore_policy: RestoreSessionPolicy,
    /// Restore file-context policy from config — controls whether to skip the per-file prompt.
    pub restore_file_policy: RestoreSessionPolicy,
    /// Session files to restore automatically (set when policy is Always and session exists).
    pub pending_session_restore: Option<Vec<String>>,
    /// Keybinding conflict warnings shown in the status bar on startup. Cleared on first keypress.
    pub startup_warnings: Vec<String>,
    /// Configured DLT devices from config.json.
    pub dlt_devices: Vec<crate::config::DltDevice>,
    /// Default MCP server port from config (used when `:enable-mcp` has no `--port`).
    pub mcp_port: Option<u16>,
    /// Shared snapshot of filtered lines, marks, and annotations exposed to the MCP server.
    pub mcp_snapshot: std::sync::Arc<tokio::sync::RwLock<crate::mcp::McpSnapshot>>,
    /// Receiver for commands sent by the MCP server tools back to the TUI.
    pub mcp_cmd_rx: Option<tokio::sync::mpsc::Receiver<crate::mcp::McpCommand>>,
    /// Handle to the running MCP server, if one is active.
    pub mcp_server_handle: Option<crate::mcp::McpServerHandle>,
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
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        log_manager: LogManager,
        file_reader: FileReader,
        theme: Theme,
        keybindings: Arc<Keybindings>,
        restore_policy: Option<RestoreSessionPolicy>,
        restore_file_policy: Option<RestoreSessionPolicy>,
        show_mode_bar: Option<bool>,
        show_borders: Option<bool>,
        show_line_numbers: Option<bool>,
        show_sidebar: Option<bool>,
        wrap: Option<bool>,
    ) -> App {
        let db = log_manager.db.clone();

        let restore_policy = resolve_policy_setting(&db, "restore_session", restore_policy).await;
        let restore_file_policy =
            resolve_policy_setting(&db, "restore_file_context", restore_file_policy).await;
        let show_mode_bar = resolve_bool_setting(&db, "show_mode_bar", show_mode_bar, true).await;
        let show_borders_default =
            resolve_bool_setting(&db, "show_borders", show_borders, false).await;
        let show_line_numbers =
            resolve_bool_setting(&db, "show_line_numbers", show_line_numbers, true).await;
        let show_sidebar = resolve_bool_setting(&db, "show_sidebar", show_sidebar, true).await;
        let wrap = resolve_bool_setting(&db, "wrap", wrap, false).await;

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
        tab.interaction.keybindings = keybindings.clone();
        tab.display.show_mode_bar = show_mode_bar;
        tab.display.show_borders = show_borders_default;
        tab.display.show_line_numbers = show_line_numbers;
        tab.display.show_sidebar = show_sidebar;
        tab.display.wrap = wrap;
        let mut pending_session_restore: Option<Vec<String>> = None;

        // Check for saved context only when we have real data (not a placeholder
        // that will be replaced by a background load started after App::new).
        if let Some(source) = tab.log_manager.source_file().map(|s| s.to_string())
            && tab.file_reader.line_count() > 0
            && let Ok(Some(ctx)) = db.load_file_context(&source).await
        {
            match restore_file_policy {
                RestoreSessionPolicy::Always => {
                    tab.apply_file_context(&ctx);
                }
                RestoreSessionPolicy::Never => {}
                RestoreSessionPolicy::Ask => {
                    tab.interaction.mode = Box::new(ConfirmRestoreMode { context: ctx });
                }
            }
        } else if no_source && no_data {
            // No file argument and no piped data — offer to restore last session.
            if let Ok(files) = db.load_session().await
                && !files.is_empty()
            {
                match restore_policy {
                    RestoreSessionPolicy::Never => {}
                    RestoreSessionPolicy::Ask => {
                        tab.interaction.mode = Box::new(ConfirmRestoreSessionMode { files });
                    }
                    RestoreSessionPolicy::Always => {
                        pending_session_restore = Some(files);
                    }
                }
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
            clipboard: None,
            show_mode_bar,
            show_borders_default,
            show_line_numbers,
            show_sidebar,
            wrap,
            startup_tail: false,
            startup_filters: false,
            preview_bytes: 16 * 1024 * 1024,
            restore_policy,
            restore_file_policy,
            pending_session_restore,
            startup_warnings: vec![],
            dlt_devices: vec![],
            mcp_port: None,
            mcp_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(
                crate::mcp::McpSnapshot::default(),
            )),
            mcp_cmd_rx: None,
            mcp_server_handle: None,
        }
    }

    /// Apply shared defaults (keybindings, display toggles) to a new tab.
    #[inline]
    pub(super) fn apply_tab_defaults(&self, tab: &mut TabState) {
        tab.interaction.keybindings = self.keybindings.clone();
        tab.display.show_mode_bar = self.show_mode_bar;
        tab.display.show_borders = self.show_borders_default;
        tab.display.show_line_numbers = self.show_line_numbers;
        tab.display.show_sidebar = self.show_sidebar;
        tab.display.wrap = self.wrap;
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
        use std::sync::atomic::Ordering;
        self.save_tab_context(&self.tabs[self.active_tab]).await;

        let tab = &self.tabs[self.active_tab];
        if let Some(ref h) = tab.search.handle {
            h.cancel.store(true, Ordering::Relaxed);
        }
        if let Some(ref h) = tab.filter.handle {
            h.cancel.store(true, Ordering::Relaxed);
        }

        // Cancel the background file load if it belongs to this tab.
        if let Some(ref fls) = self.file_load_state
            && (matches!(&fls.on_complete,
                    super::LoadContext::ReplaceTab { tab_idx } if *tab_idx == self.active_tab)
                || matches!(&fls.on_complete, super::LoadContext::ReplaceInitialTab if self.active_tab == 0)
                || matches!(&fls.on_complete,
                    super::LoadContext::SessionRestoreTab { tab_idx, .. } if *tab_idx == self.active_tab))
        {
            fls.cancel.store(true, Ordering::Relaxed);
        }

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
            let history = self.tabs[self.active_tab]
                .interaction
                .command_history
                .clone();
            self.tabs[self.active_tab].interaction.command_error = None;
            self.tabs[self.active_tab].interaction.mode =
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
                    tab.interaction.command_history.push(cmd.trim().to_string());
                }
                if !mode_was_set {
                    if let Some(idx) = tab.filter.filter_context.take() {
                        tab.interaction.mode = Box::new(FilterManagementMode {
                            selected_filter_index: idx,
                        });
                    } else {
                        tab.interaction.mode = Box::new(NormalMode::default());
                    }
                }
            }
            Err(msg) => {
                tab.interaction.command_error = Some(msg);
                let history = tab.interaction.command_history.clone();
                let cmd_len = cmd.len();
                tab.interaction.mode = Box::new(CommandMode {
                    input: cmd,
                    cursor: cmd_len,
                    history,
                    history_index: None,
                    completion_index: None,
                    completion_query: None,
                });
            }
        }
    }

    pub async fn run<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> anyhow::Result<()>
    where
        <B as Backend>::Error: Send + Sync + 'static,
    {
        if let Some(files) = self.pending_session_restore.take() {
            self.restore_session(files).await;
        }

        let mut last_tick = Instant::now();
        let tick_rate = Duration::from_millis(250);

        loop {
            terminal.draw(|frame| self.ui(frame))?;

            // Poll for background load completion, file watch updates, search, and filter.
            self.advance_file_load().await;
            self.advance_stdin_load().await;
            self.advance_file_watches();
            self.advance_stream_retries();
            self.advance_search();
            self.advance_filter_computation();

            let poll_timeout = tick_rate
                .checked_sub(last_tick.elapsed())
                .unwrap_or_else(|| Duration::from_secs(0));

            if crossterm::event::poll(poll_timeout)?
                && let crossterm::event::Event::Key(key) = crossterm::event::read()?
                && key.kind == crossterm::event::KeyEventKind::Press
            {
                self.startup_warnings.clear();
                let tab = &mut self.tabs[self.active_tab];
                let mode =
                    std::mem::replace(&mut tab.interaction.mode, Box::new(NormalMode::default()));
                let (next_mode, result) = mode.handle_key(tab, key.code, key.modifiers).await;
                tab.interaction.mode = next_mode;
                self.dispatch_key_result(result, key.code, key.modifiers)
                    .await;
                self.refresh_mcp_snapshot();
            }

            self.poll_mcp_commands().await;

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
        self.startup_warnings.clear();
        let tab = &mut self.tabs[self.active_tab];
        let mode = std::mem::replace(&mut tab.interaction.mode, Box::new(NormalMode::default()));
        let (next_mode, result) = mode.handle_key(tab, key_code, modifiers).await;
        tab.interaction.mode = next_mode;
        self.dispatch_key_result(result, key_code, modifiers).await;
    }

    async fn dispatch_key_result(
        &mut self,
        result: KeyResult,
        key_code: KeyCode,
        modifiers: KeyModifiers,
    ) {
        match result {
            KeyResult::Handled => {}
            KeyResult::Ignored => self.handle_global_key(key_code, modifiers).await,
            KeyResult::ExecuteCommand(cmd) => self.execute_command_str(cmd).await,
            KeyResult::RestoreSession(files) => self.restore_session(files).await,
            KeyResult::DockerAttach(id, name) => self.open_docker_logs(id, name).await,
            KeyResult::DltAttach(host, port, name) => self.open_dlt_stream(host, port, name).await,
            KeyResult::ApplyValueColors(disabled) => {
                self.theme.value_colors.disabled = disabled;
                for tab in &mut self.tabs {
                    tab.cache.render_gen = tab.cache.render_gen.wrapping_add(1);
                    tab.cache.render_line.clear();
                }
            }
            KeyResult::ApplyLevelColors(disabled) => {
                self.tabs[self.active_tab].display.level_colors_disabled = disabled;
            }
            KeyResult::CopyToClipboard(text) => self.copy_to_clipboard(text),
            KeyResult::ToggleModeBar => {
                self.show_mode_bar = !self.show_mode_bar;
                for tab in &mut self.tabs {
                    tab.display.show_mode_bar = self.show_mode_bar;
                }
                let _ = self
                    .db
                    .save_app_setting(
                        "show_mode_bar",
                        if self.show_mode_bar { "true" } else { "false" },
                    )
                    .await;
            }
            KeyResult::OpenFiles(paths) => {
                for path in paths {
                    if let Err(e) = self.open_file(&path).await {
                        self.tabs[self.active_tab].interaction.command_error = Some(e);
                        break;
                    }
                }
            }
            KeyResult::AlwaysRestoreFile(_) => {
                self.restore_file_policy = RestoreSessionPolicy::Always;
                let _ = self
                    .db
                    .save_app_setting("restore_file_context", "always")
                    .await;
            }
            KeyResult::NeverRestoreFile => {
                self.restore_file_policy = RestoreSessionPolicy::Never;
                let _ = self
                    .db
                    .save_app_setting("restore_file_context", "never")
                    .await;
            }
            KeyResult::AlwaysRestoreSession(files) => {
                self.restore_policy = RestoreSessionPolicy::Always;
                let _ = self.db.save_app_setting("restore_session", "always").await;
                self.restore_session(files).await;
            }
            KeyResult::NeverRestoreSession => {
                self.restore_policy = RestoreSessionPolicy::Never;
                let _ = self.db.save_app_setting("restore_session", "never").await;
            }
        }
    }

    fn copy_to_clipboard(&mut self, text: String) {
        let tab = &mut self.tabs[self.active_tab];
        let line_count = text.lines().count();

        // Lazily initialize the clipboard, keeping it alive for the session so
        // clipboard managers on Linux have time to read the contents.
        if self.clipboard.is_none() {
            match arboard::Clipboard::new() {
                Ok(cb) => self.clipboard = Some(cb),
                Err(e) => {
                    tab.interaction.command_error = Some(format!("Failed to copy: {}", e));
                    return;
                }
            }
        }
        let cb = self.clipboard.as_mut().unwrap();
        match cb.set_text(text) {
            Ok(()) => {
                tab.interaction.command_error = Some(format!(
                    "{} line{} copied to clipboard",
                    line_count,
                    if line_count == 1 { "" } else { "s" }
                ));
            }
            Err(e) => {
                tab.interaction.command_error = Some(format!("Failed to copy: {}", e));
            }
        }
    }

    pub async fn start_mcp(&mut self, port: u16) -> std::io::Result<()> {
        self.stop_mcp();
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        self.refresh_mcp_snapshot();
        let handle = crate::mcp::start_mcp_server(port, self.mcp_snapshot.clone(), tx).await?;
        self.mcp_server_handle = Some(handle);
        self.mcp_cmd_rx = Some(rx);
        Ok(())
    }

    pub fn stop_mcp(&mut self) {
        if let Some(handle) = self.mcp_server_handle.take() {
            handle.cancel.cancel();
        }
        self.mcp_cmd_rx = None;
    }

    pub fn refresh_mcp_snapshot(&mut self) {
        if self.mcp_server_handle.is_none() {
            return;
        }
        if let Some(tab) = self.tabs.get(self.active_tab) {
            let snapshot = crate::mcp::McpSnapshot {
                marked_lines: crate::mcp::build_marked_lines(&tab.file_reader, &tab.log_manager),
                annotations: crate::mcp::build_annotations(&tab.log_manager),
            };
            let arc = self.mcp_snapshot.clone();
            tokio::spawn(async move {
                *arc.write().await = snapshot;
            });
        }
    }

    pub async fn poll_mcp_commands(&mut self) {
        if self.mcp_cmd_rx.is_none() {
            return;
        }
        let mut cmds = Vec::new();
        if let Some(rx) = &mut self.mcp_cmd_rx {
            while let Ok(cmd) = rx.try_recv() {
                cmds.push(cmd);
            }
        }
        for cmd in cmds {
            self.handle_mcp_command(cmd).await;
        }
    }

    async fn handle_mcp_command(&mut self, cmd: crate::mcp::McpCommand) {
        use crate::mcp::McpCommand;
        if self.tabs.is_empty() {
            return;
        }
        match cmd {
            McpCommand::ToggleMark(idx) => {
                self.tabs[self.active_tab].log_manager.toggle_mark(idx);
                self.save_tab_context(&self.tabs[self.active_tab]).await;
                self.refresh_mcp_snapshot();
            }
            McpCommand::AddAnnotation { text, line_indices } => {
                self.tabs[self.active_tab]
                    .log_manager
                    .add_comment(text, line_indices);
                self.save_tab_context(&self.tabs[self.active_tab]).await;
                self.refresh_mcp_snapshot();
            }
            McpCommand::RemoveAnnotation(idx) => {
                self.tabs[self.active_tab].log_manager.remove_comment(idx);
                self.save_tab_context(&self.tabs[self.active_tab]).await;
                self.refresh_mcp_snapshot();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auto_complete::shell_split;
    use crate::config::{Keybindings, RestoreSessionPolicy};
    use crate::db::Database;
    use crate::file_reader::FileReader;
    use crate::log_manager::LogManager;
    use crate::types::FilterType;
    use std::sync::Arc;

    /// Awaits all pending background filter computations across all tabs.
    /// Use in tests after triggering filter commands so visible_indices is up-to-date.
    async fn await_filter_computations(app: &mut App) {
        for tab in &mut app.tabs {
            if let Some(mut h) = tab.filter.handle.take() {
                let scroll_anchor = h.scroll_anchor;
                let mut all_visible = Vec::new();
                let mut final_counts = None;
                while let Some(chunk) = h.result_rx.recv().await {
                    all_visible.extend(chunk.visible);
                    if chunk.is_last {
                        final_counts = chunk.filter_match_counts;
                        break;
                    }
                }
                tab.filter.visible_indices = crate::ui::VisibleLines::Filtered(all_visible);
                if let Some(counts) = final_counts {
                    tab.filter.match_counts = counts;
                }
                if let Some(line_idx) = scroll_anchor {
                    if let Some(pos) = tab.filter.visible_indices.position_of(line_idx) {
                        tab.scroll.scroll_offset = pos;
                        continue;
                    }
                }
                if tab.filter.visible_indices.is_empty() {
                    tab.scroll.scroll_offset = 0;
                } else {
                    tab.scroll.scroll_offset = tab
                        .scroll
                        .scroll_offset
                        .min(tab.filter.visible_indices.len() - 1);
                }
            }
        }
    }

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
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
    }

    #[tokio::test]
    async fn test_toggle_wrap_command() {
        let mut app = make_app(&["INFO something", "WARN warning", "ERROR error"]).await;
        app.execute_command_str("wrap".to_string()).await;
        assert!(app.tab().display.wrap);
        app.execute_command_str("wrap".to_string()).await;
        assert!(!app.tab().display.wrap);
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
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;

        assert_eq!(app.tab().filter.visible_indices.len(), 3);

        app.execute_command_str("filter INFO".to_string()).await;

        assert_eq!(app.tab().filter.visible_indices.len(), 1);
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
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;

        app.tab_mut().scroll.scroll_offset = 0;
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
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;

        // 'G' goes to end
        app.handle_key_event_with_modifiers(KeyCode::Char('G'), KeyModifiers::NONE)
            .await;
        assert_eq!(app.tab().scroll.scroll_offset, 19);

        // 'gg' goes to top
        app.handle_key_event_with_modifiers(KeyCode::Char('g'), KeyModifiers::NONE)
            .await;
        app.handle_key_event_with_modifiers(KeyCode::Char('g'), KeyModifiers::NONE)
            .await;
        assert_eq!(app.tab().scroll.scroll_offset, 0);
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
        assert_eq!(app.tab().filter.visible_indices.len(), 3);
    }

    #[tokio::test]
    async fn test_disable_filters_command() {
        let mut app = make_app(&["INFO a", "WARN b", "ERROR c"]).await;
        app.execute_command_str("filter INFO".to_string()).await;
        assert_eq!(app.tab().filter.visible_indices.len(), 1);

        app.execute_command_str("disable-filters".to_string()).await;
        assert!(!app.tab().log_manager.get_filters()[0].enabled);
        assert_eq!(app.tab().filter.visible_indices.len(), 3);
    }

    #[tokio::test]
    async fn test_enable_filters_command() {
        let mut app = make_app(&["INFO a", "WARN b", "ERROR c"]).await;
        app.execute_command_str("filter INFO".to_string()).await;
        app.execute_command_str("disable-filters".to_string()).await;
        assert!(!app.tab().log_manager.get_filters()[0].enabled);

        app.execute_command_str("enable-filters".to_string()).await;
        assert!(app.tab().log_manager.get_filters()[0].enabled);
        await_filter_computations(&mut app).await;
        assert_eq!(app.tab().filter.visible_indices.len(), 1);
    }

    #[tokio::test]
    async fn test_filtering_command_toggles_bypass() {
        let mut app = make_app(&["INFO a", "WARN b", "ERROR c"]).await;
        app.execute_command_str("filter INFO".to_string()).await;
        assert_eq!(app.tab().filter.visible_indices.len(), 1);
        assert!(app.tab().filter.enabled);

        app.execute_command_str("filtering".to_string()).await;
        assert!(!app.tab().filter.enabled);
        assert_eq!(app.tab().filter.visible_indices.len(), 3);

        app.execute_command_str("filtering".to_string()).await;
        assert!(app.tab().filter.enabled);
        await_filter_computations(&mut app).await;
        assert_eq!(app.tab().filter.visible_indices.len(), 1);
    }

    #[tokio::test]
    async fn test_hide_field_by_name() {
        let mut app = make_app(&[r#"{"level":"INFO","msg":"hello"}"#]).await;
        assert!(app.tab().display.hidden_fields.is_empty());
        app.execute_command_str("hide-field msg".to_string()).await;
        assert!(app.tab().display.hidden_fields.contains("msg"));
        assert!(!app.tab().display.hidden_fields.contains("level"));
    }

    #[tokio::test]
    async fn test_hide_field_by_index_resolves_to_visible_field() {
        let mut app = make_app(&[r#"{"level":"INFO","msg":"hello"}"#]).await;
        let first_visible = app
            .tab_mut()
            .collect_field_names()
            .into_iter()
            .find(|n| !app.tab().display.hidden_fields.contains(n.as_str()))
            .unwrap();
        app.execute_command_str("hide-field 0".to_string()).await;
        assert!(
            app.tab().display.hidden_fields.contains(&first_visible),
            "hide-field 0 should resolve to the first visible field '{first_visible}'"
        );
    }

    #[tokio::test]
    async fn test_show_field_removes_hidden_name() {
        let mut app = make_app(&[r#"{"level":"INFO","msg":"hello"}"#]).await;
        app.execute_command_str("hide-field msg".to_string()).await;
        assert!(app.tab().display.hidden_fields.contains("msg"));
        app.execute_command_str("show-field msg".to_string()).await;
        assert!(!app.tab().display.hidden_fields.contains("msg"));
    }

    #[tokio::test]
    async fn test_show_field_removes_hidden_string() {
        let mut app = make_app(&[r#"{"level":"INFO","msg":"hello"}"#]).await;
        app.execute_command_str("hide-field level".to_string())
            .await;
        assert!(app.tab().display.hidden_fields.contains("level"));
        app.execute_command_str("show-field level".to_string())
            .await;
        assert!(!app.tab().display.hidden_fields.contains("level"));
    }

    #[tokio::test]
    async fn test_show_all_fields_clears_everything() {
        let mut app = make_app(&[r#"{"level":"INFO","msg":"hello"}"#]).await;
        app.execute_command_str("hide-field msg".to_string()).await;
        app.execute_command_str("hide-field level".to_string())
            .await;
        assert!(!app.tab().display.hidden_fields.is_empty());
        app.execute_command_str("show-all-fields".to_string()).await;
        assert!(app.tab().display.hidden_fields.is_empty());
    }

    #[tokio::test]
    async fn test_set_color_preserves_match_only() {
        let mut app = make_app(&["INFO something", "WARN warning", "ERROR error"]).await;
        // Create a filter with full-line mode (-l → match_only=false).
        app.execute_command_str("filter INFO --fg red -l".to_string())
            .await;
        let filters = app.tab().log_manager.get_filters();
        assert!(!filters[0].color_config.as_ref().unwrap().match_only);

        // Enter filter management mode so filter_context is set.
        app.tab_mut().filter.filter_context = Some(0);

        // Change color without -l flag — match_only=false should be preserved.
        app.execute_command_str("set-color --fg blue".to_string())
            .await;
        let filters = app.tab().log_manager.get_filters();
        let cc = filters[0].color_config.as_ref().unwrap();
        assert!(
            !cc.match_only,
            "match_only should be preserved when -l is not passed"
        );
        assert_eq!(cc.fg, Some(ratatui::style::Color::Blue));
    }

    // ── close_tab ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_close_tab_single_tab_returns_true() {
        let mut app = make_app(&["line"]).await;
        let should_quit = app.close_tab().await;
        assert!(should_quit);
    }

    #[tokio::test]
    async fn test_close_tab_multiple_tabs_returns_false() {
        let mut app = make_app(&["line"]).await;
        let data: Vec<u8> = b"tab2\n".to_vec();
        let fr = FileReader::from_bytes(data);
        let lm = LogManager::new(app.db.clone(), None).await;
        let mut t = super::super::TabState::new(fr, lm, "tab2".to_string());
        t.interaction.keybindings = app.keybindings.clone();
        app.tabs.push(t);

        let should_quit = app.close_tab().await;
        assert!(!should_quit);
        assert_eq!(app.tabs.len(), 1);
    }

    #[tokio::test]
    async fn test_close_tab_clamps_active_tab_index() {
        let mut app = make_app(&["line"]).await;
        for _ in 0..2 {
            let data: Vec<u8> = b"extra\n".to_vec();
            let fr = FileReader::from_bytes(data);
            let lm = LogManager::new(app.db.clone(), None).await;
            let mut t = super::super::TabState::new(fr, lm, "extra".to_string());
            t.interaction.keybindings = app.keybindings.clone();
            app.tabs.push(t);
        }
        app.active_tab = 2; // last tab
        let should_quit = app.close_tab().await;
        assert!(!should_quit);
        assert!(app.active_tab < app.tabs.len());
    }

    // ── handle_global_key ───────────────────────────────────────────────

    #[tokio::test]
    async fn test_handle_global_key_quit() {
        let mut app = make_app(&["line"]).await;
        app.handle_global_key(KeyCode::Char('q'), KeyModifiers::NONE)
            .await;
        assert!(app.should_quit);
    }

    #[tokio::test]
    async fn test_handle_global_key_next_tab() {
        let mut app = make_app(&["line"]).await;
        let data: Vec<u8> = b"tab2\n".to_vec();
        let fr = FileReader::from_bytes(data);
        let lm = LogManager::new(app.db.clone(), None).await;
        let mut t = super::super::TabState::new(fr, lm, "tab2".to_string());
        t.interaction.keybindings = app.keybindings.clone();
        app.tabs.push(t);

        assert_eq!(app.active_tab, 0);
        app.handle_global_key(KeyCode::Tab, KeyModifiers::NONE)
            .await;
        assert_eq!(app.active_tab, 1);
        app.handle_global_key(KeyCode::Tab, KeyModifiers::NONE)
            .await;
        assert_eq!(app.active_tab, 0); // wraps around
    }

    #[tokio::test]
    async fn test_handle_global_key_prev_tab() {
        let mut app = make_app(&["line"]).await;
        let data: Vec<u8> = b"tab2\n".to_vec();
        let fr = FileReader::from_bytes(data);
        let lm = LogManager::new(app.db.clone(), None).await;
        let mut t = super::super::TabState::new(fr, lm, "tab2".to_string());
        t.interaction.keybindings = app.keybindings.clone();
        app.tabs.push(t);

        assert_eq!(app.active_tab, 0);
        app.handle_global_key(KeyCode::BackTab, KeyModifiers::NONE)
            .await;
        assert_eq!(app.active_tab, 1); // wraps to end
    }

    #[tokio::test]
    async fn test_handle_global_key_close_last_tab_quits() {
        let mut app = make_app(&["line"]).await;
        app.handle_global_key(KeyCode::Char('w'), KeyModifiers::CONTROL)
            .await;
        assert!(app.should_quit);
    }

    #[tokio::test]
    async fn test_handle_global_key_new_tab() {
        use crate::mode::app_mode::ModeRenderState;
        let mut app = make_app(&["line"]).await;
        app.handle_global_key(KeyCode::Char('t'), KeyModifiers::CONTROL)
            .await;
        // Should enter command mode with "open " prefilled
        match app.tabs[0].interaction.mode.render_state() {
            ModeRenderState::Command { input, .. } => {
                assert!(input.starts_with("open "));
            }
            other => panic!("expected Command mode, got {:?}", other),
        }
    }

    // ── execute_command_str ─────────────────────────────────────────────

    #[tokio::test]
    async fn test_execute_command_str_success_pushes_history() {
        let mut app = make_app(&["INFO a", "WARN b"]).await;
        app.execute_command_str("wrap".to_string()).await;
        assert!(
            app.tab()
                .interaction
                .command_history
                .contains(&"wrap".to_string())
        );
    }

    #[tokio::test]
    async fn test_execute_command_str_success_normal_mode() {
        use crate::mode::app_mode::ModeRenderState;
        let mut app = make_app(&["line"]).await;
        app.execute_command_str("wrap".to_string()).await;
        assert!(matches!(
            app.tab().interaction.mode.render_state(),
            ModeRenderState::Normal
        ));
    }

    #[tokio::test]
    async fn test_execute_command_str_failure_sets_error() {
        use crate::mode::app_mode::ModeRenderState;
        let mut app = make_app(&["line"]).await;
        app.execute_command_str("nonexistent-cmd".to_string()).await;
        assert!(app.tab().interaction.command_error.is_some());
        assert!(matches!(
            app.tab().interaction.mode.render_state(),
            ModeRenderState::Command { .. }
        ));
    }

    #[tokio::test]
    async fn test_execute_command_str_with_filter_context() {
        use crate::mode::app_mode::ModeRenderState;
        let mut app = make_app(&["INFO a", "WARN b"]).await;
        app.execute_command_str("filter INFO".to_string()).await;
        app.tab_mut().filter.filter_context = Some(0);
        app.execute_command_str("set-color --fg red".to_string())
            .await;
        // After success with filter_context, should return to FilterManagement
        assert!(matches!(
            app.tab().interaction.mode.render_state(),
            ModeRenderState::FilterManagement { .. }
        ));
    }

    // ── save_all_contexts ───────────────────────────────────────────────

    #[tokio::test]
    async fn test_save_all_contexts_no_source_files() {
        let app = make_app(&["line"]).await;
        // Should not panic with no source files
        app.save_all_contexts().await;
    }

    // ── tab() / tab_mut() ───────────────────────────────────────────────

    #[tokio::test]
    async fn test_tab_accessors() {
        let mut app = make_app(&["line"]).await;
        assert_eq!(app.tab().title, "stdin");
        app.tab_mut().title = "modified".to_string();
        assert_eq!(app.tab().title, "modified");
    }

    #[tokio::test]
    async fn test_app_new_with_empty_file() {
        let app = make_app(&[]).await;
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.active_tab, 0);
        assert_eq!(app.tab().filter.visible_indices.len(), 0);
    }

    #[tokio::test]
    async fn test_empty_command_no_history_push() {
        let mut app = make_app(&["line"]).await;
        app.execute_command_str("  ".to_string()).await;
        assert!(app.tab().interaction.command_history.is_empty());
    }

    #[tokio::test]
    async fn test_app_debug_impl() {
        let app = make_app(&["line"]).await;
        let debug = format!("{:?}", app);
        assert!(debug.contains("active_tab"));
        assert!(debug.contains("num_tabs"));
    }

    #[tokio::test]
    async fn test_save_tab_context_no_source() {
        let app = make_app(&["line"]).await;
        // No source file — save_tab_context should be a no-op (no panic).
        let tab = &app.tabs[0];
        app.save_tab_context(tab).await;
    }

    #[tokio::test]
    async fn test_open_files_key_result_opens_new_tabs() {
        let mut app = make_app(&["line"]).await;
        // Create real temp files so open_file succeeds.
        let tmp = tempfile::tempdir().unwrap();
        let path_a = tmp.path().join("a.log");
        let path_b = tmp.path().join("b.log");
        std::fs::write(&path_a, b"aaa\n").unwrap();
        std::fs::write(&path_b, b"bbb\n").unwrap();
        let paths = vec![
            path_a.to_str().unwrap().to_string(),
            path_b.to_str().unwrap().to_string(),
        ];
        // Simulate the OpenFiles result being handled.
        for path in &paths {
            app.open_file(path).await.unwrap();
        }
        assert_eq!(app.tabs.len(), 3); // initial + 2 new
    }

    #[tokio::test]
    async fn test_save_tab_context_with_source() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let data: Vec<u8> = b"hello\nworld\n".to_vec();
        let fr = FileReader::from_bytes(data);
        let lm = LogManager::new(db.clone(), Some("test.log".to_string())).await;
        let app = App::new(
            lm,
            fr,
            Theme::default(),
            Arc::new(Keybindings::default()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        let tab = &app.tabs[0];
        app.save_tab_context(tab).await;
        // Verify it was saved
        let ctx = db.load_file_context("test.log").await.unwrap();
        assert!(ctx.is_some());
    }

    #[tokio::test]
    async fn test_save_all_contexts_with_source_files() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let data: Vec<u8> = b"hello\n".to_vec();
        let fr = FileReader::from_bytes(data);
        let lm = LogManager::new(db.clone(), Some("test.log".to_string())).await;
        let app = App::new(
            lm,
            fr,
            Theme::default(),
            Arc::new(Keybindings::default()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        app.save_all_contexts().await;
        // Session should be saved
        let files = db.load_session().await.unwrap();
        assert!(!files.is_empty());
    }

    #[tokio::test]
    async fn test_handle_global_key_prev_tab_non_wrapping() {
        let mut app = make_app(&["line"]).await;
        // Add two more tabs so we can test non-wrapping prev
        for _ in 0..2 {
            let data: Vec<u8> = b"extra\n".to_vec();
            let fr = FileReader::from_bytes(data);
            let lm = LogManager::new(app.db.clone(), None).await;
            let mut t = super::super::TabState::new(fr, lm, "extra".to_string());
            t.interaction.keybindings = app.keybindings.clone();
            app.tabs.push(t);
        }
        app.active_tab = 2;
        app.handle_global_key(KeyCode::BackTab, KeyModifiers::NONE)
            .await;
        assert_eq!(app.active_tab, 1); // non-wrapping: 2 -> 1
    }

    #[tokio::test]
    async fn test_handle_key_event() {
        use crate::mode::app_mode::ModeRenderState;
        let mut app = make_app(&["line"]).await;
        // Press ':' to enter command mode
        app.handle_key_event(KeyCode::Char(':')).await;
        assert!(matches!(
            app.tab().interaction.mode.render_state(),
            ModeRenderState::Command { .. }
        ));
    }

    #[tokio::test]
    async fn test_handle_key_event_with_modifiers() {
        let mut app = make_app(&["line"]).await;
        // Ctrl+W should quit when single tab
        app.handle_key_event_with_modifiers(KeyCode::Char('w'), KeyModifiers::CONTROL)
            .await;
        assert!(app.should_quit);
    }

    #[tokio::test]
    async fn test_handle_global_key_close_tab_with_multiple() {
        let mut app = make_app(&["line"]).await;
        let data: Vec<u8> = b"tab2\n".to_vec();
        let fr = FileReader::from_bytes(data);
        let lm = LogManager::new(app.db.clone(), None).await;
        let mut t = super::super::TabState::new(fr, lm, "tab2".to_string());
        t.interaction.keybindings = app.keybindings.clone();
        app.tabs.push(t);

        // Ctrl+W with 2 tabs should close tab, not quit
        app.handle_global_key(KeyCode::Char('w'), KeyModifiers::CONTROL)
            .await;
        assert!(!app.should_quit);
        assert_eq!(app.tabs.len(), 1);
    }

    #[tokio::test]
    async fn test_app_new_with_source_file() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let data: Vec<u8> = b"hello\nworld\n".to_vec();
        let fr = FileReader::from_bytes(data);
        let lm = LogManager::new(db, Some("/tmp/test.log".to_string())).await;
        let app = App::new(
            lm,
            fr,
            Theme::default(),
            Arc::new(Keybindings::default()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(app.tab().title, "test.log");
    }

    fn make_file_context() -> crate::db::FileContext {
        crate::db::FileContext {
            source_file: "/tmp/test.log".to_string(),
            scroll_offset: 0,
            search_query: String::new(),
            level_colors_disabled: std::collections::HashSet::new(),
            horizontal_scroll: 0,
            marked_lines: vec![],
            file_hash: None,
            comments: vec![],
            show_keys: false,
            raw_mode: false,
            sidebar_width: 30,
            hidden_fields: std::collections::HashSet::new(),
            field_layout_columns: None,
            filtering_enabled: true,
        }
    }

    #[tokio::test]
    async fn test_always_restore_file_key_sets_file_policy_not_session() {
        let mut app = make_app(&["line"]).await;
        app.tabs[0].interaction.mode = Box::new(crate::mode::app_mode::ConfirmRestoreMode {
            context: make_file_context(),
        });

        app.handle_key_event_with_modifiers(KeyCode::Char('Y'), KeyModifiers::NONE)
            .await;

        assert_eq!(app.restore_file_policy, RestoreSessionPolicy::Always);
        assert_eq!(app.restore_policy, RestoreSessionPolicy::Ask);

        let file_setting = app
            .db
            .load_app_setting("restore_file_context")
            .await
            .unwrap();
        let session_setting = app.db.load_app_setting("restore_session").await.unwrap();
        assert_eq!(file_setting.as_deref(), Some("always"));
        // session policy not touched by the file-context key press
        assert!(session_setting.is_none());
    }

    #[tokio::test]
    async fn test_never_restore_file_key_sets_file_policy_not_session() {
        let mut app = make_app(&["line"]).await;
        app.tabs[0].interaction.mode = Box::new(crate::mode::app_mode::ConfirmRestoreMode {
            context: make_file_context(),
        });

        app.handle_key_event_with_modifiers(KeyCode::Char('N'), KeyModifiers::NONE)
            .await;

        assert_eq!(app.restore_file_policy, RestoreSessionPolicy::Never);
        assert_eq!(app.restore_policy, RestoreSessionPolicy::Ask);

        let file_setting = app
            .db
            .load_app_setting("restore_file_context")
            .await
            .unwrap();
        let session_setting = app.db.load_app_setting("restore_session").await.unwrap();
        assert_eq!(file_setting.as_deref(), Some("never"));
        // session policy not touched by the file-context key press
        assert!(session_setting.is_none());
    }

    #[tokio::test]
    async fn test_app_new_with_config_overrides() {
        let (file_reader, log_manager) = make_tab(&["line"]).await;
        let app = App::new(
            log_manager,
            file_reader,
            Theme::default(),
            Arc::new(Keybindings::default()),
            Some(RestoreSessionPolicy::Always),
            Some(RestoreSessionPolicy::Never),
            Some(false),
            Some(true),
            Some(false),
            Some(false),
            Some(true),
        )
        .await;
        assert_eq!(app.restore_policy, RestoreSessionPolicy::Always);
        assert_eq!(app.restore_file_policy, RestoreSessionPolicy::Never);
        assert!(!app.show_mode_bar);
        assert!(app.show_borders_default);
        assert!(!app.show_line_numbers);
        assert!(!app.show_sidebar);
        assert!(app.wrap);
    }

    #[tokio::test]
    async fn test_app_new_settings_from_db() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        db.save_app_setting("restore_session", "always")
            .await
            .unwrap();
        db.save_app_setting("restore_file_context", "never")
            .await
            .unwrap();
        db.save_app_setting("show_mode_bar", "false").await.unwrap();
        let fr = FileReader::from_bytes(vec![]);
        let lm = LogManager::new(db, None).await;
        let app = App::new(
            lm,
            fr,
            Theme::default(),
            Arc::new(Keybindings::default()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(app.restore_policy, RestoreSessionPolicy::Always);
        assert_eq!(app.restore_file_policy, RestoreSessionPolicy::Never);
        assert!(!app.show_mode_bar);
    }

    #[tokio::test]
    async fn test_dispatch_apply_value_colors() {
        let mut app = make_app(&["line"]).await;
        let gen_before = app.tabs[0].cache.render_gen;
        let disabled: std::collections::HashSet<String> =
            std::iter::once("http_get".to_string()).collect();
        app.dispatch_key_result(
            KeyResult::ApplyValueColors(disabled.clone()),
            KeyCode::Null,
            KeyModifiers::NONE,
        )
        .await;
        assert_eq!(app.theme.value_colors.disabled, disabled);
        assert_ne!(app.tabs[0].cache.render_gen, gen_before);
    }

    #[tokio::test]
    async fn test_dispatch_apply_level_colors() {
        let mut app = make_app(&["line"]).await;
        let disabled: std::collections::HashSet<String> =
            std::iter::once("info".to_string()).collect();
        app.dispatch_key_result(
            KeyResult::ApplyLevelColors(disabled.clone()),
            KeyCode::Null,
            KeyModifiers::NONE,
        )
        .await;
        assert_eq!(app.tabs[0].display.level_colors_disabled, disabled);
    }

    #[tokio::test]
    async fn test_dispatch_toggle_mode_bar() {
        let mut app = make_app(&["line"]).await;
        let initial = app.show_mode_bar;
        app.dispatch_key_result(KeyResult::ToggleModeBar, KeyCode::Null, KeyModifiers::NONE)
            .await;
        assert_eq!(app.show_mode_bar, !initial);
        for tab in &app.tabs {
            assert_eq!(tab.display.show_mode_bar, !initial);
        }
    }

    #[tokio::test]
    async fn test_dispatch_open_files_nonexistent_sets_error() {
        let mut app = make_app(&["line"]).await;
        let paths = vec!["/nonexistent/missing.log".to_string()];
        app.dispatch_key_result(
            KeyResult::OpenFiles(paths),
            KeyCode::Null,
            KeyModifiers::NONE,
        )
        .await;
        assert!(app.tabs[app.active_tab].interaction.command_error.is_some());
    }

    #[tokio::test]
    async fn test_dispatch_never_restore_session() {
        let mut app = make_app(&["line"]).await;
        app.dispatch_key_result(
            KeyResult::NeverRestoreSession,
            KeyCode::Null,
            KeyModifiers::NONE,
        )
        .await;
        assert_eq!(app.restore_policy, RestoreSessionPolicy::Never);
        let setting = app.db.load_app_setting("restore_session").await.unwrap();
        assert_eq!(setting.as_deref(), Some("never"));
    }

    #[tokio::test]
    async fn test_always_restore_session_via_key_event() {
        let mut app = make_app(&["line"]).await;
        app.tabs[0].interaction.mode = Box::new(crate::mode::app_mode::ConfirmRestoreSessionMode {
            files: vec!["/nonexistent/file.log".to_string()],
        });
        app.handle_key_event_with_modifiers(KeyCode::Char('Y'), KeyModifiers::SHIFT)
            .await;
        assert_eq!(app.restore_policy, RestoreSessionPolicy::Always);
        let setting = app.db.load_app_setting("restore_session").await.unwrap();
        assert_eq!(setting.as_deref(), Some("always"));
    }

    #[tokio::test]
    async fn test_never_restore_session_via_key_event() {
        let mut app = make_app(&["line"]).await;
        app.tabs[0].interaction.mode = Box::new(crate::mode::app_mode::ConfirmRestoreSessionMode {
            files: vec!["/nonexistent/file.log".to_string()],
        });
        app.handle_key_event_with_modifiers(KeyCode::Char('N'), KeyModifiers::SHIFT)
            .await;
        assert_eq!(app.restore_policy, RestoreSessionPolicy::Never);
    }

    #[tokio::test]
    async fn test_close_tab_with_active_search_handle() {
        use std::sync::atomic::Ordering;
        let mut app = make_app(&["line1", "line2"]).await;
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancel_clone = cancel.clone();
        let (_result_tx, result_rx) = tokio::sync::mpsc::channel(1);
        let (_prog_tx, prog_rx) = tokio::sync::watch::channel(0.0_f64);
        app.tabs[0].search.handle = Some(super::super::SearchHandle {
            result_rx,
            cancel: cancel_clone,
            progress_rx: prog_rx,
            pattern: "test".to_string(),
            forward: true,
            navigate: false,
        });
        app.close_tab().await;
        assert!(cancel.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn test_app_new_restore_policy_from_db_never() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        db.save_app_setting("restore_session", "never")
            .await
            .unwrap();
        db.save_app_setting("restore_file_context", "ask")
            .await
            .unwrap();
        let fr = FileReader::from_bytes(vec![]);
        let lm = LogManager::new(db, None).await;
        let app = App::new(
            lm,
            fr,
            Theme::default(),
            Arc::new(Keybindings::default()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(app.restore_policy, RestoreSessionPolicy::Never);
        assert_eq!(app.restore_file_policy, RestoreSessionPolicy::Ask);
    }

    #[tokio::test]
    async fn test_app_new_restore_policy_db_always() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        db.save_app_setting("restore_session", "always")
            .await
            .unwrap();
        db.save_app_setting("restore_file_context", "always")
            .await
            .unwrap();
        let fr = FileReader::from_bytes(vec![]);
        let lm = LogManager::new(db, None).await;
        let app = App::new(
            lm,
            fr,
            Theme::default(),
            Arc::new(Keybindings::default()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(app.restore_policy, RestoreSessionPolicy::Always);
        assert_eq!(app.restore_file_policy, RestoreSessionPolicy::Always);
    }

    #[tokio::test]
    async fn test_app_new_restore_session_policy_default() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        db.save_app_setting("restore_session", "other_unknown")
            .await
            .unwrap();
        let fr = FileReader::from_bytes(vec![]);
        let lm = LogManager::new(db, None).await;
        let app = App::new(
            lm,
            fr,
            Theme::default(),
            Arc::new(Keybindings::default()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(app.restore_policy, RestoreSessionPolicy::Ask);
    }

    #[tokio::test]
    async fn test_app_new_bool_settings_from_db() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        db.save_app_setting("show_borders", "true").await.unwrap();
        db.save_app_setting("show_line_numbers", "false")
            .await
            .unwrap();
        db.save_app_setting("show_sidebar", "false").await.unwrap();
        db.save_app_setting("wrap", "true").await.unwrap();
        let fr = FileReader::from_bytes(vec![]);
        let lm = LogManager::new(db, None).await;
        let app = App::new(
            lm,
            fr,
            Theme::default(),
            Arc::new(Keybindings::default()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        assert!(app.show_borders_default);
        assert!(!app.show_line_numbers);
        assert!(!app.show_sidebar);
        assert!(app.wrap);
    }

    #[tokio::test]
    async fn test_app_new_file_context_always_applies() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let ctx = crate::db::FileContext {
            source_file: "/tmp/ctx_always.log".to_string(),
            scroll_offset: 5,
            search_query: String::new(),
            level_colors_disabled: std::collections::HashSet::new(),
            horizontal_scroll: 0,
            marked_lines: vec![],
            file_hash: None,
            comments: vec![],
            show_keys: false,
            raw_mode: false,
            sidebar_width: 30,
            hidden_fields: std::collections::HashSet::new(),
            field_layout_columns: None,
            filtering_enabled: true,
        };
        db.save_file_context(&ctx).await.unwrap();
        let data: Vec<u8> = b"line0\nline1\nline2\n".to_vec();
        let fr = FileReader::from_bytes(data);
        let lm = LogManager::new(db, Some("/tmp/ctx_always.log".to_string())).await;
        let app = App::new(
            lm,
            fr,
            Theme::default(),
            Arc::new(Keybindings::default()),
            None,
            Some(RestoreSessionPolicy::Always),
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(app.tab().scroll.scroll_offset, 5);
    }

    #[tokio::test]
    async fn test_app_new_file_context_never_ignores() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let ctx = crate::db::FileContext {
            source_file: "/tmp/ctx_never.log".to_string(),
            scroll_offset: 7,
            search_query: String::new(),
            level_colors_disabled: std::collections::HashSet::new(),
            horizontal_scroll: 0,
            marked_lines: vec![],
            file_hash: None,
            comments: vec![],
            show_keys: false,
            raw_mode: false,
            sidebar_width: 30,
            hidden_fields: std::collections::HashSet::new(),
            field_layout_columns: None,
            filtering_enabled: true,
        };
        db.save_file_context(&ctx).await.unwrap();
        let data: Vec<u8> = b"line0\nline1\n".to_vec();
        let fr = FileReader::from_bytes(data);
        let lm = LogManager::new(db, Some("/tmp/ctx_never.log".to_string())).await;
        let app = App::new(
            lm,
            fr,
            Theme::default(),
            Arc::new(Keybindings::default()),
            None,
            Some(RestoreSessionPolicy::Never),
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        assert_eq!(app.tab().scroll.scroll_offset, 0);
    }

    #[tokio::test]
    async fn test_app_new_file_context_ask_sets_confirm_mode() {
        use crate::mode::app_mode::ModeRenderState;
        let db = Arc::new(Database::in_memory().await.unwrap());
        let ctx = crate::db::FileContext {
            source_file: "/tmp/ctx_ask.log".to_string(),
            scroll_offset: 3,
            search_query: String::new(),
            level_colors_disabled: std::collections::HashSet::new(),
            horizontal_scroll: 0,
            marked_lines: vec![],
            file_hash: None,
            comments: vec![],
            show_keys: false,
            raw_mode: false,
            sidebar_width: 30,
            hidden_fields: std::collections::HashSet::new(),
            field_layout_columns: None,
            filtering_enabled: true,
        };
        db.save_file_context(&ctx).await.unwrap();
        let data: Vec<u8> = b"line0\nline1\n".to_vec();
        let fr = FileReader::from_bytes(data);
        let lm = LogManager::new(db, Some("/tmp/ctx_ask.log".to_string())).await;
        let app = App::new(
            lm,
            fr,
            Theme::default(),
            Arc::new(Keybindings::default()),
            None,
            Some(RestoreSessionPolicy::Ask),
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        assert!(matches!(
            app.tab().interaction.mode.render_state(),
            ModeRenderState::ConfirmRestore
        ));
    }

    #[tokio::test]
    async fn test_app_new_session_restore_always_sets_pending() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        db.save_session(&["/tmp/a.log".to_string()]).await.unwrap();
        let fr = FileReader::from_bytes(vec![]);
        let lm = LogManager::new(db, None).await;
        let app = App::new(
            lm,
            fr,
            Theme::default(),
            Arc::new(Keybindings::default()),
            Some(RestoreSessionPolicy::Always),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        assert!(app.pending_session_restore.is_some());
    }

    #[tokio::test]
    async fn test_app_new_session_restore_ask_sets_confirm_mode() {
        use crate::mode::app_mode::ModeRenderState;
        let db = Arc::new(Database::in_memory().await.unwrap());
        db.save_session(&["/tmp/b.log".to_string()]).await.unwrap();
        let fr = FileReader::from_bytes(vec![]);
        let lm = LogManager::new(db, None).await;
        let app = App::new(
            lm,
            fr,
            Theme::default(),
            Arc::new(Keybindings::default()),
            Some(RestoreSessionPolicy::Ask),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        assert!(matches!(
            app.tab().interaction.mode.render_state(),
            ModeRenderState::ConfirmRestoreSession { .. }
        ));
    }

    #[tokio::test]
    async fn test_app_new_session_restore_never_no_pending() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        db.save_session(&["/tmp/c.log".to_string()]).await.unwrap();
        let fr = FileReader::from_bytes(vec![]);
        let lm = LogManager::new(db, None).await;
        let app = App::new(
            lm,
            fr,
            Theme::default(),
            Arc::new(Keybindings::default()),
            Some(RestoreSessionPolicy::Never),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        assert!(app.pending_session_restore.is_none());
    }

    #[tokio::test]
    async fn test_close_tab_with_active_filter_handle() {
        use std::sync::atomic::Ordering;
        let mut app = make_app(&["line1", "line2"]).await;
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancel_clone = cancel.clone();
        let (_tx, result_rx) = tokio::sync::mpsc::channel(1);
        app.tabs[0].filter.handle = Some(super::super::FilterHandle {
            result_rx,
            cancel: cancel_clone,
            displayed_progress: 0.0,
            scroll_anchor: None,
            received_first_chunk: false,
            scan_fingerprint: vec![],
            scan_line_count: 0,
            scan_raw_mode: false,
        });
        app.close_tab().await;
        assert!(cancel.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn test_dispatch_restore_session() {
        let mut app = make_app(&["line"]).await;
        app.dispatch_key_result(
            KeyResult::RestoreSession(vec!["/nonexistent/file.log".to_string()]),
            KeyCode::Null,
            KeyModifiers::NONE,
        )
        .await;
    }

    #[tokio::test]
    async fn test_dispatch_docker_attach() {
        let mut app = make_app(&["line"]).await;
        app.dispatch_key_result(
            KeyResult::DockerAttach("container123".to_string(), "myapp".to_string()),
            KeyCode::Null,
            KeyModifiers::NONE,
        )
        .await;
        assert!(app.tabs.len() >= 1);
    }

    #[tokio::test]
    async fn test_dispatch_dlt_attach() {
        let mut app = make_app(&["line"]).await;
        app.dispatch_key_result(
            KeyResult::DltAttach("127.0.0.1".to_string(), 3490, "dlt-device".to_string()),
            KeyCode::Null,
            KeyModifiers::NONE,
        )
        .await;
        assert!(app.tabs.len() >= 1);
    }

    #[tokio::test]
    async fn test_dispatch_always_restore_session() {
        let mut app = make_app(&["line"]).await;
        app.dispatch_key_result(
            KeyResult::AlwaysRestoreSession(vec!["/nonexistent/file.log".to_string()]),
            KeyCode::Null,
            KeyModifiers::NONE,
        )
        .await;
        assert_eq!(app.restore_policy, RestoreSessionPolicy::Always);
        let setting = app.db.load_app_setting("restore_session").await.unwrap();
        assert_eq!(setting.as_deref(), Some("always"));
    }

    #[tokio::test]
    async fn test_stop_mcp_clears_handle() {
        let mut app = make_app(&["line"]).await;
        app.stop_mcp();
        assert!(app.mcp_server_handle.is_none());
        assert!(app.mcp_cmd_rx.is_none());
    }

    #[tokio::test]
    async fn test_refresh_mcp_snapshot_no_server_is_noop() {
        let mut app = make_app(&["line"]).await;
        assert!(app.mcp_server_handle.is_none());
        app.refresh_mcp_snapshot();
    }

    #[tokio::test]
    async fn test_poll_mcp_commands_no_receiver_is_noop() {
        let mut app = make_app(&["line"]).await;
        assert!(app.mcp_cmd_rx.is_none());
        app.poll_mcp_commands().await;
    }

    #[tokio::test]
    async fn test_poll_mcp_commands_with_empty_channel() {
        let mut app = make_app(&["line"]).await;
        let (_tx, rx) = tokio::sync::mpsc::channel::<crate::mcp::McpCommand>(8);
        app.mcp_cmd_rx = Some(rx);
        app.poll_mcp_commands().await;
    }

    #[tokio::test]
    async fn test_handle_mcp_command_toggle_mark() {
        let mut app = make_app(&["line0", "line1"]).await;
        assert!(!app.tabs[0].log_manager.is_marked(0));
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        app.mcp_cmd_rx = Some(rx);
        tx.send(crate::mcp::McpCommand::ToggleMark(0))
            .await
            .unwrap();
        app.poll_mcp_commands().await;
        assert!(app.tabs[0].log_manager.is_marked(0));
    }

    #[tokio::test]
    async fn test_handle_mcp_command_add_annotation() {
        let mut app = make_app(&["line0", "line1"]).await;
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        app.mcp_cmd_rx = Some(rx);
        tx.send(crate::mcp::McpCommand::AddAnnotation {
            text: "note".to_string(),
            line_indices: vec![0],
        })
        .await
        .unwrap();
        app.poll_mcp_commands().await;
        assert!(!app.tabs[0].log_manager.get_comments().is_empty());
    }

    #[tokio::test]
    async fn test_handle_mcp_command_remove_annotation() {
        let mut app = make_app(&["line0", "line1"]).await;
        app.tabs[0]
            .log_manager
            .add_comment("test note".to_string(), vec![0]);
        assert_eq!(app.tabs[0].log_manager.get_comments().len(), 1);
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        app.mcp_cmd_rx = Some(rx);
        tx.send(crate::mcp::McpCommand::RemoveAnnotation(0))
            .await
            .unwrap();
        app.poll_mcp_commands().await;
        assert!(app.tabs[0].log_manager.get_comments().is_empty());
    }

    #[tokio::test]
    async fn test_apply_tab_defaults() {
        let mut app = make_app(&["line"]).await;
        app.show_mode_bar = false;
        app.show_borders_default = true;
        app.show_line_numbers = false;
        app.show_sidebar = false;
        app.wrap = true;
        let data: Vec<u8> = b"new\n".to_vec();
        let fr = FileReader::from_bytes(data);
        let lm = LogManager::new(app.db.clone(), None).await;
        let mut tab = super::super::TabState::new(fr, lm, "new".to_string());
        app.apply_tab_defaults(&mut tab);
        assert!(!tab.display.show_mode_bar);
        assert!(tab.display.show_borders);
        assert!(!tab.display.show_line_numbers);
        assert!(!tab.display.show_sidebar);
        assert!(tab.display.wrap);
    }

    #[tokio::test]
    async fn test_startup_warnings_cleared_on_key_event() {
        let mut app = make_app(&["line"]).await;
        app.startup_warnings = vec!["warning 1".to_string(), "warning 2".to_string()];
        app.handle_key_event(KeyCode::Char('j')).await;
        assert!(app.startup_warnings.is_empty());
    }
}
