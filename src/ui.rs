use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    Frame, Terminal,
    prelude::*,
    style::Modifier,
    widgets::{Block, Borders, Paragraph, Wrap},
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use unicode_width::UnicodeWidthStr;

use crate::auto_complete::{
    complete_color, complete_file_path, extract_color_partial, find_command_completions,
    find_matching_command, shell_split,
};
use crate::db::{FileContext, FileContextStore};
use crate::file_reader::FileReader;
use crate::filters::{SEARCH_STYLE_ID, render_line};
use crate::log_manager::LogManager;
use crate::search::Search;
use crate::theme::{Theme, fuzzy_match};
use crate::types::{FilterType, LogLevel};

// ---------------------------------------------------------------------------
// KeyResult
// ---------------------------------------------------------------------------

pub enum KeyResult {
    Handled,
    Ignored,
    ExecuteCommand(String),
}

// ---------------------------------------------------------------------------
// Mode trait
// ---------------------------------------------------------------------------

pub trait Mode: std::fmt::Debug {
    fn handle_key(
        self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult);

    fn status_line(&self) -> &str;

    fn selected_filter_index(&self) -> Option<usize> {
        None
    }
    fn command_state(&self) -> Option<(&str, usize)> {
        None
    }
    fn search_state(&self) -> Option<(&str, bool)> {
        None
    }
    fn needs_input_bar(&self) -> bool {
        false
    }
    fn confirm_restore_context(&self) -> Option<&FileContext> {
        None
    }
}

// ---------------------------------------------------------------------------
// NormalMode
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct NormalMode;

impl Mode for NormalMode {
    fn handle_key(
        self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        // Pass these to the global handler
        if key == KeyCode::Char('q') && modifiers.is_empty() {
            return (self, KeyResult::Ignored);
        }
        if matches!(key, KeyCode::Tab | KeyCode::BackTab) {
            return (self, KeyResult::Ignored);
        }
        if key == KeyCode::Char('w') && modifiers.contains(KeyModifiers::CONTROL) {
            return (self, KeyResult::Ignored);
        }
        if key == KeyCode::Char('t') && modifiers.contains(KeyModifiers::CONTROL) {
            return (self, KeyResult::Ignored);
        }

        // Ctrl+d: half page down
        if key == KeyCode::Char('d') && modifiers.contains(KeyModifiers::CONTROL) {
            let half = (tab.visible_height / 2).max(1);
            tab.scroll_offset = tab.scroll_offset.saturating_add(half);
            tab.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }
        // Ctrl+u: half page up
        if key == KeyCode::Char('u') && modifiers.contains(KeyModifiers::CONTROL) {
            let half = (tab.visible_height / 2).max(1);
            tab.scroll_offset = tab.scroll_offset.saturating_sub(half);
            tab.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }

        match key {
            KeyCode::PageDown => {
                let page = tab.visible_height.max(1);
                tab.scroll_offset = tab.scroll_offset.saturating_add(page);
                tab.g_key_pressed = false;
            }
            KeyCode::PageUp => {
                let page = tab.visible_height.max(1);
                tab.scroll_offset = tab.scroll_offset.saturating_sub(page);
                tab.g_key_pressed = false;
            }
            KeyCode::Char(':') => {
                let history = tab.command_history.clone();
                return (
                    Box::new(CommandMode::with_history(String::new(), 0, history)),
                    KeyResult::Handled,
                );
            }
            KeyCode::Char('f') => {
                return (
                    Box::new(FilterManagementMode {
                        selected_filter_index: 0,
                    }),
                    KeyResult::Handled,
                );
            }
            KeyCode::Char('s') => {
                tab.show_sidebar = !tab.show_sidebar;
                tab.g_key_pressed = false;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                tab.scroll_offset = tab.scroll_offset.saturating_add(1);
                tab.g_key_pressed = false;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                tab.scroll_offset = tab.scroll_offset.saturating_sub(1);
                tab.g_key_pressed = false;
            }
            KeyCode::Char('h') => {
                if !tab.wrap {
                    tab.horizontal_scroll = tab.horizontal_scroll.saturating_sub(1);
                }
                tab.g_key_pressed = false;
            }
            KeyCode::Char('l') => {
                if !tab.wrap {
                    tab.horizontal_scroll = tab.horizontal_scroll.saturating_add(1);
                }
                tab.g_key_pressed = false;
            }
            KeyCode::Char('w') => {
                tab.wrap = !tab.wrap;
                tab.g_key_pressed = false;
            }
            KeyCode::Char('G') => {
                let n = tab.visible_indices.len();
                if n > 0 {
                    tab.scroll_offset = n - 1;
                }
                tab.g_key_pressed = false;
            }
            KeyCode::Char('g') => {
                if tab.g_key_pressed {
                    tab.scroll_offset = 0;
                    tab.g_key_pressed = false;
                } else {
                    tab.g_key_pressed = true;
                }
            }
            KeyCode::Char('m') => {
                if let Some(&line_idx) = tab.visible_indices.get(tab.scroll_offset) {
                    tab.log_manager.toggle_mark(line_idx);
                }
                tab.g_key_pressed = false;
            }
            KeyCode::Char('/') => {
                tab.g_key_pressed = false;
                return (
                    Box::new(SearchMode {
                        input: String::new(),
                        forward: true,
                    }),
                    KeyResult::Handled,
                );
            }
            KeyCode::Char('?') => {
                tab.g_key_pressed = false;
                return (
                    Box::new(SearchMode {
                        input: String::new(),
                        forward: false,
                    }),
                    KeyResult::Handled,
                );
            }
            KeyCode::Char('n') => {
                if let Some(result) = tab.search.next_match() {
                    let line_idx = result.line_idx;
                    tab.scroll_to_line_idx(line_idx);
                }
                tab.g_key_pressed = false;
            }
            KeyCode::Char('N') => {
                if let Some(result) = tab.search.previous_match() {
                    let line_idx = result.line_idx;
                    tab.scroll_to_line_idx(line_idx);
                }
                tab.g_key_pressed = false;
            }
            _ => {
                tab.g_key_pressed = false;
            }
        }
        (self, KeyResult::Handled)
    }

    fn status_line(&self) -> &str {
        "[NORMAL] [q]uit | : => command Mode | [f]ilter mode | [s]idebar | [m]ark Line | / => search | ? => search backward | [n]ext match | N => prev match | PgDn/Ctrl+d PgUp/Ctrl+u | Tab/Shift+Tab => switch tab"
    }
}

// ---------------------------------------------------------------------------
// CommandMode
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct CommandMode {
    pub input: String,
    pub cursor: usize,
    pub history: Vec<String>,
    pub history_index: Option<usize>,
    pub completion_index: Option<usize>,
}

impl CommandMode {
    pub fn with_history(input: String, cursor: usize, history: Vec<String>) -> Self {
        CommandMode {
            input,
            cursor,
            history,
            history_index: None,
            completion_index: None,
        }
    }
}

impl Mode for CommandMode {
    fn handle_key(
        mut self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        _modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        match key {
            KeyCode::Enter => {
                let cmd = self.input.trim().to_string();
                return (Box::new(NormalMode), KeyResult::ExecuteCommand(cmd));
            }
            KeyCode::Esc => {
                tab.filter_context = None;
                tab.editing_filter_id = None;
                return (Box::new(NormalMode), KeyResult::Handled);
            }
            KeyCode::Backspace => {
                if self.cursor > 0 && !self.input.is_empty() {
                    self.input.remove(self.cursor - 1);
                    self.cursor -= 1;
                    self.completion_index = None;
                }
            }
            KeyCode::Char(c) => {
                self.input.insert(self.cursor, c);
                self.cursor += 1;
                tab.command_error = None;
                self.completion_index = None;
                self.history_index = None;
            }
            KeyCode::Left => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                }
            }
            KeyCode::Right => {
                if self.cursor < self.input.len() {
                    self.cursor += 1;
                }
            }
            KeyCode::Up => {
                if self.history.is_empty() {
                    return (self, KeyResult::Handled);
                }
                let new_index = match self.history_index {
                    None => Some(self.history.len() - 1),
                    Some(0) => Some(0),
                    Some(i) => Some(i - 1),
                };
                if let Some(i) = new_index {
                    self.input = self.history[i].clone();
                    self.cursor = self.input.len();
                    self.history_index = Some(i);
                }
            }
            KeyCode::Down => {
                if self.history.is_empty() {
                    return (self, KeyResult::Handled);
                }
                let new_index = match self.history_index {
                    None => return (self, KeyResult::Handled),
                    Some(i) if i + 1 >= self.history.len() => {
                        self.input = String::new();
                        self.cursor = 0;
                        self.history_index = None;
                        return (self, KeyResult::Handled);
                    }
                    Some(i) => Some(i + 1),
                };
                if let Some(i) = new_index {
                    self.input = self.history[i].clone();
                    self.cursor = self.input.len();
                    self.history_index = Some(i);
                }
            }
            KeyCode::Tab => {
                let trimmed = self.input.trim().to_string();

                // Color completion for --fg/--bg arguments
                if let Some(partial) = extract_color_partial(&trimmed) {
                    let completions = complete_color(partial);
                    if !completions.is_empty() {
                        let idx = self.completion_index.unwrap_or(0) % completions.len();
                        let color_name = completions[idx];
                        let prefix = if partial.is_empty() {
                            trimmed.clone()
                        } else {
                            trimmed[..trimmed.len() - partial.len()].to_string()
                        };
                        self.input = format!("{}{}", prefix, color_name);
                        self.cursor = self.input.len();
                        self.completion_index = Some(idx + 1);
                        return (self, KeyResult::Handled);
                    }
                }

                // File path completion
                let file_commands = ["open", "load-filters", "save-filters", "export-marked"];
                let file_cmd = file_commands
                    .iter()
                    .find(|cmd| trimmed.starts_with(&format!("{} ", cmd)));
                if let Some(&cmd) = file_cmd {
                    let partial = trimmed[cmd.len()..].trim_start();
                    let completions = complete_file_path(partial);
                    if !completions.is_empty() {
                        let idx = self.completion_index.unwrap_or(0) % completions.len();
                        let completed = completions[idx].clone();
                        self.input = format!("{} {}", cmd, completed);
                        self.cursor = self.input.len();
                        self.completion_index = Some(idx + 1);
                    }
                    return (self, KeyResult::Handled);
                }

                // set-theme completion with fuzzy match
                if let Some(after_prefix) = trimmed.strip_prefix("set-theme") {
                    let partial = after_prefix.trim_start();
                    let mut themes = Theme::list_available_themes();
                    if !partial.is_empty() {
                        themes.retain(|t| fuzzy_match(partial, t));
                    }
                    if !themes.is_empty() {
                        let idx = self.completion_index.unwrap_or(0) % themes.len();
                        let theme_name = themes[idx].clone();
                        self.input = format!("set-theme {}", theme_name);
                        self.cursor = self.input.len();
                        self.completion_index = Some(idx + 1);
                    }
                    return (self, KeyResult::Handled);
                }

                // Command name completion
                let completions = find_command_completions(&trimmed);
                if !completions.is_empty() {
                    let idx = self.completion_index.unwrap_or(0) % completions.len();
                    self.input = completions[idx].to_string();
                    self.cursor = self.input.len();
                    self.completion_index = Some(idx + 1);
                }
            }
            _ => {}
        }
        (self, KeyResult::Handled)
    }

    fn status_line(&self) -> &str {
        "[COMMAND] filter | exclude | set-color | export-marked | save-filters | load-filters | wrap | set-theme | level-colors | open | close-tab | Esc | Enter"
    }

    fn command_state(&self) -> Option<(&str, usize)> {
        Some((&self.input, self.cursor))
    }

    fn needs_input_bar(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// FilterManagementMode
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct FilterManagementMode {
    pub selected_filter_index: usize,
}

impl Mode for FilterManagementMode {
    fn handle_key(
        self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        _modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        if matches!(key, KeyCode::Tab | KeyCode::BackTab) {
            return (self, KeyResult::Ignored);
        }

        let selected = self.selected_filter_index;

        match key {
            KeyCode::Esc => (Box::new(NormalMode), KeyResult::Handled),
            KeyCode::Up => (
                Box::new(FilterManagementMode {
                    selected_filter_index: selected.saturating_sub(1),
                }),
                KeyResult::Handled,
            ),
            KeyCode::Down => {
                let num_filters = tab.log_manager.get_filters().len();
                let new_idx = if num_filters > 0 {
                    (selected + 1).min(num_filters - 1)
                } else {
                    0
                };
                (
                    Box::new(FilterManagementMode {
                        selected_filter_index: new_idx,
                    }),
                    KeyResult::Handled,
                )
            }
            KeyCode::Char(' ') => {
                let filter_id = tab.log_manager.get_filters().get(selected).map(|f| f.id);
                if let Some(id) = filter_id {
                    tab.log_manager.toggle_filter(id);
                    tab.refresh_visible();
                }
                (
                    Box::new(FilterManagementMode {
                        selected_filter_index: selected,
                    }),
                    KeyResult::Handled,
                )
            }
            KeyCode::Char('d') => {
                let filter_id = tab.log_manager.get_filters().get(selected).map(|f| f.id);
                if let Some(id) = filter_id {
                    tab.log_manager.remove_filter(id);
                    tab.refresh_visible();
                    let remaining_len = tab.log_manager.get_filters().len();
                    let new_idx = if remaining_len > 0 && selected >= remaining_len {
                        remaining_len - 1
                    } else {
                        selected
                    };
                    (
                        Box::new(FilterManagementMode {
                            selected_filter_index: new_idx,
                        }),
                        KeyResult::Handled,
                    )
                } else {
                    (
                        Box::new(FilterManagementMode {
                            selected_filter_index: selected,
                        }),
                        KeyResult::Handled,
                    )
                }
            }
            KeyCode::Char('K') => {
                let filter_id = tab.log_manager.get_filters().get(selected).map(|f| f.id);
                if let Some(id) = filter_id {
                    tab.log_manager.move_filter_up(id);
                    tab.refresh_visible();
                    let new_idx = selected.saturating_sub(1);
                    (
                        Box::new(FilterManagementMode {
                            selected_filter_index: new_idx,
                        }),
                        KeyResult::Handled,
                    )
                } else {
                    (
                        Box::new(FilterManagementMode {
                            selected_filter_index: selected,
                        }),
                        KeyResult::Handled,
                    )
                }
            }
            KeyCode::Char('J') => {
                let filter_id = tab.log_manager.get_filters().get(selected).map(|f| f.id);
                if let Some(id) = filter_id {
                    tab.log_manager.move_filter_down(id);
                    tab.refresh_visible();
                    let total = tab.log_manager.get_filters().len();
                    let new_idx = if selected + 1 < total {
                        selected + 1
                    } else {
                        selected
                    };
                    (
                        Box::new(FilterManagementMode {
                            selected_filter_index: new_idx,
                        }),
                        KeyResult::Handled,
                    )
                } else {
                    (
                        Box::new(FilterManagementMode {
                            selected_filter_index: selected,
                        }),
                        KeyResult::Handled,
                    )
                }
            }
            KeyCode::Char('e') => {
                let filter_info = tab.log_manager.get_filters().get(selected).map(|f| {
                    (
                        f.id,
                        f.filter_type.clone(),
                        f.color_config.clone(),
                        f.pattern.clone(),
                    )
                });
                if let Some((id, ft, cc, pattern)) = filter_info {
                    tab.editing_filter_id = Some(id);
                    tab.filter_context = Some(selected);
                    let mut cmd = if ft == FilterType::Include {
                        String::from("filter")
                    } else {
                        String::from("exclude")
                    };
                    if ft == FilterType::Include {
                        if let Some(cfg) = &cc {
                            if let Some(fg) = cfg.fg {
                                cmd.push_str(&format!(" --fg {:?}", fg));
                            }
                            if let Some(bg) = cfg.bg {
                                cmd.push_str(&format!(" --bg {:?}", bg));
                            }
                            if cfg.match_only {
                                cmd.push_str(" -m");
                            }
                        }
                    }
                    cmd.push(' ');
                    cmd.push_str(&pattern);
                    let len = cmd.len();
                    let history = tab.command_history.clone();
                    (
                        Box::new(CommandMode::with_history(cmd, len, history)),
                        KeyResult::Handled,
                    )
                } else {
                    (
                        Box::new(FilterManagementMode {
                            selected_filter_index: selected,
                        }),
                        KeyResult::Handled,
                    )
                }
            }
            KeyCode::Char('c') => {
                let color_config = tab
                    .log_manager
                    .get_filters()
                    .get(selected)
                    .and_then(|f| f.color_config.clone());
                tab.filter_context = Some(selected);
                let mut cmd = String::from("set-color");
                if let Some(cfg) = color_config {
                    if let Some(fg) = cfg.fg {
                        cmd.push_str(&format!(" --fg {:?}", fg));
                    }
                    if let Some(bg) = cfg.bg {
                        cmd.push_str(&format!(" --bg {:?}", bg));
                    }
                }
                let len = cmd.len();
                let history = tab.command_history.clone();
                (
                    Box::new(CommandMode::with_history(cmd, len, history)),
                    KeyResult::Handled,
                )
            }
            KeyCode::Char('i') => {
                let history = tab.command_history.clone();
                (
                    Box::new(CommandMode::with_history("filter ".to_string(), 7, history)),
                    KeyResult::Handled,
                )
            }
            KeyCode::Char('x') => {
                let history = tab.command_history.clone();
                (
                    Box::new(CommandMode::with_history(
                        "exclude ".to_string(),
                        8,
                        history,
                    )),
                    KeyResult::Handled,
                )
            }
            _ => (
                Box::new(FilterManagementMode {
                    selected_filter_index: selected,
                }),
                KeyResult::Handled,
            ),
        }
    }

    fn status_line(&self) -> &str {
        "[FILTER] [i]nclude | e[x]clude | Space => toggle | [d]elete | [e]dit | set [c]olor | [J/K] move down/up | Esc => normal mode"
    }

    fn selected_filter_index(&self) -> Option<usize> {
        Some(self.selected_filter_index)
    }
}

// ---------------------------------------------------------------------------
// FilterEditMode
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct FilterEditMode {
    pub filter_id: Option<usize>,
    pub filter_input: String,
}

impl Mode for FilterEditMode {
    fn handle_key(
        self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        _modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        if matches!(key, KeyCode::Tab | KeyCode::BackTab) {
            return (self, KeyResult::Ignored);
        }
        match key {
            KeyCode::Enter => {
                if let Some(id) = self.filter_id {
                    tab.log_manager.edit_filter(id, self.filter_input);
                    tab.refresh_visible();
                }
                (
                    Box::new(FilterManagementMode {
                        selected_filter_index: 0,
                    }),
                    KeyResult::Handled,
                )
            }
            KeyCode::Esc => (
                Box::new(FilterManagementMode {
                    selected_filter_index: 0,
                }),
                KeyResult::Handled,
            ),
            KeyCode::Backspace => {
                let mut input = self.filter_input;
                input.pop();
                (
                    Box::new(FilterEditMode {
                        filter_id: self.filter_id,
                        filter_input: input,
                    }),
                    KeyResult::Handled,
                )
            }
            KeyCode::Char(c) => {
                let mut input = self.filter_input;
                input.push(c);
                (
                    Box::new(FilterEditMode {
                        filter_id: self.filter_id,
                        filter_input: input,
                    }),
                    KeyResult::Handled,
                )
            }
            _ => (self, KeyResult::Handled),
        }
    }

    fn status_line(&self) -> &str {
        "[FILTER EDIT] Esc => cancel | Enter => save"
    }
}

// ---------------------------------------------------------------------------
// SearchMode
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct SearchMode {
    pub input: String,
    pub forward: bool,
}

impl Mode for SearchMode {
    fn handle_key(
        mut self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        _modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        if matches!(key, KeyCode::Tab | KeyCode::BackTab) {
            return (self, KeyResult::Ignored);
        }
        match key {
            KeyCode::Enter => {
                let visible = tab.visible_indices.clone();
                let _ = tab.search.search(&self.input, &visible, &tab.file_reader);
                let result = if self.forward {
                    tab.search.next_match()
                } else {
                    tab.search.previous_match()
                };
                if let Some(r) = result {
                    let line_idx = r.line_idx;
                    tab.scroll_to_line_idx(line_idx);
                }
                (Box::new(NormalMode), KeyResult::Handled)
            }
            KeyCode::Esc => {
                self.input.clear();
                (Box::new(NormalMode), KeyResult::Handled)
            }
            KeyCode::Backspace => {
                self.input.pop();
                (self, KeyResult::Handled)
            }
            KeyCode::Char(c) => {
                self.input.push(c);
                (self, KeyResult::Handled)
            }
            _ => (self, KeyResult::Handled),
        }
    }

    fn status_line(&self) -> &str {
        "[SEARCH] Esc => cancel | Enter => search"
    }

    fn search_state(&self) -> Option<(&str, bool)> {
        Some((&self.input, self.forward))
    }

    fn needs_input_bar(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// ConfirmRestoreMode
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct ConfirmRestoreMode {
    pub context: FileContext,
}

impl Mode for ConfirmRestoreMode {
    fn handle_key(
        self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        _modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        match key {
            KeyCode::Char('y') => {
                tab.apply_file_context(&self.context);
                (Box::new(NormalMode), KeyResult::Handled)
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                tab.log_manager.clear_filters();
                tab.log_manager.set_marks(vec![]);
                tab.refresh_visible();
                (Box::new(NormalMode), KeyResult::Handled)
            }
            _ => (self, KeyResult::Handled),
        }
    }

    fn status_line(&self) -> &str {
        "[RESTORE] Restore previous session? [y]es / [n]o"
    }

    fn confirm_restore_context(&self) -> Option<&FileContext> {
        Some(&self.context)
    }
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
    pub filter_context: Option<usize>,
    pub editing_filter_id: Option<usize>,
    pub visible_height: usize,
    pub title: String,
    pub command_history: Vec<String>,
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
            filter_context: None,
            editing_filter_id: None,
            visible_height: 0,
            title,
            command_history: Vec::new(),
        };
        tab.refresh_visible();
        tab
    }

    /// Recompute which file lines are visible under the current filters.
    pub fn refresh_visible(&mut self) {
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
// App
// ---------------------------------------------------------------------------

pub struct App {
    pub tabs: Vec<TabState>,
    pub active_tab: usize,
    pub theme: Theme,
    pub db: Arc<crate::db::Database>,
    pub rt: Arc<tokio::runtime::Runtime>,
    pub should_quit: bool,
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
    pub fn new(log_manager: LogManager, file_reader: FileReader, theme: Theme) -> App {
        let db = log_manager.db.clone();
        let rt = log_manager.rt.clone();

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

        let mut tab = TabState::new(file_reader, log_manager, title);

        // Check for saved context for the initial file
        if let Some(source) = tab.log_manager.source_file() {
            let source = source.to_string();
            if let Ok(Some(ctx)) = rt.block_on(db.load_file_context(&source)) {
                tab.mode = Box::new(ConfirmRestoreMode { context: ctx });
            }
        }

        App {
            tabs: vec![tab],
            active_tab: 0,
            theme,
            db,
            rt,
            should_quit: false,
        }
    }

    pub fn tab(&self) -> &TabState {
        &self.tabs[self.active_tab]
    }

    pub fn tab_mut(&mut self) -> &mut TabState {
        &mut self.tabs[self.active_tab]
    }

    pub fn open_file(&mut self, path: &str) -> Result<(), String> {
        let file_path_obj = std::path::Path::new(path);
        if !file_path_obj.exists() {
            return Err(format!("File '{}' not found.", path));
        }
        if file_path_obj.is_dir() {
            return Err(format!("'{}' is a directory, not a file.", path));
        }

        let file_reader =
            FileReader::new(path).map_err(|e| format!("Failed to read '{}': {}", path, e))?;
        let log_manager = LogManager::new(self.db.clone(), self.rt.clone(), Some(path.to_string()));

        let title = file_path_obj
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path)
            .to_string();

        let mut tab = TabState::new(file_reader, log_manager, title);

        if let Ok(Some(ctx)) = self.rt.block_on(self.db.load_file_context(path)) {
            tab.mode = Box::new(ConfirmRestoreMode { context: ctx });
        }

        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
        Ok(())
    }

    fn save_tab_context(&self, tab: &TabState) {
        if let Some(ctx) = tab.to_file_context() {
            let db = self.db.clone();
            self.rt.spawn(async move {
                let _ = db.save_file_context(&ctx).await;
            });
        }
    }

    fn save_all_contexts(&self) {
        for tab in &self.tabs {
            self.save_tab_context(tab);
        }
    }

    pub fn close_tab(&mut self) -> bool {
        self.save_tab_context(&self.tabs[self.active_tab]);
        if self.tabs.len() <= 1 {
            return true; // signal to quit
        }
        self.tabs.remove(self.active_tab);
        if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        }
        false
    }

    fn handle_global_key(&mut self, key: KeyCode, modifiers: KeyModifiers) {
        let ctrl = modifiers.contains(KeyModifiers::CONTROL);
        match key {
            KeyCode::Char('q') if modifiers.is_empty() => {
                self.save_all_contexts();
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
                if self.close_tab() {
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
    pub fn execute_command_str(&mut self, cmd: String) {
        let result = self.run_command(&cmd);
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

    fn run_command(&mut self, input: &str) -> Result<(), String> {
        use crate::commands::{CommandLine, Commands};
        use clap::Parser;

        let args = CommandLine::try_parse_from(shell_split(input))
            .map_err(|e| format!("Invalid command: {}", e))?;

        match args.command {
            Some(Commands::Filter { pattern, fg, bg, m }) => {
                if let Some(old_id) = self.tabs[self.active_tab].editing_filter_id.take() {
                    self.tabs[self.active_tab].log_manager.remove_filter(old_id);
                }
                self.tabs[self.active_tab]
                    .log_manager
                    .add_filter_with_color(
                        pattern,
                        FilterType::Include,
                        fg.as_deref(),
                        bg.as_deref(),
                        m,
                    );
                self.tabs[self.active_tab].scroll_offset = 0;
                self.tabs[self.active_tab].refresh_visible();
            }
            Some(Commands::Exclude { pattern }) => {
                if let Some(old_id) = self.tabs[self.active_tab].editing_filter_id.take() {
                    self.tabs[self.active_tab].log_manager.remove_filter(old_id);
                }
                self.tabs[self.active_tab]
                    .log_manager
                    .add_filter_with_color(pattern, FilterType::Exclude, None, None, false);
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
                    self.tabs[self.active_tab].log_manager.set_color_config(
                        filter_id,
                        fg.as_deref(),
                        bg.as_deref(),
                        m,
                    );
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
                self.open_file(&path)?;
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
            None => {}
        }
        Ok(())
    }

    pub fn run(&mut self, terminal: &mut Terminal<impl Backend>) -> anyhow::Result<()> {
        let mut last_tick = Instant::now();
        let tick_rate = Duration::from_millis(250);

        loop {
            terminal.draw(|frame| self.ui(frame))?;

            let poll_timeout = tick_rate
                .checked_sub(last_tick.elapsed())
                .unwrap_or_else(|| Duration::from_secs(0));

            if crossterm::event::poll(poll_timeout)?
                && let crossterm::event::Event::Key(key) = crossterm::event::read()?
                && key.kind == crossterm::event::KeyEventKind::Press
            {
                let tab = &mut self.tabs[self.active_tab];
                let mode = std::mem::replace(&mut tab.mode, Box::new(NormalMode));
                let (next_mode, result) = mode.handle_key(tab, key.code, key.modifiers);
                tab.mode = next_mode;
                match result {
                    KeyResult::Handled => {}
                    KeyResult::Ignored => self.handle_global_key(key.code, key.modifiers),
                    KeyResult::ExecuteCommand(cmd) => self.execute_command_str(cmd),
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

    pub fn handle_key_event(&mut self, key_code: KeyCode) {
        self.handle_key_event_with_modifiers(key_code, KeyModifiers::NONE);
    }

    pub fn handle_key_event_with_modifiers(&mut self, key_code: KeyCode, modifiers: KeyModifiers) {
        let tab = &mut self.tabs[self.active_tab];
        let mode = std::mem::replace(&mut tab.mode, Box::new(NormalMode));
        let (next_mode, result) = mode.handle_key(tab, key_code, modifiers);
        tab.mode = next_mode;
        match result {
            KeyResult::Handled => {}
            KeyResult::Ignored => self.handle_global_key(key_code, modifiers),
            KeyResult::ExecuteCommand(cmd) => self.execute_command_str(cmd),
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
        let selected_filter_idx = self.tabs[self.active_tab]
            .mode
            .selected_filter_index()
            .unwrap_or(0);
        let status_line = self.tabs[self.active_tab].mode.status_line().to_string();

        let mut constraints = vec![];
        if has_multiple_tabs {
            constraints.push(Constraint::Length(1)); // Tab bar
        }
        constraints.push(Constraint::Min(1)); // Main content
        if has_input_bar {
            constraints.push(Constraint::Length(1)); // input line
            constraints.push(Constraint::Length(1)); // hint line
        }
        constraints.push(Constraint::Length(3)); // command list
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(size);

        let mut chunk_idx = 0;

        // Render tab bar
        if has_multiple_tabs {
            let tab_bar_area = chunks[chunk_idx];
            chunk_idx += 1;

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

        let log_lines: Vec<Line> = self.tabs[self.active_tab].visible_indices[start..end]
            .iter()
            .enumerate()
            .map(|(vis_idx, &line_idx)| {
                let line_bytes = self.tabs[self.active_tab].file_reader.get_line(line_idx);
                let is_current = start + vis_idx == current_scroll;
                let is_marked = self.tabs[self.active_tab].log_manager.is_marked(line_idx);

                let mut collector = filter_manager.evaluate_line(line_bytes);

                if let Some(sr) = search_map.get(&line_idx) {
                    collector.with_priority(1000);
                    for &(s, e) in &sr.matches {
                        collector.push(s, e, SEARCH_STYLE_ID);
                    }
                }

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

                let mut line = render_line(&collector, &styles);
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

        let logs_title = format!("Logs ({})", num_visible);

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

            let sidebar = Paragraph::new(filters_text).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(self.theme.border))
                    .title("Filters")
                    .title_style(Style::default().fg(self.theme.border_title)),
            );
            frame.render_widget(sidebar, sidebar_area);
        }

        // Command input bar
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

        // Search input bar
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

        // ConfirmRestore modal
        if is_confirm_restore {
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

        let command_list = Paragraph::new(status_line)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(self.theme.border)),
            )
            .wrap(Wrap { trim: true })
            .style(Style::default().fg(self.theme.text));
        let mut area = *chunks.last().unwrap();
        if area.height > 5 {
            area.height = 5;
        }
        frame.render_widget(command_list, area);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auto_complete::shell_split;
    use crate::db::Database;
    use crate::file_reader::FileReader;
    use crate::log_manager::LogManager;
    use std::sync::Arc;

    fn make_tab(lines: &[&str]) -> (FileReader, LogManager) {
        let data: Vec<u8> = lines.join("\n").into_bytes();
        let file_reader = FileReader::from_bytes(data);
        let rt = Arc::new(tokio::runtime::Runtime::new().unwrap());
        let db = Arc::new(rt.block_on(Database::in_memory()).unwrap());
        let log_manager = LogManager::new(db, rt, None);
        (file_reader, log_manager)
    }

    fn make_app(lines: &[&str]) -> App {
        let (file_reader, log_manager) = make_tab(lines);
        App::new(log_manager, file_reader, Theme::default())
    }

    #[test]
    fn test_toggle_wrap_command() {
        let mut app = make_app(&["INFO something", "WARN warning", "ERROR error"]);
        app.execute_command_str("wrap".to_string());
        assert!(!app.tab().wrap);
        app.execute_command_str("wrap".to_string());
        assert!(app.tab().wrap);
    }

    #[test]
    fn test_add_filter_command() {
        let mut app = make_app(&["INFO something", "WARN warning", "ERROR error"]);
        app.execute_command_str("filter foo".to_string());
        let filters = app.tab().log_manager.get_filters();
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].filter_type, FilterType::Include);
        assert_eq!(filters[0].pattern, "foo");
    }

    #[test]
    fn test_add_exclude_command() {
        let mut app = make_app(&["INFO something", "WARN warning", "ERROR error"]);
        app.execute_command_str("exclude bar".to_string());
        let filters = app.tab().log_manager.get_filters();
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].filter_type, FilterType::Exclude);
        assert_eq!(filters[0].pattern, "bar");
    }

    #[test]
    fn test_shell_split_basic() {
        assert_eq!(shell_split("filter foo"), vec!["filter", "foo"]);
        assert_eq!(shell_split("  filter  foo  "), vec!["filter", "foo"]);
        assert_eq!(shell_split(""), Vec::<String>::new());
    }

    #[test]
    fn test_shell_split_quoted() {
        assert_eq!(
            shell_split(r#"filter "hello world""#),
            vec!["filter", "hello world"]
        );
        assert_eq!(
            shell_split(r#"exclude "foo bar baz""#),
            vec!["exclude", "foo bar baz"]
        );
    }

    #[test]
    fn test_filter_command_with_quoted_pattern() {
        let mut app = make_app(&["INFO something", "WARN warning", "ERROR error"]);
        app.execute_command_str(r#"filter "hello world""#.to_string());
        let filters = app.tab().log_manager.get_filters();
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].pattern, "hello world");
        assert_eq!(filters[0].filter_type, FilterType::Include);
    }

    #[test]
    fn test_filter_reduces_visible() {
        let lines = vec!["INFO something", "WARN warning", "ERROR error"];
        let (file_reader, log_manager) = make_tab(&lines);
        let mut app = App::new(log_manager, file_reader, Theme::default());

        assert_eq!(app.tab().visible_indices.len(), 3);

        app.execute_command_str("filter INFO".to_string());

        assert_eq!(app.tab().visible_indices.len(), 1);
    }

    #[test]
    fn test_mark_toggle() {
        let lines = vec!["line0", "line1", "line2"];
        let (file_reader, log_manager) = make_tab(&lines);
        let mut app = App::new(log_manager, file_reader, Theme::default());

        app.tab_mut().scroll_offset = 0;
        app.handle_key_event_with_modifiers(KeyCode::Char('m'), KeyModifiers::NONE);
        assert!(app.tab().log_manager.is_marked(0));

        app.handle_key_event_with_modifiers(KeyCode::Char('m'), KeyModifiers::NONE);
        assert!(!app.tab().log_manager.is_marked(0));
    }

    #[test]
    fn test_scroll_g_key() {
        let lines: Vec<&str> = (0..20).map(|_| "line").collect();
        let (file_reader, log_manager) = make_tab(&lines);
        let mut app = App::new(log_manager, file_reader, Theme::default());

        // 'G' goes to end
        app.handle_key_event_with_modifiers(KeyCode::Char('G'), KeyModifiers::NONE);
        assert_eq!(app.tab().scroll_offset, 19);

        // 'gg' goes to top
        app.handle_key_event_with_modifiers(KeyCode::Char('g'), KeyModifiers::NONE);
        app.handle_key_event_with_modifiers(KeyCode::Char('g'), KeyModifiers::NONE);
        assert_eq!(app.tab().scroll_offset, 0);
    }

    #[test]
    fn test_to_file_context_none_without_source() {
        let app = make_app(&["line"]);
        assert!(app.tab().to_file_context().is_none());
    }
}
