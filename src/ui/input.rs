use std::time::Instant;

use crossterm::event::{KeyCode, KeyModifiers};

use super::App;
use super::KeyResult;
use crate::config::RestoreSessionPolicy;
use crate::db::SettingsKey;
use crate::mode::app_mode::ModeRenderState;
use crate::mode::command_mode::CommandMode;
use crate::mode::filter_mode::FilterManagementMode;
use crate::mode::normal_mode::NormalMode;

use super::app::DOUBLE_CLICK_MS;

impl App {
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
                        tab.interaction.mode = Box::new(FilterManagementMode::new(idx));
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

    pub async fn handle_key_event(&mut self, key_code: KeyCode) {
        self.handle_key_event_with_modifiers(key_code, KeyModifiers::NONE)
            .await;
    }

    pub async fn handle_key_event_with_modifiers(
        &mut self,
        key_code: KeyCode,
        modifiers: KeyModifiers,
    ) {
        self.session.startup_warnings.clear();
        let tab = &mut self.tabs[self.active_tab];
        let mode = std::mem::replace(&mut tab.interaction.mode, Box::new(NormalMode::default()));
        let (next_mode, result) = mode.handle_key(tab, key_code, modifiers).await;
        tab.interaction.mode = next_mode;
        self.dispatch_key_result(result, key_code, modifiers).await;
    }

    pub(super) async fn handle_mouse_event(&mut self, event: crossterm::event::MouseEvent) {
        use crossterm::event::{MouseButton, MouseEventKind};
        match event.kind {
            MouseEventKind::ScrollUp => {
                if self.is_over_sidebar(event.column, event.row) {
                    self.filter_sidebar_scroll(-1);
                } else {
                    let h = self.tabs[self.active_tab].scroll.visible_height;
                    self.mouse_scroll(-((h / 2).max(1) as i32));
                }
            }
            MouseEventKind::ScrollDown => {
                if self.is_over_sidebar(event.column, event.row) {
                    self.filter_sidebar_scroll(1);
                } else {
                    let h = self.tabs[self.active_tab].scroll.visible_height;
                    self.mouse_scroll((h / 2).max(1) as i32);
                }
            }
            // Touchpad two-finger horizontal swipe — crossterm reports this
            // as ScrollLeft/ScrollRight (distinct from a mouse wheel's
            // ScrollUp/ScrollDown).
            MouseEventKind::ScrollLeft => self.mouse_scroll_horizontal(-4),
            MouseEventKind::ScrollRight => self.mouse_scroll_horizontal(4),
            MouseEventKind::Down(MouseButton::Left) => {
                let hit_scrollbar = {
                    let tab = &self.tabs[self.active_tab];
                    self.input
                        .hit_test_scrollbar(event.column, event.row, tab)
                        .is_some()
                };
                if hit_scrollbar {
                    self.input.scrollbar_dragging = true;
                }
                self.handle_left_down(event.column, event.row).await;
            }
            MouseEventKind::Drag(MouseButton::Left) if self.input.scrollbar_dragging => {
                let scroll_pos = {
                    let tab = &self.tabs[self.active_tab];
                    self.input.hit_test_scrollbar(event.column, event.row, tab)
                };
                if let Some(pos) = scroll_pos {
                    self.tabs[self.active_tab].scroll.scroll_offset = pos;
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.input.scrollbar_dragging = false;
            }
            _ => {}
        }
    }

    pub(super) async fn handle_left_down(&mut self, col: u16, row: u16) {
        let now = Instant::now();
        if let Some((t, c, r)) = self.input.last_click.take() {
            if t.elapsed().as_millis() < DOUBLE_CLICK_MS && c == col && r == row {
                self.handle_double_click(col, row);
                return;
            }
            self.handle_left_click(c, r).await;
        }
        let hit_log_panel = {
            let tab = &self.tabs[self.active_tab];
            self.input.hit_test_log_panel(col, row, tab).is_some()
        };
        if hit_log_panel {
            self.input.last_click = Some((now, col, row));
        } else {
            self.handle_left_click(col, row).await;
        }
    }

    pub(super) async fn flush_pending_click(&mut self) {
        if let Some((t, c, r)) = self.input.last_click {
            if t.elapsed().as_millis() < DOUBLE_CLICK_MS {
                return;
            }
            self.input.last_click = None;
            self.handle_left_click(c, r).await;
        }
    }

    pub(super) fn handle_double_click(&mut self, col: u16, row: u16) {
        use crate::mode::visual_char_mode::{VisualMode, display_line_text, word_bounds_at};
        let (visible_idx_opt, char_col) = {
            let tab = &self.tabs[self.active_tab];
            (
                self.input.hit_test_log_panel(col, row, tab),
                self.input.col_to_char_offset(col, tab),
            )
        };
        let Some(visible_idx) = visible_idx_opt else {
            return;
        };
        self.tabs[self.active_tab].scroll.scroll_offset = visible_idx;
        let line_text = display_line_text(&self.tabs[self.active_tab]);
        if let Some((word_start, word_end)) = word_bounds_at(&line_text, char_col) {
            let mut mode = VisualMode::new(line_text);
            mode.anchor_col = Some(word_start);
            mode.cursor_col = word_end;
            self.tabs[self.active_tab].interaction.mode = Box::new(mode);
        }
    }

    fn is_over_sidebar(&self, col: u16, row: u16) -> bool {
        self.input
            .sidebar_area
            .is_some_and(|a| a.contains(ratatui::layout::Position::new(col, row)))
    }

    /// Move the filter-sidebar selection by `delta` (clamped to bounds) and
    /// switch into `FilterManagementMode`, mirroring keyboard j/k navigation.
    fn filter_sidebar_scroll(&mut self, delta: i32) {
        let tab = &mut self.tabs[self.active_tab];
        let num_filters = tab.log_manager.get_filters().len();
        if num_filters == 0 {
            return;
        }
        let current = match tab.interaction.mode.render_state() {
            ModeRenderState::FilterManagement { selected_index, .. } => selected_index,
            _ => tab.filter.filter_context.unwrap_or(0),
        };
        let new_idx = if delta < 0 {
            current.saturating_sub(delta.unsigned_abs() as usize)
        } else {
            (current + delta as usize).min(num_filters - 1)
        };
        tab.interaction.mode = Box::new(FilterManagementMode::new(new_idx));
    }

    pub(super) fn mouse_scroll(&mut self, delta: i32) {
        let tab = &mut self.tabs[self.active_tab];
        let max_scroll = tab.filter.visible_indices.len().saturating_sub(1);
        if delta < 0 {
            tab.stream.tail_mode = false;
            tab.scroll.scroll_offset = tab
                .scroll
                .scroll_offset
                .saturating_sub(delta.unsigned_abs() as usize);
        } else {
            let new_offset = (tab.scroll.scroll_offset + delta as usize).min(max_scroll);
            tab.scroll.scroll_offset = new_offset;
            if new_offset >= max_scroll {
                tab.stream.tail_mode = true;
            }
        }
        if matches!(
            tab.interaction.mode.render_state(),
            crate::mode::app_mode::ModeRenderState::Visual { .. }
        ) {
            let mut mode =
                std::mem::replace(&mut tab.interaction.mode, Box::new(NormalMode::default()));
            mode.on_scroll_line_change(tab);
            tab.interaction.mode = mode;
        }
    }

    /// Pans the log panel horizontally by `delta` columns — the touchpad
    /// equivalent of `mouse_scroll`. No-op while wrapped (there's nothing to
    /// pan: every line already fits within `visible_width`). Clamped to
    /// `max_line_width`, both written each render pass by `prepare_log_panel`.
    pub(super) fn mouse_scroll_horizontal(&mut self, delta: i32) {
        let tab = &mut self.tabs[self.active_tab];
        if tab.display.wrap {
            return;
        }
        let max_scroll = tab
            .scroll
            .max_line_width
            .saturating_sub(tab.scroll.visible_width);
        if delta < 0 {
            tab.scroll.horizontal_scroll = tab
                .scroll
                .horizontal_scroll
                .saturating_sub(delta.unsigned_abs() as usize);
        } else {
            tab.scroll.horizontal_scroll =
                (tab.scroll.horizontal_scroll + delta as usize).min(max_scroll);
        }
    }

    pub(super) async fn handle_left_click(&mut self, col: u16, row: u16) {
        let (scroll_pos, filter_idx, visible_idx) = {
            let tab = &self.tabs[self.active_tab];
            (
                self.input.hit_test_scrollbar(col, row, tab),
                self.input.hit_test_sidebar(col, row, tab),
                self.input.hit_test_log_panel(col, row, tab),
            )
        };
        if let Some(pos) = scroll_pos {
            self.tabs[self.active_tab].scroll.scroll_offset = pos;
            return;
        }
        if let Some(idx) = filter_idx {
            self.tabs[self.active_tab].interaction.mode = Box::new(FilterManagementMode::new(idx));
            return;
        }
        if let Some(idx) = visible_idx {
            self.tabs[self.active_tab].scroll.scroll_offset = idx;
            self.tabs[self.active_tab].interaction.mode = Box::new(NormalMode::default());
        }
    }

    async fn save_app_bool(&self, key: SettingsKey, value: bool) {
        self.session.save_app_bool(key, value).await;
    }

    async fn handle_toggle_mode_bar(&mut self) {
        self.display.show_mode_bar = !self.display.show_mode_bar;
        for tab in &mut self.tabs {
            tab.display.show_mode_bar = self.display.show_mode_bar;
        }
        self.save_app_bool(SettingsKey::ShowModeBar, self.display.show_mode_bar)
            .await;
    }

    async fn handle_toggle_sidebar(&mut self) {
        self.display.show_sidebar = !self.display.show_sidebar;
        for tab in &mut self.tabs {
            tab.display.show_sidebar = self.display.show_sidebar;
        }
        self.save_app_bool(SettingsKey::ShowSidebar, self.display.show_sidebar)
            .await;
    }

    async fn handle_toggle_borders(&mut self) {
        self.display.show_borders_default = !self.display.show_borders_default;
        for tab in &mut self.tabs {
            tab.display.show_borders = self.display.show_borders_default;
        }
        self.save_app_bool(SettingsKey::ShowBorders, self.display.show_borders_default)
            .await;
    }

    async fn handle_toggle_wrap(&mut self) {
        self.display.wrap = !self.display.wrap;
        for tab in &mut self.tabs {
            tab.display.wrap = self.display.wrap;
        }
        self.save_app_bool(SettingsKey::Wrap, self.display.wrap)
            .await;
    }

    async fn handle_toggle_line_numbers(&mut self) {
        self.display.show_line_numbers = !self.display.show_line_numbers;
        for tab in &mut self.tabs {
            tab.display.show_line_numbers = self.display.show_line_numbers;
        }
        self.save_app_bool(SettingsKey::ShowLineNumbers, self.display.show_line_numbers)
            .await;
    }

    async fn handle_apply_value_colors(&mut self, disabled: std::collections::HashSet<String>) {
        self.theme.value_colors.disabled = disabled;
        for tab in &mut self.tabs {
            tab.cache.render_gen = tab.cache.render_gen.wrapping_add(1);
            tab.cache.render_line.clear();
        }
    }

    async fn handle_set_default_filter_file(&mut self, format: String, path: Option<String>) {
        match path {
            Some(p) => {
                self.default_filter_files.insert(format, p);
            }
            None => {
                self.default_filter_files.remove(&format);
            }
        }
        self.persist_default_filter_files().await;
    }

    async fn handle_open_files(&mut self, paths: Vec<String>) {
        for path in paths {
            if let Err(e) = self.open_file(&path).await {
                self.tabs[self.active_tab].interaction.command_error = Some(e);
                break;
            }
        }
        self.remove_empty_placeholder();
    }

    /// Applies a directory-sourced archive picker (`:open`'d a directory —
    /// reuses the archive picker's tree/checkbox/merge-mark UI, see
    /// `ArchiveTree::list_directory_tree`). Most files are `disk_path: true`
    /// (plain files anywhere under the directory, including inside
    /// subdirectories) — ticked ones are opened directly via the same path
    /// `handle_open_files` uses, merge-marked ones are read directly into a
    /// temp copy, in the background so a large file can't stall the UI, then
    /// merged exactly like an archive picker's merge-marked files are. A
    /// file found inside an archive discovered along the way is
    /// `disk_path: false` — its `full_path` is only meaningful relative to
    /// that archive's own bytes, so it's routed through the same
    /// extract-to-temp path `apply_archive_picker` uses instead.
    pub(super) async fn apply_directory_picker(
        &mut self,
        source_path: String,
        tree: crate::ingestion::ArchiveTree,
    ) {
        let selected: Vec<&crate::ingestion::ArchiveNode> = tree
            .nodes
            .iter()
            .filter(|n| matches!(n.kind, crate::ingestion::NodeKind::File) && n.selected)
            .collect();
        let disk_selected_paths: Vec<String> = selected
            .iter()
            .filter(|n| n.disk_path)
            .map(|n| n.full_path.clone())
            .collect();
        self.handle_open_files(disk_selected_paths).await;

        let archived_selected_ids: Vec<crate::ingestion::NodeId> = selected
            .iter()
            .filter(|n| !n.disk_path)
            .map(|n| n.id)
            .collect();
        if !archived_selected_ids.is_empty() {
            self.begin_directory_archived_extraction(
                source_path.clone(),
                tree.clone(),
                archived_selected_ids,
            )
            .await;
        }

        let merge_marked: Vec<(String, crate::ingestion::NodeId, bool)> = tree
            .nodes
            .iter()
            .filter(|n| matches!(n.kind, crate::ingestion::NodeKind::File) && n.merge_marked)
            .map(|n| (n.name.clone(), n.id, n.disk_path))
            .collect();
        if merge_marked.is_empty() {
            return;
        }

        // The destination merged tab is created right away (and its own
        // progress channel started) so it's visible with real progress from
        // the moment the merge begins — for big files, reading/copying them
        // below is the slow part, and without this the tab wouldn't appear
        // at all until it finished.
        let labels: Vec<String> = merge_marked.iter().map(|(name, ..)| name.clone()).collect();
        let total = labels.len();
        let tab_idx = self.create_pending_merged_tab(labels).await;

        let (progress_tx, progress_rx) = tokio::sync::watch::channel(0usize);
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        tokio::task::spawn_blocking(move || {
            let mut sources = Vec::with_capacity(merge_marked.len());
            for (i, (name, node_id, disk_path)) in merge_marked.into_iter().enumerate() {
                let path = tree.nodes[node_id].full_path.clone();
                // Copied into temp rather than read from `path` directly —
                // like an archive's merge-marked entries, which are always
                // extracted to temp — so the merged tab this feeds into is
                // self-contained and never needs to re-open the original
                // directory file (see `TabState::merge_source_temps`).
                let temp_file = match tempfile::NamedTempFile::new() {
                    Ok(t) => t,
                    Err(e) => {
                        let _ = result_tx
                            .send(Err(format!("Failed to create temp file for '{path}': {e}")));
                        return;
                    }
                };
                let write_result = if disk_path {
                    std::fs::copy(&path, temp_file.path())
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                } else {
                    crate::ingestion::archive_tree::resolve_node_bytes(&tree, node_id, &source_path)
                        .and_then(|bytes| {
                            std::fs::write(temp_file.path(), bytes).map_err(|e| e.to_string())
                        })
                };
                if let Err(e) = write_result {
                    let _ = result_tx.send(Err(format!("Failed to read '{path}': {e}")));
                    return;
                }
                let temp_path = temp_file.path().to_string_lossy().into_owned();
                let reader = match crate::ingestion::FileReader::new(&temp_path) {
                    Ok(r) => r,
                    Err(e) => {
                        let _ = result_tx.send(Err(format!("Failed to open '{path}': {e}")));
                        return;
                    }
                };
                let detected = crate::ingestion::format_detect::detect_format_for_reader(&reader);
                sources.push(crate::ingestion::MergeMarkedSource {
                    label: name,
                    reader,
                    detected,
                    temp_file,
                });
                let _ = progress_tx.send(i + 1);
            }
            let unrecognized: Vec<&str> = sources
                .iter()
                .filter(|s| s.detected.format.is_none())
                .map(|s| s.label.as_str())
                .collect();
            let result = if unrecognized.is_empty() {
                Ok(sources)
            } else {
                Err(format!(
                    "Cannot merge \u{2014} unrecognized log format for: {}",
                    unrecognized.join(", ")
                ))
            };
            let _ = result_tx.send(result);
        });

        self.pending_directory_merge = Some(crate::ui::DirectoryMergeState {
            tab_idx,
            total,
            progress_rx,
            result_rx,
        });
    }

    /// Extracts `ids` (files found inside an archive discovered inside the
    /// directory) to temp copies in the background, then feeds them through
    /// the exact same `pending_archive`/`poll_archive_extraction` machinery
    /// `apply_archive_picker` uses to open each as its own tab — `disk_path`
    /// files never reach here, so `merge_result` is always `None`; a
    /// directory picker's merge-marked handling lives entirely in
    /// `pending_directory_merge` instead.
    async fn begin_directory_archived_extraction(
        &mut self,
        source_path: String,
        tree: crate::ingestion::ArchiveTree,
        ids: Vec<crate::ingestion::NodeId>,
    ) {
        let (progress_tx, progress_rx) =
            tokio::sync::watch::channel(crate::ingestion::ArchiveExtractionProgress {
                file_index: 0,
                fraction: 0.0,
            });
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        self.decompression_message = Some("Extracting selected files\u{2026}".to_string());

        tokio::task::spawn_blocking(move || {
            let selected_files =
                crate::ingestion::archive_tree::extract_ids(&source_path, &tree, &ids, progress_tx);
            let _ = result_tx.send(crate::ui::ArchivePickerApplyResult {
                selected_files,
                merge_result: None,
            });
        });

        self.pending_archive = Some(crate::ui::ArchiveExtractionState {
            progress_rx,
            result_rx,
            merge_tab_idx: None,
            merge_progress_rx: None,
            merge_total: 0,
        });
    }

    /// Poll the pending directory-picker merge each frame. When the
    /// background read+detect finishes, feeds the merged tab created by
    /// `apply_directory_picker` (or removes it, on error) and clears
    /// `pending_directory_merge`.
    pub async fn poll_directory_merge(&mut self) {
        let Some(state) = &mut self.pending_directory_merge else {
            return;
        };

        let progress_done = *state.progress_rx.borrow();
        if state.tab_idx < self.tabs.len() {
            self.tabs[state.tab_idx].set_notification(format!(
                "Reading\u{2026} {}/{} files",
                progress_done.min(state.total),
                state.total
            ));
        }

        match state.result_rx.try_recv() {
            Ok(Ok(sources)) => {
                let tab_idx = state.tab_idx;
                self.pending_directory_merge = None;
                let inputs = Self::merge_inputs_from_extracted(sources);
                self.start_merge_build_streaming(tab_idx, inputs).await;
            }
            Ok(Err(e)) => {
                let tab_idx = state.tab_idx;
                self.pending_directory_merge = None;
                self.remove_pending_merged_tab(tab_idx);
                self.tabs[self.active_tab].set_notification(e);
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                let tab_idx = state.tab_idx;
                self.pending_directory_merge = None;
                self.remove_pending_merged_tab(tab_idx);
            }
        }
    }

    pub(super) async fn dispatch_key_result(
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
            KeyResult::ApplyValueColors(disabled) => self.handle_apply_value_colors(disabled).await,
            KeyResult::ApplyLevelColors(disabled) => {
                self.tabs[self.active_tab].display.level_colors_disabled = disabled;
            }
            KeyResult::CopyToClipboard(text) => self.copy_to_clipboard(text),
            KeyResult::ToggleModeBar => self.handle_toggle_mode_bar().await,
            KeyResult::ToggleSidebar => self.handle_toggle_sidebar().await,
            KeyResult::ToggleBorders => self.handle_toggle_borders().await,
            KeyResult::ToggleWrap => self.handle_toggle_wrap().await,
            KeyResult::ToggleLineNumbers => self.handle_toggle_line_numbers().await,
            KeyResult::OpenFiles(paths) => self.handle_open_files(paths).await,
            KeyResult::AlwaysRestoreFile(_) => {
                self.session
                    .set_restore_file_policy(RestoreSessionPolicy::Always)
                    .await;
            }
            KeyResult::NeverRestoreFile => {
                self.session
                    .set_restore_file_policy(RestoreSessionPolicy::Never)
                    .await;
            }
            KeyResult::AlwaysRestoreSession(files) => {
                self.session
                    .set_restore_policy(RestoreSessionPolicy::Always)
                    .await;
                self.restore_session(files).await;
            }
            KeyResult::NeverRestoreSession => {
                self.session
                    .set_restore_policy(RestoreSessionPolicy::Never)
                    .await;
            }
            KeyResult::OpenMergeSelect => self.handle_open_merge_select(),
            KeyResult::OpenMergedView { source_tab_indices } => {
                self.open_merge_tab(source_tab_indices).await;
            }
            KeyResult::ExportWithFooter {
                path,
                template_name,
                footer_fields,
            } => {
                self.cmd_export_with_footer(path, template_name, footer_fields);
            }
            KeyResult::ApplyArchivePicker { source_path, tree } => {
                if std::path::Path::new(&source_path).is_dir() {
                    self.apply_directory_picker(source_path, tree).await;
                } else {
                    self.apply_archive_picker(source_path, tree).await;
                }
            }
            KeyResult::ExpandArchiveNode { node_id } => {
                self.begin_archive_node_expand(node_id).await;
            }
            KeyResult::SetDefaultFilterFile { format, path } => {
                self.handle_set_default_filter_file(format, path).await;
            }
        }
    }

    pub(super) fn copy_to_clipboard(&mut self, text: String) {
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
}

#[cfg(test)]
mod tests {
    use crate::config::Keybindings;
    use crate::db::{Database, LogManager};
    use crate::ingestion::{ArchiveNode, ArchiveTree, FileReader, NodeKind};
    use crate::theme::Theme;
    use crate::ui::App;
    use std::sync::Arc;

    async fn make_app() -> App {
        // Non-empty starting tab so `remove_empty_placeholder` (called after
        // opening real files) doesn't remove it out from under our tab-count
        // assertions below.
        let file_reader = FileReader::from_bytes(b"initial line\n".to_vec());
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        App::builder(
            log_manager,
            file_reader,
            Theme::default(),
            Arc::new(Keybindings::default()),
        )
        .build()
        .await
    }

    fn file_node(
        id: usize,
        name: &str,
        full_path: &str,
        selected: bool,
        merge_marked: bool,
    ) -> ArchiveNode {
        ArchiveNode {
            id,
            parent: None,
            name: name.to_string(),
            full_path: full_path.to_string(),
            depth: 0,
            kind: NodeKind::File,
            selected,
            merge_marked,
            cached_bytes: None,
            collapsed: false,
            disk_path: true,
        }
    }

    /// Waits for `app.pending_directory_merge` to clear.
    async fn drain_pending_directory_merge(app: &mut App) {
        for _ in 0..100 {
            app.poll_directory_merge().await;
            if app.pending_directory_merge.is_none() {
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        }
    }

    /// Waits for every in-progress background merge build (the tab created
    /// by `apply_directory_picker`'s merge path fills in on a background
    /// thread — see `App::start_merge_build_streaming`) to finish.
    async fn drain_pending_merge_builds(app: &mut App) {
        for _ in 0..100 {
            app.poll_merge_builds();
            if app.pending_merge_builds.is_empty() {
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn test_apply_directory_picker_opens_ticked_files_as_separate_tabs() {
        let mut app = make_app().await;
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.log");
        let b = tmp.path().join("b.log");
        std::fs::write(&a, b"hello").unwrap();
        std::fs::write(&b, b"world").unwrap();

        let tree = ArchiveTree {
            nodes: vec![
                file_node(0, "a.log", a.to_str().unwrap(), true, false),
                file_node(1, "b.log", b.to_str().unwrap(), true, false),
            ],
            roots: vec![0, 1],
        };
        let initial_tabs = app.tabs.len();
        app.apply_directory_picker(String::new(), tree).await;

        assert_eq!(app.tabs.len(), initial_tabs + 2);
        let titles: Vec<&str> = app.tabs.iter().map(|t| t.title.as_str()).collect();
        assert!(titles.contains(&"a.log"));
        assert!(titles.contains(&"b.log"));
    }

    #[tokio::test]
    async fn test_apply_directory_picker_ignores_unticked_files() {
        let mut app = make_app().await;
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.log");
        let b = tmp.path().join("b.log");
        std::fs::write(&a, b"hello").unwrap();
        std::fs::write(&b, b"world").unwrap();

        let tree = ArchiveTree {
            nodes: vec![
                file_node(0, "a.log", a.to_str().unwrap(), true, false),
                file_node(1, "b.log", b.to_str().unwrap(), false, false),
            ],
            roots: vec![0, 1],
        };
        let initial_tabs = app.tabs.len();
        app.apply_directory_picker(String::new(), tree).await;

        assert_eq!(app.tabs.len(), initial_tabs + 1);
        assert_eq!(app.tabs.last().unwrap().title, "a.log");
    }

    #[tokio::test]
    async fn test_apply_directory_picker_merges_merge_marked_files_into_one_tab() {
        let mut app = make_app().await;
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.log");
        let b = tmp.path().join("b.log");
        std::fs::write(&a, "2024-01-01 10:00:00 INFO line from a\n").unwrap();
        std::fs::write(&b, "2024-01-01 09:00:00 INFO line from b\n").unwrap();

        let tree = ArchiveTree {
            nodes: vec![
                file_node(0, "a.log", a.to_str().unwrap(), false, true),
                file_node(1, "b.log", b.to_str().unwrap(), false, true),
            ],
            roots: vec![0, 1],
        };
        let initial_tabs = app.tabs.len();
        app.apply_directory_picker(String::new(), tree).await;
        drain_pending_directory_merge(&mut app).await;
        drain_pending_merge_builds(&mut app).await;

        assert_eq!(
            app.tabs.len(),
            initial_tabs + 1,
            "exactly one merged tab, no separate tabs for merge-marked files"
        );
        let merged = app.tabs.last().unwrap();
        assert!(merged.merged.is_some());
        // Sorted by timestamp: b (09:00) before a (10:00).
        assert_eq!(
            merged.file_reader.get_line(0),
            b"2024-01-01 09:00:00 INFO line from b"
        );
        assert_eq!(
            merged.file_reader.get_line(1),
            b"2024-01-01 10:00:00 INFO line from a"
        );
    }

    /// The merged tab must appear the instant `apply_directory_picker`
    /// returns — before the (potentially slow, for big files) background
    /// read/copy phase has even started polling — with real source
    /// filenames already shown, not just once that phase finishes.
    /// A tab removed anywhere else (a placeholder cleanup, `:close-tab`, a
    /// different merge finishing) while THIS merge's index build is still
    /// running in the background must not desync `pending_merge_builds`'
    /// tracked tab index — otherwise the next update lands on the wrong
    /// tab, corrupting it (or, if that tab's line count doesn't match,
    /// crashing on the next render).
    #[tokio::test]
    async fn test_removing_an_earlier_tab_keeps_a_still_building_merge_pointed_at_the_right_tab() {
        let mut app = make_app().await;

        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.log");
        let b = tmp.path().join("b.log");
        std::fs::write(&a, "2024-01-01 10:00:00 INFO line from a\n").unwrap();
        std::fs::write(&b, "2024-01-01 09:00:00 INFO line from b\n").unwrap();
        let tree = ArchiveTree {
            nodes: vec![
                file_node(0, "a.log", a.to_str().unwrap(), false, true),
                file_node(1, "b.log", b.to_str().unwrap(), false, true),
            ],
            roots: vec![0, 1],
        };
        app.apply_directory_picker(String::new(), tree).await;
        drain_pending_directory_merge(&mut app).await;

        // Phase 2 (the merge index build) is pending — intentionally not
        // drained yet, so it's still "in flight" when the removal below
        // happens.
        assert_eq!(app.pending_merge_builds.len(), 1);
        let merge_tab_idx_before = app.pending_merge_builds[0].tab_idx;
        assert!(merge_tab_idx_before > 0);

        // Some unrelated tab before the still-building merge tab gets
        // removed (a placeholder cleanup, `:close-tab`, ...) — must go
        // through `remove_tab_at` so `pending_merge_builds` stays correct.
        app.remove_tab_at(0);
        assert_eq!(
            app.pending_merge_builds[0].tab_idx,
            merge_tab_idx_before - 1,
            "the still-building merge's tracked tab index must shift down \
             after an earlier tab is removed"
        );

        drain_pending_merge_builds(&mut app).await;

        let merged_tab = &app.tabs[merge_tab_idx_before - 1];
        assert!(merged_tab.merged.is_some(), "must still be the merged tab");
        assert_eq!(merged_tab.file_reader.line_count(), 2);
        assert_eq!(
            merged_tab.file_reader.get_line(0),
            b"2024-01-01 09:00:00 INFO line from b"
        );
    }

    #[tokio::test]
    async fn test_apply_directory_picker_merge_tab_appears_before_reading_starts() {
        let mut app = make_app().await;
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.log");
        let b = tmp.path().join("b.log");
        std::fs::write(&a, "2024-01-01 10:00:00 INFO line from a\n").unwrap();
        std::fs::write(&b, "2024-01-01 09:00:00 INFO line from b\n").unwrap();

        let tree = ArchiveTree {
            nodes: vec![
                file_node(0, "a.log", a.to_str().unwrap(), false, true),
                file_node(1, "b.log", b.to_str().unwrap(), false, true),
            ],
            roots: vec![0, 1],
        };
        let initial_tabs = app.tabs.len();
        app.apply_directory_picker(String::new(), tree).await;

        assert_eq!(
            app.tabs.len(),
            initial_tabs + 1,
            "the destination tab must exist immediately, before any reading happened"
        );
        let merged_tab = app.tabs.last().unwrap();
        let merged = merged_tab.merged.as_ref().unwrap();
        assert_eq!(merged.source_labels, vec!["a.log", "b.log"]);
        assert_eq!(merged_tab.file_reader.line_count(), 0);
        assert_eq!(app.active_tab, app.tabs.len() - 1);
    }

    /// Same "must not freeze" guarantee as the archive-picker path — the
    /// merged tab is visible before its background build has folded any
    /// source in, and only fills in once `poll_merge_builds` runs.
    #[tokio::test]
    async fn test_apply_directory_picker_merge_tab_appears_before_background_build_completes() {
        let mut app = make_app().await;
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.log");
        let b = tmp.path().join("b.log");
        std::fs::write(&a, "2024-01-01 10:00:00 INFO line from a\n").unwrap();
        std::fs::write(&b, "2024-01-01 09:00:00 INFO line from b\n").unwrap();

        let tree = ArchiveTree {
            nodes: vec![
                file_node(0, "a.log", a.to_str().unwrap(), false, true),
                file_node(1, "b.log", b.to_str().unwrap(), false, true),
            ],
            roots: vec![0, 1],
        };
        app.apply_directory_picker(String::new(), tree).await;
        drain_pending_directory_merge(&mut app).await;

        assert_eq!(
            app.pending_merge_builds.len(),
            1,
            "a background merge build must be pending right after apply"
        );
        let merged_tab = app.tabs.last().unwrap();
        assert!(merged_tab.merged.is_some());
        assert_eq!(
            merged_tab.file_reader.line_count(),
            0,
            "no lines are folded in yet — the apply call must return without blocking"
        );

        drain_pending_merge_builds(&mut app).await;

        let merged_tab = app.tabs.last().unwrap();
        assert_eq!(merged_tab.file_reader.line_count(), 2);
        assert_eq!(merged_tab.merged.as_ref().unwrap().building, None);
    }

    /// While the background read/copy phase is still running, the
    /// destination tab must show a progress notification reflecting how
    /// many of the merge-marked files have been read so far — proof the
    /// feedback is visible right away for a merge that's slow to read, not
    /// just once it's done.
    #[tokio::test]
    async fn test_apply_directory_picker_merge_shows_reading_progress() {
        let mut app = make_app().await;
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.log");
        std::fs::write(&a, "2024-01-01 10:00:00 INFO line from a\n").unwrap();

        let tree = ArchiveTree {
            nodes: vec![file_node(0, "a.log", a.to_str().unwrap(), false, true)],
            roots: vec![0],
        };
        app.apply_directory_picker(String::new(), tree).await;
        let tab_idx = app.pending_directory_merge.as_ref().unwrap().tab_idx;
        // Poll a few times rather than once — the background read is a real
        // OS thread and may not have run yet on the very first poll.
        let mut notification = String::new();
        for _ in 0..50 {
            app.poll_directory_merge().await;
            notification = app.tabs[tab_idx]
                .interaction
                .notification
                .clone()
                .unwrap_or_default();
            if notification.contains('/') {
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
        }
        assert!(
            notification.starts_with("Reading") && notification.contains("/1"),
            "expected reading progress in the notification, got: {notification:?}"
        );
    }

    /// If every merge-marked file turns out unrecognized, the pending
    /// destination tab created up front must be removed again rather than
    /// left behind stuck showing "building" forever with nothing to fill it.
    #[tokio::test]
    async fn test_apply_directory_picker_merge_removes_pending_tab_on_unrecognized_format() {
        let mut app = make_app().await;
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.log");
        std::fs::write(&a, b"just some random bytes with no structure\n").unwrap();

        let tree = ArchiveTree {
            nodes: vec![file_node(0, "a.log", a.to_str().unwrap(), false, true)],
            roots: vec![0],
        };
        let initial_tabs = app.tabs.len();
        app.apply_directory_picker(String::new(), tree).await;
        assert_eq!(
            app.tabs.len(),
            initial_tabs + 1,
            "the pending tab exists while reading is in progress"
        );

        drain_pending_directory_merge(&mut app).await;

        assert_eq!(
            app.tabs.len(),
            initial_tabs,
            "the pending tab must be removed once the merge fails, not left dangling"
        );
        let notification = app.tabs[app.active_tab]
            .interaction
            .notification
            .clone()
            .unwrap_or_default();
        assert!(notification.contains("a.log"), "{notification:?}");
    }

    /// A directory merge must be self-contained: each merge-marked source is
    /// copied into its own retained temp file (not read live from the
    /// original directory path), and the fully-merged result is also saved
    /// to one temp file — so the merged tab keeps working even if the
    /// original directory is deleted out from under it, and shows the
    /// `[TEMP]` marker to make clear its data isn't the permanent original.
    #[tokio::test]
    async fn test_apply_directory_picker_merge_is_self_contained_in_temp() {
        let mut app = make_app().await;
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.log");
        let b = tmp.path().join("b.log");
        std::fs::write(&a, "2024-01-01 10:00:00 INFO line from a\n").unwrap();
        std::fs::write(&b, "2024-01-01 09:00:00 INFO line from b\n").unwrap();

        let tree = ArchiveTree {
            nodes: vec![
                file_node(0, "a.log", a.to_str().unwrap(), false, true),
                file_node(1, "b.log", b.to_str().unwrap(), false, true),
            ],
            roots: vec![0, 1],
        };
        app.apply_directory_picker(String::new(), tree).await;
        drain_pending_directory_merge(&mut app).await;
        drain_pending_merge_builds(&mut app).await;

        let merged_tab = app.tabs.last().unwrap();
        assert_eq!(
            merged_tab.merge_source_temps.len(),
            2,
            "each merge-marked source must have its own retained temp copy"
        );
        assert!(
            merged_tab.merged_temp.is_some(),
            "the fully-merged result must be saved to its own temp file"
        );
        assert!(merged_tab.is_temp_backed());

        // The original directory is gone; the merged tab must still be
        // fully readable — it never needs to re-open the originals.
        drop(tmp);
        let merged_tab = app.tabs.last().unwrap();
        assert_eq!(merged_tab.file_reader.line_count(), 2);
        assert_eq!(
            merged_tab.file_reader.get_line(0),
            b"2024-01-01 09:00:00 INFO line from b"
        );
        assert_eq!(
            merged_tab.file_reader.get_line(1),
            b"2024-01-01 10:00:00 INFO line from a"
        );
    }

    #[tokio::test]
    async fn test_apply_directory_picker_mixed_ticked_and_merge_marked() {
        let mut app = make_app().await;
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.log");
        let b = tmp.path().join("b.log");
        let c = tmp.path().join("c.log");
        std::fs::write(&a, "2024-01-01 10:00:00 INFO a\n").unwrap();
        std::fs::write(&b, "2024-01-01 09:00:00 INFO b\n").unwrap();
        std::fs::write(&c, "plain ticked file").unwrap();

        let tree = ArchiveTree {
            nodes: vec![
                file_node(0, "a.log", a.to_str().unwrap(), false, true),
                file_node(1, "b.log", b.to_str().unwrap(), false, true),
                file_node(2, "c.log", c.to_str().unwrap(), true, false),
            ],
            roots: vec![0, 1, 2],
        };
        let initial_tabs = app.tabs.len();
        app.apply_directory_picker(String::new(), tree).await;
        drain_pending_directory_merge(&mut app).await;

        assert_eq!(
            app.tabs.len(),
            initial_tabs + 2,
            "one separate tab for the ticked file, one merged tab for the marked files"
        );
        let titles: Vec<&str> = app.tabs.iter().map(|t| t.title.as_str()).collect();
        assert!(titles.contains(&"c.log"));
        assert!(app.tabs.iter().any(|t| t.merged.is_some()));
    }

    /// Waits for `app.pending_archive` to clear.
    async fn drain_pending_archive(app: &mut App) {
        for _ in 0..100 {
            app.poll_archive_extraction().await;
            if app.pending_archive.is_none() {
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn test_apply_directory_picker_opens_a_ticked_file_inside_a_discovered_archive() {
        let mut app = make_app().await;
        let zip_tmp = crate::ingestion::archive::test_helpers::make_zip(&[(
            "inner.log",
            b"content from inside the zip".as_slice(),
        )]);
        let tmp_dir = tempfile::tempdir().unwrap();
        std::fs::copy(zip_tmp.path(), tmp_dir.path().join("bundle.zip")).unwrap();

        let mut tree =
            crate::ingestion::list_directory_tree(tmp_dir.path().to_str().unwrap()).unwrap();
        let inner_id = tree
            .nodes
            .iter()
            .find(|n| n.name == "inner.log")
            .unwrap()
            .id;
        tree.nodes[inner_id].selected = true;
        assert!(
            !tree.nodes[inner_id].disk_path,
            "the entry lives inside the discovered archive, not directly on disk"
        );

        let initial_tabs = app.tabs.len();
        app.apply_directory_picker(tmp_dir.path().to_str().unwrap().to_string(), tree)
            .await;
        drain_pending_archive(&mut app).await;

        assert_eq!(app.tabs.len(), initial_tabs + 1);
        let titles: Vec<&str> = app.tabs.iter().map(|t| t.title.as_str()).collect();
        assert!(titles.contains(&"inner.log"));
    }

    #[tokio::test]
    async fn test_apply_directory_picker_merges_a_file_inside_a_discovered_archive() {
        let mut app = make_app().await;
        let zip_tmp = crate::ingestion::archive::test_helpers::make_zip(&[(
            "inner.log",
            b"2024-01-01T00:00:00Z hello\n".as_slice(),
        )]);
        let tmp_dir = tempfile::tempdir().unwrap();
        std::fs::copy(zip_tmp.path(), tmp_dir.path().join("bundle.zip")).unwrap();

        let mut tree =
            crate::ingestion::list_directory_tree(tmp_dir.path().to_str().unwrap()).unwrap();
        let inner_id = tree
            .nodes
            .iter()
            .find(|n| n.name == "inner.log")
            .unwrap()
            .id;
        tree.nodes[inner_id].merge_marked = true;

        app.apply_directory_picker(tmp_dir.path().to_str().unwrap().to_string(), tree)
            .await;
        drain_pending_directory_merge(&mut app).await;

        assert!(app.tabs.iter().any(|t| t.merged.is_some()));
    }

    #[tokio::test]
    async fn test_apply_directory_picker_merge_applies_default_filter_for_shared_format() {
        let mut app = make_app().await;
        let dir = tempfile::tempdir().unwrap();
        let filter_path = dir.path().join("f.json");
        std::fs::write(
            &filter_path,
            r#"[{"id":1,"pattern":"error","filter_type":"Include","enabled":true,"color_config":null,"use_regex":false,"ignore_case":false,"group":null}]"#,
        )
        .unwrap();
        app.default_filter_files.insert(
            "common-log".to_string(),
            filter_path.to_str().unwrap().to_string(),
        );

        let a = dir.path().join("a.log");
        let b = dir.path().join("b.log");
        std::fs::write(&a, "2024-01-01 10:00:00 INFO line from a\n").unwrap();
        std::fs::write(&b, "2024-01-01 09:00:00 INFO line from b\n").unwrap();
        let tree = ArchiveTree {
            nodes: vec![
                file_node(0, "a.log", a.to_str().unwrap(), false, true),
                file_node(1, "b.log", b.to_str().unwrap(), false, true),
            ],
            roots: vec![0, 1],
        };

        app.apply_directory_picker(String::new(), tree).await;
        drain_pending_directory_merge(&mut app).await;
        drain_pending_merge_builds(&mut app).await;

        let merged = app.tabs.last().unwrap();
        assert!(merged.merged.is_some());
        assert_eq!(
            merged.log_manager.get_filters().len(),
            1,
            "merged tab should pick up the shared format's default filter file"
        );
        assert_eq!(merged.log_manager.get_filters()[0].pattern, "error");
    }
}
