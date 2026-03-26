use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::config::RestoreSessionPolicy;
use crate::db::FileContextStore;
use crate::db::LogManager;
use crate::ingestion::{FileReader, VisibilityPredicate};
use crate::mode::app_mode::ConfirmRestoreMode;
use crate::mode::normal_mode::NormalMode;

use super::{
    App, ConnectFn, FileLoadState, LoadContext, StreamRetryState, TabState, VisibleLines,
    dlt_connect_fn, docker_connect_fn, otlp_connect_fn, otlp_grpc_connect_fn,
    watch_state_from_connection, watch_state_from_file,
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
    } else if let Some(port_str) = source.strip_prefix("otlp://") {
        let port = port_str.parse::<u16>().unwrap_or(4318);
        Some(otlp_connect_fn(port))
    } else if let Some(port_str) = source.strip_prefix("otlp-grpc://") {
        let port = port_str.parse::<u16>().unwrap_or(4317);
        Some(otlp_grpc_connect_fn(port))
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
                    tab.interaction.mode = Box::new(ConfirmRestoreMode { context: ctx });
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
            Ok(conn) => {
                tab.stream.watch = Some(watch_state_from_connection(conn));
            }
            Err(e) => {
                let err_msg = e.to_string();
                tab.interaction.command_error = Some(format!("Docker attach failed: {}", err_msg));
                tab.stream.retry = Some(StreamRetryState::new(
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
            Ok(conn) => {
                tab.stream.watch = Some(watch_state_from_connection(conn));
            }
            Err(e) => {
                let err_msg = e.to_string();
                tab.interaction.command_error = Some(format!("DLT connection failed: {}", err_msg));
                tab.stream.retry = Some(StreamRetryState::new(dlt_connect_fn(host, port), err_msg));
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

        tab.stream.retry = Some(StreamRetryState::new(
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

        tab.stream.retry = Some(StreamRetryState::new(
            docker_connect_fn(name.to_string()),
            "reconnecting…".to_string(),
        ));

        if let Ok(Some(ctx)) = self.db.load_file_context(source).await {
            tab.apply_file_context(&ctx);
        }

        self.tabs.push(tab);
    }

    /// Open a new tab that listens for OTLP HTTP/JSON log exports on `port`.
    pub(super) async fn open_otlp_stream(&mut self, port: u16) {
        let source_label = format!("otlp://{port}");
        let file_reader = FileReader::from_bytes(vec![]);
        let log_manager = LogManager::new(self.db.clone(), Some(source_label.clone())).await;
        let title = format!("otlp:{port}");

        let mut tab = TabState::new(file_reader, log_manager, title);
        self.apply_tab_defaults(&mut tab);

        match crate::ingestion::spawn_otlp_http_receiver(port).await {
            Ok(conn) => {
                tab.stream.watch = Some(watch_state_from_connection(conn));
            }
            Err(e) => {
                let err_msg = e.to_string();
                tab.interaction.command_error = Some(format!("OTLP receiver failed: {err_msg}"));
                tab.stream.retry = Some(StreamRetryState::new(otlp_connect_fn(port), err_msg));
            }
        }

        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
    }

    async fn restore_otlp_tab(&mut self, source: &str) {
        let port = source
            .strip_prefix("otlp://")
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(4318);
        let file_reader = FileReader::from_bytes(vec![]);
        let log_manager = LogManager::new(self.db.clone(), Some(source.to_string())).await;
        let title = source.to_string();

        let mut tab = TabState::new(file_reader, log_manager, title);
        self.apply_tab_defaults(&mut tab);

        tab.stream.retry = Some(StreamRetryState::new(
            otlp_connect_fn(port),
            "reconnecting…".to_string(),
        ));

        if let Ok(Some(ctx)) = self.db.load_file_context(source).await {
            tab.apply_file_context(&ctx);
        }

        self.tabs.push(tab);
    }

    pub(super) async fn open_otlp_grpc_stream(&mut self, port: u16) {
        let source_label = format!("otlp-grpc://{port}");
        let file_reader = FileReader::from_bytes(vec![]);
        let log_manager = LogManager::new(self.db.clone(), Some(source_label.clone())).await;
        let title = format!("otlp-grpc:{port}");

        let mut tab = TabState::new(file_reader, log_manager, title);
        self.apply_tab_defaults(&mut tab);

        match crate::ingestion::spawn_otlp_grpc_receiver(port).await {
            Ok(conn) => {
                tab.stream.watch = Some(watch_state_from_connection(conn));
            }
            Err(e) => {
                let err_msg = e.to_string();
                tab.interaction.command_error =
                    Some(format!("OTLP gRPC receiver failed: {err_msg}"));
                tab.stream.retry = Some(StreamRetryState::new(otlp_grpc_connect_fn(port), err_msg));
            }
        }

        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
    }

    async fn restore_otlp_grpc_tab(&mut self, source: &str) {
        let port = source
            .strip_prefix("otlp-grpc://")
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(4317);
        let file_reader = FileReader::from_bytes(vec![]);
        let log_manager = LogManager::new(self.db.clone(), Some(source.to_string())).await;
        let title = source.to_string();

        let mut tab = TabState::new(file_reader, log_manager, title);
        self.apply_tab_defaults(&mut tab);

        tab.stream.retry = Some(StreamRetryState::new(
            otlp_grpc_connect_fn(port),
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
            if next.starts_with("otlp://") {
                self.restore_otlp_tab(&next).await;
                continue;
            }
            if next.starts_with("otlp-grpc://") {
                self.restore_otlp_grpc_tab(&next).await;
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
                            .filter(|&i| pred.is_visible(self.tabs[0].file_reader.get_line(i)))
                            .collect();
                        self.tabs[0].filter.visible_indices = VisibleLines::Filtered(visible);
                        self.tabs[0].rebuild_filter_manager_cache();
                        self.tabs[0].invalidate_parse_cache();
                    } else {
                        self.tabs[0].begin_filter_refresh();
                    }
                    if tail && self.tabs[0].filter.handle.is_none() {
                        // Fast path completed synchronously: jump to the last visible line.
                        // Slow path: advance_filter_computation clamps scroll_offset to
                        // visible_len-1 when the background scan finishes, landing at the tail.
                        self.tabs[0].scroll.scroll_offset =
                            self.tabs[0].filter.visible_indices.len().saturating_sub(1);
                    }
                    // Non-tail: stay at line 0; user sees the top of the file immediately.
                }
            }

            let tab_idx = match &context {
                LoadContext::ReplaceInitialTab => 0,
                LoadContext::ReplaceTab { tab_idx } => *tab_idx,
                LoadContext::SessionRestoreTab { tab_idx, .. } => *tab_idx,
            };
            let cancel = Arc::new(AtomicBool::new(false));
            match FileReader::load(path.clone(), predicate, tail, cancel.clone(), false).await {
                Ok(handle) => {
                    if tab_idx < self.tabs.len() {
                        self.tabs[tab_idx].load_state = Some(FileLoadState {
                            path,
                            progress_rx: handle.progress_rx,
                            result_rx: handle.result_rx,
                            total_bytes: handle.total_bytes,
                            on_complete: context,
                            cancel,
                        });
                    }
                }
                Err(_) => self.skip_or_fail_load(context).await,
            }
        })
    }

    /// Start streaming stdin in the background.  Stored in a dedicated slot so
    /// session-restore file loads cannot overwrite it.
    pub async fn begin_stdin_load(&mut self) {
        let (snapshot_rx, temp_file) = FileReader::stream_stdin().await;
        let temp_path = temp_file.path().to_owned();
        self.stdin_load_state = Some(super::StdinLoadState {
            snapshot_rx,
            temp_path,
            temp_file,
        });
    }

    /// Spawn archive extraction in the background (non-blocking).
    /// Shows an "Extracting…" notification on the active tab.
    /// Call [`Self::poll_archive_extraction`] each frame to check for completion.
    pub fn begin_archive_extraction(&mut self, path: &str) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let path_str = path.to_string();
        tokio::task::spawn_blocking(move || {
            let _ = tx.send(crate::ingestion::extract(&path_str));
        });
        self.pending_archive = Some(super::ArchiveExtractionState { result_rx: rx });
        let name = std::path::Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path)
            .to_string();
        self.tabs[self.active_tab].set_notification(format!("Extracting '{name}'…"));
    }

    /// Poll the pending archive extraction each frame.  When the background task
    /// finishes, opens one tab per extracted file and clears `pending_archive`.
    pub async fn poll_archive_extraction(&mut self) {
        let Some(ref mut state) = self.pending_archive else {
            return;
        };
        match state.result_rx.try_recv() {
            Ok(Ok(files)) => {
                self.pending_archive = None;
                if files.is_empty() {
                    self.tabs[self.active_tab].set_notification("Archive contains no files.");
                    return;
                }
                for file in files {
                    let tmp_path = file.temp_file.path().to_string_lossy().to_string();
                    let preview = FileReader::from_file_head(&tmp_path, self.preview_bytes)
                        .await
                        .unwrap_or_else(|_| FileReader::from_bytes(vec![]));
                    let log_manager =
                        LogManager::new(self.db.clone(), Some(tmp_path.clone())).await;
                    let mut tab = TabState::new(preview, log_manager, file.name);
                    tab.archive_temp = Some(file.temp_file);
                    self.apply_tab_defaults(&mut tab);
                    self.tabs.push(tab);
                    self.active_tab = self.tabs.len() - 1;
                    let tab_idx = self.active_tab;
                    self.begin_file_load(
                        tmp_path,
                        LoadContext::ReplaceTab { tab_idx },
                        None,
                        false,
                    )
                    .await;
                }
            }
            Ok(Err(e)) => {
                self.pending_archive = None;
                self.tabs[self.active_tab]
                    .set_notification(format!("Failed to extract archive: {e}"));
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                self.pending_archive = None;
            }
        }
    }

    /// Extract an archive synchronously (blocking on a thread pool) and open each
    /// contained file as a new tab.  Suitable for use before the event loop starts
    /// (startup) or in headless mode where there is no UI to keep responsive.
    pub async fn open_archive_blocking(&mut self, path: &str) -> Result<(), String> {
        let path_str = path.to_string();
        let files = tokio::task::spawn_blocking(move || crate::ingestion::extract(&path_str))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| format!("Failed to extract '{path}': {e}"))?;
        if files.is_empty() {
            return Err(format!("'{path}' contains no files."));
        }
        for file in files {
            let tmp_path = file.temp_file.path().to_string_lossy().to_string();
            let preview = FileReader::from_file_head(&tmp_path, self.preview_bytes)
                .await
                .unwrap_or_else(|_| FileReader::from_bytes(vec![]));
            let log_manager = LogManager::new(self.db.clone(), Some(tmp_path.clone())).await;
            let mut tab = TabState::new(preview, log_manager, file.name);
            tab.archive_temp = Some(file.temp_file);
            self.apply_tab_defaults(&mut tab);
            self.tabs.push(tab);
            self.active_tab = self.tabs.len() - 1;
            let tab_idx = self.active_tab;
            self.begin_file_load(tmp_path, LoadContext::ReplaceTab { tab_idx }, None, false)
                .await;
        }
        Ok(())
    }

    /// Poll for new stdin data each frame and apply it to the stdin tab.
    pub(super) async fn advance_stdin_load(&mut self) {
        let status = self
            .stdin_load_state
            .as_mut()
            .map(|s| s.snapshot_rx.has_changed());

        match status {
            Some(Ok(true)) => {
                let _ = self
                    .stdin_load_state
                    .as_mut()
                    .unwrap()
                    .snapshot_rx
                    .borrow_and_update();
                let path = self.stdin_load_state.as_ref().unwrap().temp_path.clone();
                self.update_stdin_tab(&path).await;
            }
            Some(Err(_)) => {
                // Sender dropped — stdin closed.  mmap FIRST, then unlink temp file.
                let path = self.stdin_load_state.as_ref().unwrap().temp_path.clone();
                self.update_stdin_tab(&path).await;
                self.stdin_load_state = None;
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
    async fn update_stdin_tab(&mut self, path: &std::path::Path) {
        let path_str = path.to_str().unwrap_or_default();
        if let Some(idx) = self
            .tabs
            .iter()
            .position(|t| t.log_manager.source_file().is_none())
        {
            if self.tabs[idx].stream.paused {
                return;
            }
            let old_count = self.tabs[idx].file_reader.line_count();
            let tail_mode = self.tabs[idx].stream.tail_mode;
            let incremental = self.tabs[idx]
                .file_reader
                .try_extend_from_read()
                .unwrap_or(false);
            if !incremental {
                match FileReader::new(path_str) {
                    Ok(r) => self.tabs[idx].file_reader = r,
                    Err(_) => return,
                }
            }
            if self.tabs[idx].file_reader.line_count() == 0 {
                return;
            }
            if self.tabs[idx].display.format.is_none() {
                self.tabs[idx].detect_and_apply_format();
            }
            if incremental {
                self.tabs[idx].filter_new_lines(old_count);
            } else {
                self.tabs[idx].begin_filter_refresh();
            }
            if tail_mode {
                let new_count = self.tabs[idx].filter.visible_indices.len();
                self.tabs[idx].scroll.scroll_offset = new_count.saturating_sub(1);
            }
        } else {
            // Placeholder was removed by session restore — push a new stdin tab.
            match FileReader::new(path_str) {
                Ok(file_reader) if file_reader.line_count() > 0 => {
                    let log_manager = LogManager::new(self.db.clone(), None).await;
                    let mut tab = TabState::new(file_reader, log_manager, "stdin".to_string());
                    self.apply_tab_defaults(&mut tab);
                    tab.scroll.scroll_offset = tab.filter.visible_indices.len().saturating_sub(1);
                    self.tabs.push(tab);
                }
                _ => {}
            }
        }
    }

    pub(super) fn remove_empty_placeholder(&mut self) {
        if self.tabs.len() > 1
            && self.stdin_load_state.is_none()
            && let Some(idx) = self.tabs.iter().position(|t| {
                t.log_manager.source_file().is_none() && t.file_reader.line_count() == 0
            })
        {
            self.tabs.remove(idx);
            if self.active_tab > idx {
                self.active_tab -= 1;
            }
            self.active_tab = self.active_tab.min(self.tabs.len().saturating_sub(1));
        }
    }

    /// Poll for completion of background file loads across all tabs (called every frame).
    pub(super) async fn advance_file_load(&mut self) {
        let mut completed = Vec::new();
        for tab in &mut self.tabs {
            if let Some(ref mut ls) = tab.load_state
                && let Ok(result) = ls.result_rx.try_recv()
            {
                let ls = tab.load_state.take().unwrap();
                completed.push((ls.path, ls.total_bytes, ls.on_complete, result));
            }
        }
        for (path, total_bytes, context, result) in completed {
            match result {
                Ok(r) => self.on_load_success(path, total_bytes, context, r).await,
                Err(_) => self.skip_or_fail_load(context).await,
            }
        }
    }

    /// Handle a completed successful load, then start a file watcher for the tab.
    async fn on_load_success(
        &mut self,
        path: String,
        total_bytes: u64,
        context: LoadContext,
        result: crate::ingestion::FileLoadResult,
    ) {
        match context {
            LoadContext::ReplaceInitialTab => {
                if self.tabs.is_empty() {
                    return;
                }
                self.tabs[0].file_reader = result.reader;
                // Reset visible_indices to All(n) so that the scroll anchor
                // computed below (from the saved absolute file line) is resolved
                // against the full file, not the small preview.
                self.tabs[0].filter.visible_indices =
                    VisibleLines::All(self.tabs[0].file_reader.line_count());
                self.tabs[0].detect_and_apply_format();
                let had_precomputed = result.precomputed_visible.is_some();
                if let Some(visible) = result.precomputed_visible {
                    self.tabs[0].filter.visible_indices = VisibleLines::Filtered(visible);
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
                            self.tabs[0].interaction.mode =
                                Box::new(ConfirmRestoreMode { context: ctx });
                        }
                    }
                }
                if had_precomputed {
                    self.tabs[0].rebuild_filter_manager_cache();
                    self.tabs[0].invalidate_parse_cache();
                    if let Some(text_counts) = result.precomputed_text_counts {
                        let all_filter_defs = self.tabs[0].log_manager.get_filters().to_vec();
                        self.tabs[0].filter.match_counts =
                            crate::ui::tab_state::merge_filter_counts(
                                &all_filter_defs,
                                &text_counts,
                                &[],
                                &[],
                            );
                    } else {
                        self.tabs[0].filter.match_counts = Vec::new();
                    }
                    // The precomputed path skips begin_filter_refresh, so there
                    // is no scroll-anchor mechanism.  Translate the saved absolute
                    // file line (set by apply_file_context above) to its position
                    // in the already-computed filtered view.
                    let saved_line = self.tabs[0].scroll.scroll_offset;
                    self.tabs[0].restore_scroll_to_line(Some(saved_line));
                } else {
                    self.tabs[0].begin_filter_refresh();
                }
                // Apply startup tail: jump to the last visible line and enable tail mode.
                if self.startup_tail {
                    self.tabs[0].stream.tail_mode = true;
                    self.tabs[0].scroll.scroll_offset =
                        self.tabs[0].filter.visible_indices.len().saturating_sub(1);
                }
                let rx = FileReader::spawn_file_watcher(path.clone(), total_bytes).await;
                self.tabs[0].stream.watch = Some(watch_state_from_file(rx, path));
            }
            LoadContext::ReplaceTab { tab_idx } => {
                if tab_idx >= self.tabs.len() {
                    return;
                }
                self.tabs[tab_idx].file_reader = result.reader;
                self.tabs[tab_idx].detect_and_apply_format();
                self.tabs[tab_idx].begin_filter_refresh();
                let rx = FileReader::spawn_file_watcher(path.clone(), total_bytes).await;
                self.tabs[tab_idx].stream.watch = Some(watch_state_from_file(rx, path));
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
                // Reset visible_indices to All(n) so that the scroll anchor
                // computed in begin_filter_refresh reflects the full file, not
                // a stale preview.  apply_file_context stores an absolute file
                // line (saved by to_file_context) in scroll_offset; begin_filter_refresh
                // converts it via visible_indices.get_opt — so visible_indices
                // must cover the full file at that point.
                self.tabs[tab_idx].filter.visible_indices =
                    VisibleLines::All(self.tabs[tab_idx].file_reader.line_count());
                self.tabs[tab_idx].detect_and_apply_format();
                if let Ok(Some(ctx)) = self.db.load_file_context(&path).await {
                    self.tabs[tab_idx].apply_file_context(&ctx);
                }
                self.tabs[tab_idx].begin_filter_refresh();
                let rx = FileReader::spawn_file_watcher(path.clone(), total_bytes).await;
                self.tabs[tab_idx].stream.watch = Some(watch_state_from_file(rx, path));

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
                .stream
                .watch
                .as_mut()
                .map(|ws| ws.snapshot_rx.has_changed());

            match status {
                Some(Ok(true)) => {
                    let _ = self.tabs[i]
                        .stream
                        .watch
                        .as_mut()
                        .unwrap()
                        .snapshot_rx
                        .borrow_and_update();
                    if self.tabs[i].stream.paused {
                        continue;
                    }
                    let reader_path = self.tabs[i]
                        .stream
                        .watch
                        .as_ref()
                        .unwrap()
                        .reader_path
                        .clone();
                    let path_str = reader_path.to_str().unwrap_or_default();
                    let tail_mode = self.tabs[i].stream.tail_mode;
                    let old_line_count = self.tabs[i].file_reader.line_count();
                    let incremental = self.tabs[i]
                        .file_reader
                        .try_extend_from_read()
                        .unwrap_or(false);
                    if !incremental {
                        match crate::ingestion::FileReader::new(path_str) {
                            Ok(r) => self.tabs[i].file_reader = r,
                            Err(_) => continue,
                        }
                    }
                    if self.tabs[i].display.format.is_none()
                        && self.tabs[i].file_reader.line_count() > 0
                    {
                        self.tabs[i].detect_and_apply_format();
                    }
                    if incremental {
                        self.tabs[i].filter_new_lines(old_line_count);
                    } else {
                        self.tabs[i].begin_filter_refresh();
                    }
                    if tail_mode {
                        let new_count = self.tabs[i].filter.visible_indices.len();
                        self.tabs[i].scroll.scroll_offset = new_count.saturating_sub(1);
                    }
                }
                Some(Err(_)) => {
                    self.tabs[i].stream.watch = None;
                    self.tabs[i].interaction.command_error =
                        Some("Disconnected: connection lost".to_string());
                    if let Some(retry) = &mut self.tabs[i].stream.retry {
                        // Reuse the existing retry state to preserve the backoff counter.
                        retry.connected = false;
                        retry.last_error = "connection lost".to_string();
                        retry.schedule_retry();
                    } else if let Some(connect_fn) =
                        connect_fn_for_source(self.tabs[i].log_manager.source_file())
                    {
                        self.tabs[i].stream.retry = Some(StreamRetryState::new(
                            connect_fn,
                            "connection lost".to_string(),
                        ));
                    }
                }
                _ => {}
            }
        }
    }

    /// Poll DLT retry channels for reconnection results.
    pub(super) fn advance_stream_retries(&mut self) {
        for tab in &mut self.tabs {
            let retry = match &mut tab.stream.retry {
                Some(r) => r,
                None => continue,
            };
            let rx = match &mut retry.retry_rx {
                Some(rx) => rx,
                None => continue,
            };
            match rx.try_recv() {
                Ok(Ok(conn)) => {
                    tab.file_reader = crate::ingestion::FileReader::from_bytes(vec![]);
                    tab.display.format = None;
                    tab.filter.visible_indices = crate::ui::VisibleLines::default();
                    tab.filter.handle = None;
                    tab.scroll.scroll_offset = 0;
                    tab.stream.watch = Some(watch_state_from_connection(conn));
                    tab.interaction.command_error = None;
                    // Keep stream_retry alive so the attempt counter is preserved
                    // across reconnect cycles; clear the pending rx and mark as connected
                    // so the [RETRY #N] label is suppressed.
                    if let Some(retry) = &mut tab.stream.retry {
                        retry.connected = true;
                        retry.retry_rx = None;
                    }
                }
                Ok(Err(e)) => {
                    retry.last_error = e.clone();
                    tab.interaction.command_error = Some(format!(
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
    /// On completion, results are written into `tab.search.query` and the view
    /// is scrolled to the first match when `navigate` was set.
    pub(super) fn advance_search(&mut self) {
        use tokio::sync::mpsc::error::TryRecvError;

        for tab in &mut self.tabs {
            let Some(ref mut h) = tab.search.handle else {
                continue;
            };
            let forward = h.forward;
            let navigate = h.navigate;
            let mut done = false;

            loop {
                match h.result_rx.try_recv() {
                    Ok(batch) => {
                        tab.search.query.extend_results(batch);
                        tab.cache.search_result_gen = tab.cache.search_result_gen.wrapping_add(1);
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        done = true;
                        break;
                    }
                }
            }

            if done {
                tab.search.handle = None;

                if navigate && !tab.search.query.get_results().is_empty() {
                    let current_line_idx = tab
                        .filter
                        .visible_indices
                        .get_opt(tab.scroll.scroll_offset)
                        .unwrap_or(0);
                    tab.search
                        .query
                        .set_position_for_search(current_line_idx, forward);
                    if forward {
                        tab.search.query.next_match();
                    } else {
                        tab.search.query.previous_match();
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
            if tab.filter.handle.is_none() {
                continue;
            }

            // Phase 1: drain available chunks into a local buffer (limits borrow scope).
            let (chunks, done) = {
                let h = tab.filter.handle.as_mut().unwrap();
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

            let already_had_first = tab.filter.handle.as_ref().unwrap().received_first_chunk;
            let scroll_anchor = tab.filter.handle.as_ref().unwrap().scroll_anchor;

            if !chunks.is_empty() {
                tab.filter.handle.as_mut().unwrap().received_first_chunk = true;
            }

            // Phase 2: apply chunks to visible_indices.
            let mut should_replace = !already_had_first;
            for chunk in chunks {
                let is_last = chunk.is_last;
                if let Some(h) = tab.filter.handle.as_mut() {
                    h.displayed_progress = chunk.progress;
                }
                if should_replace {
                    tab.filter.visible_indices = VisibleLines::Filtered(chunk.visible);
                    should_replace = false;
                } else if let VisibleLines::Filtered(ref mut v) = tab.filter.visible_indices {
                    v.extend(chunk.visible);
                }
                if let Some(counts) = chunk.filter_match_counts {
                    tab.filter.match_counts = counts;
                }
                if is_last {
                    if let Some(idx) = scroll_anchor
                        && let Some(pos) = tab.filter.visible_indices.position_of(idx)
                    {
                        tab.scroll.scroll_offset = pos;
                    } else if tab.filter.visible_indices.is_empty() {
                        tab.scroll.scroll_offset = 0;
                    } else {
                        tab.scroll.scroll_offset = tab
                            .scroll
                            .scroll_offset
                            .min(tab.filter.visible_indices.len() - 1);
                    }
                } else if tab.filter.visible_indices.is_empty() {
                    tab.scroll.scroll_offset = 0;
                } else {
                    tab.scroll.scroll_offset = tab
                        .scroll
                        .scroll_offset
                        .min(tab.filter.visible_indices.len() - 1);
                }
            }
            if done {
                if let Some(h) = &tab.filter.handle {
                    tab.filter.cached_scan = Some(super::CachedScanResult {
                        filter_fingerprint: h.scan_fingerprint.clone(),
                        line_count: h.scan_line_count,
                        raw_mode: h.scan_raw_mode,
                        view: (
                            tab.filter.visible_indices.clone(),
                            tab.filter.manager.clone(),
                            tab.filter.text_styles.clone(),
                            tab.filter.date_styles.clone(),
                            tab.filter.field_styles.clone(),
                        ),
                        match_counts: tab.filter.match_counts.clone(),
                    });
                }
                tab.filter.handle = None;
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
        self.tabs[self.active_tab].interaction.mode = Box::new(NormalMode::default());
        self.continue_session_restore(queue, total, initial_tab_idx)
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Keybindings;
    use crate::db::Database;
    use crate::db::LogManager;
    use crate::ingestion::{FileLoadResult, FileReader};
    use crate::mode::app_mode::ModeRenderState;
    use crate::theme::Theme;
    use crate::ui::{FileWatchState, SearchHandle, StdinLoadState, VisibleLines};
    use std::collections::VecDeque;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    fn make_stdin_file(data: &[u8]) -> tempfile::NamedTempFile {
        use std::io::Write as _;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(data).unwrap();
        f.flush().unwrap();
        f
    }

    /// Create a stream-type `FileWatchState` backed by a temp file pre-populated
    /// with `data`.  Returns the sender so tests can trigger notifications.
    fn make_watch_state(data: &[u8]) -> (tokio::sync::watch::Sender<()>, FileWatchState) {
        use std::io::Write as _;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(data).unwrap();
        f.flush().unwrap();
        let reader_path = f.path().to_owned();
        let (tx, rx) = tokio::sync::watch::channel(());
        let state = FileWatchState {
            snapshot_rx: rx,
            reader_path,
            temp_file: Some(f),
        };
        (tx, state)
    }

    fn append_to_watch_file(state: &FileWatchState, data: &[u8]) {
        use std::io::Write as _;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&state.reader_path)
            .unwrap();
        f.write_all(data).unwrap();
        f.flush().unwrap();
    }

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
        let f = make_stdin_file(b"");
        app.update_stdin_tab(f.path()).await;
        assert_eq!(app.tabs[0].file_reader.line_count(), line_count_before);
    }

    #[tokio::test]
    async fn test_update_stdin_tab_replace_placeholder() {
        let mut app = make_app(&[]).await;
        assert_eq!(app.tabs[0].file_reader.line_count(), 0);

        let f = make_stdin_file(b"line1\nline2\n");
        app.update_stdin_tab(f.path()).await;

        assert_eq!(app.tabs[0].file_reader.line_count(), 2);
        assert_eq!(app.tabs[0].filter.visible_indices.len(), 2);
    }

    #[tokio::test]
    async fn test_update_stdin_tab_detects_format_on_first_data() {
        let mut app = make_app(&[]).await;
        assert!(app.tabs[0].display.format.is_none());

        let journalctl_line = b"Mar 15 10:00:00 myhost sshd[1234]: Accepted password for user\n";
        let f = make_stdin_file(journalctl_line);
        app.update_stdin_tab(f.path()).await;

        assert!(app.tabs[0].display.format.is_some());
        assert_eq!(
            app.tabs[0].display.format.as_ref().unwrap().name(),
            "journalctl"
        );
    }

    #[tokio::test]
    async fn test_update_stdin_tab_does_not_redetect_after_format_known() {
        let mut app = make_app(&[]).await;
        let f1 = make_stdin_file(b"Mar 15 10:00:00 myhost sshd[1234]: first line\n");
        app.update_stdin_tab(f1.path()).await;
        assert_eq!(
            app.tabs[0].display.format.as_ref().unwrap().name(),
            "journalctl"
        );

        // Manually set a different format to verify it is NOT overwritten.
        use crate::parser::SyslogParser;
        use std::sync::Arc;
        app.tabs[0].display.format = Some(Arc::new(SyslogParser::default()));

        let f2 = make_stdin_file(b"Mar 15 10:00:00 myhost sshd[1234]: first line\nMar 15 10:00:01 myhost sshd[1234]: second line\n");
        app.update_stdin_tab(f2.path()).await;

        assert_eq!(
            app.tabs[0].display.format.as_ref().unwrap().name(),
            "syslog"
        );
    }

    #[tokio::test]
    async fn test_update_stdin_tab_tail_mode_scrolls_to_last() {
        let mut app = make_app(&["first", "second"]).await;
        app.tabs[0].stream.tail_mode = true;
        app.tabs[0].scroll.scroll_offset = 0;

        let f = make_stdin_file(b"first\nsecond\nthird\nfourth\n");
        app.update_stdin_tab(f.path()).await;

        // With tail_mode on, scroll_offset should be at the new last line.
        let new_last = app.tabs[0].filter.visible_indices.len().saturating_sub(1);
        assert_eq!(app.tabs[0].scroll.scroll_offset, new_last);
        assert!(new_last > 0);
    }

    #[tokio::test]
    async fn test_update_stdin_tab_no_tail_no_scroll() {
        let mut app = make_app(&["first", "second"]).await;
        app.tabs[0].stream.tail_mode = false;
        app.tabs[0].scroll.scroll_offset = 0;

        let f = make_stdin_file(b"first\nsecond\nthird\nfourth\n");
        app.update_stdin_tab(f.path()).await;

        // With tail_mode off, scroll_offset should stay where it was.
        assert_eq!(app.tabs[0].scroll.scroll_offset, 0);
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

        let f = make_stdin_file(b"stdin line\n");
        app.update_stdin_tab(f.path()).await;

        assert_eq!(app.tabs.len(), 2);
    }

    #[tokio::test]
    async fn test_advance_file_watches_no_watchers() {
        let mut app = make_app(&["line1", "line2"]).await;
        assert!(app.tabs[0].stream.watch.is_none());
        // Should not panic.
        app.advance_file_watches();
    }

    #[tokio::test]
    async fn test_advance_file_watches_with_data() {
        // Stream tabs start empty; all data comes through the watch state.
        let mut app = make_app(&[]).await;
        let original_count = app.tabs[0].file_reader.line_count();

        let (tx, state) = make_watch_state(b"new line\n");
        app.tabs[0].stream.watch = Some(state);

        tx.send(()).unwrap();
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
        // Stream tab starts empty; all 6 lines come through the watch state.
        let mut app = make_app(&[]).await;
        app.tabs[0].stream.tail_mode = true;
        app.tabs[0].scroll.scroll_offset = 0;

        let (tx, state) = make_watch_state(b"line1\nline2\nline3\nline4\nline5\nline6\n");
        app.tabs[0].stream.watch = Some(state);

        tx.send(()).unwrap();
        app.advance_file_watches();

        let last = app.tabs[0].filter.visible_indices.len().saturating_sub(1);
        assert_eq!(
            app.tabs[0].scroll.scroll_offset, last,
            "tail_mode on: scroll should track the last line"
        );
        assert_eq!(app.tabs[0].file_reader.line_count(), 6);
    }

    #[tokio::test]
    async fn test_file_stream_tail_off_preserves_scroll() {
        // Same setup but tail_mode is off — scroll must not move.
        // Stream tab starts empty; all 6 lines come through the watch state.
        let mut app = make_app(&[]).await;
        app.tabs[0].stream.tail_mode = false;
        app.tabs[0].scroll.scroll_offset = 0;

        // First delivery: 3 lines so user has a position to stay at.
        let (tx, state) = make_watch_state(b"line1\nline2\nline3\n");
        app.tabs[0].stream.watch = Some(state);
        tx.send(()).unwrap();
        app.advance_file_watches();
        app.tabs[0].scroll.scroll_offset = 1; // user moves to middle

        // Second delivery: 3 more lines.
        append_to_watch_file(
            app.tabs[0].stream.watch.as_ref().unwrap(),
            b"line4\nline5\nline6\n",
        );
        tx.send(()).unwrap();
        app.advance_file_watches();

        assert_eq!(
            app.tabs[0].scroll.scroll_offset, 1,
            "tail_mode off: scroll should stay where the user left it"
        );
        assert_eq!(app.tabs[0].file_reader.line_count(), 6);
    }

    #[tokio::test]
    async fn test_file_stream_multiple_batches_tail_on() {
        // New lines arrive in multiple watch batches; tail_mode keeps up.
        let mut app = make_app(&[]).await;
        app.tabs[0].stream.tail_mode = true;

        let (tx, state) = make_watch_state(b"");
        app.tabs[0].stream.watch = Some(state);

        for batch in &[b"a\nb\nc\n".as_ref(), b"d\ne\n".as_ref(), b"f\n".as_ref()] {
            append_to_watch_file(app.tabs[0].stream.watch.as_ref().unwrap(), batch);
            tx.send(()).unwrap();
            app.advance_file_watches();
            let last = app.tabs[0].filter.visible_indices.len().saturating_sub(1);
            assert_eq!(
                app.tabs[0].scroll.scroll_offset, last,
                "after each batch, scroll should be at last line"
            );
        }
        assert_eq!(app.tabs[0].file_reader.line_count(), 6);
    }

    #[tokio::test]
    async fn test_file_stream_tail_on_with_real_file() {
        // Write initial content to a temp file, set up the app via open_file,
        // then append more lines to the same file and deliver a notification —
        // verifying that the scroll is dragged to the end.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        std::fs::write(&path, b"first\nsecond\nthird\n").unwrap();

        let mut app = make_app(&[]).await;
        app.open_file(&path).await.unwrap();

        // open_file adds a new tab (index 1).
        let tab_idx = app.tabs.len() - 1;
        app.active_tab = tab_idx;
        app.tabs[tab_idx].stream.tail_mode = true;
        app.tabs[tab_idx].scroll.scroll_offset = 0; // scroll to top

        // Replace the real watcher with a manual channel watching the original file.
        let (tx, rx) = tokio::sync::watch::channel(());
        app.tabs[tab_idx].stream.watch = Some(super::watch_state_from_file(rx, path.clone()));

        // Append new lines to the original file, then notify.
        use std::io::Write as _;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        f.write_all(b"fourth\nfifth\n").unwrap();
        f.flush().unwrap();

        tx.send(()).unwrap();
        app.advance_file_watches();

        let last = app.tabs[tab_idx]
            .filter
            .visible_indices
            .len()
            .saturating_sub(1);
        assert_eq!(
            app.tabs[tab_idx].scroll.scroll_offset, last,
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

        app.tabs[0].stream.tail_mode = false;
        app.tabs[0].scroll.scroll_offset = 0;

        let (tx, state) = make_watch_state(b"");
        app.tabs[0].stream.watch = Some(state);

        // First batch with tail off — scroll stays.
        append_to_watch_file(app.tabs[0].stream.watch.as_ref().unwrap(), b"l4\nl5\n");
        tx.send(()).unwrap();
        app.advance_file_watches();
        assert_eq!(
            app.tabs[0].scroll.scroll_offset, 0,
            "tail off: should not scroll"
        );

        // Enable tail (like the user runs :tail).
        app.tabs[0].stream.tail_mode = true;

        // Second batch — now scroll should follow.
        append_to_watch_file(app.tabs[0].stream.watch.as_ref().unwrap(), b"l6\nl7\n");
        tx.send(()).unwrap();
        app.advance_file_watches();
        let last = app.tabs[0].filter.visible_indices.len().saturating_sub(1);
        assert_eq!(
            app.tabs[0].scroll.scroll_offset, last,
            "tail on: should scroll to last after enable"
        );
    }

    #[tokio::test]
    async fn test_advance_file_watches_paused_skips_update() {
        let mut app = make_app(&["old line"]).await;
        app.tabs[0].stream.paused = true;
        let initial_count = app.tabs[0].filter.visible_indices.len();

        let (tx, state) = make_watch_state(b"new line\n");
        app.tabs[0].stream.watch = Some(state);

        tx.send(()).unwrap();
        app.advance_file_watches();

        // Paused: no new lines should have been appended.
        assert_eq!(app.tabs[0].filter.visible_indices.len(), initial_count);
        drop(tx);
    }

    #[tokio::test]
    async fn test_advance_file_watches_unpaused_applies_update() {
        let mut app = make_app(&[]).await;

        let (tx, state) = make_watch_state(b"new line\n");
        app.tabs[0].stream.watch = Some(state);

        tx.send(()).unwrap();
        app.advance_file_watches();

        // Not paused: new line should have been appended.
        assert!(app.tabs[0].file_reader.line_count() > 0);
        drop(tx);
    }

    #[tokio::test]
    async fn test_update_stdin_tab_paused_skips_update() {
        let mut app = make_app(&["old"]).await;
        // Make tab[0] behave as the stdin placeholder (no source_file).
        assert!(app.tabs[0].log_manager.source_file().is_none());
        let initial_count = app.tabs[0].filter.visible_indices.len();

        app.tabs[0].stream.paused = true;
        let f = make_stdin_file(b"new line\nmore data\n");
        app.update_stdin_tab(f.path()).await;

        assert_eq!(app.tabs[0].filter.visible_indices.len(), initial_count);
    }

    #[tokio::test]
    async fn test_advance_file_watches_sender_dropped() {
        let mut app = make_app(&["line"]).await;
        let (tx, state) = make_watch_state(b"");
        app.tabs[0].stream.watch = Some(state);

        drop(tx);
        app.advance_file_watches();

        assert!(app.tabs[0].stream.watch.is_none());
    }

    #[tokio::test]
    async fn test_advance_file_watches_format_redetection() {
        let mut app = make_app(&[]).await;
        assert!(app.tabs[0].display.format.is_none());

        let json_data = b"{\"level\":\"INFO\",\"msg\":\"hello\"}\n";
        let (tx, state) = make_watch_state(json_data);
        app.tabs[0].stream.watch = Some(state);

        tx.send(()).unwrap();
        app.advance_file_watches();

        assert!(app.tabs[0].display.format.is_some());
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
            app.tabs[0].interaction.mode.render_state(),
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
            app.tabs[0].interaction.mode.render_state(),
            ModeRenderState::Normal
        ));

        // With a non-empty vec, mode is set to NormalMode at the start.
        // Use a non-existent file so begin_file_load fails gracefully
        // and skip_or_fail_load handles it.
        app.restore_session(vec!["/nonexistent/file.log".to_string()])
            .await;
        assert!(matches!(
            app.tabs[0].interaction.mode.render_state(),
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
        let temp_file = make_stdin_file(b"stdin line\n");
        let temp_path = temp_file.path().to_owned();
        let (tx, rx) = tokio::sync::watch::channel(());
        app.stdin_load_state = Some(StdinLoadState {
            snapshot_rx: rx,
            temp_path,
            temp_file,
        });

        tx.send(()).unwrap();
        app.advance_stdin_load().await;

        assert_eq!(app.tabs[0].file_reader.line_count(), 1);
    }

    #[tokio::test]
    async fn test_advance_stdin_load_sender_dropped() {
        let mut app = make_app(&[]).await;
        let temp_file = make_stdin_file(b"final line\n");
        let temp_path = temp_file.path().to_owned();
        let (tx, rx) = tokio::sync::watch::channel(());
        app.stdin_load_state = Some(StdinLoadState {
            snapshot_rx: rx,
            temp_path,
            temp_file,
        });

        drop(tx);

        app.advance_stdin_load().await;

        assert!(app.stdin_load_state.is_none());
        assert_eq!(app.tabs[0].file_reader.line_count(), 1);
    }

    #[tokio::test]
    async fn test_advance_file_load_no_state() {
        let mut app = make_app(&[]).await;
        assert!(app.tabs.iter().all(|t| t.load_state.is_none()));
        // Should not panic.
        app.advance_file_load().await;
    }

    #[tokio::test]
    async fn test_advance_file_load_concurrent_tabs_both_complete() {
        let tmp0 = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp0.path(), b"file0\n").unwrap();
        let tmp1 = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp1.path(), b"file1\n").unwrap();

        let mut app = make_app(&[]).await;

        let path0 = tmp0.path().to_str().unwrap().to_string();
        let path1 = tmp1.path().to_str().unwrap().to_string();

        app.open_file(&path0).await.unwrap();
        app.open_file(&path1).await.unwrap();

        assert_eq!(app.tabs.len(), 3, "two new tabs should have been added");
        assert!(
            app.tabs[1].load_state.is_some(),
            "tab 1 load should be in progress"
        );
        assert!(
            app.tabs[2].load_state.is_some(),
            "tab 2 load should be in progress"
        );

        for _ in 0..200 {
            if app.tabs[1].load_state.is_none() && app.tabs[2].load_state.is_none() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            app.advance_file_load().await;
        }

        assert!(
            app.tabs[1].load_state.is_none(),
            "tab 1 load should have completed"
        );
        assert!(
            app.tabs[2].load_state.is_none(),
            "tab 2 load should have completed"
        );
        assert!(
            app.tabs[1].stream.watch.is_some(),
            "tab 1 should have a file watcher"
        );
        assert!(
            app.tabs[2].stream.watch.is_some(),
            "tab 2 should have a file watcher"
        );
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
    async fn test_remove_empty_placeholder_removes_initial_tab() {
        let tmp0 = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp0.path(), b"line0\n").unwrap();
        let tmp1 = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp1.path(), b"line1\n").unwrap();

        let mut app = make_app(&[]).await;
        assert_eq!(app.tabs.len(), 1);
        assert!(app.tabs[0].log_manager.source_file().is_none());

        app.open_file(tmp0.path().to_str().unwrap()).await.unwrap();
        app.open_file(tmp1.path().to_str().unwrap()).await.unwrap();
        assert_eq!(app.tabs.len(), 3);

        app.remove_empty_placeholder();

        assert_eq!(app.tabs.len(), 2, "placeholder tab should be removed");
        assert!(
            app.tabs[0].log_manager.source_file().is_some(),
            "remaining tabs should have a source file"
        );
        assert!(
            app.tabs[1].log_manager.source_file().is_some(),
            "remaining tabs should have a source file"
        );
    }

    #[tokio::test]
    async fn test_remove_empty_placeholder_keeps_only_tab() {
        let mut app = make_app(&[]).await;
        assert_eq!(app.tabs.len(), 1);

        app.remove_empty_placeholder();

        assert_eq!(
            app.tabs.len(),
            1,
            "should not remove when only one tab exists"
        );
    }

    #[tokio::test]
    async fn test_remove_empty_placeholder_skips_when_stdin_active() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"line\n").unwrap();

        let mut app = make_app(&[]).await;
        app.open_file(tmp.path().to_str().unwrap()).await.unwrap();
        assert_eq!(app.tabs.len(), 2);

        let fake_file = tempfile::NamedTempFile::new().unwrap();
        let (_, rx) = tokio::sync::watch::channel(());
        app.stdin_load_state = Some(StdinLoadState {
            snapshot_rx: rx,
            temp_path: fake_file.path().to_owned(),
            temp_file: fake_file,
        });

        app.remove_empty_placeholder();

        assert_eq!(
            app.tabs.len(),
            2,
            "should not remove while stdin is loading"
        );
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
            app.tabs[1].load_state.is_some(),
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
            if app.tabs[0].load_state.is_none() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            app.advance_file_load().await;
        }

        assert!(
            !matches!(
                app.tabs[0].interaction.mode.render_state(),
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
        assert!(app.tabs[0].load_state.is_some());
        assert_eq!(app.tabs[0].load_state.as_ref().unwrap().path, path);
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
            precomputed_text_counts: None,
        }));
        drop(progress_tx);

        app.tabs[0].load_state = Some(super::FileLoadState {
            path: "test.log".to_string(),
            progress_rx,
            result_rx,
            total_bytes: 7,
            on_complete: LoadContext::ReplaceInitialTab,
            cancel: Arc::new(AtomicBool::new(false)),
        });

        app.advance_file_load().await;

        // Load state should be consumed.
        assert!(app.tabs[0].load_state.is_none());
        // The initial tab should have the loaded data.
        assert_eq!(app.tabs[0].file_reader.line_count(), 1);
    }

    #[tokio::test]
    async fn test_advance_file_load_redetects_format() {
        let mut app = make_app(&[]).await;
        // Initial tab has no detected format (empty placeholder).
        assert!(app.tabs[0].display.format.is_none());

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
            precomputed_text_counts: None,
        }));
        drop(progress_tx);

        app.tabs[0].load_state = Some(super::FileLoadState {
            path: "test.log".to_string(),
            progress_rx,
            result_rx,
            total_bytes: 60,
            on_complete: LoadContext::ReplaceInitialTab,
            cancel: Arc::new(AtomicBool::new(false)),
        });

        app.advance_file_load().await;

        assert!(app.tabs[0].load_state.is_none());
        assert!(
            app.tabs[0].display.format.is_some(),
            "Format should be re-detected after ReplaceInitialTab load"
        );
    }

    #[tokio::test]
    async fn test_on_load_success_precomputed_skips_filter_refresh() {
        let mut app = make_app(&[]).await;

        let (progress_tx, progress_rx) = tokio::sync::watch::channel(1.0_f64);
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        let fr = FileReader::from_bytes(b"ERROR line\nINFO line\nERROR again\n".to_vec());
        let _ = result_tx.send(Ok(FileLoadResult {
            reader: fr,
            precomputed_visible: Some(vec![0, 2]),
            precomputed_text_counts: None,
        }));
        drop(progress_tx);

        app.tabs[0].load_state = Some(super::FileLoadState {
            path: "test.log".to_string(),
            progress_rx,
            result_rx,
            total_bytes: 32,
            on_complete: LoadContext::ReplaceInitialTab,
            cancel: Arc::new(AtomicBool::new(false)),
        });

        app.advance_file_load().await;

        assert!(
            app.tabs[0].filter.handle.is_none(),
            "no background filter scan should be started when precomputed_visible was set"
        );
        assert_eq!(
            app.tabs[0].filter.visible_indices.len(),
            2,
            "visible_indices should reflect the precomputed result"
        );
    }

    #[tokio::test]
    async fn test_on_load_success_precomputed_sets_filter_styles() {
        use crate::filters::{FilterDecision, FilterManager, SubstringFilter};
        let mut app = make_app(&[]).await;

        let filter = SubstringFilter::new("ERROR", FilterDecision::Include, false, 0).unwrap();
        let fm = FilterManager::new(vec![Box::new(filter)], true);
        app.tabs[0]
            .log_manager
            .add_filter_with_color(
                "ERROR".into(),
                crate::filters::FilterType::Include,
                None,
                None,
                true,
            )
            .await;

        let (progress_tx, progress_rx) = tokio::sync::watch::channel(1.0_f64);
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        let fr = FileReader::from_bytes(b"ERROR line\nINFO line\nERROR again\n".to_vec());
        let _ = result_tx.send(Ok(FileLoadResult {
            reader: fr,
            precomputed_visible: Some(vec![0, 2]),
            precomputed_text_counts: Some(vec![2]),
        }));
        drop(progress_tx);

        app.tabs[0].load_state = Some(super::FileLoadState {
            path: "test.log".to_string(),
            progress_rx,
            result_rx,
            total_bytes: 32,
            on_complete: LoadContext::ReplaceInitialTab,
            cancel: Arc::new(AtomicBool::new(false)),
        });
        let _ = fm; // suppress unused warning

        app.advance_file_load().await;

        assert!(
            app.tabs[0].filter.handle.is_none(),
            "no background filter scan when precomputed"
        );
        assert_eq!(app.tabs[0].filter.visible_indices.len(), 2);
        assert!(
            !app.tabs[0].filter.text_styles.is_empty(),
            "filter_styles must be populated after precomputed load so highlighting works"
        );
    }

    #[tokio::test]
    async fn test_on_load_success_no_precomputed_starts_filter_refresh() {
        let mut app = make_app(&[]).await;

        let (progress_tx, progress_rx) = tokio::sync::watch::channel(1.0_f64);
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        let fr = FileReader::from_bytes(b"ERROR line\nINFO line\n".to_vec());
        let _ = result_tx.send(Ok(FileLoadResult {
            reader: fr,
            precomputed_visible: None,
            precomputed_text_counts: None,
        }));
        drop(progress_tx);

        app.tabs[0].load_state = Some(super::FileLoadState {
            path: "test.log".to_string(),
            progress_rx,
            result_rx,
            total_bytes: 21,
            on_complete: LoadContext::ReplaceInitialTab,
            cancel: Arc::new(AtomicBool::new(false)),
        });

        app.advance_file_load().await;

        assert!(
            app.tabs[0].filter.handle.is_none(),
            "no background scan when there are no active filters"
        );
        assert_eq!(app.tabs[0].file_reader.line_count(), 2);
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

        app.tabs[0].load_state = Some(super::FileLoadState {
            path: "missing.log".to_string(),
            progress_rx,
            result_rx,
            total_bytes: 0,
            on_complete: LoadContext::ReplaceInitialTab,
            cancel: Arc::new(AtomicBool::new(false)),
        });

        app.advance_file_load().await;

        // Load state should be consumed even on failure.
        assert!(app.tabs[0].load_state.is_none());
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
            app.tabs[0].interaction.mode.render_state(),
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
            app.tabs[0].display.show_keys,
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

        use crate::filters::{FilterDecision, FilterManager, SubstringFilter};
        let filter = SubstringFilter::new("match", FilterDecision::Include, false, 0).unwrap();
        let fm = FilterManager::new(vec![Box::new(filter)], true);
        let pred = crate::ingestion::VisibilityPredicate::new(fm);

        app.begin_file_load(path, LoadContext::ReplaceInitialTab, Some(pred), false)
            .await;

        // Preview should be filtered: only lines starting with "match".
        assert!(
            matches!(&app.tabs[0].filter.visible_indices, VisibleLines::Filtered(v) if v.len() == 2),
            "filtered preview should contain only matching lines"
        );
    }

    #[tokio::test]
    async fn test_update_stdin_tab_updates_visible_indices() {
        let mut app = make_app(&[]).await;
        let f = make_stdin_file(b"ERROR bad\nINFO ok\nWARN maybe\n");
        app.update_stdin_tab(f.path()).await;
        assert_eq!(app.tabs[0].filter.visible_indices.len(), 3);
        assert_eq!(app.tabs[0].next_error_position(0), None);
        assert_eq!(app.tabs[0].prev_error_position(1), Some(0));
        assert_eq!(app.tabs[0].next_warning_position(0), Some(2));
    }

    #[tokio::test]
    async fn test_advance_file_watches_updates_visible_indices() {
        let mut app = make_app(&[]).await;

        let (tx, state) = make_watch_state(b"INFO start\nERROR bad\nWARN careful\n");
        app.tabs[0].stream.watch = Some(state);

        tx.send(()).unwrap();
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
            scan_fingerprint: Vec::new(),
            scan_line_count: 0,
            scan_raw_mode: false,
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
        app.tabs[0].filter.handle = Some(handle);
        app.advance_filter_computation();
        assert_eq!(
            app.tabs[0].filter.visible_indices,
            VisibleLines::Filtered(vec![0, 2])
        );
        assert!(
            app.tabs[0].filter.handle.is_some(),
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
        app.tabs[0].filter.handle = Some(handle);
        app.advance_filter_computation();
        assert_eq!(
            app.tabs[0].filter.visible_indices,
            VisibleLines::Filtered(vec![0, 1, 2, 3])
        );
        assert_eq!(app.tabs[0].filter.match_counts, vec![4]);
        assert!(
            app.tabs[0].filter.handle.is_none(),
            "handle should be cleared after last chunk"
        );
    }

    #[tokio::test]
    async fn test_advance_filter_computation_scroll_clamped_on_intermediate() {
        let mut app = make_app(&["a", "b", "c"]).await;
        app.tabs[0].scroll.scroll_offset = 100;
        let (handle, _tx) = make_filter_handle_with_chunks(vec![super::super::FilterChunk {
            visible: vec![0],
            filter_match_counts: None,
            is_last: false,
            progress: 0.3,
        }]);
        app.tabs[0].filter.handle = Some(handle);
        app.advance_filter_computation();
        assert!(
            app.tabs[0].scroll.scroll_offset
                <= app.tabs[0].filter.visible_indices.len().saturating_sub(1),
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
        app.tabs[0].filter.handle = Some(handle);
        app.advance_filter_computation();
        // Line index 3 is at position 3 in the combined visible vec [0,1,2,3].
        assert_eq!(app.tabs[0].scroll.scroll_offset, 3);
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
            scan_fingerprint: Vec::new(),
            scan_line_count: 0,
            scan_raw_mode: false,
        };
        app.tabs[0].filter.handle = Some(handle);
        app.advance_filter_computation();
        assert!(
            app.tabs[0].filter.handle.is_none(),
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
            precomputed_text_counts: None,
        }));
        drop(progress_tx);

        app.tabs[0].load_state = Some(super::FileLoadState {
            path: abs_path,
            progress_rx,
            result_rx,
            total_bytes: 18,
            on_complete: LoadContext::ReplaceInitialTab,
            cancel: Arc::new(AtomicBool::new(false)),
        });

        app.advance_file_load().await;

        assert!(
            app.tabs[0].filter.handle.is_none(),
            "filter_handle must be None when filtering_enabled=false was restored"
        );
        assert!(
            matches!(app.tabs[0].filter.visible_indices, VisibleLines::All(_)),
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
        let temp_file = tempfile::NamedTempFile::new().unwrap();
        let (tx, rx) = tokio::sync::watch::channel(());
        let conn: super::super::StreamConnection = (rx, temp_file);

        let (result_tx, result_rx) = tokio::sync::mpsc::channel(1);
        result_tx.send(Ok(conn)).await.unwrap();

        app.tabs[0].stream.retry = Some(StreamRetryState {
            attempt: 3,
            last_error: "connection refused".to_string(),
            retry_rx: Some(result_rx),
            connected: false,
            connect: make_dummy_connect_fn(),
        });
        app.tabs[0].interaction.command_error = Some("connection failed".to_string());

        app.advance_stream_retries();

        let retry = app.tabs[0]
            .stream
            .retry
            .as_ref()
            .expect("retry state should be kept");
        assert!(retry.connected, "retry should be marked connected");
        assert!(retry.retry_rx.is_none(), "pending rx should be cleared");
        assert_eq!(retry.attempt, 3, "attempt count should be preserved");
        assert!(
            app.tabs[0].interaction.command_error.is_none(),
            "error should be cleared"
        );
        assert!(
            app.tabs[0].stream.watch.is_some(),
            "watch_state should be set"
        );
        assert_eq!(
            app.tabs[0].file_reader.line_count(),
            0,
            "file_reader should be reset to empty on reconnect"
        );
        assert!(
            app.tabs[0].display.format.is_none(),
            "detected_format should be reset on reconnect"
        );

        tx.send(()).unwrap();
        assert!(
            app.tabs[0]
                .stream
                .watch
                .as_mut()
                .unwrap()
                .snapshot_rx
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

        app.tabs[0].stream.retry = Some(StreamRetryState {
            attempt: 1,
            last_error: "old error".to_string(),
            retry_rx: Some(result_rx),
            connected: false,
            connect: make_dummy_connect_fn(),
        });

        app.advance_stream_retries();

        let retry = app.tabs[0].stream.retry.as_ref().unwrap();
        assert_eq!(retry.attempt, 2, "attempt should be incremented");
        assert_eq!(retry.last_error, "connection refused");
        assert!(retry.retry_rx.is_some(), "new retry should be scheduled");
        assert!(
            app.tabs[0]
                .interaction
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

        app.tabs[0].stream.retry = Some(StreamRetryState {
            attempt: 1,
            last_error: "waiting".to_string(),
            retry_rx: Some(result_rx),
            connected: false,
            connect: make_dummy_connect_fn(),
        });

        app.advance_stream_retries();

        let retry = app.tabs[0].stream.retry.as_ref().unwrap();
        assert_eq!(retry.attempt, 1, "attempt should not change while pending");
    }

    #[tokio::test]
    async fn test_disconnect_triggers_retry_for_dlt() {
        let mut app = make_app(&["line"]).await;

        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, Some("dlt://192.168.1.1:3490".to_string())).await;
        let file_reader = FileReader::from_bytes(vec![]);
        let mut tab = TabState::new(file_reader, log_manager, "dlt:test".to_string());

        let (tx, state) = make_watch_state(b"");
        tab.stream.watch = Some(state);
        drop(tx);

        app.tabs.push(tab);
        let tab_idx = app.tabs.len() - 1;

        app.advance_file_watches();

        assert!(app.tabs[tab_idx].stream.watch.is_none());
        assert!(app.tabs[tab_idx].stream.retry.is_some());
        assert_eq!(app.tabs[tab_idx].stream.retry.as_ref().unwrap().attempt, 1);
        assert!(app.tabs[tab_idx].interaction.command_error.is_some());
    }

    #[tokio::test]
    async fn test_disconnect_triggers_retry_for_docker() {
        let mut app = make_app(&["line"]).await;

        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, Some("docker:mycontainer".to_string())).await;
        let file_reader = FileReader::from_bytes(vec![]);
        let mut tab = TabState::new(file_reader, log_manager, "docker:mycontainer".to_string());

        let (tx, state) = make_watch_state(b"");
        tab.stream.watch = Some(state);
        drop(tx);

        app.tabs.push(tab);
        let tab_idx = app.tabs.len() - 1;

        app.advance_file_watches();

        assert!(app.tabs[tab_idx].stream.watch.is_none());
        assert!(app.tabs[tab_idx].stream.retry.is_some());
        assert_eq!(app.tabs[tab_idx].stream.retry.as_ref().unwrap().attempt, 1);
        assert!(app.tabs[tab_idx].interaction.command_error.is_some());
    }

    #[tokio::test]
    async fn test_disconnect_after_retry_preserves_backoff_count() {
        // Simulates: retry#3 succeeds → connection established → drops again immediately.
        // The new retry attempt must start at #4 (not reset to #1).
        let mut app = make_app(&["line"]).await;

        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, Some("docker:mycontainer".to_string())).await;
        let file_reader = FileReader::from_bytes(vec![]);
        let mut tab = TabState::new(file_reader, log_manager, "docker:mycontainer".to_string());

        // Simulate the state after retry #3 succeeded: connected=true, attempt=3.
        let (tx, state) = make_watch_state(b"");
        tab.stream.watch = Some(state);
        tab.stream.retry = Some(StreamRetryState {
            attempt: 3,
            last_error: String::new(),
            retry_rx: None,
            connected: true,
            connect: make_dummy_connect_fn(),
        });
        drop(tx); // drop sender so has_changed() returns Err

        app.tabs.push(tab);
        let tab_idx = app.tabs.len() - 1;

        app.advance_file_watches();

        let retry = app.tabs[tab_idx].stream.retry.as_ref().unwrap();
        assert!(!retry.connected, "should be back in retry mode");
        assert_eq!(
            retry.attempt, 4,
            "attempt should continue from 3, not reset to 1"
        );
    }

    #[tokio::test]
    async fn test_restore_dlt_tab_uses_non_blocking_retry() {
        let mut app = make_app(&["line"]).await;
        app.restore_dlt_tab("dlt://192.168.1.1:3490").await;

        let tab_idx = app.tabs.len() - 1;
        assert!(
            app.tabs[tab_idx].stream.retry.is_some(),
            "stream_retry should be set immediately without blocking"
        );
        assert_eq!(app.tabs[tab_idx].stream.retry.as_ref().unwrap().attempt, 1);
        assert!(app.tabs[tab_idx].stream.watch.is_none());
    }

    #[tokio::test]
    async fn test_restore_docker_tab_uses_non_blocking_retry() {
        let mut app = make_app(&["line"]).await;
        app.restore_docker_tab("docker:mycontainer").await;

        let tab_idx = app.tabs.len() - 1;
        assert!(
            app.tabs[tab_idx].stream.retry.is_some(),
            "stream_retry should be set immediately without blocking"
        );
        assert_eq!(app.tabs[tab_idx].stream.retry.as_ref().unwrap().attempt, 1);
        assert!(app.tabs[tab_idx].stream.watch.is_none());
    }

    // ── Journalctl JSON default hidden fields ─────────────────────────────────

    #[tokio::test]
    async fn test_journalctl_json_hidden_fields_applied_on_stdin() {
        // Simulate `journalctl --user -o json | logana`
        let mut app = make_app(&[]).await;
        assert!(app.tabs[0].display.hidden_fields.is_empty());

        let jctl_line = b"{\"__CURSOR\":\"s=abc\",\"__REALTIME_TIMESTAMP\":\"1699999999000000\",\"_BOOT_ID\":\"abc\",\"PRIORITY\":\"6\",\"_HOSTNAME\":\"myhost\",\"SYSLOG_IDENTIFIER\":\"sshd\",\"_PID\":\"1234\",\"_TRANSPORT\":\"journal\",\"MESSAGE\":\"Accepted password\"}\n";
        let mut data = Vec::new();
        for _ in 0..25 {
            data.extend_from_slice(jctl_line);
        }

        let f = make_stdin_file(&data);
        app.update_stdin_tab(f.path()).await;

        assert!(
            !app.tabs[0].display.hidden_fields.is_empty(),
            "hidden_fields should be non-empty for journalctl JSON stdin"
        );
        assert!(
            app.tabs[0].display.hidden_fields.contains("__CURSOR"),
            "expected __CURSOR to be hidden"
        );
        assert!(
            !app.tabs[0].display.hidden_fields.contains("MESSAGE"),
            "MESSAGE should remain visible"
        );
    }

    #[tokio::test]
    async fn test_journalctl_json_hidden_fields_applied_on_load() {
        // Simulate startup: empty initial tab, then full journalctl JSON loaded
        // via ReplaceInitialTab (the normal file-open path).
        let mut app = make_app(&[]).await;
        assert!(app.tabs[0].display.hidden_fields.is_empty());

        let jctl_line = br#"{"__CURSOR":"s=abc","__REALTIME_TIMESTAMP":"1699999999000000","__MONOTONIC_TIMESTAMP":"123","_BOOT_ID":"abc","PRIORITY":"6","_HOSTNAME":"myhost","SYSLOG_IDENTIFIER":"sshd","_PID":"1234","_UID":"1000","_COMM":"sshd","_TRANSPORT":"journal","MESSAGE":"Accepted password"}"#;
        let mut data = Vec::new();
        for _ in 0..25 {
            data.extend_from_slice(jctl_line);
            data.push(b'\n');
        }
        let fr = FileReader::from_bytes(data);

        let (progress_tx, progress_rx) = tokio::sync::watch::channel(1.0_f64);
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        let _ = result_tx.send(Ok(FileLoadResult {
            reader: fr,
            precomputed_visible: None,
            precomputed_text_counts: None,
        }));
        drop(progress_tx);

        app.tabs[0].load_state = Some(super::FileLoadState {
            path: "journalctl.json".to_string(),
            progress_rx,
            result_rx,
            total_bytes: 200,
            on_complete: LoadContext::ReplaceInitialTab,
            cancel: Arc::new(AtomicBool::new(false)),
        });

        app.advance_file_load().await;

        assert!(
            !app.tabs[0].display.hidden_fields.is_empty(),
            "hidden_fields should be non-empty for journalctl JSON"
        );
        assert!(
            app.tabs[0].display.hidden_fields.contains("__CURSOR"),
            "expected __CURSOR to be hidden"
        );
        assert!(
            app.tabs[0].display.hidden_fields.contains("_TRANSPORT"),
            "expected _TRANSPORT to be hidden"
        );
        assert!(
            !app.tabs[0].display.hidden_fields.contains("MESSAGE"),
            "MESSAGE should remain visible"
        );
        assert!(
            !app.tabs[0].display.hidden_fields.contains("_HOSTNAME"),
            "_HOSTNAME should remain visible"
        );
    }

    #[tokio::test]
    async fn test_open_file_journalctl_json_hidden_fields() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let jctl_line = b"{\"__CURSOR\":\"s=abc\",\"__REALTIME_TIMESTAMP\":\"1699999999000000\",\"__MONOTONIC_TIMESTAMP\":\"123\",\"_BOOT_ID\":\"abc\",\"PRIORITY\":\"6\",\"_HOSTNAME\":\"myhost\",\"SYSLOG_IDENTIFIER\":\"sshd\",\"_PID\":\"1234\",\"_UID\":\"1000\",\"_TRANSPORT\":\"journal\",\"MESSAGE\":\"Accepted\"}\n";
        let mut data = Vec::new();
        for _ in 0..25 {
            data.extend_from_slice(jctl_line);
        }
        std::fs::write(tmp.path(), &data).unwrap();

        let mut app = make_app(&[]).await;
        app.open_file(tmp.path().to_str().unwrap()).await.unwrap();
        let tab_idx = app.tabs.len() - 1;

        assert!(
            !app.tabs[tab_idx].display.hidden_fields.is_empty(),
            "hidden_fields should be non-empty for journalctl JSON file"
        );
        assert!(
            app.tabs[tab_idx].display.hidden_fields.contains("__CURSOR"),
            "expected __CURSOR to be hidden"
        );
        assert!(
            !app.tabs[tab_idx].display.hidden_fields.contains("MESSAGE"),
            "MESSAGE should remain visible"
        );
    }

    #[tokio::test]
    async fn test_startup_journalctl_json_full_flow() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let jctl_line = b"{\"__CURSOR\":\"s=abc\",\"__REALTIME_TIMESTAMP\":\"1699999999000000\",\"__MONOTONIC_TIMESTAMP\":\"123\",\"_BOOT_ID\":\"abc\",\"PRIORITY\":\"6\",\"_HOSTNAME\":\"myhost\",\"SYSLOG_IDENTIFIER\":\"sshd\",\"_PID\":\"1234\",\"_UID\":\"1000\",\"_TRANSPORT\":\"journal\",\"MESSAGE\":\"Accepted\"}\n";
        let mut data = Vec::new();
        for _ in 0..25 {
            data.extend_from_slice(jctl_line);
        }
        std::fs::write(tmp.path(), &data).unwrap();

        let path = tmp.path().to_str().unwrap().to_string();
        let mut app = make_app(&[]).await;

        // Simulate CLI startup: begin_file_load with ReplaceInitialTab loads the
        // preview synchronously (calls detect_and_apply_format) then spawns the
        // background full-file load.
        app.begin_file_load(path.clone(), LoadContext::ReplaceInitialTab, None, false)
            .await;

        // After begin_file_load, preview has been loaded and detect_and_apply_format called.
        assert!(
            !app.tabs[0].display.hidden_fields.is_empty(),
            "hidden_fields should be set from preview"
        );
        assert!(app.tabs[0].display.hidden_fields.contains("__CURSOR"));

        // Poll until the background full-file load completes.
        for _ in 0..100 {
            if app.tabs[0].load_state.is_none() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            app.advance_file_load().await;
        }
        assert!(
            app.tabs[0].load_state.is_none(),
            "file load should complete"
        );

        // After full load, hidden_fields must still be set.
        assert!(
            !app.tabs[0].display.hidden_fields.is_empty(),
            "hidden_fields should survive the full load"
        );
        assert!(app.tabs[0].display.hidden_fields.contains("__CURSOR"));
        assert!(!app.tabs[0].display.hidden_fields.contains("MESSAGE"));

        // The rendered text must NOT contain hidden field names.
        let text = app.tabs[0].get_display_text(0);
        assert!(
            !text.contains("__CURSOR"),
            "rendered line should not contain __CURSOR: {text}"
        );
        assert!(
            !text.contains("_TRANSPORT"),
            "rendered line should not contain _TRANSPORT: {text}"
        );
        // The message must be visible.
        assert!(
            text.contains("Accepted"),
            "rendered line must contain the message: {text}"
        );
    }

    #[tokio::test]
    async fn test_advance_filter_computation_saves_cached_scan_result() {
        use crate::filters::FilterType;
        let mut app = make_app(&["error line", "info line", "error again"]).await;
        app.tabs[0]
            .log_manager
            .add_filter_with_color("error".to_string(), FilterType::Include, None, None, true)
            .await;
        app.tabs[0].begin_filter_refresh();
        assert!(app.tabs[0].filter.handle.is_some());

        // Drain until completion via advance_filter_computation.
        loop {
            app.advance_filter_computation();
            if app.tabs[0].filter.handle.is_none() {
                break;
            }
            tokio::task::yield_now().await;
        }

        let cached = app.tabs[0]
            .filter
            .cached_scan
            .as_ref()
            .expect("cached_scan_result must be set after scan completes");
        assert_eq!(cached.line_count, 3);
        assert!(!cached.filter_fingerprint.is_empty());
        assert_eq!(cached.raw_mode, false);
    }

    #[tokio::test]
    async fn test_advance_file_watches_otlp_json_shows_lines() {
        let otlp_lines = concat!(
            r#"{"timeUnixNano":"1700046000000000000","severityNumber":9,"severityText":"INFO","body":{"stringValue":"hello from test"},"service.name":"my-svc"}"#,
            "\n",
            r#"{"timeUnixNano":"1700046001000000000","severityNumber":17,"severityText":"ERROR","body":{"stringValue":"something failed"},"service.name":"my-svc"}"#,
            "\n",
        );

        let mut app = make_app(&[]).await;
        let (tx, state) = make_watch_state(otlp_lines.as_bytes());
        app.tabs[0].stream.watch = Some(state);

        tx.send(()).unwrap();
        app.advance_file_watches();

        let tab = &app.tabs[0];
        assert!(
            tab.filter.visible_indices.len() > 0,
            "OTLP lines should be visible"
        );
        assert!(
            tab.display.format.is_some(),
            "OTLP format should be detected"
        );
        assert_eq!(
            tab.display.format.as_deref().map(|p| p.name()),
            Some("otlp"),
            "format should be OTLP not plain JSON"
        );
    }

    #[tokio::test]
    async fn test_continue_session_restore_docker_source() {
        let mut app = make_app(&[]).await;
        let mut queue = VecDeque::new();
        queue.push_back("docker:mycontainer".to_string());
        app.continue_session_restore(queue, 1, 0).await;
        let tab_idx = app.tabs.len() - 1;
        assert!(app.tabs[tab_idx].title.contains("docker:mycontainer"));
        assert!(app.tabs[tab_idx].stream.retry.is_some());
    }

    #[tokio::test]
    async fn test_continue_session_restore_dlt_source() {
        let mut app = make_app(&[]).await;
        let mut queue = VecDeque::new();
        queue.push_back("dlt://192.168.1.1:3490".to_string());
        app.continue_session_restore(queue, 1, 0).await;
        let tab_idx = app.tabs.len() - 1;
        assert!(app.tabs[tab_idx].stream.retry.is_some());
    }

    #[tokio::test]
    async fn test_continue_session_restore_otlp_source() {
        let mut app = make_app(&[]).await;
        let mut queue = VecDeque::new();
        queue.push_back("otlp://4318".to_string());
        app.continue_session_restore(queue, 1, 0).await;
        let tab_idx = app.tabs.len() - 1;
        assert!(app.tabs[tab_idx].stream.retry.is_some());
    }

    #[tokio::test]
    async fn test_continue_session_restore_otlp_grpc_source() {
        let mut app = make_app(&[]).await;
        let mut queue = VecDeque::new();
        queue.push_back("otlp-grpc://4317".to_string());
        app.continue_session_restore(queue, 1, 0).await;
        let tab_idx = app.tabs.len() - 1;
        assert!(app.tabs[tab_idx].stream.retry.is_some());
    }

    #[tokio::test]
    async fn test_restore_otlp_tab_sets_retry() {
        let mut app = make_app(&["line"]).await;
        app.restore_otlp_tab("otlp://4318").await;
        let tab_idx = app.tabs.len() - 1;
        assert!(app.tabs[tab_idx].stream.retry.is_some());
        assert!(app.tabs[tab_idx].stream.watch.is_none());
    }

    #[tokio::test]
    async fn test_restore_otlp_grpc_tab_sets_retry() {
        let mut app = make_app(&["line"]).await;
        app.restore_otlp_grpc_tab("otlp-grpc://4317").await;
        let tab_idx = app.tabs.len() - 1;
        assert!(app.tabs[tab_idx].stream.retry.is_some());
        assert!(app.tabs[tab_idx].stream.watch.is_none());
    }

    #[tokio::test]
    async fn test_open_dlt_stream_error_sets_retry() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let mut app = make_app(&["line"]).await;
        app.open_dlt_stream("127.0.0.1".to_string(), port, "test-device".to_string())
            .await;
        let tab_idx = app.tabs.len() - 1;
        assert!(
            app.tabs[tab_idx].stream.retry.is_some() || app.tabs[tab_idx].stream.watch.is_some(),
            "tab should have retry or watch state"
        );
        assert!(app.tabs[tab_idx].title.contains("dlt:test-device"));
    }

    #[tokio::test]
    async fn test_advance_search_no_handle_is_noop() {
        let mut app = make_app(&["line one", "line two"]).await;
        app.tabs[0].search.handle = None;
        app.advance_search();
        assert!(app.tabs[0].search.handle.is_none());
    }

    #[tokio::test]
    async fn test_advance_search_accumulates_results() {
        use crate::search::SearchResult;
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;

        let mut app = make_app(&["hello", "world", "hello again"]).await;

        let (result_tx, result_rx) = tokio::sync::mpsc::channel::<Vec<SearchResult>>(16);
        let (_progress_tx, progress_rx) = tokio::sync::watch::channel(0.5_f64);

        let results = vec![SearchResult {
            line_idx: 0,
            matches: vec![],
        }];
        result_tx.try_send(results).unwrap();

        app.tabs[0].search.handle = Some(SearchHandle {
            result_rx,
            cancel: Arc::new(AtomicBool::new(false)),
            progress_rx,
            pattern: "hello".to_string(),
            forward: true,
            navigate: false,
        });

        app.advance_search();

        assert_eq!(app.tabs[0].search.query.get_results().len(), 1);
        assert!(app.tabs[0].search.handle.is_some(), "channel still open");
    }

    #[tokio::test]
    async fn test_advance_search_done_clears_handle() {
        use crate::search::SearchResult;
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;

        let mut app = make_app(&["hello", "world"]).await;

        let (result_tx, result_rx) = tokio::sync::mpsc::channel::<Vec<SearchResult>>(16);
        let (_progress_tx, progress_rx) = tokio::sync::watch::channel(1.0_f64);

        drop(result_tx);

        app.tabs[0].search.handle = Some(SearchHandle {
            result_rx,
            cancel: Arc::new(AtomicBool::new(false)),
            progress_rx,
            pattern: "hello".to_string(),
            forward: true,
            navigate: false,
        });

        app.advance_search();

        assert!(
            app.tabs[0].search.handle.is_none(),
            "handle should be cleared when sender drops"
        );
    }

    #[tokio::test]
    async fn test_advance_search_navigate_scrolls_to_match() {
        use crate::search::SearchResult;
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;

        let mut app = make_app(&["no match", "hello match", "no match"]).await;
        app.tabs[0].filter.visible_indices =
            VisibleLines::All(app.tabs[0].file_reader.line_count());
        app.tabs[0].scroll.scroll_offset = 0;

        let (result_tx, result_rx) = tokio::sync::mpsc::channel::<Vec<SearchResult>>(16);
        let (_progress_tx, progress_rx) = tokio::sync::watch::channel(1.0_f64);

        result_tx
            .try_send(vec![SearchResult {
                line_idx: 1,
                matches: vec![],
            }])
            .unwrap();
        drop(result_tx);

        app.tabs[0].search.handle = Some(SearchHandle {
            result_rx,
            cancel: Arc::new(AtomicBool::new(false)),
            progress_rx,
            pattern: "hello".to_string(),
            forward: true,
            navigate: true,
        });

        app.advance_search();

        assert!(app.tabs[0].search.handle.is_none());
        assert_eq!(app.tabs[0].scroll.scroll_offset, 1);
    }

    #[test]
    fn test_connect_fn_for_source_none() {
        assert!(connect_fn_for_source(None).is_none());
    }

    #[test]
    fn test_connect_fn_for_source_dlt_with_port() {
        assert!(connect_fn_for_source(Some("dlt://192.168.1.1:3490")).is_some());
    }

    #[test]
    fn test_connect_fn_for_source_dlt_no_port() {
        assert!(connect_fn_for_source(Some("dlt://192.168.1.1")).is_some());
    }

    #[test]
    fn test_connect_fn_for_source_docker() {
        assert!(connect_fn_for_source(Some("docker:mycontainer")).is_some());
    }

    #[test]
    fn test_connect_fn_for_source_otlp() {
        assert!(connect_fn_for_source(Some("otlp://4318")).is_some());
    }

    #[test]
    fn test_connect_fn_for_source_otlp_grpc() {
        assert!(connect_fn_for_source(Some("otlp-grpc://4317")).is_some());
    }

    #[test]
    fn test_connect_fn_for_source_unknown_returns_none() {
        assert!(connect_fn_for_source(Some("/var/log/app.log")).is_none());
    }

    #[tokio::test]
    async fn test_advance_file_watches_otlp_grpc_shows_lines() {
        use crate::ingestion::spawn_otlp_grpc_receiver;
        use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
        use opentelemetry_proto::tonic::collector::logs::v1::logs_service_client::LogsServiceClient;
        use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue, any_value::Value};
        use opentelemetry_proto::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
        use opentelemetry_proto::tonic::resource::v1::Resource;

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let (rx, tmp) = spawn_otlp_grpc_receiver(port).await.unwrap();

        let mut app = make_app(&[]).await;
        let state = super::watch_state_from_connection((rx, tmp));
        app.tabs[0].stream.watch = Some(state);

        let endpoint = format!("http://127.0.0.1:{port}");
        let mut client = LogsServiceClient::connect(endpoint).await.unwrap();

        let request = ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                resource: Some(Resource {
                    attributes: vec![KeyValue {
                        key: "service.name".into(),
                        value: Some(AnyValue {
                            value: Some(Value::StringValue("ui-grpc-svc".into())),
                        }),
                    }],
                    dropped_attributes_count: 0,
                    entity_refs: vec![],
                }),
                scope_logs: vec![ScopeLogs {
                    log_records: vec![LogRecord {
                        time_unix_nano: 1_700_046_000_000_000_000,
                        severity_number: 9,
                        body: Some(AnyValue {
                            value: Some(Value::StringValue("grpc ui test".into())),
                        }),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                schema_url: String::new(),
            }],
        };
        client.export(request).await.unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        app.advance_file_watches();

        let tab = &app.tabs[0];
        assert!(
            tab.filter.visible_indices.len() > 0,
            "gRPC lines should be visible"
        );
    }

    #[tokio::test]
    async fn test_open_file_restore_policy_always_applies_context() {
        use crate::config::RestoreSessionPolicy;
        use crate::db::{FileContext, FileContextStore};
        use std::collections::HashSet;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"line1\nline2\nline3\n").unwrap();
        let abs_path = std::fs::canonicalize(tmp.path())
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let mut app = make_app(&[]).await;
        app.restore_file_policy = RestoreSessionPolicy::Always;

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

        app.open_file(&abs_path).await.unwrap();
        let tab_idx = app.tabs.len() - 1;
        assert!(app.tabs[tab_idx].display.show_keys);
    }

    #[tokio::test]
    async fn test_open_file_restore_policy_never_ignores_context() {
        use crate::config::RestoreSessionPolicy;
        use crate::db::{FileContext, FileContextStore};
        use std::collections::HashSet;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"line1\nline2\n").unwrap();
        let abs_path = std::fs::canonicalize(tmp.path())
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let mut app = make_app(&[]).await;
        app.restore_file_policy = RestoreSessionPolicy::Never;

        let ctx = FileContext {
            source_file: abs_path.clone(),
            scroll_offset: 1,
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

        app.open_file(&abs_path).await.unwrap();
        let tab_idx = app.tabs.len() - 1;
        assert!(!app.tabs[tab_idx].display.show_keys);
    }

    #[tokio::test]
    async fn test_open_file_restore_policy_ask_shows_confirm_mode() {
        use crate::config::RestoreSessionPolicy;
        use crate::db::{FileContext, FileContextStore};
        use crate::mode::app_mode::ModeRenderState;
        use std::collections::HashSet;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"line1\nline2\n").unwrap();
        let abs_path = std::fs::canonicalize(tmp.path())
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let mut app = make_app(&[]).await;
        app.restore_file_policy = RestoreSessionPolicy::Ask;

        let ctx = FileContext {
            source_file: abs_path.clone(),
            scroll_offset: 0,
            search_query: String::new(),
            level_colors_disabled: HashSet::new(),
            horizontal_scroll: 0,
            marked_lines: vec![],
            file_hash: None,
            comments: vec![],
            show_keys: false,
            raw_mode: false,
            sidebar_width: 30,
            hidden_fields: HashSet::new(),
            field_layout_columns: None,
            filtering_enabled: true,
        };
        app.db.save_file_context(&ctx).await.unwrap();

        app.open_file(&abs_path).await.unwrap();
        let tab_idx = app.tabs.len() - 1;
        assert!(matches!(
            app.tabs[tab_idx].interaction.mode.render_state(),
            ModeRenderState::ConfirmRestore
        ));
    }

    #[tokio::test]
    async fn test_open_otlp_stream_opens_tab() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let mut app = make_app(&["line"]).await;
        app.open_otlp_stream(port).await;
        assert_eq!(app.tabs.len(), 2);
        let tab_idx = app.tabs.len() - 1;
        assert!(
            app.tabs[tab_idx].stream.watch.is_some() || app.tabs[tab_idx].stream.retry.is_some()
        );
    }

    #[tokio::test]
    async fn test_skip_or_fail_load_replace_tab() {
        let mut app = make_app(&["existing"]).await;
        let fr = FileReader::from_bytes(vec![]);
        let lm = LogManager::new(app.db.clone(), None).await;
        let tab = TabState::new(fr, lm, "preview".to_string());
        app.tabs.push(tab);
        assert_eq!(app.tabs.len(), 2);

        app.skip_or_fail_load(LoadContext::ReplaceTab { tab_idx: 1 })
            .await;
        assert_eq!(app.tabs.len(), 1);
    }
}
