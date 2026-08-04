use super::input_handler::InputHandler;
use super::{StdinLoadState, TabState};
use crate::config::{DEFAULT_PREVIEW_BYTES, Keybindings, RestoreSessionPolicy};
use crate::db::LogManager;
use crate::db::{AppSettingsStore, FileContextStore, SessionStore, SettingsKey};
use crate::ingestion::FileReader;
use crate::mode::app_mode::{ConfirmRestoreMode, ConfirmRestoreSessionMode};
use crate::mode::normal_mode::NormalMode;
use crate::theme::Theme;
use crate::ui::SidebarSide;
use crate::ui::session::SessionManager;
use ratatui::{Terminal, prelude::*};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(super) const DOUBLE_CLICK_MS: u128 = 300;

async fn resolve_bool_setting(
    db: &crate::db::Database,
    key: SettingsKey,
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
    key: SettingsKey,
    config_override: Option<RestoreSessionPolicy>,
) -> RestoreSessionPolicy {
    if let Some(v) = config_override {
        return v;
    }
    if let Ok(Some(val)) = db.load_app_setting(key).await {
        return val.parse().unwrap_or(RestoreSessionPolicy::Ask);
    }
    RestoreSessionPolicy::Always
}

struct StartupOverrides {
    show_mode_bar: Option<bool>,
    show_borders: Option<bool>,
    show_line_numbers: Option<bool>,
    show_sidebar: Option<bool>,
    wrap: Option<bool>,
    sidebar_side: Option<SidebarSide>,
    restore_policy: Option<RestoreSessionPolicy>,
    restore_file_policy: Option<RestoreSessionPolicy>,
}

struct StartupSettings {
    restore_policy: RestoreSessionPolicy,
    restore_file_policy: RestoreSessionPolicy,
    show_mode_bar: bool,
    show_borders_default: bool,
    show_line_numbers: bool,
    show_sidebar: bool,
    wrap: bool,
    sidebar_side: SidebarSide,
}

async fn resolve_startup_settings(
    db: &crate::db::Database,
    ov: StartupOverrides,
) -> StartupSettings {
    let sidebar_side = if let Some(s) = ov.sidebar_side {
        s
    } else if let Ok(Some(val)) = db.load_app_setting(SettingsKey::SidebarLeft).await {
        if val == "true" {
            SidebarSide::Left
        } else {
            SidebarSide::Right
        }
    } else {
        SidebarSide::Right
    };
    StartupSettings {
        restore_policy: resolve_policy_setting(db, SettingsKey::RestoreSession, ov.restore_policy)
            .await,
        restore_file_policy: resolve_policy_setting(
            db,
            SettingsKey::RestoreFileContext,
            ov.restore_file_policy,
        )
        .await,
        show_mode_bar: resolve_bool_setting(db, SettingsKey::ShowModeBar, ov.show_mode_bar, true)
            .await,
        show_borders_default: resolve_bool_setting(
            db,
            SettingsKey::ShowBorders,
            ov.show_borders,
            false,
        )
        .await,
        show_line_numbers: resolve_bool_setting(
            db,
            SettingsKey::ShowLineNumbers,
            ov.show_line_numbers,
            true,
        )
        .await,
        show_sidebar: resolve_bool_setting(db, SettingsKey::ShowSidebar, ov.show_sidebar, true)
            .await,
        wrap: resolve_bool_setting(db, SettingsKey::Wrap, ov.wrap, false).await,
        sidebar_side,
    }
}

use crate::mcp::McpState;

pub struct DisplaySettings {
    pub show_mode_bar: bool,
    pub show_borders_default: bool,
    pub show_line_numbers: bool,
    pub show_sidebar: bool,
    pub wrap: bool,
    pub sidebar_side: SidebarSide,
}

pub struct App {
    pub tabs: Vec<TabState>,
    pub active_tab: usize,
    pub theme: Theme,
    pub db: Arc<crate::db::Database>,
    pub should_quit: bool,
    /// In-progress stdin read — separate slot so session-restore cannot overwrite it.
    pub stdin_load_state: Option<StdinLoadState>,
    /// Shared keybindings — propagated to every new tab.
    pub keybindings: Arc<Keybindings>,
    /// Persistent clipboard instance — kept alive so clipboard managers can read the contents.
    pub clipboard: Option<arboard::Clipboard>,
    pub display: DisplaySettings,
    /// Number of bytes to read for the instant preview (from config `preview_bytes`).
    pub preview_bytes: u64,
    /// Configured DLT devices from config.json.
    pub dlt_devices: Vec<crate::config::DltDevice>,
    /// Format name -> filter file path, auto-loaded into a tab when its
    /// format is assigned and it has no filters yet. Resolved once at
    /// startup from config.json + the DB-persisted map (see
    /// `config::resolve_default_filter_files`); kept up to date afterward by
    /// `:default-filters`/its popup.
    pub default_filter_files: std::collections::HashMap<String, String>,
    pub mcp: McpState,
    pub input: InputHandler,
    pub session: SessionManager,
    /// In-progress background archive extraction.
    pub pending_archive: Option<crate::ui::ArchiveExtractionState>,
    /// In-progress background archive listing (pre-extraction file tree scan).
    pub pending_archive_listing: Option<crate::ui::ArchiveListingState>,
    /// In-progress background fetch of a single lazy archive node's bytes.
    pub pending_archive_expand: Option<crate::ui::ArchiveExpandState>,
    /// In-progress background read+format-detect of files merge-marked in a
    /// directory picker (the file-picker UI reused for opening directories).
    pub pending_directory_merge: Option<crate::ui::DirectoryMergeState>,
    /// In-progress background builds of picker-triggered merged tabs (see
    /// `App::start_merge_build_streaming`). A `Vec` since more than one
    /// archive/directory merge can be in flight at once (different tabs).
    pub pending_merge_builds: Vec<crate::ui::MergeBuildState>,
    /// App-level decompression progress message shown while an archive is being extracted.
    pub decompression_message: Option<String>,
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("active_tab", &self.active_tab)
            .field("num_tabs", &self.tabs.len())
            .finish()
    }
}

pub struct AppBuilder {
    log_manager: LogManager,
    file_reader: FileReader,
    theme: Theme,
    keybindings: Arc<Keybindings>,
    overrides: StartupOverrides,
}

impl AppBuilder {
    pub fn restore_policy(mut self, v: Option<RestoreSessionPolicy>) -> Self {
        self.overrides.restore_policy = v;
        self
    }

    pub fn restore_file_policy(mut self, v: Option<RestoreSessionPolicy>) -> Self {
        self.overrides.restore_file_policy = v;
        self
    }

    pub fn show_mode_bar(mut self, v: Option<bool>) -> Self {
        self.overrides.show_mode_bar = v;
        self
    }

    pub fn show_borders(mut self, v: Option<bool>) -> Self {
        self.overrides.show_borders = v;
        self
    }

    pub fn show_line_numbers(mut self, v: Option<bool>) -> Self {
        self.overrides.show_line_numbers = v;
        self
    }

    pub fn show_sidebar(mut self, v: Option<bool>) -> Self {
        self.overrides.show_sidebar = v;
        self
    }

    pub fn wrap(mut self, v: Option<bool>) -> Self {
        self.overrides.wrap = v;
        self
    }

    pub fn sidebar_side(mut self, v: Option<SidebarSide>) -> Self {
        self.overrides.sidebar_side = v;
        self
    }

    pub async fn build(self) -> App {
        let db = self.log_manager.db.clone();
        let settings = resolve_startup_settings(&db, self.overrides).await;
        let StartupSettings {
            restore_policy,
            restore_file_policy,
            show_mode_bar,
            show_borders_default,
            show_line_numbers,
            show_sidebar,
            wrap,
            sidebar_side,
        } = settings;

        let title = self
            .log_manager
            .source_file()
            .map(|s| {
                std::path::Path::new(s)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(s)
                    .to_string()
            })
            .unwrap_or_else(|| "stdin".to_string());

        let no_source = self.log_manager.source_file().is_none();
        let no_data = self.file_reader.line_count() == 0;

        let mut tab = TabState::new(self.file_reader, self.log_manager, title);
        tab.interaction.keybindings = self.keybindings.clone();
        tab.display.show_mode_bar = show_mode_bar;
        tab.display.show_borders = show_borders_default;
        tab.display.show_line_numbers = show_line_numbers;
        tab.display.show_sidebar = show_sidebar;
        tab.display.wrap = wrap;
        tab.display.sidebar_side = sidebar_side;
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
                        tab.interaction.mode = Box::new(ConfirmRestoreSessionMode {
                            files: std::sync::Arc::new(files),
                        });
                    }
                    RestoreSessionPolicy::Always => {
                        pending_session_restore = Some(files);
                    }
                }
            }
        }

        let session_db = db.clone();
        App {
            tabs: vec![tab],
            active_tab: 0,
            theme: self.theme,
            db,
            should_quit: false,
            stdin_load_state: None,
            keybindings: self.keybindings,
            clipboard: None,
            display: DisplaySettings {
                show_mode_bar,
                show_borders_default,
                show_line_numbers,
                show_sidebar,
                wrap,
                sidebar_side,
            },
            preview_bytes: DEFAULT_PREVIEW_BYTES,
            dlt_devices: vec![],
            default_filter_files: std::collections::HashMap::new(),
            mcp: McpState {
                port: None,
                snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(
                    crate::mcp::McpSnapshot::default(),
                )),
                cmd_rx: None,
                server_handle: None,
            },
            input: InputHandler {
                log_panel_area: Rect::default(),
                sidebar_area: None,
                tab_bar_area: None,
                last_click: None,
                scrollbar_dragging: false,
            },
            session: SessionManager {
                db: session_db,
                restore_policy,
                restore_file_policy,
                pending_session_restore,
                startup_tail: false,
                startup_filters: false,
                startup_warnings: vec![],
            },
            pending_archive: None,
            pending_archive_listing: None,
            pending_archive_expand: None,
            pending_directory_merge: None,
            pending_merge_builds: Vec::new(),
            decompression_message: None,
        }
    }
}

impl App {
    pub fn builder(
        log_manager: LogManager,
        file_reader: FileReader,
        theme: Theme,
        keybindings: Arc<Keybindings>,
    ) -> AppBuilder {
        AppBuilder {
            log_manager,
            file_reader,
            theme,
            keybindings,
            overrides: StartupOverrides {
                show_mode_bar: None,
                show_borders: None,
                show_line_numbers: None,
                show_sidebar: None,
                wrap: None,
                sidebar_side: None,
                restore_policy: None,
                restore_file_policy: None,
            },
        }
    }

    pub(super) async fn save_tab_context(&self, tab: &TabState) {
        self.session.save_tab_context(tab).await;
    }

    pub(super) async fn save_all_contexts(&self) {
        self.session.save_all_contexts(&self.tabs).await;
    }

    pub async fn run<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> anyhow::Result<()>
    where
        <B as Backend>::Error: Send + Sync + 'static,
    {
        if let Some(files) = self.session.pending_session_restore.take() {
            self.restore_session(files).await;
        }

        let mut last_tick = Instant::now();
        let tick_rate = Duration::from_millis(250);

        loop {
            terminal.draw(|frame| self.ui(frame))?;

            // Poll for background load completion, file watch updates, search, and filter.
            self.advance_file_load().await;
            self.advance_stdin_load().await;
            self.advance_file_watches().await;
            self.advance_merged_tabs();
            self.advance_stream_retries();
            self.advance_search();
            self.advance_filter_computation();
            self.poll_archive_extraction().await;
            self.poll_archive_listing().await;
            self.poll_archive_expand().await;
            self.poll_directory_merge().await;
            self.poll_merge_builds();

            let mut poll_timeout = tick_rate
                .checked_sub(last_tick.elapsed())
                .unwrap_or_else(|| Duration::from_secs(0));
            if let Some((t, _, _)) = self.input.last_click {
                let remaining =
                    Duration::from_millis(DOUBLE_CLICK_MS as u64).saturating_sub(t.elapsed());
                poll_timeout = poll_timeout.min(remaining);
            }

            if crossterm::event::poll(poll_timeout)? {
                match crossterm::event::read()? {
                    crossterm::event::Event::Key(key)
                        if key.kind == crossterm::event::KeyEventKind::Press =>
                    {
                        self.session.startup_warnings.clear();
                        let tab = &mut self.tabs[self.active_tab];
                        let mode = std::mem::replace(
                            &mut tab.interaction.mode,
                            Box::new(NormalMode::default()),
                        );
                        let (next_mode, result) =
                            mode.handle_key(tab, key.code, key.modifiers).await;
                        tab.interaction.mode = next_mode;
                        self.dispatch_key_result(result, key.code, key.modifiers)
                            .await;
                        self.refresh_mcp_snapshot();
                    }
                    crossterm::event::Event::Mouse(mouse) => {
                        self.handle_mouse_event(mouse).await;
                    }
                    _ => {}
                }
            }

            self.flush_pending_click().await;
            self.poll_mcp_commands().await;

            if self.should_quit {
                return Ok(());
            }

            if last_tick.elapsed() >= tick_rate {
                last_tick = Instant::now();
            }
        }
    }

    pub async fn start_mcp(&mut self, port: u16) -> std::io::Result<()> {
        let snapshot = self.build_mcp_snapshot();
        self.mcp.start(port, snapshot).await
    }

    pub fn stop_mcp(&mut self) {
        self.mcp.stop();
    }

    fn build_mcp_snapshot(&self) -> crate::mcp::McpSnapshot {
        if let Some(tab) = self.tabs.get(self.active_tab) {
            crate::mcp::McpSnapshot {
                marked_lines: crate::mcp::build_marked_lines(&tab.file_reader, &tab.mark_manager),
                annotations: crate::mcp::build_annotations(&tab.comment_manager),
            }
        } else {
            crate::mcp::McpSnapshot::default()
        }
    }

    pub fn refresh_mcp_snapshot(&mut self) {
        if self.mcp.server_handle.is_none() {
            return;
        }
        let snapshot = self.build_mcp_snapshot();
        self.mcp.push_snapshot(snapshot);
    }

    pub async fn poll_mcp_commands(&mut self) {
        for cmd in self.mcp.drain_commands() {
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
                self.tabs[self.active_tab].mark_manager.toggle(idx);
                self.save_tab_context(&self.tabs[self.active_tab]).await;
                self.refresh_mcp_snapshot();
            }
            McpCommand::AddAnnotation { text, line_indices } => {
                self.tabs[self.active_tab]
                    .comment_manager
                    .add(text, line_indices);
                self.save_tab_context(&self.tabs[self.active_tab]).await;
                self.refresh_mcp_snapshot();
            }
            McpCommand::RemoveAnnotation(idx) => {
                self.tabs[self.active_tab].comment_manager.remove(idx);
                self.save_tab_context(&self.tabs[self.active_tab]).await;
                self.refresh_mcp_snapshot();
            }
            McpCommand::ServerError(msg) => {
                // The background server task has already exited; drop our
                // side too so the app stops believing MCP is still running.
                self.mcp.stop();
                self.tabs[self.active_tab].set_notification(msg);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::auto_complete::shell_split;
    use crate::config::{Keybindings, RestoreSessionPolicy};
    use crate::db::Database;
    use crate::db::LogManager;
    use crate::filters::FilterType;
    use crate::ingestion::FileReader;
    use crate::ui::KeyResult;
    use crossterm::event::{KeyCode, KeyModifiers};
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
                tab.restore_scroll_to_line(scroll_anchor);
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
        App::builder(
            log_manager,
            file_reader,
            Theme::default(),
            Arc::new(Keybindings::default()),
        )
        .build()
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
    async fn test_add_highlight_command() {
        let mut app = make_app(&["INFO something", "WARN warning", "ERROR error"]).await;
        app.execute_command_str("highlight foo".to_string()).await;
        let filters = app.tab().log_manager.get_filters();
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].filter_type, FilterType::Highlight);
        assert_eq!(filters[0].pattern, "foo");
    }

    #[tokio::test]
    async fn test_highlight_alias_h_command() {
        let mut app = make_app(&["INFO something", "WARN warning", "ERROR error"]).await;
        app.execute_command_str("h foo".to_string()).await;
        let filters = app.tab().log_manager.get_filters();
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].filter_type, FilterType::Highlight);
        assert_eq!(filters[0].pattern, "foo");
    }

    #[tokio::test]
    async fn test_highlight_does_not_reduce_visible_lines() {
        let mut app = make_app(&["INFO something", "WARN warning", "ERROR error"]).await;
        app.execute_command_str("highlight ERROR".to_string()).await;
        await_filter_computations(&mut app).await;
        assert_eq!(app.tab().filter.visible_indices.len(), 3);
    }

    #[tokio::test]
    async fn test_set_color_applies_to_highlight_filter() {
        let mut app = make_app(&["INFO something", "WARN warning", "ERROR error"]).await;
        app.execute_command_str("highlight ERROR".to_string()).await;
        app.tabs[0].filter.filter_context = Some(0);
        app.execute_command_str("set-color --fg Red".to_string())
            .await;
        let filters = app.tab().log_manager.get_filters();
        assert!(filters[0].color_config.is_some());
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
        let mut app = App::builder(
            log_manager,
            file_reader,
            Theme::default(),
            Arc::new(Keybindings::default()),
        )
        .build()
        .await;

        assert_eq!(app.tab().filter.visible_indices.len(), 3);

        app.execute_command_str("filter INFO".to_string()).await;

        assert_eq!(app.tab().filter.visible_indices.len(), 1);
    }

    #[tokio::test]
    async fn test_mark_toggle() {
        let lines = vec!["line0", "line1", "line2"];
        let (file_reader, log_manager) = make_tab(&lines).await;
        let mut app = App::builder(
            log_manager,
            file_reader,
            Theme::default(),
            Arc::new(Keybindings::default()),
        )
        .build()
        .await;

        app.tab_mut().scroll.scroll_offset = 0;
        app.handle_key_event_with_modifiers(KeyCode::Char('m'), KeyModifiers::NONE)
            .await;
        assert!(app.tab().mark_manager.is_marked(0));

        app.handle_key_event_with_modifiers(KeyCode::Char('m'), KeyModifiers::NONE)
            .await;
        assert!(!app.tab().mark_manager.is_marked(0));
    }

    #[tokio::test]
    async fn test_scroll_g_key() {
        let lines: Vec<&str> = (0..20).map(|_| "line").collect();
        let (file_reader, log_manager) = make_tab(&lines).await;
        let mut app = App::builder(
            log_manager,
            file_reader,
            Theme::default(),
            Arc::new(Keybindings::default()),
        )
        .build()
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
        app.execute_command_str("filter --fg red -l INFO".to_string())
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
        let app = App::builder(lm, fr, Theme::default(), Arc::new(Keybindings::default()))
            .build()
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

        assert_eq!(
            app.session.restore_file_policy,
            RestoreSessionPolicy::Always
        );
        assert_eq!(app.session.restore_policy, RestoreSessionPolicy::Always);

        let file_setting = app
            .db
            .load_app_setting(SettingsKey::RestoreFileContext)
            .await
            .unwrap();
        let session_setting = app
            .db
            .load_app_setting(SettingsKey::RestoreSession)
            .await
            .unwrap();
        assert_eq!(
            file_setting.as_deref(),
            Some(RestoreSessionPolicy::Always.to_string().as_str())
        );
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

        assert_eq!(app.session.restore_file_policy, RestoreSessionPolicy::Never);
        assert_eq!(app.session.restore_policy, RestoreSessionPolicy::Always);

        let file_setting = app
            .db
            .load_app_setting(SettingsKey::RestoreFileContext)
            .await
            .unwrap();
        let session_setting = app
            .db
            .load_app_setting(SettingsKey::RestoreSession)
            .await
            .unwrap();
        assert_eq!(
            file_setting.as_deref(),
            Some(RestoreSessionPolicy::Never.to_string().as_str())
        );
        // session policy not touched by the file-context key press
        assert!(session_setting.is_none());
    }

    #[tokio::test]
    async fn test_app_new_with_config_overrides() {
        let (file_reader, log_manager) = make_tab(&["line"]).await;
        let app = App::builder(
            log_manager,
            file_reader,
            Theme::default(),
            Arc::new(Keybindings::default()),
        )
        .restore_policy(Some(RestoreSessionPolicy::Always))
        .restore_file_policy(Some(RestoreSessionPolicy::Never))
        .show_mode_bar(Some(false))
        .show_borders(Some(true))
        .show_line_numbers(Some(false))
        .show_sidebar(Some(false))
        .wrap(Some(true))
        .build()
        .await;
        assert_eq!(app.session.restore_policy, RestoreSessionPolicy::Always);
        assert_eq!(app.session.restore_file_policy, RestoreSessionPolicy::Never);
        assert!(!app.display.show_mode_bar);
        assert!(app.display.show_borders_default);
        assert!(!app.display.show_line_numbers);
        assert!(!app.display.show_sidebar);
        assert!(app.display.wrap);
    }

    #[tokio::test]
    async fn test_app_new_settings_from_db() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        db.save_app_setting(
            SettingsKey::RestoreSession,
            &RestoreSessionPolicy::Always.to_string(),
        )
        .await
        .unwrap();
        db.save_app_setting(
            SettingsKey::RestoreFileContext,
            &RestoreSessionPolicy::Never.to_string(),
        )
        .await
        .unwrap();
        db.save_app_setting(SettingsKey::ShowModeBar, "false")
            .await
            .unwrap();
        let fr = FileReader::from_bytes(vec![]);
        let lm = LogManager::new(db, None).await;
        let app = App::builder(lm, fr, Theme::default(), Arc::new(Keybindings::default()))
            .build()
            .await;
        assert_eq!(app.session.restore_policy, RestoreSessionPolicy::Always);
        assert_eq!(app.session.restore_file_policy, RestoreSessionPolicy::Never);
        assert!(!app.display.show_mode_bar);
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
    async fn test_dispatch_set_default_filter_file_inserts_mapping() {
        let mut app = make_app(&["line"]).await;
        app.dispatch_key_result(
            KeyResult::SetDefaultFilterFile {
                format: "acme".to_string(),
                path: Some("/tmp/a.json".to_string()),
            },
            KeyCode::Null,
            KeyModifiers::NONE,
        )
        .await;
        assert_eq!(
            app.default_filter_files.get("acme"),
            Some(&"/tmp/a.json".to_string())
        );
    }

    #[tokio::test]
    async fn test_dispatch_set_default_filter_file_none_path_removes_mapping() {
        let mut app = make_app(&["line"]).await;
        app.default_filter_files
            .insert("acme".to_string(), "/tmp/a.json".to_string());
        app.dispatch_key_result(
            KeyResult::SetDefaultFilterFile {
                format: "acme".to_string(),
                path: None,
            },
            KeyCode::Null,
            KeyModifiers::NONE,
        )
        .await;
        assert!(!app.default_filter_files.contains_key("acme"));
    }

    #[tokio::test]
    async fn test_dispatch_set_default_filter_file_persists_to_sqlite() {
        use crate::db::AppSettingsStore;
        let mut app = make_app(&["line"]).await;
        app.dispatch_key_result(
            KeyResult::SetDefaultFilterFile {
                format: "acme".to_string(),
                path: Some("/tmp/a.json".to_string()),
            },
            KeyCode::Null,
            KeyModifiers::NONE,
        )
        .await;
        let saved = app
            .db
            .load_app_setting(crate::db::SettingsKey::DefaultFilterFiles)
            .await
            .unwrap()
            .unwrap();
        assert!(saved.contains("acme"));
        assert!(saved.contains("/tmp/a.json"));
    }

    #[tokio::test]
    async fn test_dispatch_toggle_mode_bar() {
        let mut app = make_app(&["line"]).await;
        let initial = app.display.show_mode_bar;
        app.dispatch_key_result(KeyResult::ToggleModeBar, KeyCode::Null, KeyModifiers::NONE)
            .await;
        assert_eq!(app.display.show_mode_bar, !initial);
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
        assert_eq!(app.session.restore_policy, RestoreSessionPolicy::Never);
        let setting = app
            .db
            .load_app_setting(SettingsKey::RestoreSession)
            .await
            .unwrap();
        assert_eq!(
            setting.as_deref(),
            Some(RestoreSessionPolicy::Never.to_string().as_str())
        );
    }

    #[tokio::test]
    async fn test_always_restore_session_via_key_event() {
        let mut app = make_app(&["line"]).await;
        app.tabs[0].interaction.mode = Box::new(crate::mode::app_mode::ConfirmRestoreSessionMode {
            files: std::sync::Arc::new(vec!["/nonexistent/file.log".to_string()]),
        });
        app.handle_key_event_with_modifiers(KeyCode::Char('Y'), KeyModifiers::SHIFT)
            .await;
        assert_eq!(app.session.restore_policy, RestoreSessionPolicy::Always);
        let setting = app
            .db
            .load_app_setting(SettingsKey::RestoreSession)
            .await
            .unwrap();
        assert_eq!(
            setting.as_deref(),
            Some(RestoreSessionPolicy::Always.to_string().as_str())
        );
    }

    #[tokio::test]
    async fn test_never_restore_session_via_key_event() {
        let mut app = make_app(&["line"]).await;
        app.tabs[0].interaction.mode = Box::new(crate::mode::app_mode::ConfirmRestoreSessionMode {
            files: std::sync::Arc::new(vec!["/nonexistent/file.log".to_string()]),
        });
        app.handle_key_event_with_modifiers(KeyCode::Char('N'), KeyModifiers::SHIFT)
            .await;
        assert_eq!(app.session.restore_policy, RestoreSessionPolicy::Never);
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
        db.save_app_setting(
            SettingsKey::RestoreSession,
            &RestoreSessionPolicy::Never.to_string(),
        )
        .await
        .unwrap();
        db.save_app_setting(
            SettingsKey::RestoreFileContext,
            &RestoreSessionPolicy::Ask.to_string(),
        )
        .await
        .unwrap();
        let fr = FileReader::from_bytes(vec![]);
        let lm = LogManager::new(db, None).await;
        let app = App::builder(lm, fr, Theme::default(), Arc::new(Keybindings::default()))
            .build()
            .await;
        assert_eq!(app.session.restore_policy, RestoreSessionPolicy::Never);
        assert_eq!(app.session.restore_file_policy, RestoreSessionPolicy::Ask);
    }

    #[tokio::test]
    async fn test_app_new_restore_policy_db_always() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        db.save_app_setting(
            SettingsKey::RestoreSession,
            &RestoreSessionPolicy::Always.to_string(),
        )
        .await
        .unwrap();
        db.save_app_setting(
            SettingsKey::RestoreFileContext,
            &RestoreSessionPolicy::Always.to_string(),
        )
        .await
        .unwrap();
        let fr = FileReader::from_bytes(vec![]);
        let lm = LogManager::new(db, None).await;
        let app = App::builder(lm, fr, Theme::default(), Arc::new(Keybindings::default()))
            .build()
            .await;
        assert_eq!(app.session.restore_policy, RestoreSessionPolicy::Always);
        assert_eq!(
            app.session.restore_file_policy,
            RestoreSessionPolicy::Always
        );
    }

    #[tokio::test]
    async fn test_app_new_restore_session_policy_default() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        db.save_app_setting(SettingsKey::RestoreSession, "other_unknown")
            .await
            .unwrap();
        let fr = FileReader::from_bytes(vec![]);
        let lm = LogManager::new(db, None).await;
        let app = App::builder(lm, fr, Theme::default(), Arc::new(Keybindings::default()))
            .build()
            .await;
        assert_eq!(app.session.restore_policy, RestoreSessionPolicy::Ask);
    }

    #[tokio::test]
    async fn test_app_new_bool_settings_from_db() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        db.save_app_setting(SettingsKey::ShowBorders, "true")
            .await
            .unwrap();
        db.save_app_setting(SettingsKey::ShowLineNumbers, "false")
            .await
            .unwrap();
        db.save_app_setting(SettingsKey::ShowSidebar, "false")
            .await
            .unwrap();
        db.save_app_setting(SettingsKey::Wrap, "true")
            .await
            .unwrap();
        let fr = FileReader::from_bytes(vec![]);
        let lm = LogManager::new(db, None).await;
        let app = App::builder(lm, fr, Theme::default(), Arc::new(Keybindings::default()))
            .build()
            .await;
        assert!(app.display.show_borders_default);
        assert!(!app.display.show_line_numbers);
        assert!(!app.display.show_sidebar);
        assert!(app.display.wrap);
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
        let app = App::builder(lm, fr, Theme::default(), Arc::new(Keybindings::default()))
            .restore_file_policy(Some(RestoreSessionPolicy::Always))
            .build()
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
        let app = App::builder(lm, fr, Theme::default(), Arc::new(Keybindings::default()))
            .restore_file_policy(Some(RestoreSessionPolicy::Never))
            .build()
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
        let app = App::builder(lm, fr, Theme::default(), Arc::new(Keybindings::default()))
            .restore_file_policy(Some(RestoreSessionPolicy::Ask))
            .build()
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
        let app = App::builder(lm, fr, Theme::default(), Arc::new(Keybindings::default()))
            .restore_policy(Some(RestoreSessionPolicy::Always))
            .build()
            .await;
        assert!(app.session.pending_session_restore.is_some());
    }

    #[tokio::test]
    async fn test_app_new_session_restore_ask_sets_confirm_mode() {
        use crate::mode::app_mode::ModeRenderState;
        let db = Arc::new(Database::in_memory().await.unwrap());
        db.save_session(&["/tmp/b.log".to_string()]).await.unwrap();
        let fr = FileReader::from_bytes(vec![]);
        let lm = LogManager::new(db, None).await;
        let app = App::builder(lm, fr, Theme::default(), Arc::new(Keybindings::default()))
            .restore_policy(Some(RestoreSessionPolicy::Ask))
            .build()
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
        let app = App::builder(lm, fr, Theme::default(), Arc::new(Keybindings::default()))
            .restore_policy(Some(RestoreSessionPolicy::Never))
            .build()
            .await;
        assert!(app.session.pending_session_restore.is_none());
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
            scan_highlight_mode: false,
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
        assert!(!app.tabs.is_empty());
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
        assert!(!app.tabs.is_empty());
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
        assert_eq!(app.session.restore_policy, RestoreSessionPolicy::Always);
        let setting = app
            .db
            .load_app_setting(SettingsKey::RestoreSession)
            .await
            .unwrap();
        assert_eq!(
            setting.as_deref(),
            Some(RestoreSessionPolicy::Always.to_string().as_str())
        );
    }

    #[tokio::test]
    async fn test_refresh_mcp_snapshot_no_server_is_noop() {
        let mut app = make_app(&["line"]).await;
        assert!(app.mcp.server_handle.is_none());
        app.refresh_mcp_snapshot();
    }

    #[tokio::test]
    async fn test_poll_mcp_commands_no_receiver_is_noop() {
        let mut app = make_app(&["line"]).await;
        assert!(app.mcp.cmd_rx.is_none());
        app.poll_mcp_commands().await;
    }

    #[tokio::test]
    async fn test_poll_mcp_commands_with_empty_channel() {
        let mut app = make_app(&["line"]).await;
        let (_tx, rx) = tokio::sync::mpsc::channel::<crate::mcp::McpCommand>(8);
        app.mcp.cmd_rx = Some(rx);
        app.poll_mcp_commands().await;
    }

    #[tokio::test]
    async fn test_handle_mcp_command_toggle_mark() {
        let mut app = make_app(&["line0", "line1"]).await;
        assert!(!app.tabs[0].mark_manager.is_marked(0));
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        app.mcp.cmd_rx = Some(rx);
        tx.send(crate::mcp::McpCommand::ToggleMark(0))
            .await
            .unwrap();
        app.poll_mcp_commands().await;
        assert!(app.tabs[0].mark_manager.is_marked(0));
    }

    #[tokio::test]
    async fn test_handle_mcp_command_add_annotation() {
        let mut app = make_app(&["line0", "line1"]).await;
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        app.mcp.cmd_rx = Some(rx);
        tx.send(crate::mcp::McpCommand::AddAnnotation {
            text: "note".to_string(),
            line_indices: vec![0],
        })
        .await
        .unwrap();
        app.poll_mcp_commands().await;
        assert!(!app.tabs[0].comment_manager.get().is_empty());
    }

    #[tokio::test]
    async fn test_handle_mcp_command_remove_annotation() {
        let mut app = make_app(&["line0", "line1"]).await;
        app.tabs[0]
            .comment_manager
            .add("test note".to_string(), vec![0]);
        assert_eq!(app.tabs[0].comment_manager.get().len(), 1);
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        app.mcp.cmd_rx = Some(rx);
        tx.send(crate::mcp::McpCommand::RemoveAnnotation(0))
            .await
            .unwrap();
        app.poll_mcp_commands().await;
        assert!(app.tabs[0].comment_manager.get().is_empty());
    }

    /// A background MCP server failure must surface as a notification
    /// (never stderr — the terminal is already in raw/alt-screen mode by
    /// the time this can happen) and must clear `server_handle`, so the
    /// app stops believing MCP is still running.
    #[tokio::test]
    async fn test_handle_mcp_command_server_error_notifies_and_stops_mcp() {
        let mut app = make_app(&["line0"]).await;
        app.mcp.server_handle = Some(crate::mcp::McpServerHandle {
            cancel: tokio_util::sync::CancellationToken::new(),
            port: 9876,
        });
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        app.mcp.cmd_rx = Some(rx);
        tx.send(crate::mcp::McpCommand::ServerError(
            "MCP server error: address in use".to_string(),
        ))
        .await
        .unwrap();
        app.poll_mcp_commands().await;

        assert!(
            app.mcp.server_handle.is_none(),
            "server_handle must be cleared once the background task has died"
        );
        assert_eq!(
            app.tabs[0].interaction.notification.as_deref(),
            Some("MCP server error: address in use")
        );
    }

    #[tokio::test]
    async fn test_apply_tab_defaults() {
        let mut app = make_app(&["line"]).await;
        app.display.show_mode_bar = false;
        app.display.show_borders_default = true;
        app.display.show_line_numbers = false;
        app.display.show_sidebar = false;
        app.display.wrap = true;
        let data: Vec<u8> = b"new\n".to_vec();
        let fr = FileReader::from_bytes(data);
        let lm = LogManager::new(app.db.clone(), None).await;
        let mut tab = super::super::TabState::new(fr, lm, "new".to_string());
        app.apply_tab_defaults(&mut tab).await;
        assert!(!tab.display.show_mode_bar);
        assert!(tab.display.show_borders);
        assert!(!tab.display.show_line_numbers);
        assert!(!tab.display.show_sidebar);
        assert!(tab.display.wrap);
    }

    async fn make_tab_with_format(app: &App, format_name: &str) -> super::super::TabState {
        let fr = FileReader::from_bytes(b"line\n".to_vec());
        let lm = LogManager::new(app.db.clone(), None).await;
        let mut tab = super::super::TabState::new(fr, lm, "test".to_string());
        tab.display.format = crate::parser::find_builtin_parser(format_name).map(Arc::from);
        tab
    }

    #[tokio::test]
    async fn test_apply_default_filters_noop_when_no_format() {
        let mut app = make_app(&["line"]).await;
        app.default_filter_files
            .insert("syslog".to_string(), "/nonexistent.json".to_string());
        let fr = FileReader::from_bytes(b"line\n".to_vec());
        let lm = LogManager::new(app.db.clone(), None).await;
        let mut tab = super::super::TabState::new(fr, lm, "test".to_string());
        tab.display.format = None;
        app.apply_default_filters_if_empty(&mut tab).await;
        assert!(tab.log_manager.get_filters().is_empty());
        assert!(tab.interaction.notification.is_none());
    }

    #[tokio::test]
    async fn test_apply_default_filters_noop_when_no_mapping() {
        let app = make_app(&["line"]).await;
        let mut tab = make_tab_with_format(&app, "syslog").await;
        app.apply_default_filters_if_empty(&mut tab).await;
        assert!(tab.log_manager.get_filters().is_empty());
    }

    #[tokio::test]
    async fn test_apply_default_filters_noop_when_tab_has_filters() {
        let mut app = make_app(&["line"]).await;
        app.default_filter_files
            .insert("syslog".to_string(), "/nonexistent.json".to_string());
        let mut tab = make_tab_with_format(&app, "syslog").await;
        tab.log_manager
            .add_filter_with_color("error".to_string(), FilterType::Include, Default::default())
            .await;
        app.apply_default_filters_if_empty(&mut tab).await;
        assert_eq!(tab.log_manager.get_filters().len(), 1);
        assert!(tab.interaction.notification.is_none());
    }

    #[tokio::test]
    async fn test_apply_default_filters_disabled_filter_still_counts_as_nonempty() {
        let mut app = make_app(&["line"]).await;
        app.default_filter_files
            .insert("syslog".to_string(), "/nonexistent.json".to_string());
        let mut tab = make_tab_with_format(&app, "syslog").await;
        tab.log_manager
            .add_filter_with_color("error".to_string(), FilterType::Include, Default::default())
            .await;
        let id = tab.log_manager.get_filters()[0].id;
        tab.log_manager.toggle_filter(id).await;
        assert!(!tab.log_manager.get_filters()[0].enabled);
        app.apply_default_filters_if_empty(&mut tab).await;
        assert!(tab.interaction.notification.is_none());
    }

    #[tokio::test]
    async fn test_apply_default_filters_loads_file_into_empty_tab() {
        let mut app = make_app(&["line"]).await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.json");
        std::fs::write(
            &path,
            r#"[{"id":1,"pattern":"error","filter_type":"Include","enabled":true,"color_config":null,"use_regex":false,"ignore_case":false,"group":null}]"#,
        )
        .unwrap();
        app.default_filter_files
            .insert("syslog".to_string(), path.to_str().unwrap().to_string());
        let mut tab = make_tab_with_format(&app, "syslog").await;
        app.apply_default_filters_if_empty(&mut tab).await;
        assert_eq!(tab.log_manager.get_filters().len(), 1);
        assert_eq!(tab.log_manager.get_filters()[0].pattern, "error");
    }

    #[tokio::test]
    async fn test_apply_default_filters_sets_notification_on_load_error() {
        let mut app = make_app(&["line"]).await;
        app.default_filter_files
            .insert("syslog".to_string(), "/nonexistent-path.json".to_string());
        let mut tab = make_tab_with_format(&app, "syslog").await;
        app.apply_default_filters_if_empty(&mut tab).await;
        assert!(tab.log_manager.get_filters().is_empty());
        assert!(tab.interaction.notification.is_some());
    }

    #[tokio::test]
    async fn test_apply_default_filters_if_empty_at_loads_by_index() {
        let mut app = make_app(&["line"]).await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.json");
        std::fs::write(
            &path,
            r#"[{"id":1,"pattern":"error","filter_type":"Include","enabled":true,"color_config":null,"use_regex":false,"ignore_case":false,"group":null}]"#,
        )
        .unwrap();
        app.default_filter_files
            .insert("syslog".to_string(), path.to_str().unwrap().to_string());
        app.tabs[0].display.format = crate::parser::find_builtin_parser("syslog").map(Arc::from);
        app.apply_default_filters_if_empty_at(0).await;
        assert_eq!(app.tabs[0].log_manager.get_filters().len(), 1);
    }

    #[tokio::test]
    async fn test_apply_default_filters_if_empty_at_out_of_bounds_is_noop() {
        let mut app = make_app(&["line"]).await;
        app.apply_default_filters_if_empty_at(99).await;
    }

    #[tokio::test]
    async fn test_startup_warnings_cleared_on_key_event() {
        let mut app = make_app(&["line"]).await;
        app.session.startup_warnings = vec!["warning 1".to_string(), "warning 2".to_string()];
        app.handle_key_event(KeyCode::Char('j')).await;
        assert!(app.session.startup_warnings.is_empty());
    }

    async fn app_with_areas(
        visible_lines: usize,
        visible_height: usize,
        log_area: Rect,
        sidebar_area: Option<Rect>,
    ) -> App {
        let db = Arc::new(crate::db::Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        let data: Vec<u8> = (0..visible_lines)
            .map(|i| format!("line {}\n", i))
            .collect::<String>()
            .into_bytes();
        let file_reader = FileReader::from_bytes(data);
        let mut app = App::builder(
            log_manager,
            file_reader,
            crate::theme::Theme::default(),
            Arc::new(crate::config::Keybindings::default()),
        )
        .build()
        .await;
        app.input.log_panel_area = log_area;
        app.input.sidebar_area = sidebar_area;
        app.tabs[0].scroll.visible_height = visible_height;
        app
    }

    #[tokio::test]
    async fn test_hit_test_sidebar_inside_maps_to_filter_index() {
        let log_area = Rect {
            x: 0,
            y: 0,
            width: 60,
            height: 40,
        };
        let sidebar_area = Rect {
            x: 60,
            y: 0,
            width: 20,
            height: 40,
        };
        let mut app = app_with_areas(10, 10, log_area, Some(sidebar_area)).await;
        app.execute_command_str("filter foo".to_string()).await;
        app.execute_command_str("filter bar".to_string()).await;
        let tab = app.tab();
        assert_eq!(app.input.hit_test_sidebar(65, 0, tab), Some(0));
        assert_eq!(app.input.hit_test_sidebar(65, 1, tab), Some(0));
        assert_eq!(app.input.hit_test_sidebar(65, 2, tab), Some(1));
        assert_eq!(app.input.hit_test_sidebar(65, 5, tab), Some(1));
    }

    #[tokio::test]
    async fn test_hit_test_sidebar_accounts_for_scroll_offset() {
        use crate::mode::filter_mode::FilterManagementMode;
        let log_area = Rect {
            x: 0,
            y: 0,
            width: 60,
            height: 40,
        };
        // Wide enough that no filter row wraps; only 10 rows tall:
        // 1 title row + 9 content rows.
        let sidebar_area = Rect {
            x: 60,
            y: 0,
            width: 40,
            height: 10,
        };
        let mut app = app_with_areas(10, 40, log_area, Some(sidebar_area)).await;
        app.tabs[0].display.show_borders = false;
        for i in 0..30 {
            app.execute_command_str(format!("filter pattern_{i}")).await;
        }
        // Select a filter near the bottom so the sidebar scrolls down.
        app.tabs[0].interaction.mode = Box::new(FilterManagementMode::new(25));
        let tab = app.tab();
        // With 9 visible content rows and selection at 25, the sidebar scrolls
        // so filters 17..=25 occupy content rows 0..=8.
        assert_eq!(app.input.hit_test_sidebar(65, 1, tab), Some(17));
        assert_eq!(app.input.hit_test_sidebar(65, 9, tab), Some(25));
    }

    #[tokio::test]
    async fn test_hit_test_sidebar_matches_rendered_rows_when_wrapped_and_scrolled() {
        use crate::mode::filter_mode::FilterManagementMode;
        use crate::ui::widgets::Sidebar;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let log_area = Rect {
            x: 0,
            y: 0,
            width: 60,
            height: 40,
        };
        // Narrow enough that the long filter patterns below wrap onto
        // multiple lines; short enough that the selection forces scrolling.
        let sidebar_area = Rect {
            x: 60,
            y: 0,
            width: 16,
            height: 10,
        };
        let mut app = app_with_areas(10, 40, log_area, Some(sidebar_area)).await;
        app.tabs[0].display.show_borders = false;
        // Each pattern uses a disjoint character so a rendered row can be
        // attributed to exactly one filter regardless of where it wraps.
        // 'b' and 'd' patterns are long enough to wrap across several rows.
        let patterns = [
            "aaaaaaa".to_string(),
            "b".repeat(50),
            "ccccccc".to_string(),
            "d".repeat(50),
            "eeeeeee".to_string(),
            "fffffff".to_string(),
        ];
        for p in &patterns {
            app.execute_command_str(format!("filter {p}")).await;
        }
        app.tabs[0].interaction.mode = Box::new(FilterManagementMode::new(patterns.len() - 1));

        let tab = app.tab();
        let filters = tab.log_manager.get_filters();
        let match_counts = tab.filter.match_counts.clone();
        let (content_w, content_h) =
            crate::ui::widgets::sidebar::sidebar_inner_dims(sidebar_area, false);
        let scroll_offset = crate::ui::widgets::sidebar::compute_scroll_offset(
            filters,
            patterns.len() - 1,
            &match_counts,
            content_w,
            content_h,
            tab.filter.sidebar_scroll,
        );
        let sidebar = Sidebar {
            filters,
            groups: &[],
            match_counts: &match_counts,
            selected_filter_idx: patterns.len() - 1,
            filter_enabled: tab.filter.enabled,
            show_marks_only: tab.filter.show_marks_only,
            filter_progress: None,
            show_borders: false,
            is_filter_mode: false,
            scroll_offset,
            highlight_mode: false,
            search: "",
            searching: false,
            theme: &app.theme,
        };
        let mut terminal =
            Terminal::new(TestBackend::new(sidebar_area.width, sidebar_area.height)).unwrap();
        let buf = terminal
            .draw(|f| f.render_widget(sidebar, f.area()))
            .unwrap();

        let mut checked_rows = 0;
        for row in 1..sidebar_area.height {
            let row_text: String = (0..sidebar_area.width)
                .map(|c| {
                    buf.buffer
                        .cell(ratatui::prelude::Position::new(c, row))
                        .unwrap()
                        .symbol()
                        .to_string()
                })
                .collect();
            let Some(expected_idx) = "abcdef".find(|c| row_text.contains(c)) else {
                continue;
            };
            checked_rows += 1;
            let tab = app.tab();
            let actual = app
                .input
                .hit_test_sidebar(sidebar_area.x, sidebar_area.y + row, tab);
            assert_eq!(
                actual,
                Some(expected_idx),
                "row {row} shows {row_text:?}, expected filter {expected_idx}"
            );
        }
        assert!(checked_rows > 0, "test should have inspected some rows");
    }

    #[tokio::test]
    async fn test_click_sidebar_filter_enters_filter_management_mode() {
        use crate::mode::app_mode::ModeRenderState;
        let log_area = Rect {
            x: 0,
            y: 0,
            width: 60,
            height: 20,
        };
        let sidebar_area = Rect {
            x: 60,
            y: 0,
            width: 20,
            height: 20,
        };
        let mut app = app_with_areas(10, 20, log_area, Some(sidebar_area)).await;
        app.execute_command_str("filter foo".to_string()).await;
        app.execute_command_str("filter bar".to_string()).await;
        // row 2 is where filter 1 renders (row 0 = title, row 1 = filter 0, row 2 = filter 1)
        app.handle_left_click(65, 2).await;
        assert!(matches!(
            app.tabs[0].interaction.mode.render_state(),
            ModeRenderState::FilterManagement {
                selected_index: 1,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn test_click_sidebar_filter_selects_correct_index() {
        use crate::mode::app_mode::ModeRenderState;
        let log_area = Rect {
            x: 0,
            y: 0,
            width: 60,
            height: 20,
        };
        let sidebar_area = Rect {
            x: 60,
            y: 0,
            width: 20,
            height: 20,
        };
        let mut app = app_with_areas(10, 20, log_area, Some(sidebar_area)).await;
        app.execute_command_str("filter foo".to_string()).await;
        app.execute_command_str("filter bar".to_string()).await;
        app.execute_command_str("filter baz".to_string()).await;
        // row 1 = filter 0
        app.handle_left_click(65, 1).await;
        assert!(matches!(
            app.tabs[0].interaction.mode.render_state(),
            ModeRenderState::FilterManagement {
                selected_index: 0,
                ..
            }
        ));
        // row 3 = filter 2
        app.handle_left_click(65, 3).await;
        assert!(matches!(
            app.tabs[0].interaction.mode.render_state(),
            ModeRenderState::FilterManagement {
                selected_index: 2,
                ..
            }
        ));
        // whitespace below all filters clamps to last filter (index 2)
        app.handle_left_click(65, 15).await;
        assert!(matches!(
            app.tabs[0].interaction.mode.render_state(),
            ModeRenderState::FilterManagement {
                selected_index: 2,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn test_mouse_scroll_down_clamps_to_max() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 10,
        };
        let mut app = app_with_areas(15, 10, area, None).await;
        app.tabs[0].scroll.scroll_offset = 0;
        app.mouse_scroll(100);
        // max_scroll = 15 - 1 = 14 (scroll_offset is the focused/last-visible line)
        assert_eq!(app.tabs[0].scroll.scroll_offset, 14);
    }

    #[tokio::test]
    async fn test_mouse_scroll_up_disables_tail_mode() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 10,
        };
        let mut app = app_with_areas(20, 10, area, None).await;
        app.tabs[0].stream.tail_mode = true;
        app.tabs[0].scroll.scroll_offset = 10;
        app.mouse_scroll(-5);
        assert!(
            !app.tabs[0].stream.tail_mode,
            "scroll up should disable tail mode"
        );
    }

    #[tokio::test]
    async fn test_mouse_scroll_down_to_bottom_reenables_tail_mode() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 10,
        };
        // 20 lines, max_scroll = 20 - 1 = 19 (scroll_offset is the focused/last-visible line)
        let mut app = app_with_areas(20, 10, area, None).await;
        app.tabs[0].stream.tail_mode = false;
        app.tabs[0].scroll.scroll_offset = 0;
        app.mouse_scroll(100); // large delta → clamps to max_scroll=19
        assert!(
            app.tabs[0].stream.tail_mode,
            "reaching bottom should re-enable tail mode"
        );
        assert_eq!(app.tabs[0].scroll.scroll_offset, 19);
    }

    #[tokio::test]
    async fn test_mouse_scroll_up_clamps_to_zero() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 10,
        };
        let mut app = app_with_areas(15, 10, area, None).await;
        app.tabs[0].scroll.scroll_offset = 2;
        app.mouse_scroll(-100);
        assert_eq!(app.tabs[0].scroll.scroll_offset, 0);
    }

    #[tokio::test]
    async fn test_mouse_scroll_clears_visual_char_selection() {
        use crate::mode::app_mode::ModeRenderState;
        use crate::mode::visual_char_mode::VisualMode;
        let area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 10,
        };
        let mut app = app_with_areas(15, 10, area, None).await;
        let mut mode = VisualMode::new("hello world".to_string());
        mode.anchor_col = Some(0);
        mode.cursor_col = 4;
        app.tabs[0].interaction.mode = Box::new(mode);
        app.mouse_scroll(1);
        match app.tabs[0].interaction.mode.render_state() {
            ModeRenderState::Visual {
                anchor_col,
                cursor_col,
                ..
            } => {
                assert_eq!(anchor_col, None);
                // cursor_col clamped to new line length, not reset to 0
                assert!(cursor_col <= "line 1".len());
            }
            other => panic!("expected Visual mode, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_mouse_scroll_does_not_affect_other_modes() {
        use crate::mode::app_mode::ModeRenderState;
        let area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 10,
        };
        let mut app = app_with_areas(15, 10, area, None).await;
        app.mouse_scroll(1);
        assert!(matches!(
            app.tabs[0].interaction.mode.render_state(),
            ModeRenderState::Normal
        ));
    }

    #[tokio::test]
    async fn test_left_click_log_panel_selects_line() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 20,
        };
        let mut app = app_with_areas(50, 20, area, None).await;
        app.tabs[0].scroll.scroll_offset = 0;
        app.handle_left_click(10, 7).await;
        assert_eq!(app.tabs[0].scroll.scroll_offset, 7);
    }

    #[tokio::test]
    async fn test_left_click_log_panel_stays_normal_mode() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 20,
        };
        let mut app = app_with_areas(50, 20, area, None).await;
        app.handle_left_click(10, 3).await;
        use crate::mode::app_mode::ModeRenderState;
        assert!(matches!(
            app.tabs[0].interaction.mode.render_state(),
            ModeRenderState::Normal
        ));
    }

    #[tokio::test]
    async fn test_left_click_log_panel_returns_to_normal_from_visual() {
        use crate::mode::app_mode::ModeRenderState;
        use crate::mode::visual_mode::VisualLineMode;
        let area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 20,
        };
        let mut app = app_with_areas(50, 20, area, None).await;
        app.tabs[0].interaction.mode = Box::new(VisualLineMode {
            anchor: 0,
            count: None,
        });
        app.handle_left_click(10, 5).await;
        assert!(matches!(
            app.tabs[0].interaction.mode.render_state(),
            ModeRenderState::Normal
        ));
    }

    #[tokio::test]
    async fn test_handle_double_click_selects_word() {
        use crate::mode::app_mode::ModeRenderState;
        let area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 20,
        };
        let mut app = app_with_areas(5, 20, area, None).await;
        // no borders, no line numbers: char_col = col - area.x
        app.tabs[0].display.show_borders = false;
        app.tabs[0].display.show_line_numbers = false;
        // col 3 is inside "line 0" at visible row 0 → char_col = 3 → 'e' in "line"
        app.handle_double_click(3, 0);
        assert_eq!(app.tabs[0].scroll.scroll_offset, 0);
        match app.tabs[0].interaction.mode.render_state() {
            ModeRenderState::Visual {
                anchor_col,
                cursor_col,
                ..
            } => {
                assert_eq!(anchor_col, Some(0));
                assert_eq!(cursor_col, 3);
            }
            other => panic!("expected Visual mode, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_handle_double_click_on_whitespace_selects_line() {
        use crate::mode::app_mode::ModeRenderState;
        let area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 20,
        };
        let mut app = app_with_areas(5, 20, area, None).await;
        app.tabs[0].display.show_borders = false;
        app.tabs[0].display.show_line_numbers = false;
        // col 4 is the space in "line 0"
        app.handle_double_click(4, 0);
        assert_eq!(app.tabs[0].scroll.scroll_offset, 0);
        assert!(
            matches!(
                app.tabs[0].interaction.mode.render_state(),
                ModeRenderState::Normal
            ),
            "whitespace double-click should stay in Normal mode"
        );
    }

    #[tokio::test]
    async fn test_left_down_no_double_click_without_repeat() {
        use crate::mode::app_mode::ModeRenderState;
        let area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 20,
        };
        let mut app = app_with_areas(20, 20, area, None).await;
        app.handle_left_down(10, 5).await;
        assert!(matches!(
            app.tabs[0].interaction.mode.render_state(),
            ModeRenderState::Normal
        ));
    }

    #[tokio::test]
    async fn test_log_panel_click_is_deferred_until_flush() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 20,
        };
        let mut app = app_with_areas(50, 20, area, None).await;
        app.handle_left_down(10, 7).await;
        // click is deferred: scroll_offset unchanged
        assert_eq!(app.tabs[0].scroll.scroll_offset, 0);
        assert!(app.input.last_click.is_some());
        // flush before timeout: still deferred
        app.flush_pending_click().await;
        assert_eq!(app.tabs[0].scroll.scroll_offset, 0);
        // simulate timeout by backdating the pending click
        app.input.last_click = Some((Instant::now() - Duration::from_millis(400), 10, 7));
        app.flush_pending_click().await;
        assert_eq!(app.tabs[0].scroll.scroll_offset, 7);
        assert!(app.input.last_click.is_none());
    }

    #[tokio::test]
    async fn test_double_click_cancels_deferred_single_click() {
        use crate::mode::app_mode::ModeRenderState;
        let area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 20,
        };
        let mut app = app_with_areas(50, 20, area, None).await;
        app.tabs[0].display.show_borders = false;
        app.tabs[0].display.show_line_numbers = false;
        // first click defers
        app.handle_left_down(3, 0).await;
        assert!(app.input.last_click.is_some());
        // second click at same position within window → double-click, no single-click
        app.handle_left_down(3, 0).await;
        assert!(app.input.last_click.is_none());
        // should be in Visual or VisualLine mode, not Normal
        assert!(!matches!(
            app.tabs[0].interaction.mode.render_state(),
            ModeRenderState::Normal
        ));
    }

    #[tokio::test]
    async fn test_sidebar_click_is_immediate_not_deferred() {
        use crate::mode::app_mode::ModeRenderState;
        let log_area = Rect {
            x: 0,
            y: 0,
            width: 60,
            height: 20,
        };
        let sidebar_area = Rect {
            x: 60,
            y: 0,
            width: 20,
            height: 20,
        };
        let mut app = app_with_areas(10, 20, log_area, Some(sidebar_area)).await;
        app.execute_command_str("filter foo".to_string()).await;
        // sidebar click should take effect immediately, not be deferred
        app.handle_left_down(65, 1).await;
        assert!(app.input.last_click.is_none());
        assert!(matches!(
            app.tabs[0].interaction.mode.render_state(),
            ModeRenderState::FilterManagement { .. }
        ));
    }

    #[tokio::test]
    async fn test_scrollbar_drag_updates_scroll_offset() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        let area = Rect {
            x: 0,
            y: 0,
            width: 20,
            height: 10,
        };
        let mut app = app_with_areas(50, 10, area, None).await;
        app.tabs[0].display.show_borders = false;
        // scrollbar column is area.right() - 1 = 19
        let scrollbar_col = 19u16;
        // simulate pressing down on the scrollbar
        app.handle_mouse_event(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: scrollbar_col,
            row: 0,
            modifiers: crossterm::event::KeyModifiers::NONE,
        })
        .await;
        assert!(app.input.scrollbar_dragging);
        // drag to halfway down the bar
        app.handle_mouse_event(MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: scrollbar_col,
            row: 5,
            modifiers: crossterm::event::KeyModifiers::NONE,
        })
        .await;
        assert!(app.tabs[0].scroll.scroll_offset > 0);
    }

    #[tokio::test]
    async fn test_scrollbar_drag_stopped_on_mouse_up() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        let area = Rect {
            x: 0,
            y: 0,
            width: 20,
            height: 10,
        };
        let mut app = app_with_areas(50, 10, area, None).await;
        app.tabs[0].display.show_borders = false;
        app.input.scrollbar_dragging = true;
        app.handle_mouse_event(MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: 19,
            row: 5,
            modifiers: crossterm::event::KeyModifiers::NONE,
        })
        .await;
        assert!(!app.input.scrollbar_dragging);
    }

    #[tokio::test]
    async fn test_drag_outside_scrollbar_when_not_dragging_does_nothing() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        let area = Rect {
            x: 0,
            y: 0,
            width: 20,
            height: 10,
        };
        let mut app = app_with_areas(50, 10, area, None).await;
        app.tabs[0].display.show_borders = false;
        let initial_offset = app.tabs[0].scroll.scroll_offset;
        app.handle_mouse_event(MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: 5,
            row: 5,
            modifiers: crossterm::event::KeyModifiers::NONE,
        })
        .await;
        assert_eq!(app.tabs[0].scroll.scroll_offset, initial_offset);
    }

    #[tokio::test]
    async fn test_mouse_scroll_down_half_page() {
        use crossterm::event::{MouseEvent, MouseEventKind};
        let area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 10,
        };
        // 20 lines of content, visible_height = 10 → half page = 5
        let mut app = app_with_areas(20, 10, area, None).await;
        app.tabs[0].scroll.visible_height = 10;
        app.tabs[0].scroll.scroll_offset = 0;
        app.handle_mouse_event(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: crossterm::event::KeyModifiers::NONE,
        })
        .await;
        assert_eq!(app.tabs[0].scroll.scroll_offset, 5);
    }

    #[tokio::test]
    async fn test_mouse_scroll_over_sidebar_moves_filter_selection_down() {
        use crate::mode::app_mode::ModeRenderState;
        use crossterm::event::{MouseEvent, MouseEventKind};
        let log_area = Rect {
            x: 0,
            y: 0,
            width: 60,
            height: 20,
        };
        let sidebar_area = Rect {
            x: 60,
            y: 0,
            width: 20,
            height: 20,
        };
        let mut app = app_with_areas(10, 20, log_area, Some(sidebar_area)).await;
        app.execute_command_str("filter foo".to_string()).await;
        app.execute_command_str("filter bar".to_string()).await;
        let initial_offset = app.tabs[0].scroll.scroll_offset;

        app.handle_mouse_event(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 65,
            row: 1,
            modifiers: crossterm::event::KeyModifiers::NONE,
        })
        .await;

        match app.tabs[0].interaction.mode.render_state() {
            ModeRenderState::FilterManagement { selected_index, .. } => {
                assert_eq!(selected_index, 1);
            }
            other => panic!("expected FilterManagement mode, got {other:?}"),
        }
        // Scrolling over the sidebar must not scroll the log panel.
        assert_eq!(app.tabs[0].scroll.scroll_offset, initial_offset);
    }

    #[tokio::test]
    async fn test_mouse_scroll_over_sidebar_moves_filter_selection_up() {
        use crate::mode::app_mode::ModeRenderState;
        use crate::mode::filter_mode::FilterManagementMode;
        use crossterm::event::{MouseEvent, MouseEventKind};
        let log_area = Rect {
            x: 0,
            y: 0,
            width: 60,
            height: 20,
        };
        let sidebar_area = Rect {
            x: 60,
            y: 0,
            width: 20,
            height: 20,
        };
        let mut app = app_with_areas(10, 20, log_area, Some(sidebar_area)).await;
        app.execute_command_str("filter foo".to_string()).await;
        app.execute_command_str("filter bar".to_string()).await;
        app.tabs[0].interaction.mode = Box::new(FilterManagementMode::new(1));

        app.handle_mouse_event(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 65,
            row: 1,
            modifiers: crossterm::event::KeyModifiers::NONE,
        })
        .await;

        match app.tabs[0].interaction.mode.render_state() {
            ModeRenderState::FilterManagement { selected_index, .. } => {
                assert_eq!(selected_index, 0);
            }
            other => panic!("expected FilterManagement mode, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_mouse_scroll_up_half_page() {
        use crossterm::event::{MouseEvent, MouseEventKind};
        let area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 10,
        };
        // 20 lines of content, visible_height = 10 → half page = 5
        let mut app = app_with_areas(20, 10, area, None).await;
        app.tabs[0].scroll.visible_height = 10;
        app.tabs[0].scroll.scroll_offset = 10;
        app.handle_mouse_event(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: crossterm::event::KeyModifiers::NONE,
        })
        .await;
        assert_eq!(app.tabs[0].scroll.scroll_offset, 5);
    }
}
