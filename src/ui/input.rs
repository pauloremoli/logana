use std::time::Instant;

use crossterm::event::{KeyCode, KeyModifiers};

use super::App;
use super::KeyResult;
use crate::config::RestoreSessionPolicy;
use crate::db::SettingsKey;
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
                let h = self.tabs[self.active_tab].scroll.visible_height;
                self.mouse_scroll(-((h / 2).max(1) as i32));
            }
            MouseEventKind::ScrollDown => {
                let h = self.tabs[self.active_tab].scroll.visible_height;
                self.mouse_scroll((h / 2).max(1) as i32);
            }
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
            MouseEventKind::Drag(MouseButton::Left) => {
                if self.input.scrollbar_dragging {
                    let scroll_pos = {
                        let tab = &self.tabs[self.active_tab];
                        self.input.hit_test_scrollbar(event.column, event.row, tab)
                    };
                    if let Some(pos) = scroll_pos {
                        self.tabs[self.active_tab].scroll.scroll_offset = pos;
                    }
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
            self.tabs[self.active_tab].interaction.mode = Box::new(FilterManagementMode {
                selected_filter_index: idx,
            });
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

    async fn handle_open_files(&mut self, paths: Vec<String>) {
        for path in paths {
            if let Err(e) = self.open_file(&path).await {
                self.tabs[self.active_tab].interaction.command_error = Some(e);
                break;
            }
        }
        self.remove_empty_placeholder();
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
