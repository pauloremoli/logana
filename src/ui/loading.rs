use std::collections::VecDeque;

use crate::db::FileContextStore;
use crate::file_reader::FileReader;
use crate::log_manager::LogManager;
use crate::mode::app_mode::ConfirmRestoreMode;
use crate::mode::normal_mode::NormalMode;

use super::{App, FileLoadState, FileWatchState, LoadContext, TabState};

impl App {
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
        tab.keybindings = self.keybindings.clone();

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

    /// Open a new tab streaming logs from a Docker container.
    pub(super) async fn open_docker_logs(&mut self, container_id: String, container_name: String) {
        let file_reader = FileReader::from_bytes(vec![]);
        let source_label = format!("docker:{}", container_name);
        let log_manager = LogManager::new(self.db.clone(), Some(source_label)).await;
        let title = format!("docker:{}", container_name);

        let mut tab = TabState::new(file_reader, log_manager, title);
        tab.keybindings = self.keybindings.clone();

        match FileReader::spawn_process_stream("docker", &["logs", "-f", &container_id]).await {
            Ok(rx) => {
                tab.watch_state = Some(FileWatchState { new_data_rx: rx });
            }
            Err(e) => {
                tab.command_error = Some(format!("Failed to attach to container: {}", e));
            }
        }

        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
    }

    /// Create a docker streaming tab from a `"docker:name"` source string
    /// (used during session restore).  The container name is passed directly
    /// to `docker logs -f`.
    async fn restore_docker_tab(&mut self, source: &str) {
        let name = source.strip_prefix("docker:").unwrap_or(source);
        let file_reader = FileReader::from_bytes(vec![]);
        let log_manager = LogManager::new(self.db.clone(), Some(source.to_string())).await;
        let title = source.to_string();

        let mut tab = TabState::new(file_reader, log_manager, title);
        tab.keybindings = self.keybindings.clone();

        match FileReader::spawn_process_stream("docker", &["logs", "-f", name]).await {
            Ok(rx) => {
                tab.watch_state = Some(FileWatchState { new_data_rx: rx });
            }
            Err(e) => {
                tab.command_error = Some(format!("Failed to attach to container: {}", e));
            }
        }

        if let Ok(Some(ctx)) = self.db.load_file_context(source).await {
            tab.apply_file_context(&ctx);
        }

        self.tabs.push(tab);
    }

    /// Consume the session-restore queue, handling both docker tabs and file
    /// tabs.  Docker tabs are created immediately; file tabs are handed off
    /// to `begin_file_load` for background indexing.
    async fn continue_session_restore(
        &mut self,
        mut remaining: VecDeque<String>,
        total: usize,
        initial_tab_idx: usize,
    ) {
        loop {
            let next = match remaining.pop_front() {
                Some(n) => n,
                None => {
                    // Queue exhausted — remove the placeholder tab if it exists
                    if self.tabs.len() > 1 {
                        let is_placeholder = self.tabs[initial_tab_idx]
                            .log_manager
                            .source_file()
                            .is_none()
                            && self.tabs[initial_tab_idx].file_reader.line_count() == 0;
                        if is_placeholder {
                            self.tabs.remove(initial_tab_idx);
                            self.active_tab = 0;
                        }
                    }
                    return;
                }
            };
            if next.starts_with("docker:") {
                self.restore_docker_tab(&next).await;
                continue;
            }
            // Regular file — hand off to background loader
            self.begin_file_load(
                next,
                LoadContext::SessionRestoreTab {
                    remaining,
                    total,
                    initial_tab_idx,
                },
            )
            .await;
            return;
        }
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
        self.stdin_load_state = Some(super::StdinLoadState { snapshot_rx });
    }

    /// Poll for new stdin data each frame and apply it to the stdin tab.
    pub(super) async fn advance_stdin_load(&mut self) {
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
                tab.keybindings = self.keybindings.clone();
                tab.scroll_offset = tab.visible_indices.len().saturating_sub(1);
                self.tabs.push(tab);
            }
        }
    }

    /// Poll for completion of the current background file load (called every frame).
    pub(super) async fn advance_file_load(&mut self) {
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
                remaining,
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
                tab.keybindings = self.keybindings.clone();
                if let Ok(Some(ctx)) = self.db.load_file_context(&path).await {
                    tab.apply_file_context(&ctx);
                }
                let watch_rx = FileReader::spawn_file_watcher(path.clone(), total_bytes).await;
                tab.watch_state = Some(FileWatchState {
                    new_data_rx: watch_rx,
                });
                self.tabs.push(tab);

                self.continue_session_restore(remaining, total, initial_tab_idx)
                    .await;
            }
        }
    }

    /// Poll each tab's file watcher for new appended content (called every frame).
    ///
    /// If the user's scroll position is at the last visible line (follow mode),
    /// it is advanced to stay at the new last line after content is appended.
    pub(super) fn advance_file_watches(&mut self) {
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
                    // Re-detect format if not yet known (e.g. docker-logs
                    // tab that started empty).
                    if self.tabs[i].detected_format.is_none()
                        && self.tabs[i].file_reader.line_count() > 0
                    {
                        let limit = self.tabs[i].file_reader.line_count().min(200);
                        let sample: Vec<&[u8]> = (0..limit)
                            .map(|j| self.tabs[i].file_reader.get_line(j))
                            .collect();
                        self.tabs[i].detected_format = crate::format::detect_format(&sample);
                    }
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
            remaining,
            total,
            initial_tab_idx,
        } = context
        {
            self.continue_session_restore(remaining, total, initial_tab_idx)
                .await;
        }
        // ReplaceInitialTab failure: stay with the empty initial tab.
    }

    /// Begin a session restore: kick off the first file load (or docker tab).
    pub(super) async fn restore_session(&mut self, files: Vec<String>) {
        if files.is_empty() {
            return;
        }
        let total = files.len();
        let queue: VecDeque<String> = files.into_iter().collect();
        let initial_tab_idx = self.active_tab;
        self.tabs[self.active_tab].mode = Box::new(NormalMode);
        self.continue_session_restore(queue, total, initial_tab_idx)
            .await;
    }
}
