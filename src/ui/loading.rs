use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::config::RestoreSessionPolicy;
use crate::db::FileContextStore;
use crate::file_reader::{FileReader, VisibilityPredicate};
use crate::log_manager::LogManager;
use crate::mode::app_mode::ConfirmRestoreMode;
use crate::mode::normal_mode::NormalMode;

use super::{
    App, ConnectFn, FileLoadState, FileWatchState, LoadContext, StreamRetryState, TabState,
    VisibleLines, dlt_connect_fn, docker_connect_fn,
};

fn connect_fn_for_source(source: Option<&str>) -> Option<ConnectFn> {
    let source = source?;
    if let Some(stripped) = source.strip_prefix("dlt://") {
        let (host, port) = match stripped.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse::<u16>().unwrap_or(3490)),
            None => (stripped.to_string(), 3490),
        };
        Some(dlt_connect_fn(host, port))
    } else if let Some(name) = source.strip_prefix("docker:") {
        Some(docker_connect_fn(name.to_string()))
    } else {
        None
    }
}

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

        let title = file_path_obj
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path)
            .to_string();

        // Show a preview immediately, then load the full index in the background.
        let preview = FileReader::from_file_head(path, self.preview_bytes)
            .await
            .unwrap_or_else(|_| FileReader::from_bytes(vec![]));
        let log_manager = LogManager::new(self.db.clone(), Some(abs_path.clone())).await;
        let mut tab = TabState::new(preview, log_manager, title);
        self.apply_tab_defaults(&mut tab);

        if let Ok(Some(ctx)) = self.db.load_file_context(&abs_path).await {
            match self.restore_file_policy {
                RestoreSessionPolicy::Always => {
                    tab.apply_file_context(&ctx);
                }
                RestoreSessionPolicy::Never => {}
                RestoreSessionPolicy::Ask => {
                    tab.mode = Box::new(ConfirmRestoreMode { context: ctx });
                }
            }
        }

        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
        let tab_idx = self.active_tab;

        self.begin_file_load(abs_path, LoadContext::ReplaceTab { tab_idx }, None, false)
            .await;
        Ok(())
    }

    /// Open a new tab streaming logs from a Docker container.
    pub(super) async fn open_docker_logs(&mut self, container_id: String, container_name: String) {
        let file_reader = FileReader::from_bytes(vec![]);
        let source_label = format!("docker:{}", container_name);
        let log_manager = LogManager::new(self.db.clone(), Some(source_label)).await;
        let title = format!("docker:{}", container_name);

        let mut tab = TabState::new(file_reader, log_manager, title);
        self.apply_tab_defaults(&mut tab);

        match FileReader::spawn_process_stream("docker", &["logs", "-f", &container_id]).await {
            Ok(rx) => {
                tab.watch_state = Some(FileWatchState { new_data_rx: rx });
            }
            Err(e) => {
                let err_msg = e.to_string();
                tab.command_error = Some(format!("Docker attach failed: {}", err_msg));
                tab.stream_retry = Some(StreamRetryState::new(
                    docker_connect_fn(container_id),
                    err_msg,
                ));
            }
        }

        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
    }

    pub(super) async fn open_dlt_stream(&mut self, host: String, port: u16, name: String) {
        let source_label = format!("dlt://{}:{}", host, port);
        let file_reader = FileReader::from_bytes(vec![]);
        let log_manager = LogManager::new(self.db.clone(), Some(source_label.clone())).await;
        let title = format!("dlt:{}", name);

        let mut tab = TabState::new(file_reader, log_manager, title);
        self.apply_tab_defaults(&mut tab);

        match FileReader::spawn_dlt_tcp_stream(host.clone(), port).await {
            Ok(rx) => {
                tab.watch_state = Some(FileWatchState { new_data_rx: rx });
            }
            Err(e) => {
                let err_msg = e.to_string();
                tab.command_error = Some(format!("DLT connection failed: {}", err_msg));
                tab.stream_retry = Some(StreamRetryState::new(dlt_connect_fn(host, port), err_msg));
            }
        }

        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
    }

    async fn restore_dlt_tab(&mut self, source: &str) {
        let stripped = source.strip_prefix("dlt://").unwrap_or(source);
        let (host, port) = match stripped.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse::<u16>().unwrap_or(3490)),
            None => (stripped.to_string(), 3490),
        };
        let file_reader = FileReader::from_bytes(vec![]);
        let log_manager = LogManager::new(self.db.clone(), Some(source.to_string())).await;
        let title = source.to_string();

        let mut tab = TabState::new(file_reader, log_manager, title);
        self.apply_tab_defaults(&mut tab);

        tab.stream_retry = Some(StreamRetryState::new(
            dlt_connect_fn(host, port),
            "reconnecting…".to_string(),
        ));

        if let Ok(Some(ctx)) = self.db.load_file_context(source).await {
            tab.apply_file_context(&ctx);
        }

        self.tabs.push(tab);
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
        self.apply_tab_defaults(&mut tab);

        tab.stream_retry = Some(StreamRetryState::new(
            docker_connect_fn(name.to_string()),
            "reconnecting…".to_string(),
        ));

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
                    // Queue exhausted — remove the placeholder if it was never replaced.
                    if self.tabs.len() > 1
                        && initial_tab_idx < self.tabs.len()
                        && self.tabs[initial_tab_idx]
                            .log_manager
                            .source_file()
                            .is_none()
                        && self.tabs[initial_tab_idx].file_reader.line_count() == 0
                    {
                        self.tabs.remove(initial_tab_idx);
                        self.active_tab = self.active_tab.min(self.tabs.len().saturating_sub(1));
                    }
                    return;
                }
            };
            if next.starts_with("docker:") {
                self.restore_docker_tab(&next).await;
                continue;
            }
            if next.starts_with("dlt://") {
                self.restore_dlt_tab(&next).await;
                continue;
            }
            // Regular file — create a preview tab immediately, then load the full index in the background.
            let preview = FileReader::from_file_head(&next, self.preview_bytes)
                .await
                .unwrap_or_else(|_| FileReader::from_bytes(vec![]));
            let title = std::path::Path::new(&next)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&next)
                .to_string();
            let log_manager = LogManager::new(self.db.clone(), Some(next.clone())).await;
            let mut tab = TabState::new(preview, log_manager, title);
            self.apply_tab_defaults(&mut tab);
            let abs_path = std::fs::canonicalize(&next)
                .ok()
                .and_then(|c| c.to_str().map(|s| s.to_string()))
                .unwrap_or_else(|| next.clone());
            if let Ok(Some(ctx)) = self.db.load_file_context(&abs_path).await {
                tab.apply_file_context(&ctx);
            }
            self.tabs.push(tab);
            let mut tab_idx = self.tabs.len() - 1;

            // If the initial placeholder is still empty and active, remove it now and
            // switch immediately to the new file tab so the user sees content right away.
            let placeholder_is_empty = initial_tab_idx < tab_idx
                && self.tabs[initial_tab_idx]
                    .log_manager
                    .source_file()
                    .is_none()
                && self.tabs[initial_tab_idx].file_reader.line_count() == 0;
            if placeholder_is_empty {
                self.tabs.remove(initial_tab_idx);
                tab_idx -= 1; // pushed after the placeholder, so index shifts by one
            }
            self.active_tab = tab_idx;

            self.begin_file_load(
                next,
                LoadContext::SessionRestoreTab {
                    tab_idx,
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
            // Initial-tab preview: show the first/last chunk immediately
            // while the full index builds in the background. When a filter predicate
            // is active it is applied to the preview lines so the filtered view is
            // visible straight away.
            let preview_bytes = self.preview_bytes;
            if let LoadContext::ReplaceInitialTab = context
                && !self.tabs.is_empty()
            {
                let preview_result = if tail {
                    FileReader::from_file_tail(&path, preview_bytes).await
                } else {
                    FileReader::from_file_head(&path, preview_bytes).await
                };
                if let Ok(preview) = preview_result
                    && preview.line_count() > 0
                {
                    self.tabs[0].file_reader = preview;
                    self.tabs[0].detect_and_apply_format();
                    if let Some(ref pred) = predicate {
                        let visible: Vec<usize> = (0..self.tabs[0].file_reader.line_count())
                            .filter(|&i| pred(self.tabs[0].file_reader.get_line(i)))
                            .collect();
                        self.tabs[0].visible_indices = VisibleLines::Filtered(visible);
                    } else {
                        self.tabs[0].begin_filter_refresh();
                    }
                    if tail && self.tabs[0].filter_handle.is_none() {
                        // Fast path completed synchronously: jump to the last visible line.
                        // Slow path: advance_filter_computation clamps scroll_offset to
                        // visible_len-1 when the background scan finishes, landing at the tail.
                        self.tabs[0].scroll_offset =
                            self.tabs[0].visible_indices.len().saturating_sub(1);
                    }
                    // Non-tail: stay at line 0; user sees the top of the file immediately.
                }
            }

            let cancel = Arc::new(AtomicBool::new(false));
            match FileReader::load(path.clone(), predicate, tail, cancel.clone()).await {
                Ok(handle) => {
                    self.file_load_state = Some(FileLoadState {
                        path,
                        progress_rx: handle.progress_rx,
                        result_rx: handle.result_rx,
                        total_bytes: handle.total_bytes,
                        on_complete: context,
                        cancel,
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
            if self.tabs[idx].paused {
                return;
            }
            let tail_mode = self.tabs[idx].tail_mode;
            self.tabs[idx].file_reader = FileReader::from_bytes(data);
            self.tabs[idx].begin_filter_refresh();

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
                self.apply_tab_defaults(&mut tab);
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
                self.tabs[0].detect_and_apply_format();
                // Use precomputed visible indices when available (single-pass optimisation);
                // otherwise fall back to a full compute_visible scan.
                // Either way, run a full filter refresh so that occurrence counts are
                // computed (the single-pass predicate only tracks visibility, not counts).
                if let Some(visible) = result.precomputed_visible {
                    self.tabs[0].visible_indices = VisibleLines::Filtered(visible);
                }
                if !self.startup_filters
                    && let Ok(Some(ctx)) = self.db.load_file_context(&path).await
                {
                    match self.restore_file_policy {
                        RestoreSessionPolicy::Always => {
                            self.tabs[0].apply_file_context(&ctx);
                        }
                        RestoreSessionPolicy::Never => {}
                        RestoreSessionPolicy::Ask => {
                            self.tabs[0].mode = Box::new(ConfirmRestoreMode { context: ctx });
                        }
                    }
                }
                self.tabs[0].begin_filter_refresh();
                // Apply startup tail: jump to the last visible line and enable tail mode.
                if self.startup_tail {
                    self.tabs[0].tail_mode = true;
                    self.tabs[0].scroll_offset =
                        self.tabs[0].visible_indices.len().saturating_sub(1);
                }
                let watch_rx = FileReader::spawn_file_watcher(path, total_bytes).await;
                self.tabs[0].watch_state = Some(FileWatchState {
                    new_data_rx: watch_rx,
                });
            }
            LoadContext::ReplaceTab { tab_idx } => {
                if tab_idx >= self.tabs.len() {
                    return;
                }
                self.tabs[tab_idx].file_reader = result.reader;
                self.tabs[tab_idx].detect_and_apply_format();
                self.tabs[tab_idx].begin_filter_refresh();
                let watch_rx = FileReader::spawn_file_watcher(path, total_bytes).await;
                self.tabs[tab_idx].watch_state = Some(FileWatchState {
                    new_data_rx: watch_rx,
                });
            }
            LoadContext::SessionRestoreTab {
                tab_idx,
                remaining,
                total,
                initial_tab_idx,
            } => {
                if tab_idx >= self.tabs.len() {
                    self.continue_session_restore(remaining, total, initial_tab_idx)
                        .await;
                    return;
                }
                self.tabs[tab_idx].file_reader = result.reader;
                self.tabs[tab_idx].detect_and_apply_format();
                if let Ok(Some(ctx)) = self.db.load_file_context(&path).await {
                    self.tabs[tab_idx].apply_file_context(&ctx);
                }
                self.tabs[tab_idx].begin_filter_refresh();
                let watch_rx = FileReader::spawn_file_watcher(path, total_bytes).await;
                self.tabs[tab_idx].watch_state = Some(FileWatchState {
                    new_data_rx: watch_rx,
                });

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
                    if self.tabs[i].paused {
                        // Mark the channel as seen so we don't spin, but
                        // discard the payload — it will be re-delivered on
                        // the next change after the user resumes.
                        let _ = self.tabs[i]
                            .watch_state
                            .as_mut()
                            .unwrap()
                            .new_data_rx
                            .borrow_and_update();
                        continue;
                    }
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
                    let old_line_count = self.tabs[i].file_reader.line_count();
                    self.tabs[i].file_reader.append_bytes(&new_data);
                    // Re-detect format if not yet known (e.g. docker-logs
                    // tab that started empty).
                    if self.tabs[i].detected_format.is_none()
                        && self.tabs[i].file_reader.line_count() > 0
                    {
                        self.tabs[i].detect_and_apply_format();
                    }
                    self.tabs[i].filter_new_lines(old_line_count);
                    if tail_mode {
                        let new_count = self.tabs[i].visible_indices.len();
                        self.tabs[i].scroll_offset = new_count.saturating_sub(1);
                    }
                }
                Some(Err(_)) => {
                    self.tabs[i].watch_state = None;
                    if let Some(connect_fn) =
                        connect_fn_for_source(self.tabs[i].log_manager.source_file())
                    {
                        let err_msg = "connection lost".to_string();
                        self.tabs[i].command_error = Some(format!("Disconnected: {}", err_msg));
                        self.tabs[i].stream_retry =
                            Some(StreamRetryState::new(connect_fn, err_msg));
                    }
                }
                _ => {}
            }
        }
    }

    /// Poll DLT retry channels for reconnection results.
    pub(super) fn advance_stream_retries(&mut self) {
        for tab in &mut self.tabs {
            let retry = match &mut tab.stream_retry {
                Some(r) => r,
                None => continue,
            };
            let rx = match &mut retry.retry_rx {
                Some(rx) => rx,
                None => continue,
            };
            match rx.try_recv() {
                Ok(Ok(watch_rx)) => {
                    tab.watch_state = Some(FileWatchState {
                        new_data_rx: watch_rx,
                    });
                    tab.command_error = None;
                    tab.stream_retry = None;
                }
                Ok(Err(e)) => {
                    retry.last_error = e.clone();
                    tab.command_error = Some(format!(
                        "Connection failed (retry #{}): {}",
                        retry.attempt, e
                    ));
                    retry.schedule_retry();
                }
                Err(_) => {}
            }
        }
    }

    /// Poll each tab's in-flight background search for completion.
    ///
    /// Called every frame from the event loop (non-blocking: `try_recv`).
    /// On completion, results are written into `tab.search` and the view
    /// is scrolled to the first match when `navigate` was set.
    pub(super) fn advance_search(&mut self) {
        use tokio::sync::mpsc::error::TryRecvError;

        for tab in &mut self.tabs {
            let Some(ref mut h) = tab.search_handle else {
                continue;
            };
            let forward = h.forward;
            let navigate = h.navigate;
            let mut done = false;

            loop {
                match h.result_rx.try_recv() {
                    Ok(batch) => {
                        tab.search.extend_results(batch);
                        tab.search_result_gen = tab.search_result_gen.wrapping_add(1);
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        done = true;
                        break;
                    }
                }
            }

            if done {
                tab.search_handle = None;

                if navigate && !tab.search.get_results().is_empty() {
                    let current_line_idx =
                        tab.visible_indices.get_opt(tab.scroll_offset).unwrap_or(0);
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
    }

    /// Poll each tab's in-flight background filter computation for new chunks.
    ///
    /// Called every frame from the event loop (non-blocking: `try_recv`).
    /// Chunks are applied incrementally: the first chunk replaces `visible_indices`,
    /// subsequent chunks extend it.  Scroll and counts are updated on every chunk.
    pub(super) fn advance_filter_computation(&mut self) {
        use tokio::sync::mpsc::error::TryRecvError;
        for tab in &mut self.tabs {
            if tab.filter_handle.is_none() {
                continue;
            }

            // Phase 1: drain available chunks into a local buffer (limits borrow scope).
            let (chunks, done) = {
                let h = tab.filter_handle.as_mut().unwrap();
                let mut chunks = Vec::new();
                let mut done = false;
                loop {
                    match h.result_rx.try_recv() {
                        Ok(chunk) => {
                            let last = chunk.is_last;
                            chunks.push(chunk);
                            if last {
                                done = true;
                                break;
                            }
                        }
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            done = true;
                            break;
                        }
                    }
                }
                (chunks, done)
            };

            if chunks.is_empty() && !done {
                continue;
            }

            let already_had_first = tab.filter_handle.as_ref().unwrap().received_first_chunk;
            let scroll_anchor = tab.filter_handle.as_ref().unwrap().scroll_anchor;

            if !chunks.is_empty() {
                tab.filter_handle.as_mut().unwrap().received_first_chunk = true;
            }

            // Phase 2: apply chunks to visible_indices.
            let mut should_replace = !already_had_first;
            for chunk in chunks {
                let is_last = chunk.is_last;
                if let Some(h) = tab.filter_handle.as_mut() {
                    h.displayed_progress = chunk.progress;
                }
                if should_replace {
                    tab.visible_indices = VisibleLines::Filtered(chunk.visible);
                    should_replace = false;
                } else if let VisibleLines::Filtered(ref mut v) = tab.visible_indices {
                    v.extend(chunk.visible);
                }
                if let Some(counts) = chunk.filter_match_counts {
                    tab.filter_match_counts = counts;
                }
                if is_last {
                    if let Some(idx) = scroll_anchor
                        && let Some(pos) = tab.visible_indices.position_of(idx)
                    {
                        tab.scroll_offset = pos;
                    } else if tab.visible_indices.is_empty() {
                        tab.scroll_offset = 0;
                    } else {
                        tab.scroll_offset = tab.scroll_offset.min(tab.visible_indices.len() - 1);
                    }
                } else if tab.visible_indices.is_empty() {
                    tab.scroll_offset = 0;
                } else {
                    tab.scroll_offset = tab.scroll_offset.min(tab.visible_indices.len() - 1);
                }
            }
            if done {
                tab.filter_handle = None;
            }
        }
    }

    /// Called when a file load fails or the file cannot be opened.
    async fn skip_or_fail_load(&mut self, context: LoadContext) {
        match context {
            LoadContext::SessionRestoreTab {
                tab_idx,
                remaining,
                total,
                initial_tab_idx,
            } => {
                // Remove the preview tab created before the load started.
                if tab_idx < self.tabs.len() && self.tabs.len() > 1 {
                    self.tabs.remove(tab_idx);
                    self.active_tab = self.active_tab.min(self.tabs.len().saturating_sub(1));
                }
                self.continue_session_restore(remaining, total, initial_tab_idx)
                    .await;
            }
            LoadContext::ReplaceTab { tab_idx } => {
                // Remove the placeholder preview tab; no further action needed.
                if tab_idx < self.tabs.len() {
                    self.tabs.remove(tab_idx);
                    if self.active_tab >= self.tabs.len() {
                        self.active_tab = self.tabs.len().saturating_sub(1);
                    }
                }
            }
            // ReplaceInitialTab failure: stay with the empty initial tab.
            LoadContext::ReplaceInitialTab => {}
        }
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
    use crate::ui::{StdinLoadState, VisibleLines};
    use std::collections::VecDeque;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

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
            None,
            None,
            None,
            None,
            None,
            None,
            None,
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
            None,
            None,
            None,
            None,
            None,
            None,
            None,
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
            None,
            None,
            None,
            None,
            None,
            None,
            None,
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
            None,
            None,
            None,
            None,
            None,
            None,
            None,
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
            None,
            None,
            None,
            None,
            None,
            None,
            None,
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
            None,
            None,
            None,
            None,
            None,
            None,
            None,
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
    async fn test_advance_file_watches_paused_skips_update() {
        let mut app = make_app(&["old line"]).await;
        app.tabs[0].paused = true;
        let initial_count = app.tabs[0].visible_indices.len();

        let (tx, rx) = tokio::sync::watch::channel(vec![]);
        app.tabs[0].watch_state = Some(FileWatchState { new_data_rx: rx });

        tx.send(b"new line\n".to_vec()).unwrap();
        app.advance_file_watches();

        // Paused: no new lines should have been appended.
        assert_eq!(app.tabs[0].visible_indices.len(), initial_count);
        drop(tx);
    }

    #[tokio::test]
    async fn test_advance_file_watches_unpaused_applies_update() {
        let file_reader = FileReader::from_bytes(b"old line\n".to_vec());
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
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
        let initial_count = app.tabs[0].file_reader.line_count();

        let (tx, rx) = tokio::sync::watch::channel(vec![]);
        app.tabs[0].watch_state = Some(FileWatchState { new_data_rx: rx });

        tx.send(b"new line\n".to_vec()).unwrap();
        app.advance_file_watches();

        // Not paused: new line should have been appended.
        assert!(app.tabs[0].file_reader.line_count() > initial_count);
        drop(tx);
    }

    #[tokio::test]
    async fn test_update_stdin_tab_paused_skips_update() {
        let mut app = make_app(&["old"]).await;
        // Make tab[0] behave as the stdin placeholder (no source_file).
        assert!(app.tabs[0].log_manager.source_file().is_none());
        let initial_count = app.tabs[0].visible_indices.len();

        app.tabs[0].paused = true;
        app.update_stdin_tab(b"new line\nmore data\n".to_vec())
            .await;

        assert_eq!(app.tabs[0].visible_indices.len(), initial_count);
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
        // Simulate the preview tab created before the background load starts.
        let preview_reader = FileReader::from_bytes(vec![]);
        let log_manager = LogManager::new(app.db.clone(), None).await;
        let preview_tab = TabState::new(preview_reader, log_manager, "preview.log".to_string());
        app.tabs.push(preview_tab);
        assert_eq!(app.tabs.len(), 2);

        app.skip_or_fail_load(LoadContext::SessionRestoreTab {
            tab_idx: 1, // the preview tab, not the initial placeholder
            remaining: VecDeque::new(),
            total: 1,
            initial_tab_idx: 0,
        })
        .await;

        // Preview tab removed; initial tab remains.
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
        // Preview is shown immediately; watcher is set up after background load completes.
        assert!(
            app.tabs[1].file_reader.line_count() > 0,
            "preview should be populated"
        );
        assert!(
            app.file_load_state.is_some(),
            "background load should be in progress"
        );
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
            level_colors_disabled: HashSet::new(),
            horizontal_scroll: 0,
            marked_lines: vec![],
            file_hash: None,
            comments: vec![],
            show_keys: true,
            raw_mode: false,
            sidebar_width: 30,
            hidden_fields: HashSet::new(),
            field_layout_columns: None,
            filtering_enabled: true,
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
            cancel: Arc::new(AtomicBool::new(false)),
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
            cancel: Arc::new(AtomicBool::new(false)),
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
            cancel: Arc::new(AtomicBool::new(false)),
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
    async fn test_session_restore_switches_to_preview_tab_immediately() {
        // Startup state: one placeholder tab (empty, no source file).
        let mut app = make_app(&[]).await;
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.active_tab, 0);

        // Write a real file so from_file_head succeeds.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"line one\nline two\n").unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let queue: std::collections::VecDeque<String> = std::iter::once(path).collect();
        app.continue_session_restore(queue, 1, 0).await;

        // Placeholder should be gone; only the preview tab remains.
        assert_eq!(
            app.tabs.len(),
            1,
            "placeholder tab should be removed immediately"
        );
        assert_eq!(app.active_tab, 0);
        assert!(
            app.tabs[0].file_reader.line_count() > 0,
            "preview tab should have content"
        );
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

    #[tokio::test]
    async fn test_session_restore_preview_applies_file_context() {
        use crate::db::{FileContext, FileContextStore};
        use std::collections::HashSet;

        let mut app = make_app(&[]).await;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"line one\nline two\nline three\n").unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        let abs_path = std::fs::canonicalize(tmp.path())
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let ctx = FileContext {
            source_file: abs_path.clone(),
            scroll_offset: 2,
            search_query: String::new(),
            level_colors_disabled: HashSet::new(),
            horizontal_scroll: 0,
            marked_lines: vec![1],
            file_hash: None,
            comments: vec![],
            show_keys: true,
            raw_mode: false,
            sidebar_width: 30,
            hidden_fields: HashSet::new(),
            field_layout_columns: None,
            filtering_enabled: true,
        };
        app.db.save_file_context(&ctx).await.unwrap();

        let queue: std::collections::VecDeque<String> = std::iter::once(path).collect();
        app.continue_session_restore(queue, 1, 0).await;

        // Preview tab should have context applied before the full load completes.
        assert_eq!(
            app.tabs.len(),
            1,
            "placeholder should be replaced by preview tab"
        );
        assert!(
            app.tabs[0].show_keys,
            "show_keys=true from context should be applied"
        );
        assert_eq!(
            app.tabs[0].log_manager.get_marked_indices(),
            vec![1],
            "marks should be loaded from context"
        );
    }

    #[tokio::test]
    async fn test_begin_file_load_predicate_shows_filtered_preview() {
        use crate::ui::VisibleLines;

        let mut app = make_app(&["placeholder"]).await;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"match line\nskip line\nmatch again\n").unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let pred: crate::file_reader::VisibilityPredicate =
            Box::new(|line: &[u8]| line.starts_with(b"match"));

        app.begin_file_load(path, LoadContext::ReplaceInitialTab, Some(pred), false)
            .await;

        // Preview should be filtered: only lines starting with "match".
        assert!(
            matches!(&app.tabs[0].visible_indices, VisibleLines::Filtered(v) if v.len() == 2),
            "filtered preview should contain only matching lines"
        );
    }

    #[tokio::test]
    async fn test_update_stdin_tab_updates_visible_indices() {
        let mut app = make_app(&[]).await;
        app.update_stdin_tab(b"ERROR bad\nINFO ok\nWARN maybe\n".to_vec())
            .await;
        assert_eq!(app.tabs[0].visible_indices.len(), 3);
        assert_eq!(app.tabs[0].next_error_position(0), None);
        assert_eq!(app.tabs[0].prev_error_position(1), Some(0));
        assert_eq!(app.tabs[0].next_warning_position(0), Some(2));
    }

    #[tokio::test]
    async fn test_advance_file_watches_updates_visible_indices() {
        let data: Vec<u8> = b"INFO start\n".to_vec();
        let file_reader = FileReader::from_bytes(data);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
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

        let (tx, rx) = tokio::sync::watch::channel(vec![]);
        app.tabs[0].watch_state = Some(FileWatchState { new_data_rx: rx });

        // Append ERROR (pos 1) and WARN (pos 2) after the existing INFO (pos 0).
        tx.send(b"ERROR bad\nWARN careful\n".to_vec()).unwrap();
        app.advance_file_watches();

        assert_eq!(app.tabs[0].next_error_position(0), Some(1));
        assert_eq!(app.tabs[0].next_warning_position(0), Some(2));
    }

    // ── advance_filter_computation streaming ─────────────────────────────────

    fn make_filter_handle_with_chunks(
        chunks: Vec<super::super::FilterChunk>,
    ) -> (
        super::super::FilterHandle,
        tokio::sync::mpsc::Sender<super::super::FilterChunk>,
    ) {
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;
        let (tx, rx) = tokio::sync::mpsc::channel::<super::super::FilterChunk>(16);
        let handle = super::super::FilterHandle {
            result_rx: rx,
            cancel: Arc::new(AtomicBool::new(false)),
            displayed_progress: 0.0,
            scroll_anchor: None,
            received_first_chunk: false,
        };
        for chunk in chunks {
            tx.try_send(chunk).unwrap();
        }
        (handle, tx)
    }

    #[tokio::test]
    async fn test_advance_filter_computation_first_chunk_replaces_visible() {
        let mut app = make_app(&["line0", "line1", "line2"]).await;
        let (handle, _tx) = make_filter_handle_with_chunks(vec![super::super::FilterChunk {
            visible: vec![0, 2],
            filter_match_counts: None,
            is_last: false,
            progress: 0.5,
        }]);
        app.tabs[0].filter_handle = Some(handle);
        app.advance_filter_computation();
        assert_eq!(
            app.tabs[0].visible_indices,
            VisibleLines::Filtered(vec![0, 2])
        );
        assert!(
            app.tabs[0].filter_handle.is_some(),
            "handle should remain while not last"
        );
    }

    #[tokio::test]
    async fn test_advance_filter_computation_incremental_accumulates() {
        let mut app = make_app(&["a", "b", "c", "d"]).await;
        let (handle, _tx) = make_filter_handle_with_chunks(vec![
            super::super::FilterChunk {
                visible: vec![0, 1],
                filter_match_counts: None,
                is_last: false,
                progress: 0.5,
            },
            super::super::FilterChunk {
                visible: vec![2, 3],
                filter_match_counts: Some(vec![4]),
                is_last: true,
                progress: 1.0,
            },
        ]);
        app.tabs[0].filter_handle = Some(handle);
        app.advance_filter_computation();
        assert_eq!(
            app.tabs[0].visible_indices,
            VisibleLines::Filtered(vec![0, 1, 2, 3])
        );
        assert_eq!(app.tabs[0].filter_match_counts, vec![4]);
        assert!(
            app.tabs[0].filter_handle.is_none(),
            "handle should be cleared after last chunk"
        );
    }

    #[tokio::test]
    async fn test_advance_filter_computation_scroll_clamped_on_intermediate() {
        let mut app = make_app(&["a", "b", "c"]).await;
        app.tabs[0].scroll_offset = 100;
        let (handle, _tx) = make_filter_handle_with_chunks(vec![super::super::FilterChunk {
            visible: vec![0],
            filter_match_counts: None,
            is_last: false,
            progress: 0.3,
        }]);
        app.tabs[0].filter_handle = Some(handle);
        app.advance_filter_computation();
        assert!(
            app.tabs[0].scroll_offset <= app.tabs[0].visible_indices.len().saturating_sub(1),
            "scroll_offset should be clamped to visible length on intermediate chunk"
        );
    }

    #[tokio::test]
    async fn test_advance_filter_computation_scroll_anchor_on_final() {
        let mut app = make_app(&["a", "b", "c", "d"]).await;
        let (mut handle, _tx) = make_filter_handle_with_chunks(vec![
            super::super::FilterChunk {
                visible: vec![0, 1],
                filter_match_counts: None,
                is_last: false,
                progress: 0.5,
            },
            super::super::FilterChunk {
                visible: vec![2, 3],
                filter_match_counts: Some(vec![]),
                is_last: true,
                progress: 1.0,
            },
        ]);
        handle.scroll_anchor = Some(3);
        app.tabs[0].filter_handle = Some(handle);
        app.advance_filter_computation();
        // Line index 3 is at position 3 in the combined visible vec [0,1,2,3].
        assert_eq!(app.tabs[0].scroll_offset, 3);
    }

    #[tokio::test]
    async fn test_advance_filter_computation_disconnect_clears_handle() {
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;
        let mut app = make_app(&["a", "b"]).await;
        let (tx, rx) = tokio::sync::mpsc::channel::<super::super::FilterChunk>(4);
        drop(tx);
        let handle = super::super::FilterHandle {
            result_rx: rx,
            cancel: Arc::new(AtomicBool::new(false)),
            displayed_progress: 0.0,
            scroll_anchor: None,
            received_first_chunk: false,
        };
        app.tabs[0].filter_handle = Some(handle);
        app.advance_filter_computation();
        assert!(
            app.tabs[0].filter_handle.is_none(),
            "handle should be cleared when sender is dropped"
        );
    }

    #[tokio::test]
    async fn test_replace_initial_tab_respects_saved_filtering_disabled() {
        use crate::db::{FileContext, FileContextStore};
        use std::collections::HashSet;
        use std::sync::atomic::AtomicBool;

        let mut app = make_app(&[]).await;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"line1\nline2\nline3\n").unwrap();
        let abs_path = std::fs::canonicalize(tmp.path())
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let ctx = FileContext {
            source_file: abs_path.clone(),
            scroll_offset: 0,
            search_query: String::new(),
            level_colors_disabled: HashSet::new(),
            horizontal_scroll: 0,
            marked_lines: vec![],
            file_hash: None,
            comments: vec![],
            show_keys: true,
            raw_mode: false,
            sidebar_width: 30,
            hidden_fields: HashSet::new(),
            field_layout_columns: None,
            filtering_enabled: false,
        };
        app.db.save_file_context(&ctx).await.unwrap();

        let (progress_tx, progress_rx) = tokio::sync::watch::channel(1.0_f64);
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        let fr = FileReader::from_bytes(b"line1\nline2\nline3\n".to_vec());
        let _ = result_tx.send(Ok(FileLoadResult {
            reader: fr,
            precomputed_visible: None,
        }));
        drop(progress_tx);

        app.file_load_state = Some(super::FileLoadState {
            path: abs_path,
            progress_rx,
            result_rx,
            total_bytes: 18,
            on_complete: LoadContext::ReplaceInitialTab,
            cancel: Arc::new(AtomicBool::new(false)),
        });

        app.advance_file_load().await;

        assert!(
            app.tabs[0].filter_handle.is_none(),
            "filter_handle must be None when filtering_enabled=false was restored"
        );
        assert!(
            matches!(app.tabs[0].visible_indices, VisibleLines::All(_)),
            "visible_indices must be All when filtering is disabled"
        );
    }

    // ── Stream retry ──────────────────────────────────────────────────────────

    fn make_dummy_connect_fn() -> super::super::ConnectFn {
        std::sync::Arc::new(|| Box::pin(async { Err("test".to_string()) }))
    }

    #[tokio::test]
    async fn test_advance_stream_retries_successful_reconnect() {
        let mut app = make_app(&["line"]).await;
        let (tx, rx) = tokio::sync::watch::channel(vec![]);

        let (result_tx, result_rx) = tokio::sync::mpsc::channel(1);
        result_tx.send(Ok(rx)).await.unwrap();

        app.tabs[0].stream_retry = Some(StreamRetryState {
            attempt: 3,
            last_error: "connection refused".to_string(),
            retry_rx: Some(result_rx),
            connect: make_dummy_connect_fn(),
        });
        app.tabs[0].command_error = Some("connection failed".to_string());

        app.advance_stream_retries();

        assert!(
            app.tabs[0].stream_retry.is_none(),
            "retry should be cleared"
        );
        assert!(
            app.tabs[0].command_error.is_none(),
            "error should be cleared"
        );
        assert!(
            app.tabs[0].watch_state.is_some(),
            "watch_state should be set"
        );

        tx.send(b"test data".to_vec()).unwrap();
        assert!(
            app.tabs[0]
                .watch_state
                .as_mut()
                .unwrap()
                .new_data_rx
                .has_changed()
                .is_ok()
        );
    }

    #[tokio::test]
    async fn test_advance_stream_retries_failed_reschedules() {
        let mut app = make_app(&["line"]).await;

        let (result_tx, result_rx) = tokio::sync::mpsc::channel(1);
        result_tx
            .send(Err("connection refused".to_string()))
            .await
            .unwrap();

        app.tabs[0].stream_retry = Some(StreamRetryState {
            attempt: 1,
            last_error: "old error".to_string(),
            retry_rx: Some(result_rx),
            connect: make_dummy_connect_fn(),
        });

        app.advance_stream_retries();

        let retry = app.tabs[0].stream_retry.as_ref().unwrap();
        assert_eq!(retry.attempt, 2, "attempt should be incremented");
        assert_eq!(retry.last_error, "connection refused");
        assert!(retry.retry_rx.is_some(), "new retry should be scheduled");
        assert!(
            app.tabs[0]
                .command_error
                .as_ref()
                .unwrap()
                .contains("retry #1")
        );
    }

    #[tokio::test]
    async fn test_advance_stream_retries_pending_no_change() {
        let mut app = make_app(&["line"]).await;

        let (_result_tx, result_rx) = tokio::sync::mpsc::channel::<Result<_, String>>(1);

        app.tabs[0].stream_retry = Some(StreamRetryState {
            attempt: 1,
            last_error: "waiting".to_string(),
            retry_rx: Some(result_rx),
            connect: make_dummy_connect_fn(),
        });

        app.advance_stream_retries();

        let retry = app.tabs[0].stream_retry.as_ref().unwrap();
        assert_eq!(retry.attempt, 1, "attempt should not change while pending");
    }

    #[tokio::test]
    async fn test_disconnect_triggers_retry_for_dlt() {
        let mut app = make_app(&["line"]).await;

        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, Some("dlt://192.168.1.1:3490".to_string())).await;
        let file_reader = FileReader::from_bytes(vec![]);
        let mut tab = TabState::new(file_reader, log_manager, "dlt:test".to_string());

        let (tx, rx) = tokio::sync::watch::channel(vec![]);
        tab.watch_state = Some(FileWatchState { new_data_rx: rx });
        drop(tx);

        app.tabs.push(tab);
        let tab_idx = app.tabs.len() - 1;

        app.advance_file_watches();

        assert!(app.tabs[tab_idx].watch_state.is_none());
        assert!(app.tabs[tab_idx].stream_retry.is_some());
        assert_eq!(app.tabs[tab_idx].stream_retry.as_ref().unwrap().attempt, 1);
        assert!(app.tabs[tab_idx].command_error.is_some());
    }

    #[tokio::test]
    async fn test_disconnect_triggers_retry_for_docker() {
        let mut app = make_app(&["line"]).await;

        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, Some("docker:mycontainer".to_string())).await;
        let file_reader = FileReader::from_bytes(vec![]);
        let mut tab = TabState::new(file_reader, log_manager, "docker:mycontainer".to_string());

        let (tx, rx) = tokio::sync::watch::channel(vec![]);
        tab.watch_state = Some(FileWatchState { new_data_rx: rx });
        drop(tx);

        app.tabs.push(tab);
        let tab_idx = app.tabs.len() - 1;

        app.advance_file_watches();

        assert!(app.tabs[tab_idx].watch_state.is_none());
        assert!(app.tabs[tab_idx].stream_retry.is_some());
        assert_eq!(app.tabs[tab_idx].stream_retry.as_ref().unwrap().attempt, 1);
        assert!(app.tabs[tab_idx].command_error.is_some());
    }

    #[tokio::test]
    async fn test_restore_dlt_tab_uses_non_blocking_retry() {
        let mut app = make_app(&["line"]).await;
        app.restore_dlt_tab("dlt://192.168.1.1:3490").await;

        let tab_idx = app.tabs.len() - 1;
        assert!(
            app.tabs[tab_idx].stream_retry.is_some(),
            "stream_retry should be set immediately without blocking"
        );
        assert_eq!(app.tabs[tab_idx].stream_retry.as_ref().unwrap().attempt, 1);
        assert!(app.tabs[tab_idx].watch_state.is_none());
    }

    #[tokio::test]
    async fn test_restore_docker_tab_uses_non_blocking_retry() {
        let mut app = make_app(&["line"]).await;
        app.restore_docker_tab("docker:mycontainer").await;

        let tab_idx = app.tabs.len() - 1;
        assert!(
            app.tabs[tab_idx].stream_retry.is_some(),
            "stream_retry should be set immediately without blocking"
        );
        assert_eq!(app.tabs[tab_idx].stream_retry.as_ref().unwrap().attempt, 1);
        assert!(app.tabs[tab_idx].watch_state.is_none());
    }
}
