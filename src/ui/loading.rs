//! File, stdin, and Docker tab loading; file watchers; session restore.
//!
//! Handles opening files (mmap), piped stdin (byte accumulation), Docker log
//! streams ([`crate::file_reader::FileReader::spawn_process_stream`]),
//! directory listings, and restoring previously saved sessions from the DB.

use std::collections::VecDeque;

use crate::db::FileContextStore;
use crate::file_reader::{FileReader, VisibilityPredicate};
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

        let abs_path = std::fs::canonicalize(file_path_obj)
            .ok()
            .and_then(|c| c.to_str().map(|s| s.to_string()))
            .unwrap_or_else(|| path.to_string());

        let file_reader =
            FileReader::new(path).map_err(|e| format!("Failed to read '{}': {}", path, e))?;
        let log_manager = LogManager::new(self.db.clone(), Some(abs_path.clone())).await;

        let title = file_path_obj
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path)
            .to_string();

        let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let mut tab = TabState::new(file_reader, log_manager, title);
        tab.keybindings = self.keybindings.clone();
        tab.show_mode_bar = self.show_mode_bar_default;
        tab.show_borders = self.show_borders_default;

        if let Ok(Some(ctx)) = self.db.load_file_context(&abs_path).await {
            tab.mode = Box::new(ConfirmRestoreMode { context: ctx });
        }

        let watch_rx = FileReader::spawn_file_watcher(abs_path.clone(), file_size).await;
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
        tab.show_mode_bar = self.show_mode_bar_default;
        tab.show_borders = self.show_borders_default;

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
        tab.show_mode_bar = self.show_mode_bar_default;
        tab.show_borders = self.show_borders_default;

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
            // Regular file — hand off to background loader (no predicate for session restore).
            self.begin_file_load(
                next,
                LoadContext::SessionRestoreTab {
                    remaining,
                    total,
                    initial_tab_idx,
                },
                None,
                false,
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
        predicate: Option<VisibilityPredicate>,
        tail: bool,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + '_>> {
        Box::pin(async move {
            // Tail preview: read the last 64 KiB synchronously so the end of
            // the file is visible immediately while the full index builds in
            // the background. Only applied for the initial tab (not session
            // restores) and only when no filter predicate is in play (a
            // predicate requires the full index to compute visible indices).
            if tail
                && predicate.is_none()
                && let LoadContext::ReplaceInitialTab = context
                && !self.tabs.is_empty()
            {
                const PREVIEW_BYTES: u64 = 64 * 1024;
                if let Ok(preview) = FileReader::from_file_tail(&path, PREVIEW_BYTES) {
                    let last = preview.line_count().saturating_sub(1);
                    // Detect log format from the preview lines so
                    // structured rendering works during the load wait.
                    let sample_limit = preview.line_count().min(200);
                    if sample_limit > 0 {
                        let sample: Vec<&[u8]> =
                            (0..sample_limit).map(|j| preview.get_line(j)).collect();
                        self.tabs[0].detected_format = crate::parser::detect_format(&sample);
                    }
                    self.tabs[0].file_reader = preview;
                    self.tabs[0].refresh_visible();
                    self.tabs[0].scroll_offset = last;
                }
            }

            match FileReader::load(path.clone(), predicate, tail).await {
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
            let tail_mode = self.tabs[idx].tail_mode;
            self.tabs[idx].file_reader = FileReader::from_bytes(data);
            self.tabs[idx].refresh_visible();

            if tail_mode {
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
                tab.show_mode_bar = self.show_mode_bar_default;
                tab.show_borders = self.show_borders_default;
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
                Ok(result) => {
                    self.on_load_success(state.path, state.total_bytes, state.on_complete, result)
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
        result: crate::file_reader::FileLoadResult,
    ) {
        match context {
            LoadContext::ReplaceInitialTab => {
                if self.tabs.is_empty() {
                    return;
                }
                self.tabs[0].file_reader = result.reader;
                // Re-detect format now that real data is available (the tab was
                // created with an empty placeholder reader).
                let limit = self.tabs[0].file_reader.line_count().min(200);
                if limit > 0 {
                    let sample: Vec<&[u8]> = (0..limit)
                        .map(|j| self.tabs[0].file_reader.get_line(j))
                        .collect();
                    self.tabs[0].detected_format = crate::parser::detect_format(&sample);
                }
                // Use precomputed visible indices when available (single-pass optimisation);
                // otherwise fall back to a full compute_visible scan.
                if let Some(visible) = result.precomputed_visible {
                    self.tabs[0].visible_indices = super::VisibleLines::Filtered(visible);
                } else {
                    self.tabs[0].refresh_visible();
                }
                // Apply startup tail: jump to the last visible line and enable tail mode.
                if self.startup_tail {
                    self.tabs[0].tail_mode = true;
                    self.tabs[0].scroll_offset =
                        self.tabs[0].visible_indices.len().saturating_sub(1);
                }
                if !self.startup_filters
                    && let Ok(Some(ctx)) = self.db.load_file_context(&path).await
                {
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
                let mut tab = TabState::new(result.reader, log_manager, title);
                tab.keybindings = self.keybindings.clone();
                tab.show_mode_bar = self.show_mode_bar_default;
                tab.show_borders = self.show_borders_default;
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
                    let tail_mode = self.tabs[i].tail_mode;
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
                        self.tabs[i].detected_format = crate::parser::detect_format(&sample);
                    }
                    self.tabs[i].refresh_visible();
                    if tail_mode {
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

    /// Poll each tab's in-flight background search for completion.
    ///
    /// Called every frame from the event loop (non-blocking: `try_recv`).
    /// On completion, results are written into `tab.search` and the view
    /// is scrolled to the first match when `navigate` was set.
    pub(super) fn advance_search(&mut self) {
        for tab in &mut self.tabs {
            let Some(ref mut h) = tab.search_handle else {
                continue;
            };
            let Ok((results, regex)) = h.result_rx.try_recv() else {
                continue;
            };
            let forward = h.forward;
            let navigate = h.navigate;
            tab.search_handle = None;

            tab.search.set_results(results, regex);
            tab.search.set_forward(forward);
            tab.search_result_gen = tab.search_result_gen.wrapping_add(1);

            if navigate && !tab.search.get_results().is_empty() {
                let current_line_idx = tab.visible_indices.get_opt(tab.scroll_offset).unwrap_or(0);
                tab.search
                    .set_position_for_search(current_line_idx, forward);
                if forward {
                    tab.search.next_match();
                } else {
                    tab.search.previous_match();
                }
                tab.scroll_to_current_search_match();
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
        self.tabs[self.active_tab].mode = Box::new(NormalMode::default());
        self.continue_session_restore(queue, total, initial_tab_idx)
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Keybindings;
    use crate::db::Database;
    use crate::file_reader::{FileLoadResult, FileReader};
    use crate::log_manager::LogManager;
    use crate::mode::app_mode::ModeRenderState;
    use crate::theme::Theme;
    use crate::ui::StdinLoadState;
    use std::collections::VecDeque;
    use std::sync::Arc;

    async fn make_app(lines: &[&str]) -> App {
        let data: Vec<u8> = lines.join("\n").into_bytes();
        let file_reader = FileReader::from_bytes(data);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        App::new(
            log_manager,
            file_reader,
            Theme::default(),
            Arc::new(Keybindings::default()),
        )
        .await
    }

    #[tokio::test]
    async fn test_update_stdin_tab_empty_data() {
        let mut app = make_app(&[]).await;
        let line_count_before = app.tabs[0].file_reader.line_count();
        app.update_stdin_tab(vec![]).await;
        assert_eq!(app.tabs[0].file_reader.line_count(), line_count_before);
    }

    #[tokio::test]
    async fn test_update_stdin_tab_replace_placeholder() {
        let mut app = make_app(&[]).await;
        assert_eq!(app.tabs[0].file_reader.line_count(), 0);

        app.update_stdin_tab(b"line1\nline2\n".to_vec()).await;

        assert_eq!(app.tabs[0].file_reader.line_count(), 2);
        assert_eq!(app.tabs[0].visible_indices.len(), 2);
    }

    #[tokio::test]
    async fn test_update_stdin_tab_tail_mode_scrolls_to_last() {
        let mut app = make_app(&["first", "second"]).await;
        app.tabs[0].tail_mode = true;
        app.tabs[0].scroll_offset = 0;

        app.update_stdin_tab(b"first\nsecond\nthird\nfourth\n".to_vec())
            .await;

        // With tail_mode on, scroll_offset should be at the new last line.
        let new_last = app.tabs[0].visible_indices.len().saturating_sub(1);
        assert_eq!(app.tabs[0].scroll_offset, new_last);
        assert!(new_last > 0);
    }

    #[tokio::test]
    async fn test_update_stdin_tab_no_tail_no_scroll() {
        let mut app = make_app(&["first", "second"]).await;
        app.tabs[0].tail_mode = false;
        app.tabs[0].scroll_offset = 0;

        app.update_stdin_tab(b"first\nsecond\nthird\nfourth\n".to_vec())
            .await;

        // With tail_mode off, scroll_offset should stay where it was.
        assert_eq!(app.tabs[0].scroll_offset, 0);
    }

    #[tokio::test]
    async fn test_update_stdin_tab_creates_new_tab() {
        // Create an app whose first tab has a source file (not a placeholder).
        let data: Vec<u8> = b"existing line".to_vec();
        let file_reader = FileReader::from_bytes(data);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, Some("test.log".to_string())).await;
        let mut app = App::new(
            log_manager,
            file_reader,
            Theme::default(),
            Arc::new(Keybindings::default()),
        )
        .await;

        assert_eq!(app.tabs.len(), 1);

        app.update_stdin_tab(b"stdin line\n".to_vec()).await;

        assert_eq!(app.tabs.len(), 2);
    }

    #[tokio::test]
    async fn test_advance_file_watches_no_watchers() {
        let mut app = make_app(&["line1", "line2"]).await;
        assert!(app.tabs[0].watch_state.is_none());
        // Should not panic.
        app.advance_file_watches();
    }

    #[tokio::test]
    async fn test_advance_file_watches_with_data() {
        // Use data with trailing newline so append_bytes starts a new line.
        let data: Vec<u8> = b"original\n".to_vec();
        let file_reader = FileReader::from_bytes(data);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        let mut app = App::new(
            log_manager,
            file_reader,
            Theme::default(),
            Arc::new(Keybindings::default()),
        )
        .await;

        let (tx, rx) = tokio::sync::watch::channel(vec![]);
        app.tabs[0].watch_state = Some(FileWatchState { new_data_rx: rx });

        let original_count = app.tabs[0].file_reader.line_count();

        tx.send(b"new line\n".to_vec()).unwrap();
        app.advance_file_watches();

        assert!(app.tabs[0].file_reader.line_count() > original_count);
    }

    // ── tail mode: file-streaming tests ─────────────────────────────────────
    //
    // These tests simulate the file-watcher channel delivering new bytes (as the
    // real FileReader::spawn_file_watcher would) and assert that tail_mode
    // correctly drags (or does not drag) the scroll position.

    #[tokio::test]
    async fn test_file_stream_tail_on_always_scrolls_to_last() {
        // Start with 3 existing lines and scroll at the top.
        let data = b"line1\nline2\nline3\n".to_vec();
        let file_reader = FileReader::from_bytes(data);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        let mut app = App::new(
            log_manager,
            file_reader,
            Theme::default(),
            Arc::new(Keybindings::default()),
        )
        .await;

        app.tabs[0].tail_mode = true;
        app.tabs[0].scroll_offset = 0; // user is NOT at the end

        let (tx, rx) = tokio::sync::watch::channel(vec![]);
        app.tabs[0].watch_state = Some(FileWatchState { new_data_rx: rx });

        // Simulate the watcher delivering 3 new lines appended to the file.
        tx.send(b"line4\nline5\nline6\n".to_vec()).unwrap();
        app.advance_file_watches();

        let last = app.tabs[0].visible_indices.len().saturating_sub(1);
        assert_eq!(
            app.tabs[0].scroll_offset, last,
            "tail_mode on: scroll should track the last line"
        );
        assert_eq!(app.tabs[0].file_reader.line_count(), 6);
    }

    #[tokio::test]
    async fn test_file_stream_tail_off_preserves_scroll() {
        // Same setup but tail_mode is off — scroll must not move.
        let data = b"line1\nline2\nline3\n".to_vec();
        let file_reader = FileReader::from_bytes(data);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        let mut app = App::new(
            log_manager,
            file_reader,
            Theme::default(),
            Arc::new(Keybindings::default()),
        )
        .await;

        app.tabs[0].tail_mode = false;
        app.tabs[0].scroll_offset = 1; // user is in the middle

        let (tx, rx) = tokio::sync::watch::channel(vec![]);
        app.tabs[0].watch_state = Some(FileWatchState { new_data_rx: rx });

        tx.send(b"line4\nline5\nline6\n".to_vec()).unwrap();
        app.advance_file_watches();

        assert_eq!(
            app.tabs[0].scroll_offset, 1,
            "tail_mode off: scroll should stay where the user left it"
        );
        assert_eq!(app.tabs[0].file_reader.line_count(), 6);
    }

    #[tokio::test]
    async fn test_file_stream_multiple_batches_tail_on() {
        // New lines arrive in multiple watch batches; tail_mode keeps up.
        let data = b"a\n".to_vec();
        let file_reader = FileReader::from_bytes(data);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        let mut app = App::new(
            log_manager,
            file_reader,
            Theme::default(),
            Arc::new(Keybindings::default()),
        )
        .await;

        app.tabs[0].tail_mode = true;

        let (tx, rx) = tokio::sync::watch::channel(vec![]);
        app.tabs[0].watch_state = Some(FileWatchState { new_data_rx: rx });

        for batch in &[b"b\nc\n".as_ref(), b"d\ne\n".as_ref(), b"f\n".as_ref()] {
            tx.send(batch.to_vec()).unwrap();
            app.advance_file_watches();
            let last = app.tabs[0].visible_indices.len().saturating_sub(1);
            assert_eq!(
                app.tabs[0].scroll_offset, last,
                "after each batch, scroll should be at last line"
            );
        }
        assert_eq!(app.tabs[0].file_reader.line_count(), 6);
    }

    #[tokio::test]
    async fn test_file_stream_tail_on_with_real_file() {
        // Write initial content to a temp file, set up the app via open_file,
        // then write more lines and deliver them through the watch channel —
        // verifying that the scroll is dragged to the end.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        std::fs::write(&path, b"first\nsecond\nthird\n").unwrap();

        let mut app = make_app(&[]).await;
        app.open_file(&path).await.unwrap();

        // open_file adds a new tab (index 1).
        let tab_idx = app.tabs.len() - 1;
        app.active_tab = tab_idx;
        app.tabs[tab_idx].tail_mode = true;
        app.tabs[tab_idx].scroll_offset = 0; // scroll to top

        // Replace the real watcher with a manual channel so the test controls delivery.
        let (tx, rx) = tokio::sync::watch::channel(vec![]);
        app.tabs[tab_idx].watch_state = Some(FileWatchState { new_data_rx: rx });

        // Simulate new lines being appended.
        tx.send(b"fourth\nfifth\n".to_vec()).unwrap();
        app.advance_file_watches();

        let last = app.tabs[tab_idx].visible_indices.len().saturating_sub(1);
        assert_eq!(
            app.tabs[tab_idx].scroll_offset, last,
            "tail_mode on + real file: scroll must reach the last visible line"
        );
        assert!(
            app.tabs[tab_idx].file_reader.line_count() >= 5,
            "expected at least 5 lines after append"
        );
    }

    #[tokio::test]
    async fn test_file_stream_tail_toggled_mid_stream() {
        // Tail starts off. User enables it mid-stream; subsequent batches
        // should drag the scroll.
        let data = b"l1\nl2\nl3\n".to_vec();
        let file_reader = FileReader::from_bytes(data);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        let mut app = App::new(
            log_manager,
            file_reader,
            Theme::default(),
            Arc::new(Keybindings::default()),
        )
        .await;

        app.tabs[0].tail_mode = false;
        app.tabs[0].scroll_offset = 0;

        let (tx, rx) = tokio::sync::watch::channel(vec![]);
        app.tabs[0].watch_state = Some(FileWatchState { new_data_rx: rx });

        // First batch with tail off — scroll stays.
        tx.send(b"l4\nl5\n".to_vec()).unwrap();
        app.advance_file_watches();
        assert_eq!(app.tabs[0].scroll_offset, 0, "tail off: should not scroll");

        // Enable tail (like the user runs :tail).
        app.tabs[0].tail_mode = true;

        // Second batch — now scroll should follow.
        tx.send(b"l6\nl7\n".to_vec()).unwrap();
        app.advance_file_watches();
        let last = app.tabs[0].visible_indices.len().saturating_sub(1);
        assert_eq!(
            app.tabs[0].scroll_offset, last,
            "tail on: should scroll to last after enable"
        );
    }

    #[tokio::test]
    async fn test_advance_file_watches_sender_dropped() {
        let mut app = make_app(&["line"]).await;
        let (tx, rx) = tokio::sync::watch::channel(vec![]);
        app.tabs[0].watch_state = Some(FileWatchState { new_data_rx: rx });

        drop(tx);
        app.advance_file_watches();

        assert!(app.tabs[0].watch_state.is_none());
    }

    #[tokio::test]
    async fn test_advance_file_watches_format_redetection() {
        let mut app = make_app(&[]).await;
        assert!(app.tabs[0].detected_format.is_none());

        let (tx, rx) = tokio::sync::watch::channel(vec![]);
        app.tabs[0].watch_state = Some(FileWatchState { new_data_rx: rx });

        // Send JSON data so the format detector can pick it up.
        let json_data = b"{\"level\":\"INFO\",\"msg\":\"hello\"}\n";
        tx.send(json_data.to_vec()).unwrap();
        app.advance_file_watches();

        assert!(app.tabs[0].detected_format.is_some());
    }

    #[tokio::test]
    async fn test_skip_or_fail_load_replace_initial() {
        let mut app = make_app(&[]).await;
        assert_eq!(app.tabs.len(), 1);

        app.skip_or_fail_load(LoadContext::ReplaceInitialTab).await;

        // App should still have 1 tab (stays with empty initial).
        assert_eq!(app.tabs.len(), 1);
    }

    #[tokio::test]
    async fn test_skip_or_fail_load_session_restore() {
        let mut app = make_app(&[]).await;
        assert_eq!(app.tabs.len(), 1);

        app.skip_or_fail_load(LoadContext::SessionRestoreTab {
            remaining: VecDeque::new(),
            total: 1,
            initial_tab_idx: 0,
        })
        .await;

        // App should still be functional.
        assert_eq!(app.tabs.len(), 1);
    }

    #[tokio::test]
    async fn test_restore_session_empty() {
        let mut app = make_app(&["line"]).await;
        app.restore_session(vec![]).await;

        // Mode should be unchanged (still NormalMode from make_app).
        assert!(matches!(
            app.tabs[0].mode.render_state(),
            ModeRenderState::Normal
        ));
    }

    #[tokio::test]
    async fn test_restore_session_clears_mode() {
        let mut app = make_app(&["line"]).await;
        // restore_session with empty vec returns immediately, but still sets
        // mode to NormalMode first (only if non-empty).
        // With an empty vec it returns before setting mode, so mode stays Normal.
        // Verify the empty-vec early return path.
        app.restore_session(vec![]).await;
        assert!(matches!(
            app.tabs[0].mode.render_state(),
            ModeRenderState::Normal
        ));

        // With a non-empty vec, mode is set to NormalMode at the start.
        // Use a non-existent file so begin_file_load fails gracefully
        // and skip_or_fail_load handles it.
        app.restore_session(vec!["/nonexistent/file.log".to_string()])
            .await;
        assert!(matches!(
            app.tabs[0].mode.render_state(),
            ModeRenderState::Normal
        ));
    }

    #[tokio::test]
    async fn test_advance_stdin_load_no_state() {
        let mut app = make_app(&[]).await;
        assert!(app.stdin_load_state.is_none());
        // Should not panic.
        app.advance_stdin_load().await;
    }

    #[tokio::test]
    async fn test_advance_stdin_load_with_data() {
        let mut app = make_app(&[]).await;
        let (tx, rx) = tokio::sync::watch::channel(vec![]);
        app.stdin_load_state = Some(StdinLoadState { snapshot_rx: rx });

        tx.send(b"stdin line\n".to_vec()).unwrap();
        app.advance_stdin_load().await;

        assert_eq!(app.tabs[0].file_reader.line_count(), 1);
    }

    #[tokio::test]
    async fn test_advance_stdin_load_sender_dropped() {
        let mut app = make_app(&[]).await;
        let (tx, rx) = tokio::sync::watch::channel(vec![]);
        app.stdin_load_state = Some(StdinLoadState { snapshot_rx: rx });

        // Send some data, then drop sender.
        tx.send(b"final line\n".to_vec()).unwrap();
        drop(tx);

        app.advance_stdin_load().await;

        assert!(app.stdin_load_state.is_none());
        assert_eq!(app.tabs[0].file_reader.line_count(), 1);
    }

    #[tokio::test]
    async fn test_advance_file_load_no_state() {
        let mut app = make_app(&[]).await;
        assert!(app.file_load_state.is_none());
        // Should not panic.
        app.advance_file_load().await;
    }

    #[tokio::test]
    async fn test_open_file_nonexistent() {
        let mut app = make_app(&[]).await;
        let result = app.open_file("/nonexistent/path/file.log").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_lowercase().contains("not found"));
    }

    #[tokio::test]
    async fn test_open_file_directory() {
        let mut app = make_app(&[]).await;
        let result = app.open_file("/tmp").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_lowercase().contains("directory"));
    }

    #[tokio::test]
    async fn test_open_file_success() {
        let mut app = make_app(&["existing"]).await;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"hello\nworld\n").unwrap();
        let path = tmp.path().to_str().unwrap();
        let result = app.open_file(path).await;
        assert!(result.is_ok());
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.active_tab, 1);
        assert!(app.tabs[1].watch_state.is_some());
    }

    #[tokio::test]
    async fn test_startup_filters_suppresses_restore_prompt() {
        use crate::db::{FileContext, FileContextStore};
        use crate::mode::app_mode::ModeRenderState;
        use std::collections::HashSet;

        let mut app = make_app(&[]).await;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"line1\n").unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        // Persist a previous FileContext so restore would normally be offered.
        let ctx = FileContext {
            source_file: path.clone(),
            scroll_offset: 5,
            search_query: String::new(),
            wrap: true,
            level_colors_disabled: HashSet::new(),
            show_sidebar: false,
            horizontal_scroll: 0,
            marked_lines: vec![],
            file_hash: None,
            show_line_numbers: true,
            comments: vec![],
            show_mode_bar: true,
            show_borders: true,
            show_keys: true,
            raw_mode: false,
        };
        app.db.save_file_context(&ctx).await.unwrap();

        // With startup_filters=true the restore prompt must be suppressed.
        app.startup_filters = true;
        app.begin_file_load(path.clone(), LoadContext::ReplaceInitialTab, None, false)
            .await;
        app.advance_file_load().await;
        // Load may still be in progress; drain it.
        for _ in 0..100 {
            if app.file_load_state.is_none() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            app.advance_file_load().await;
        }

        assert!(
            !matches!(
                app.tabs[0].mode.render_state(),
                ModeRenderState::ConfirmRestore
            ),
            "restore prompt must not appear when --filters was given"
        );
    }

    #[tokio::test]
    async fn test_begin_file_load_real_file() {
        let mut app = make_app(&["placeholder"]).await;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"data\n").unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        app.begin_file_load(path.clone(), LoadContext::ReplaceInitialTab, None, false)
            .await;
        assert!(app.file_load_state.is_some());
        assert_eq!(app.file_load_state.as_ref().unwrap().path, path);
    }

    #[tokio::test]
    async fn test_advance_file_load_completed() {
        let mut app = make_app(&[]).await;

        let (progress_tx, progress_rx) = tokio::sync::watch::channel(1.0_f64);
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        let fr = FileReader::from_bytes(b"loaded\n".to_vec());
        let _ = result_tx.send(Ok(FileLoadResult {
            reader: fr,
            precomputed_visible: None,
        }));
        drop(progress_tx);

        app.file_load_state = Some(super::FileLoadState {
            path: "test.log".to_string(),
            progress_rx,
            result_rx,
            total_bytes: 7,
            on_complete: LoadContext::ReplaceInitialTab,
        });

        app.advance_file_load().await;

        // Load state should be consumed.
        assert!(app.file_load_state.is_none());
        // The initial tab should have the loaded data.
        assert_eq!(app.tabs[0].file_reader.line_count(), 1);
    }

    #[tokio::test]
    async fn test_advance_file_load_redetects_format() {
        let mut app = make_app(&[]).await;
        // Initial tab has no detected format (empty placeholder).
        assert!(app.tabs[0].detected_format.is_none());

        let (progress_tx, progress_rx) = tokio::sync::watch::channel(1.0_f64);
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        // Send JSON data so format detection finds a match.
        let fr = FileReader::from_bytes(
            b"{\"level\":\"INFO\",\"msg\":\"hello\"}\n{\"level\":\"WARN\",\"msg\":\"world\"}\n"
                .to_vec(),
        );
        let _ = result_tx.send(Ok(FileLoadResult {
            reader: fr,
            precomputed_visible: None,
        }));
        drop(progress_tx);

        app.file_load_state = Some(super::FileLoadState {
            path: "test.log".to_string(),
            progress_rx,
            result_rx,
            total_bytes: 60,
            on_complete: LoadContext::ReplaceInitialTab,
        });

        app.advance_file_load().await;

        assert!(app.file_load_state.is_none());
        assert!(
            app.tabs[0].detected_format.is_some(),
            "Format should be re-detected after ReplaceInitialTab load"
        );
    }

    #[tokio::test]
    async fn test_advance_file_load_failure() {
        let mut app = make_app(&[]).await;

        let (_progress_tx, progress_rx) = tokio::sync::watch::channel(0.0_f64);
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        let _ = result_tx.send(Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "gone",
        )));

        app.file_load_state = Some(super::FileLoadState {
            path: "missing.log".to_string(),
            progress_rx,
            result_rx,
            total_bytes: 0,
            on_complete: LoadContext::ReplaceInitialTab,
        });

        app.advance_file_load().await;

        // Load state should be consumed even on failure.
        assert!(app.file_load_state.is_none());
        // Initial tab should still exist (empty placeholder).
        assert_eq!(app.tabs.len(), 1);
    }

    #[tokio::test]
    async fn test_restore_session_with_nonexistent_files() {
        let mut app = make_app(&[]).await;
        let files = vec![
            "/nonexistent/a.log".to_string(),
            "/nonexistent/b.log".to_string(),
        ];
        app.restore_session(files).await;
        // Mode should be NormalMode after restore attempt.
        assert!(matches!(
            app.tabs[0].mode.render_state(),
            ModeRenderState::Normal
        ));
    }

    #[tokio::test]
    async fn test_continue_session_restore_empty_queue() {
        let mut app = make_app(&[]).await;
        let queue = VecDeque::new();
        // Should just return; the only tab is the placeholder.
        app.continue_session_restore(queue, 0, 0).await;
        assert_eq!(app.tabs.len(), 1);
    }

    #[tokio::test]
    async fn test_open_docker_logs() {
        let mut app = make_app(&["line"]).await;
        // Use a bogus container ID — spawn_process_stream will fail
        // because docker won't find it, but it shouldn't panic.
        app.open_docker_logs("fake_id_123".to_string(), "fake_container".to_string())
            .await;
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.active_tab, 1);
        assert!(app.tabs[1].title.contains("docker:fake_container"));
    }

    #[tokio::test]
    async fn test_begin_stdin_load() {
        let mut app = make_app(&[]).await;
        assert!(app.stdin_load_state.is_none());
        app.begin_stdin_load().await;
        assert!(app.stdin_load_state.is_some());
    }
}
