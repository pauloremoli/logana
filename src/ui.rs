use clap::Parser;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    Frame, Terminal,
    prelude::*,
    style::Modifier,
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use unicode_width::UnicodeWidthStr;

use crate::search::Search;
use crate::theme::{Theme, complete_theme};
use crate::types::{FilterType, LogLevel};
use crate::{
    auto_complete::{
        complete_color, complete_file_path, extract_color_partial, find_command_completions,
        find_matching_command, shell_split,
    },
    mode::app_mode::Mode,
};
use crate::{
    db::{FileContext, FileContextStore, SessionStore},
    mode::normal_mode::NormalMode,
};
use crate::{
    file_reader::FileReader,
    mode::app_mode::{ConfirmRestoreMode, ConfirmRestoreSessionMode},
};
use crate::{
    filters::{SEARCH_STYLE_ID, render_line},
    mode::{command_mode::CommandMode, filter_mode::FilterManagementMode},
};
use crate::{
    log_line::{build_display_json, parse_json_line},
    log_manager::LogManager,
    mode::command_mode::{CommandLine, Commands},
};

// ---------------------------------------------------------------------------
// KeyResult
// ---------------------------------------------------------------------------

pub enum KeyResult {
    Handled,
    Ignored,
    ExecuteCommand(String),
    RestoreSession(Vec<String>),
}

// ---------------------------------------------------------------------------
// TabState
// ---------------------------------------------------------------------------

pub struct TabState {
    pub file_reader: FileReader,
    pub log_manager: LogManager,
    /// Indices into `file_reader` of lines currently visible under the active filters.
    pub visible_indices: Vec<usize>,
    pub mode: Box<dyn Mode>,
    pub scroll_offset: usize,
    pub viewport_offset: usize,
    pub show_sidebar: bool,
    pub g_key_pressed: bool,
    pub wrap: bool,
    pub show_line_numbers: bool,
    pub horizontal_scroll: usize,
    pub search: Search,
    pub command_error: Option<String>,
    pub level_colors: bool,
    pub filtering_enabled: bool,
    pub filter_context: Option<usize>,
    pub editing_filter_id: Option<usize>,
    pub visible_height: usize,
    pub title: String,
    pub command_history: Vec<String>,
    /// Active file watcher for this tab (None for stdin tabs or tabs not yet watching).
    pub watch_state: Option<FileWatchState>,
    /// JSON field names that should be hidden from display (filter evaluation still uses raw line).
    pub hidden_fields: HashSet<String>,
    /// JSON field 0-based indices that should be hidden from display.
    pub hidden_field_indices: HashSet<usize>,
}

impl TabState {
    pub fn new(file_reader: FileReader, log_manager: LogManager, title: String) -> Self {
        let mut tab = TabState {
            file_reader,
            log_manager,
            visible_indices: Vec::new(),
            mode: Box::new(NormalMode),
            scroll_offset: 0,
            viewport_offset: 0,
            show_sidebar: true,
            g_key_pressed: false,
            wrap: true,
            show_line_numbers: true,
            horizontal_scroll: 0,
            search: Search::new(),
            command_error: None,
            level_colors: true,
            filtering_enabled: true,
            filter_context: None,
            editing_filter_id: None,
            visible_height: 0,
            title,
            command_history: Vec::new(),
            watch_state: None,
            hidden_fields: HashSet::new(),
            hidden_field_indices: HashSet::new(),
        };
        tab.refresh_visible();
        tab
    }

    /// Recompute which file lines are visible under the current filters.
    pub fn refresh_visible(&mut self) {
        if !self.filtering_enabled {
            self.visible_indices = (0..self.file_reader.line_count()).collect();
            return;
        }
        let (fm, _) = self.log_manager.build_filter_manager();
        self.visible_indices = fm.compute_visible(&self.file_reader);
    }

    pub fn scroll_to_line_idx(&mut self, line_idx: usize) {
        if let Some(index) = self.visible_indices.iter().position(|&i| i == line_idx) {
            self.scroll_offset = index;
        }
    }

    pub fn to_file_context(&self) -> Option<FileContext> {
        let source = self.log_manager.source_file()?;
        let marked_lines = self.log_manager.get_marked_indices();
        let file_hash = LogManager::compute_file_hash(source);
        Some(FileContext {
            source_file: source.to_string(),
            scroll_offset: self.scroll_offset,
            search_query: self.search.get_pattern().unwrap_or_default().to_string(),
            wrap: self.wrap,
            level_colors: self.level_colors,
            show_sidebar: self.show_sidebar,
            horizontal_scroll: self.horizontal_scroll,
            marked_lines,
            file_hash,
            show_line_numbers: self.show_line_numbers,
        })
    }

    pub fn apply_file_context(&mut self, ctx: &FileContext) {
        self.scroll_offset = ctx.scroll_offset;
        self.wrap = ctx.wrap;
        self.level_colors = ctx.level_colors;
        self.show_sidebar = ctx.show_sidebar;
        self.show_line_numbers = ctx.show_line_numbers;
        self.horizontal_scroll = ctx.horizontal_scroll;
        if !ctx.marked_lines.is_empty() {
            self.log_manager.set_marks(ctx.marked_lines.clone());
        }
        if !ctx.search_query.is_empty() {
            let _ = self
                .search
                .search(&ctx.search_query, &self.visible_indices, &self.file_reader);
        }
    }
}

impl std::fmt::Debug for TabState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TabState")
            .field("title", &self.title)
            .field("mode", &self.mode)
            .field("scroll_offset", &self.scroll_offset)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// FileLoadState / LoadContext
// ---------------------------------------------------------------------------

/// What to do once a background file load completes.
pub enum LoadContext {
    /// Replace the placeholder file_reader in the initial tab (startup).
    ReplaceInitialTab,
    /// Open as a new tab; continue with any remaining session-restore files.
    SessionRestoreTab {
        remaining: VecDeque<String>,
        total: usize,
        initial_tab_idx: usize,
    },
}

/// Tracks a single in-progress background file load.
pub struct FileLoadState {
    pub path: String,
    /// Current progress fraction (0.0–1.0); updated by the background task.
    pub progress_rx: tokio::sync::watch::Receiver<f64>,
    /// Delivers the finished `FileReader` (or error) when indexing is done.
    pub result_rx: tokio::sync::oneshot::Receiver<std::io::Result<FileReader>>,
    pub total_bytes: u64,
    pub on_complete: LoadContext,
}

/// Tracks an in-progress stdin stream.  Kept separate from `file_load_state`
/// so session-restore loads cannot overwrite it.
pub struct StdinLoadState {
    /// Receives snapshots of all complete lines accumulated so far.
    /// Updated every second.  When the sender is dropped stdin has closed.
    pub snapshot_rx: tokio::sync::watch::Receiver<Vec<u8>>,
}

/// Per-tab state for watching a file for new appended content.
pub struct FileWatchState {
    /// Receives stripped byte chunks whenever new lines are appended to the file.
    pub new_data_rx: tokio::sync::watch::Receiver<Vec<u8>>,
}

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
    pub async fn new(log_manager: LogManager, file_reader: FileReader, theme: Theme) -> App {
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
        }
    }

    pub fn tab(&self) -> &TabState {
        &self.tabs[self.active_tab]
    }

    pub fn tab_mut(&mut self) -> &mut TabState {
        &mut self.tabs[self.active_tab]
    }

    pub async fn open_file(&mut self, path: &str) -> Result<(), String> {
        let file_path_obj = std::path::Path::new(path);
        if !file_path_obj.exists() {
            return Err(format!("File '{}' not found.", path));
        }
        if file_path_obj.is_dir() {
            return Err(format!("'{}' is a directory, not a file.", path));
        }

        let file_reader =
            FileReader::new(path).map_err(|e| format!("Failed to read '{}': {}", path, e))?;
        let log_manager = LogManager::new(self.db.clone(), Some(path.to_string())).await;

        let title = file_path_obj
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path)
            .to_string();

        let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let mut tab = TabState::new(file_reader, log_manager, title);

        if let Ok(Some(ctx)) = self.db.load_file_context(path).await {
            tab.mode = Box::new(ConfirmRestoreMode { context: ctx });
        }

        let watch_rx = FileReader::spawn_file_watcher(path.to_string(), file_size).await;
        tab.watch_state = Some(FileWatchState {
            new_data_rx: watch_rx,
        });

        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
        Ok(())
    }

    /// Start loading `path` in the background via tokio's blocking thread pool.
    ///
    /// Progress streams through `FileLoadState::progress_rx` (0.0–1.0);
    /// the completed `FileReader` arrives on `result_rx`.  `advance_file_load`
    /// must be called each frame to poll completion and drive the progress bar.
    ///
    /// Returns a boxed future to break the mutual recursion with `skip_or_fail_load`.
    pub fn begin_file_load(
        &mut self,
        path: String,
        context: LoadContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + '_>> {
        Box::pin(async move {
            match FileReader::load(path.clone()).await {
                Ok(handle) => {
                    self.file_load_state = Some(FileLoadState {
                        path,
                        progress_rx: handle.progress_rx,
                        result_rx: handle.result_rx,
                        total_bytes: handle.total_bytes,
                        on_complete: context,
                    });
                }
                Err(_) => self.skip_or_fail_load(context).await,
            }
        })
    }

    /// Start streaming stdin in the background.  Stored in a dedicated slot so
    /// session-restore file loads cannot overwrite it.
    pub async fn begin_stdin_load(&mut self) {
        let snapshot_rx = FileReader::stream_stdin().await;
        self.stdin_load_state = Some(StdinLoadState { snapshot_rx });
    }

    /// Poll for new stdin data each frame and apply it to the stdin tab.
    async fn advance_stdin_load(&mut self) {
        let status = self
            .stdin_load_state
            .as_mut()
            .map(|s| s.snapshot_rx.has_changed());

        match status {
            Some(Ok(true)) => {
                let data = self
                    .stdin_load_state
                    .as_mut()
                    .unwrap()
                    .snapshot_rx
                    .borrow_and_update()
                    .clone();
                self.update_stdin_tab(data).await;
            }
            Some(Err(_)) => {
                // Sender dropped — stdin closed.  Apply final snapshot and clean up.
                let data = self
                    .stdin_load_state
                    .as_mut()
                    .unwrap()
                    .snapshot_rx
                    .borrow()
                    .clone();
                self.stdin_load_state = None;
                self.update_stdin_tab(data).await;
            }
            _ => {}
        }
    }

    /// Apply a stdin data snapshot to the stdin tab.
    ///
    /// If the placeholder tab (no source, empty) still exists it is updated
    /// in-place preserving its mode (e.g. session-restore modal).  Otherwise
    /// a new tab is pushed (session restore already claimed the placeholder).
    /// Follow mode: if the user was at the last line, stay there.
    async fn update_stdin_tab(&mut self, data: Vec<u8>) {
        if data.is_empty() {
            return;
        }
        if let Some(idx) = self
            .tabs
            .iter()
            .position(|t| t.log_manager.source_file().is_none())
        {
            let last_count = self.tabs[idx].visible_indices.len();
            let at_end = last_count == 0 || self.tabs[idx].scroll_offset + 1 >= last_count;

            self.tabs[idx].file_reader = FileReader::from_bytes(data);
            self.tabs[idx].refresh_visible();

            if at_end {
                let new_count = self.tabs[idx].visible_indices.len();
                self.tabs[idx].scroll_offset = new_count.saturating_sub(1);
            }
        } else {
            // Placeholder was removed by session restore — push a new stdin tab.
            let file_reader = FileReader::from_bytes(data);
            if file_reader.line_count() > 0 {
                let log_manager = LogManager::new(self.db.clone(), None).await;
                let mut tab = TabState::new(file_reader, log_manager, "stdin".to_string());
                tab.scroll_offset = tab.visible_indices.len().saturating_sub(1);
                self.tabs.push(tab);
            }
        }
    }

    /// Poll for completion of the current background file load (called every frame).
    async fn advance_file_load(&mut self) {
        // try_recv needs &mut, so we can't hold a shared borrow of file_load_state.
        let done_result = self
            .file_load_state
            .as_mut()
            .and_then(|s| s.result_rx.try_recv().ok());

        if let Some(load_result) = done_result {
            let state = self.file_load_state.take().unwrap();
            match load_result {
                Ok(file_reader) => {
                    self.on_load_success(
                        state.path,
                        state.total_bytes,
                        state.on_complete,
                        file_reader,
                    )
                    .await
                }
                Err(_) => self.skip_or_fail_load(state.on_complete).await,
            }
        }
    }

    /// Handle a completed successful load, then start a file watcher for the tab.
    async fn on_load_success(
        &mut self,
        path: String,
        total_bytes: u64,
        context: LoadContext,
        file_reader: FileReader,
    ) {
        match context {
            LoadContext::ReplaceInitialTab => {
                if self.tabs.is_empty() {
                    return;
                }
                self.tabs[0].file_reader = file_reader;
                self.tabs[0].refresh_visible();
                if let Ok(Some(ctx)) = self.db.load_file_context(&path).await {
                    self.tabs[0].mode = Box::new(ConfirmRestoreMode { context: ctx });
                }
                let watch_rx = FileReader::spawn_file_watcher(path, total_bytes).await;
                self.tabs[0].watch_state = Some(FileWatchState {
                    new_data_rx: watch_rx,
                });
            }
            LoadContext::SessionRestoreTab {
                mut remaining,
                total,
                initial_tab_idx,
            } => {
                let title = std::path::Path::new(&path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&path)
                    .to_string();
                let log_manager = LogManager::new(self.db.clone(), Some(path.clone())).await;
                let mut tab = TabState::new(file_reader, log_manager, title);
                if let Ok(Some(ctx)) = self.db.load_file_context(&path).await {
                    tab.apply_file_context(&ctx);
                }
                let watch_rx = FileReader::spawn_file_watcher(path.clone(), total_bytes).await;
                tab.watch_state = Some(FileWatchState {
                    new_data_rx: watch_rx,
                });
                self.tabs.push(tab);

                if let Some(next) = remaining.pop_front() {
                    self.begin_file_load(
                        next,
                        LoadContext::SessionRestoreTab {
                            remaining,
                            total,
                            initial_tab_idx,
                        },
                    )
                    .await;
                } else if self.tabs.len() > 1 {
                    let is_placeholder = self.tabs[initial_tab_idx]
                        .log_manager
                        .source_file()
                        .is_none()
                        && self.tabs[initial_tab_idx].file_reader.line_count() == 0;
                    if is_placeholder {
                        self.tabs.remove(initial_tab_idx);
                        self.active_tab = 0;
                    }
                    // If the tab was populated by stdin, leave active_tab as-is so
                    // the user stays on the stdin content.
                }
            }
        }
    }

    /// Poll each tab's file watcher for new appended content (called every frame).
    ///
    /// If the user's scroll position is at the last visible line (follow mode),
    /// it is advanced to stay at the new last line after content is appended.
    fn advance_file_watches(&mut self) {
        for i in 0..self.tabs.len() {
            let status = self.tabs[i]
                .watch_state
                .as_mut()
                .map(|ws| ws.new_data_rx.has_changed());

            match status {
                Some(Ok(true)) => {
                    let new_data = self.tabs[i]
                        .watch_state
                        .as_mut()
                        .unwrap()
                        .new_data_rx
                        .borrow_and_update()
                        .clone();
                    if new_data.is_empty() {
                        continue;
                    }
                    let at_end = {
                        let tab = &self.tabs[i];
                        tab.visible_indices.is_empty()
                            || tab.scroll_offset + 1 >= tab.visible_indices.len()
                    };
                    self.tabs[i].file_reader.append_bytes(&new_data);
                    self.tabs[i].refresh_visible();
                    if at_end {
                        let new_count = self.tabs[i].visible_indices.len();
                        self.tabs[i].scroll_offset = new_count.saturating_sub(1);
                    }
                }
                Some(Err(_)) => {
                    // Sender dropped — background watcher task stopped.
                    self.tabs[i].watch_state = None;
                }
                _ => {}
            }
        }
    }

    /// Called when a file load fails or the file cannot be opened.
    async fn skip_or_fail_load(&mut self, context: LoadContext) {
        if let LoadContext::SessionRestoreTab {
            mut remaining,
            total,
            initial_tab_idx,
        } = context
        {
            if let Some(next) = remaining.pop_front() {
                self.begin_file_load(
                    next,
                    LoadContext::SessionRestoreTab {
                        remaining,
                        total,
                        initial_tab_idx,
                    },
                )
                .await;
            } else if self.tabs.len() > 1 {
                self.tabs.remove(initial_tab_idx);
                self.active_tab = 0;
            }
        }
        // ReplaceInitialTab failure: stay with the empty initial tab.
    }

    /// Begin a session restore: kick off the first file load.
    async fn restore_session(&mut self, files: Vec<String>) {
        if files.is_empty() {
            return;
        }
        let total = files.len();
        let mut queue: VecDeque<String> = files.into_iter().collect();
        let first = queue.pop_front().unwrap();
        let initial_tab_idx = self.active_tab;
        self.tabs[self.active_tab].mode = Box::new(NormalMode);
        self.begin_file_load(
            first,
            LoadContext::SessionRestoreTab {
                remaining: queue,
                total,
                initial_tab_idx,
            },
        )
        .await;
    }

    async fn save_tab_context(&self, tab: &TabState) {
        if let Some(ctx) = tab.to_file_context() {
            let _ = self.db.save_file_context(&ctx).await;
        }
    }

    async fn save_all_contexts(&self) {
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

    async fn handle_global_key(&mut self, key: KeyCode, modifiers: KeyModifiers) {
        let ctrl = modifiers.contains(KeyModifiers::CONTROL);
        match key {
            KeyCode::Char('q') if modifiers.is_empty() => {
                self.save_all_contexts().await;
                self.should_quit = true;
            }
            KeyCode::Tab => {
                if self.tabs.len() > 1 {
                    self.active_tab = (self.active_tab + 1) % self.tabs.len();
                }
            }
            KeyCode::BackTab => {
                if self.tabs.len() > 1 {
                    self.active_tab = if self.active_tab == 0 {
                        self.tabs.len() - 1
                    } else {
                        self.active_tab - 1
                    };
                }
            }
            KeyCode::Char('w') if ctrl => {
                if self.close_tab().await {
                    self.save_all_contexts().await;
                    self.should_quit = true;
                }
            }
            KeyCode::Char('t') if ctrl => {
                let history = self.tabs[self.active_tab].command_history.clone();
                self.tabs[self.active_tab].mode =
                    Box::new(CommandMode::with_history("open ".to_string(), 5, history));
            }
            _ => {}
        }
    }

    /// Execute a command string, transitioning mode on success/failure.
    pub async fn execute_command_str(&mut self, cmd: String) {
        let result = self.run_command(&cmd).await;
        let tab = &mut self.tabs[self.active_tab];
        match result {
            Ok(()) => {
                if !cmd.trim().is_empty() {
                    tab.command_history.push(cmd.trim().to_string());
                }
                if let Some(idx) = tab.filter_context.take() {
                    tab.mode = Box::new(FilterManagementMode {
                        selected_filter_index: idx,
                    });
                } else {
                    tab.mode = Box::new(NormalMode);
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

    async fn run_command(&mut self, input: &str) -> Result<(), String> {
        let args = CommandLine::try_parse_from(shell_split(input))
            .map_err(|e| format!("Invalid command: {}", e))?;

        match args.command {
            Some(Commands::Filter { pattern, fg, bg, m }) => {
                if let Some(old_id) = self.tabs[self.active_tab].editing_filter_id.take() {
                    self.tabs[self.active_tab]
                        .log_manager
                        .remove_filter(old_id)
                        .await;
                }
                self.tabs[self.active_tab]
                    .log_manager
                    .add_filter_with_color(
                        pattern,
                        FilterType::Include,
                        fg.as_deref(),
                        bg.as_deref(),
                        m,
                    )
                    .await;
                self.tabs[self.active_tab].scroll_offset = 0;
                self.tabs[self.active_tab].refresh_visible();
            }
            Some(Commands::Exclude { pattern }) => {
                if let Some(old_id) = self.tabs[self.active_tab].editing_filter_id.take() {
                    self.tabs[self.active_tab]
                        .log_manager
                        .remove_filter(old_id)
                        .await;
                }
                self.tabs[self.active_tab]
                    .log_manager
                    .add_filter_with_color(pattern, FilterType::Exclude, None, None, false)
                    .await;
                self.tabs[self.active_tab].scroll_offset = 0;
                self.tabs[self.active_tab].refresh_visible();
            }
            Some(Commands::SetColor { fg, bg, m }) => {
                let selected_filter_index = self.tabs[self.active_tab].filter_context.unwrap_or(0);
                let filters = self.tabs[self.active_tab].log_manager.get_filters();
                if let Some(filter) = filters.get(selected_filter_index)
                    && filter.filter_type == FilterType::Include
                {
                    let filter_id = filter.id;
                    self.tabs[self.active_tab]
                        .log_manager
                        .set_color_config(filter_id, fg.as_deref(), bg.as_deref(), m)
                        .await;
                    self.tabs[self.active_tab].refresh_visible();
                }
            }
            Some(Commands::ExportMarked { path }) => {
                if !path.is_empty() {
                    let tab = &self.tabs[self.active_tab];
                    let marked_lines = tab.log_manager.get_marked_lines(&tab.file_reader);
                    let mut content: Vec<u8> = Vec::new();
                    for line in marked_lines {
                        content.extend_from_slice(line);
                        content.push(b'\n');
                    }
                    let _ = std::fs::write(path, content);
                }
            }
            Some(Commands::SaveFilters { path }) => {
                if !path.is_empty() {
                    self.tabs[self.active_tab]
                        .log_manager
                        .save_filters(&path)
                        .map_err(|e| format!("Failed to save filters: {}", e))?;
                }
            }
            Some(Commands::LoadFilters { path }) => {
                if !path.is_empty() {
                    self.tabs[self.active_tab]
                        .log_manager
                        .load_filters(&path)
                        .await
                        .map_err(|e| format!("Failed to load filters: {}", e))?;
                    self.tabs[self.active_tab].refresh_visible();
                }
            }
            Some(Commands::Wrap) => {
                self.tabs[self.active_tab].wrap = !self.tabs[self.active_tab].wrap;
            }
            Some(Commands::LineNumbers) => {
                self.tabs[self.active_tab].show_line_numbers =
                    !self.tabs[self.active_tab].show_line_numbers;
            }
            Some(Commands::LevelColors) => {
                self.tabs[self.active_tab].level_colors = !self.tabs[self.active_tab].level_colors;
            }
            Some(Commands::SetTheme { theme_name }) => {
                let theme_filename = format!("{}.json", theme_name.to_lowercase());
                self.theme = Theme::from_file(&theme_filename)
                    .map_err(|e| format!("Failed to load theme '{}': {}", theme_name, e))?;
            }
            Some(Commands::Open { path }) => {
                self.open_file(&path).await?;
            }
            Some(Commands::CloseTab) => {
                if self.tabs.len() <= 1 {
                    return Err("Cannot close last tab. Use 'q' to quit.".to_string());
                }
                self.tabs.remove(self.active_tab);
                if self.active_tab >= self.tabs.len() {
                    self.active_tab = self.tabs.len() - 1;
                }
            }
            Some(Commands::ClearFilters) => {
                self.tabs[self.active_tab].log_manager.clear_filters().await;
                self.tabs[self.active_tab].refresh_visible();
            }
            Some(Commands::DisableFilters) => {
                self.tabs[self.active_tab]
                    .log_manager
                    .disable_all_filters()
                    .await;
                self.tabs[self.active_tab].refresh_visible();
            }
            Some(Commands::EnableFilters) => {
                self.tabs[self.active_tab]
                    .log_manager
                    .enable_all_filters()
                    .await;
                self.tabs[self.active_tab].refresh_visible();
            }
            Some(Commands::Filtering) => {
                let tab = &mut self.tabs[self.active_tab];
                tab.filtering_enabled = !tab.filtering_enabled;
                tab.refresh_visible();
            }
            Some(Commands::HideField { field }) => {
                let tab = &mut self.tabs[self.active_tab];
                if let Ok(idx) = field.parse::<usize>() {
                    tab.hidden_field_indices.insert(idx);
                } else {
                    tab.hidden_fields.insert(field);
                }
            }
            Some(Commands::ShowField { field }) => {
                let tab = &mut self.tabs[self.active_tab];
                if let Ok(idx) = field.parse::<usize>() {
                    tab.hidden_field_indices.remove(&idx);
                } else {
                    tab.hidden_fields.remove(&field);
                }
            }
            Some(Commands::ShowAllFields) => {
                let tab = &mut self.tabs[self.active_tab];
                tab.hidden_fields.clear();
                tab.hidden_field_indices.clear();
            }
            None => {}
        }
        Ok(())
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
        }
    }

    fn ui(&mut self, frame: &mut Frame) {
        let size = frame.size();
        frame.render_widget(Block::default().bg(self.theme.root_bg), size);

        let has_multiple_tabs = self.tabs.len() > 1;

        // Extract mode-derived state up front to avoid holding a borrow over the rest of rendering
        let has_input_bar = self.tabs[self.active_tab].mode.needs_input_bar();
        let command_input: Option<(String, usize)> = self.tabs[self.active_tab]
            .mode
            .command_state()
            .map(|(s, c)| (s.to_string(), c));
        let search_input: Option<(String, bool)> = self.tabs[self.active_tab]
            .mode
            .search_state()
            .map(|(s, f)| (s.to_string(), f));
        let is_confirm_restore = self.tabs[self.active_tab]
            .mode
            .confirm_restore_context()
            .is_some();
        let session_files: Option<Vec<String>> = self.tabs[self.active_tab]
            .mode
            .confirm_restore_session_files()
            .map(|f| f.to_vec());
        let selected_filter_idx = self.tabs[self.active_tab]
            .mode
            .selected_filter_index()
            .unwrap_or(0);
        let status_line = self.tabs[self.active_tab].mode.status_line().to_string();

        if is_confirm_restore {
            self.render_confirm_restore_modal(frame);
            return;
        }

        // Compute how many rows the status bar needs so wrapped text is fully visible.
        let inner_width = (size.width as usize).saturating_sub(2); // minus 2 for L/R borders
        let content_lines = count_wrapped_lines(&status_line, inner_width);
        let status_height = (content_lines + 2).min(6).max(3) as u16; // +2 for borders

        let mut constraints = vec![];
        if has_multiple_tabs {
            constraints.push(Constraint::Length(1)); // Tab bar
        }
        constraints.push(Constraint::Min(1)); // Main content
        if has_input_bar {
            constraints.push(Constraint::Length(1)); // input line
            constraints.push(Constraint::Length(1)); // hint line
        }
        constraints.push(Constraint::Length(status_height)); // command list
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(size);

        let mut chunk_idx = 0;

        self.render_tab_bar(frame, has_multiple_tabs, &chunks, &mut chunk_idx);

        let main_chunk = chunks[chunk_idx];
        chunk_idx += 1;

        let tab = &self.tabs[self.active_tab];

        let (logs_area, sidebar_area) = if tab.show_sidebar {
            let horizontal = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Min(1), Constraint::Length(30)])
                .split(main_chunk);
            (horizontal[0], Some(horizontal[1]))
        } else {
            (main_chunk, None)
        };

        self.render_logs_panel(frame, logs_area);

        self.render_side_bar(frame, selected_filter_idx, sidebar_area);

        self.render_command_bar(frame, command_input, &chunks, chunk_idx);

        self.render_input_bar(frame, search_input, &chunks, chunk_idx);

        let command_list = Paragraph::new(status_line)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(self.theme.border)),
            )
            .wrap(Wrap { trim: true })
            .style(Style::default().fg(self.theme.text));
        frame.render_widget(command_list, *chunks.last().unwrap());

        // Session restore modal renders on top of the full TUI so stdin content
        // is visible behind it.
        if let Some(files) = session_files {
            self.render_confirm_restore_session_modal(frame, &files);
        }

        // Loading status bar renders last, on top of everything.
        self.render_loading_status_bar(frame);
    }

    fn render_logs_panel(&mut self, frame: &mut Frame<'_>, logs_area: Rect) {
        let num_visible = self.tabs[self.active_tab].visible_indices.len();

        let visible_height = (logs_area.height as usize).saturating_sub(2);
        self.tabs[self.active_tab].visible_height = visible_height;

        let show_line_numbers = self.tabs[self.active_tab].show_line_numbers;
        let line_number_width = if show_line_numbers {
            num_visible.max(1).to_string().len()
        } else {
            0
        };

        let ln_prefix_width = if show_line_numbers {
            line_number_width + 1
        } else {
            0
        };
        let inner_width = (logs_area.width as usize).saturating_sub(2 + ln_prefix_width);

        let wrap = self.tabs[self.active_tab].wrap;

        // Clamp scroll_offset
        if num_visible > 0 && self.tabs[self.active_tab].scroll_offset >= num_visible {
            self.tabs[self.active_tab].scroll_offset = num_visible - 1;
        }

        let scroll_offset = self.tabs[self.active_tab].scroll_offset;
        let viewport_offset = self.tabs[self.active_tab].viewport_offset;

        let new_viewport = if scroll_offset < viewport_offset {
            scroll_offset
        } else if wrap && inner_width > 0 && num_visible > 0 {
            let rows_used: usize = (viewport_offset..=scroll_offset)
                .map(|i| {
                    let li = self.tabs[self.active_tab].visible_indices[i];
                    line_row_count(
                        self.tabs[self.active_tab].file_reader.get_line(li),
                        inner_width,
                    )
                })
                .sum();
            if rows_used > visible_height {
                let mut rows = 0usize;
                let mut new_vp = scroll_offset + 1;
                loop {
                    if new_vp == 0 {
                        break;
                    }
                    new_vp -= 1;
                    let li = self.tabs[self.active_tab].visible_indices[new_vp];
                    let h = line_row_count(
                        self.tabs[self.active_tab].file_reader.get_line(li),
                        inner_width,
                    );
                    if rows + h > visible_height {
                        new_vp += 1;
                        break;
                    }
                    rows += h;
                    if new_vp == 0 {
                        break;
                    }
                }
                new_vp.min(scroll_offset)
            } else {
                viewport_offset
            }
        } else if visible_height > 0 && scroll_offset >= viewport_offset + visible_height {
            scroll_offset - visible_height + 1
        } else {
            viewport_offset
        };

        self.tabs[self.active_tab].viewport_offset = new_viewport;
        let start = new_viewport;

        let end = if wrap && inner_width > 0 {
            let mut rows = 0usize;
            let mut e = start;
            while e < num_visible {
                let li = self.tabs[self.active_tab].visible_indices[e];
                let h = line_row_count(
                    self.tabs[self.active_tab].file_reader.get_line(li),
                    inner_width,
                );
                if rows + h > visible_height {
                    break;
                }
                rows += h;
                e += 1;
            }
            if e == start && start < num_visible {
                e = start + 1;
            }
            e
        } else {
            (start + visible_height).min(num_visible)
        };

        let (filter_manager, mut styles) = self.tabs[self.active_tab]
            .log_manager
            .build_filter_manager();
        let search_style = Style::default()
            .fg(Color::Black)
            .bg(self.theme.text_highlight);
        styles.resize(256, Style::default());
        styles[255] = search_style;

        let search_results = self.tabs[self.active_tab].search.get_results();
        let search_map: HashMap<usize, &crate::types::SearchResult> =
            search_results.iter().map(|r| (r.line_idx, r)).collect();

        let theme = &self.theme;
        let level_colors = self.tabs[self.active_tab].level_colors;
        let current_scroll = self.tabs[self.active_tab].scroll_offset;
        // Clone the hidden-field sets so the closure doesn't borrow `self` while iterating.
        let hidden_fields = self.tabs[self.active_tab].hidden_fields.clone();
        let hidden_field_indices = self.tabs[self.active_tab].hidden_field_indices.clone();
        let _any_hidden = !hidden_fields.is_empty() || !hidden_field_indices.is_empty();

        let log_lines: Vec<Line> = self.tabs[self.active_tab].visible_indices[start..end]
            .iter()
            .enumerate()
            .map(|(vis_idx, &line_idx)| {
                let line_bytes = self.tabs[self.active_tab].file_reader.get_line(line_idx);
                let is_current = start + vis_idx == current_scroll;
                let is_marked = self.tabs[self.active_tab].log_manager.is_marked(line_idx);

                let mut base_style = Style::default().fg(theme.text);
                if level_colors {
                    match LogLevel::detect_from_bytes(line_bytes) {
                        LogLevel::Error => base_style = base_style.fg(theme.error_fg),
                        LogLevel::Warning => base_style = base_style.fg(theme.warning_fg),
                        _ => {}
                    }
                }
                if is_marked {
                    base_style = base_style
                        .fg(theme.text_highlight)
                        .add_modifier(Modifier::BOLD);
                }

                let render_style = if is_current {
                    Style::default().fg(theme.text).bg(theme.border)
                } else {
                    base_style
                };

                // For JSON lines always render parsed key=value fields instead of raw JSON.
                // Fields hidden by name or index are omitted from the display.
                // Filter evaluation (include/exclude decisions) uses the raw bytes, so
                // filtering is unaffected by field hiding.
                let json_display: Option<String> = parse_json_line(line_bytes).map(|fields| {
                    build_display_json(&fields, &hidden_fields, &hidden_field_indices)
                });

                let mut line = if let Some(display) = json_display {
                    Line::from(display)
                } else {
                    let mut collector = filter_manager.evaluate_line(line_bytes);
                    if let Some(sr) = search_map.get(&line_idx) {
                        collector.with_priority(1000);
                        for &(s, e) in &sr.matches {
                            collector.push(s, e, SEARCH_STYLE_ID);
                        }
                    }
                    render_line(&collector, &styles)
                };
                line = line.style(render_style);

                if show_line_numbers {
                    let line_num = line_idx + 1;
                    let line_num_str = format!("{:>width$} ", line_num, width = line_number_width);
                    let line_num_style = Style::default()
                        .fg(theme.border)
                        .add_modifier(Modifier::DIM);
                    let mut all_spans = vec![Span::styled(line_num_str, line_num_style)];
                    all_spans.extend(line.spans);
                    Line::from(all_spans).style(render_style)
                } else {
                    line
                }
            })
            .collect();

        let logs_title = format!(
            "{} ({})",
            self.tabs[self.active_tab]
                .log_manager
                .source_file()
                .map(|s| {
                    std::path::Path::new(s)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(s)
                        .to_string()
                })
                .unwrap_or(String::from("Logs")),
            num_visible
        );

        let mut paragraph = Paragraph::new(log_lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(self.theme.border))
                    .title(logs_title)
                    .title_style(Style::default().fg(self.theme.border_title)),
            )
            .scroll((0, self.tabs[self.active_tab].horizontal_scroll as u16));

        if self.tabs[self.active_tab].wrap {
            paragraph = paragraph.wrap(Wrap { trim: false });
        }

        frame.render_widget(paragraph, logs_area);

        if num_visible > 0 {
            let mut scrollbar_state = ScrollbarState::new(num_visible)
                .position(start)
                .viewport_content_length(end.saturating_sub(start));
            frame.render_stateful_widget(
                Scrollbar::default().orientation(ScrollbarOrientation::VerticalRight),
                logs_area,
                &mut scrollbar_state,
            );
        }
    }

    fn render_input_bar(
        &mut self,
        frame: &mut Frame<'_>,
        search_input: Option<(String, bool)>,
        chunks: &std::rc::Rc<[Rect]>,
        chunk_idx: usize,
    ) {
        if let Some((input_str, forward)) = search_input {
            let prefix = if forward { "/" } else { "?" };
            let search_line = Paragraph::new(format!("{}{}", prefix, input_str))
                .style(Style::default().fg(self.theme.text).bg(self.theme.border))
                .wrap(Wrap { trim: false });
            let input_area = chunks[chunk_idx];
            frame.render_widget(search_line, input_area);
            let cursor_x = input_area.x + 1 + input_str.len() as u16;
            if cursor_x < input_area.x + input_area.width {
                frame.set_cursor(cursor_x, input_area.y);
            }

            let hint_area = chunks[chunk_idx + 1];
            let match_count = self.tabs[self.active_tab].search.get_results().len();
            let hint_text = if !input_str.is_empty() {
                format!("  {} matches", match_count)
            } else {
                "  Type pattern and press Enter to search".to_string()
            };
            let hint = Paragraph::new(hint_text).style(
                Style::default()
                    .fg(self.theme.border)
                    .bg(self.theme.root_bg),
            );
            frame.render_widget(hint, hint_area);
        }
    }

    fn render_confirm_restore_modal(&mut self, frame: &mut Frame<'_>) {
        let modal_width = 44_u16;
        let modal_height = 5_u16;
        let area = frame.size();
        let x = area.x + (area.width.saturating_sub(modal_width)) / 2;
        let y = area.y + (area.height.saturating_sub(modal_height)) / 2;
        let modal_area = ratatui::layout::Rect::new(x, y, modal_width, modal_height);

        frame.render_widget(ratatui::widgets::Clear, modal_area);
        let modal = Paragraph::new(Line::from(vec![
            Span::styled(
                " [y]",
                Style::default()
                    .fg(self.theme.text_highlight)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("es  ", Style::default().fg(self.theme.text)),
            Span::styled(
                "[n]",
                Style::default()
                    .fg(self.theme.text_highlight)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("o ", Style::default().fg(self.theme.text)),
        ]))
        .alignment(ratatui::layout::Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(self.theme.border_title))
                .title(" Restore previous session? ")
                .title_style(
                    Style::default()
                        .fg(self.theme.text_highlight)
                        .add_modifier(Modifier::BOLD),
                )
                .title_alignment(ratatui::layout::Alignment::Center)
                .padding(ratatui::widgets::Padding::new(0, 0, 1, 0)),
        )
        .style(Style::default().bg(self.theme.root_bg));
        frame.render_widget(modal, modal_area);
    }

    fn render_loading_status_bar(&mut self, frame: &mut Frame<'_>) {
        let s = match self.file_load_state.as_ref() {
            Some(s) => s,
            None => return,
        };
        let progress = *s.progress_rx.borrow();
        let subtitle = match &s.on_complete {
            LoadContext::SessionRestoreTab {
                remaining, total, ..
            } => {
                let current = total - remaining.len();
                format!(
                    "({}/{}) {}",
                    current,
                    total,
                    std::path::Path::new(&s.path)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(&s.path)
                )
            }
            LoadContext::ReplaceInitialTab => std::path::Path::new(&s.path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&s.path)
                .to_string(),
        };

        let bar_width = 20_usize;
        let filled = ((progress * bar_width as f64) as usize).min(bar_width);
        let bar = format!(
            "{}{}",
            "\u{2588}".repeat(filled),
            "\u{2591}".repeat(bar_width - filled),
        );
        let pct = (progress * 100.0) as usize;
        let text = format!(" Loading {}  {} {}% ", subtitle, bar, pct);

        let area = frame.size();
        if area.height == 0 {
            return;
        }
        let bar_rect = ratatui::layout::Rect::new(
            area.x,
            area.y + area.height.saturating_sub(1),
            area.width,
            1,
        );
        frame.render_widget(ratatui::widgets::Clear, bar_rect);
        frame.render_widget(
            Paragraph::new(text).style(
                Style::default()
                    .fg(self.theme.root_bg)
                    .bg(self.theme.text_highlight),
            ),
            bar_rect,
        );
    }

    fn render_confirm_restore_session_modal(&mut self, frame: &mut Frame<'_>, files: &[String]) {
        let file_names: Vec<&str> = files
            .iter()
            .map(|f| {
                std::path::Path::new(f)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(f.as_str())
            })
            .collect();

        let modal_width = 50_u16;
        // borders(2) + blank(1) + header(1) + files + blank(1) + y/n(1)
        let modal_height = (file_names.len() as u16 + 6).min(frame.size().height);
        let area = frame.size();
        let x = area.x + (area.width.saturating_sub(modal_width)) / 2;
        let y = area.y + (area.height.saturating_sub(modal_height)) / 2;
        let modal_area = ratatui::layout::Rect::new(x, y, modal_width, modal_height);

        frame.render_widget(ratatui::widgets::Clear, modal_area);

        let mut lines: Vec<Line> = vec![Line::from(Span::styled(
            " Files:",
            Style::default().fg(self.theme.border),
        ))];
        for name in &file_names {
            lines.push(Line::from(Span::styled(
                format!("  • {}", name),
                Style::default().fg(self.theme.text),
            )));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(
                " [y]",
                Style::default()
                    .fg(self.theme.text_highlight)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("es  ", Style::default().fg(self.theme.text)),
            Span::styled(
                "[n]",
                Style::default()
                    .fg(self.theme.text_highlight)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("o", Style::default().fg(self.theme.text)),
        ]));

        let modal = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(self.theme.border_title))
                    .title(" Restore last session? ")
                    .title_style(
                        Style::default()
                            .fg(self.theme.text_highlight)
                            .add_modifier(Modifier::BOLD),
                    )
                    .title_alignment(ratatui::layout::Alignment::Center)
                    .padding(ratatui::widgets::Padding::new(0, 0, 1, 0)),
            )
            .style(Style::default().bg(self.theme.root_bg));
        frame.render_widget(modal, modal_area);
    }

    fn render_command_bar(
        &mut self,
        frame: &mut Frame<'_>,
        command_input: Option<(String, usize)>,
        chunks: &std::rc::Rc<[Rect]>,
        chunk_idx: usize,
    ) {
        if let Some((input_text, cursor_pos)) = command_input {
            let input_prefix = ":";
            let command_line = Paragraph::new(format!("{}{}", input_prefix, input_text))
                .style(Style::default().fg(self.theme.text).bg(self.theme.border))
                .wrap(Wrap { trim: false });
            let input_area = chunks[chunk_idx];
            frame.render_widget(command_line, input_area);
            let cursor_x = input_area.x + 1 + cursor_pos as u16;
            if cursor_x < input_area.x + input_area.width {
                frame.set_cursor(cursor_x, input_area.y);
            }

            let hint_area = chunks[chunk_idx + 1];
            if let Some(err) = &self.tabs[self.active_tab].command_error {
                let error_paragraph = Paragraph::new(err.as_str())
                    .style(Style::default().fg(Color::Red).bg(self.theme.root_bg));
                frame.render_widget(error_paragraph, hint_area);
            } else if let Some(partial) = extract_color_partial(&input_text) {
                let completions = complete_color(partial);
                if !completions.is_empty() {
                    let hint_spans: Vec<Span> = completions
                        .iter()
                        .flat_map(|name| {
                            let color = name.parse::<Color>().unwrap_or(Color::White);
                            vec![
                                Span::styled(
                                    format!(" {} ", name),
                                    Style::default().fg(color).bg(self.theme.root_bg),
                                ),
                                Span::raw(" "),
                            ]
                        })
                        .collect();
                    let hint = Paragraph::new(Line::from(hint_spans))
                        .style(Style::default().bg(self.theme.root_bg));
                    frame.render_widget(hint, hint_area);
                }
            } else {
                let file_commands = ["open", "load-filters", "save-filters", "export-marked"];
                let trimmed_input = input_text.trim();
                let file_cmd = file_commands
                    .iter()
                    .find(|cmd| trimmed_input.starts_with(&format!("{} ", cmd)));

                if let Some(&cmd) = file_cmd {
                    let partial = trimmed_input[cmd.len()..].trim_start();
                    let completions = complete_file_path(partial);
                    if !completions.is_empty() {
                        let hint_text = completions
                            .iter()
                            .map(|c| {
                                std::path::Path::new(c)
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .map(|n| {
                                        if c.ends_with('/') {
                                            format!("{}/", n.trim_end_matches('/'))
                                        } else {
                                            n.to_string()
                                        }
                                    })
                                    .unwrap_or_else(|| c.clone())
                            })
                            .collect::<Vec<_>>()
                            .join("  ");
                        let hint = Paragraph::new(hint_text).style(
                            Style::default()
                                .fg(self.theme.border)
                                .bg(self.theme.root_bg),
                        );
                        frame.render_widget(hint, hint_area);
                    }
                } else if let Some(partial_raw) = input_text.trim_start().strip_prefix("set-theme ")
                {
                    let partial = partial_raw.trim_start();
                    let completions = complete_theme(partial);
                    if !completions.is_empty() {
                        let hint_text = completions.join("  ");
                        let hint = Paragraph::new(hint_text).style(
                            Style::default()
                                .fg(self.theme.border)
                                .bg(self.theme.root_bg),
                        );
                        frame.render_widget(hint, hint_area);
                    }
                } else if let Some(cmd) = find_matching_command(&input_text) {
                    let hint = Paragraph::new(format!("  {} - {}", cmd.usage, cmd.description))
                        .style(
                            Style::default()
                                .fg(self.theme.border)
                                .bg(self.theme.root_bg),
                        );
                    frame.render_widget(hint, hint_area);
                } else {
                    let completions = find_command_completions(input_text.trim());
                    if !completions.is_empty() {
                        let hint_text = completions.join("  ");
                        let hint = Paragraph::new(hint_text).style(
                            Style::default()
                                .fg(self.theme.border)
                                .bg(self.theme.root_bg),
                        );
                        frame.render_widget(hint, hint_area);
                    }
                }
            }
        }
    }

    fn render_side_bar(
        &mut self,
        frame: &mut Frame<'_>,
        selected_filter_idx: usize,
        sidebar_area: Option<Rect>,
    ) {
        if let Some(sidebar_area) = sidebar_area {
            let filters = self.tabs[self.active_tab].log_manager.get_filters();
            let filters_text: Vec<Line> = filters
                .iter()
                .enumerate()
                .map(|(i, filter)| {
                    let status = if filter.enabled { "[x]" } else { "[ ]" };
                    let selected_prefix = if i == selected_filter_idx { ">" } else { " " };
                    let filter_type_str = match filter.filter_type {
                        FilterType::Include => "In",
                        FilterType::Exclude => "Out",
                    };
                    let mut style = Style::default().fg(self.theme.text);
                    if let Some(cfg) = &filter.color_config {
                        if let Some(fg) = cfg.fg {
                            style = style.fg(fg);
                        }
                        if let Some(bg) = cfg.bg {
                            style = style.bg(bg);
                        }
                    }
                    Line::from(format!(
                        "{}{} {}: {}",
                        selected_prefix, status, filter_type_str, filter.pattern
                    ))
                    .style(style)
                })
                .collect();

            let sidebar_title = if self.tabs[self.active_tab].filtering_enabled {
                "Filters"
            } else {
                "Filters [OFF]"
            };
            let sidebar = Paragraph::new(filters_text).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(self.theme.border))
                    .title(sidebar_title)
                    .title_style(Style::default().fg(self.theme.border_title)),
            );
            frame.render_widget(sidebar, sidebar_area);
        }
    }

    fn render_tab_bar(
        &mut self,
        frame: &mut Frame<'_>,
        has_multiple_tabs: bool,
        chunks: &std::rc::Rc<[Rect]>,
        chunk_idx: &mut usize,
    ) {
        if has_multiple_tabs {
            let tab_bar_area = chunks[*chunk_idx];
            *chunk_idx += 1;

            let tab_spans: Vec<Span> = self
                .tabs
                .iter()
                .enumerate()
                .flat_map(|(i, t)| {
                    let is_active = i == self.active_tab;
                    let label = format!(" {} ", t.title);
                    let style = if is_active {
                        Style::default()
                            .fg(self.theme.text)
                            .bg(self.theme.text_highlight)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                            .fg(self.theme.border)
                            .bg(self.theme.root_bg)
                    };
                    vec![
                        Span::styled(label, style),
                        Span::styled(" ", Style::default().bg(self.theme.root_bg)),
                    ]
                })
                .collect();

            let tab_bar = Paragraph::new(Line::from(tab_spans))
                .style(Style::default().bg(self.theme.root_bg));
            frame.render_widget(tab_bar, tab_bar_area);
        }
    }
}

/// Number of terminal rows a line occupies when wrapped to `inner_width` columns.
/// Returns 1 when `inner_width` is 0 or the line is empty.
fn line_row_count(bytes: &[u8], inner_width: usize) -> usize {
    if inner_width == 0 {
        return 1;
    }
    let w = UnicodeWidthStr::width(std::str::from_utf8(bytes).unwrap_or(""));
    if w == 0 { 1 } else { w.div_ceil(inner_width) }
}

/// Simulate word-wrap of `text` into a box of `width` columns and return the
/// number of lines that result. Used to size the status bar dynamically.
fn count_wrapped_lines(text: &str, width: usize) -> usize {
    if width == 0 {
        return 1;
    }
    let mut lines = 1usize;
    let mut col = 0usize;
    for word in text.split_whitespace() {
        let wl = word.len();
        if col == 0 {
            col = wl;
        } else if col + 1 + wl > width {
            lines += 1;
            col = wl;
        } else {
            col += 1 + wl;
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auto_complete::shell_split;
    use crate::db::Database;
    use crate::file_reader::FileReader;
    use crate::log_manager::LogManager;
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
        App::new(log_manager, file_reader, Theme::default()).await
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
        let mut app = App::new(log_manager, file_reader, Theme::default()).await;

        assert_eq!(app.tab().visible_indices.len(), 3);

        app.execute_command_str("filter INFO".to_string()).await;

        assert_eq!(app.tab().visible_indices.len(), 1);
    }

    #[tokio::test]
    async fn test_mark_toggle() {
        let lines = vec!["line0", "line1", "line2"];
        let (file_reader, log_manager) = make_tab(&lines).await;
        let mut app = App::new(log_manager, file_reader, Theme::default()).await;

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
        let mut app = App::new(log_manager, file_reader, Theme::default()).await;

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
    async fn test_hide_field_by_index() {
        let mut app = make_app(&[r#"{"level":"INFO","msg":"hello"}"#]).await;
        app.execute_command_str("hide-field 0".to_string()).await;
        assert!(app.tab().hidden_field_indices.contains(&0));
        assert!(!app.tab().hidden_field_indices.contains(&1));
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
    async fn test_show_field_removes_hidden_index() {
        let mut app = make_app(&[r#"{"level":"INFO","msg":"hello"}"#]).await;
        app.execute_command_str("hide-field 1".to_string()).await;
        assert!(app.tab().hidden_field_indices.contains(&1));
        app.execute_command_str("show-field 1".to_string()).await;
        assert!(!app.tab().hidden_field_indices.contains(&1));
    }

    #[tokio::test]
    async fn test_show_all_fields_clears_everything() {
        let mut app = make_app(&[r#"{"level":"INFO","msg":"hello"}"#]).await;
        app.execute_command_str("hide-field msg".to_string()).await;
        app.execute_command_str("hide-field 0".to_string()).await;
        assert!(!app.tab().hidden_fields.is_empty());
        assert!(!app.tab().hidden_field_indices.is_empty());
        app.execute_command_str("show-all-fields".to_string()).await;
        assert!(app.tab().hidden_fields.is_empty());
        assert!(app.tab().hidden_field_indices.is_empty());
    }
}
