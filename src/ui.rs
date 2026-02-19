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

use crate::auto_complete::{
    complete_color, complete_file_path, extract_color_partial, find_command_completions,
    find_matching_command, shell_split,
};
use crate::db::{FileContext, FileContextStore};
use crate::file_reader::FileReader;
use crate::filters::{render_line, SEARCH_STYLE_ID};
use crate::log_manager::LogManager;
use crate::search::Search;
use crate::theme::Theme;
use crate::types::{FilterType, LogLevel};

#[derive(Debug, PartialEq)]
pub enum AppMode {
    Normal,
    Command {
        input: String,
        cursor: usize,
        history: Vec<String>,
        history_index: Option<usize>,
    },
    FilterManagement {
        selected_filter_index: usize,
    },
    FilterEdit {
        filter_id: Option<usize>,
        filter_input: String,
    },
    Search {
        input: String,
        forward: bool,
    },
    ConfirmRestore {
        context: FileContext,
    },
}

pub struct TabState {
    pub file_reader: FileReader,
    pub log_manager: LogManager,
    /// Indices into `file_reader` of lines currently visible under the active filters.
    pub visible_indices: Vec<usize>,
    pub mode: AppMode,
    pub scroll_offset: usize,
    pub viewport_offset: usize,
    pub show_sidebar: bool,
    pub g_key_pressed: bool,
    pub wrap: bool,
    pub show_line_numbers: bool,
    pub horizontal_scroll: usize,
    pub search: Search,
    pub tab_completion_index: Option<usize>,
    pub command_error: Option<String>,
    pub level_colors: bool,
    pub filter_context: Option<usize>,
    pub editing_filter_id: Option<usize>,
    pub visible_height: usize,
    pub title: String,
}

impl TabState {
    pub fn new(file_reader: FileReader, log_manager: LogManager, title: String) -> Self {
        let mut tab = TabState {
            file_reader,
            log_manager,
            visible_indices: Vec::new(),
            mode: AppMode::Normal,
            scroll_offset: 0,
            viewport_offset: 0,
            show_sidebar: true,
            g_key_pressed: false,
            wrap: true,
            show_line_numbers: true,
            horizontal_scroll: 0,
            search: Search::new(),
            tab_completion_index: None,
            command_error: None,
            level_colors: true,
            filter_context: None,
            editing_filter_id: None,
            visible_height: 0,
            title,
        };
        tab.refresh_visible();
        tab
    }

    /// Recompute which file lines are visible under the current filters.
    pub fn refresh_visible(&mut self) {
        let (fm, _) = self.log_manager.build_filter_manager();
        self.visible_indices = fm.compute_visible(&self.file_reader);
    }

    fn handle_normal_mode_key(&mut self, key_code: KeyCode, modifiers: KeyModifiers) {
        // Ctrl+d: half page down
        if key_code == KeyCode::Char('d') && modifiers.contains(KeyModifiers::CONTROL) {
            let half = (self.visible_height / 2).max(1);
            self.scroll_offset = self.scroll_offset.saturating_add(half);
            self.g_key_pressed = false;
            return;
        }
        // Ctrl+u: half page up
        if key_code == KeyCode::Char('u') && modifiers.contains(KeyModifiers::CONTROL) {
            let half = (self.visible_height / 2).max(1);
            self.scroll_offset = self.scroll_offset.saturating_sub(half);
            self.g_key_pressed = false;
            return;
        }
        match key_code {
            KeyCode::PageDown => {
                let page = self.visible_height.max(1);
                self.scroll_offset = self.scroll_offset.saturating_add(page);
                self.g_key_pressed = false;
            }
            KeyCode::PageUp => {
                let page = self.visible_height.max(1);
                self.scroll_offset = self.scroll_offset.saturating_sub(page);
                self.g_key_pressed = false;
            }
            KeyCode::Char('q') => {} // handled in run()
            KeyCode::Char(':') => {
                self.mode = AppMode::Command {
                    input: String::new(),
                    cursor: 0,
                    history: Vec::new(),
                    history_index: None,
                }
            }
            KeyCode::Char('f') => {
                self.mode = AppMode::FilterManagement {
                    selected_filter_index: 0,
                }
            }
            KeyCode::Char('s') => self.show_sidebar = !self.show_sidebar,
            KeyCode::Char('j') => {
                self.scroll_offset = self.scroll_offset.saturating_add(1);
                self.g_key_pressed = false;
            }
            KeyCode::Char('k') => {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
                self.g_key_pressed = false;
            }
            KeyCode::Char('h') => {
                if !self.wrap {
                    self.horizontal_scroll = self.horizontal_scroll.saturating_sub(1);
                }
                self.g_key_pressed = false;
            }
            KeyCode::Char('l') => {
                if !self.wrap {
                    self.horizontal_scroll = self.horizontal_scroll.saturating_add(1);
                }
                self.g_key_pressed = false;
            }
            KeyCode::Char('w') => {
                self.wrap = !self.wrap;
                self.g_key_pressed = false;
            }
            KeyCode::Char('G') => {
                let n = self.visible_indices.len();
                if n > 0 {
                    self.scroll_offset = n - 1;
                }
                self.g_key_pressed = false;
            }
            KeyCode::Char('g') => {
                if self.g_key_pressed {
                    self.scroll_offset = 0;
                    self.g_key_pressed = false;
                } else {
                    self.g_key_pressed = true;
                }
            }
            KeyCode::Down => {
                self.scroll_offset = self.scroll_offset.saturating_add(1);
                self.g_key_pressed = false;
            }
            KeyCode::Up => {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
                self.g_key_pressed = false;
            }
            KeyCode::Char('m') => {
                if let Some(&line_idx) = self.visible_indices.get(self.scroll_offset) {
                    self.log_manager.toggle_mark(line_idx);
                }
                self.g_key_pressed = false;
            }
            KeyCode::Char('/') => {
                self.mode = AppMode::Search {
                    input: String::new(),
                    forward: true,
                };
                self.g_key_pressed = false;
            }
            KeyCode::Char('?') => {
                self.mode = AppMode::Search {
                    input: String::new(),
                    forward: false,
                };
                self.g_key_pressed = false;
            }
            KeyCode::Char('n') => {
                if let Some(result) = self.search.next_match() {
                    let line_idx = result.line_idx;
                    self.scroll_to_line_idx(line_idx);
                }
                self.g_key_pressed = false;
            }
            KeyCode::Char('N') => {
                if let Some(result) = self.search.previous_match() {
                    let line_idx = result.line_idx;
                    self.scroll_to_line_idx(line_idx);
                }
                self.g_key_pressed = false;
            }
            _ => {
                self.g_key_pressed = false;
            }
        }
    }

    fn handle_filter_management_mode_key(&mut self, key_code: KeyCode) {
        // Extract the current selected index (Copy type) to avoid holding a &mut self.mode
        // borrow while calling methods that need &mut self.
        let selected = match self.mode {
            AppMode::FilterManagement { selected_filter_index } => selected_filter_index,
            _ => return,
        };

        match key_code {
            KeyCode::Esc => {
                self.mode = AppMode::Normal;
            }
            KeyCode::Up => {
                let new_idx = selected.saturating_sub(1);
                self.mode = AppMode::FilterManagement { selected_filter_index: new_idx };
            }
            KeyCode::Down => {
                let num_filters = self.log_manager.get_filters().len();
                let new_idx = if num_filters > 0 {
                    (selected + 1).min(num_filters - 1)
                } else {
                    0
                };
                self.mode = AppMode::FilterManagement { selected_filter_index: new_idx };
            }
            KeyCode::Char(' ') => {
                let filter_id = self.log_manager.get_filters().get(selected).map(|f| f.id);
                if let Some(id) = filter_id {
                    self.log_manager.toggle_filter(id);
                    self.refresh_visible();
                }
                self.mode = AppMode::FilterManagement { selected_filter_index: selected };
            }
            KeyCode::Char('d') => {
                let filter_id = self.log_manager.get_filters().get(selected).map(|f| f.id);
                if let Some(id) = filter_id {
                    self.log_manager.remove_filter(id);
                    self.refresh_visible();
                    let remaining_len = self.log_manager.get_filters().len();
                    let new_idx = if remaining_len > 0 && selected >= remaining_len {
                        remaining_len - 1
                    } else {
                        selected
                    };
                    self.mode = AppMode::FilterManagement { selected_filter_index: new_idx };
                } else {
                    self.mode = AppMode::FilterManagement { selected_filter_index: selected };
                }
            }
            KeyCode::Char('K') => {
                let filter_id = self.log_manager.get_filters().get(selected).map(|f| f.id);
                if let Some(id) = filter_id {
                    self.log_manager.move_filter_up(id);
                    self.refresh_visible();
                    let new_idx = selected.saturating_sub(1);
                    self.mode = AppMode::FilterManagement { selected_filter_index: new_idx };
                } else {
                    self.mode = AppMode::FilterManagement { selected_filter_index: selected };
                }
            }
            KeyCode::Char('J') => {
                let filter_id = self.log_manager.get_filters().get(selected).map(|f| f.id);
                if let Some(id) = filter_id {
                    self.log_manager.move_filter_down(id);
                    self.refresh_visible();
                    let total = self.log_manager.get_filters().len();
                    let new_idx = if selected + 1 < total { selected + 1 } else { selected };
                    self.mode = AppMode::FilterManagement { selected_filter_index: new_idx };
                } else {
                    self.mode = AppMode::FilterManagement { selected_filter_index: selected };
                }
            }
            KeyCode::Char('e') => {
                let filter_info = self.log_manager.get_filters().get(selected).map(|f| {
                    (f.id, f.filter_type.clone(), f.color_config.clone(), f.pattern.clone())
                });
                if let Some((id, ft, cc, pattern)) = filter_info {
                    self.editing_filter_id = Some(id);
                    self.filter_context = Some(selected);
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
                    self.mode = AppMode::Command {
                        input: cmd,
                        cursor: len,
                        history: Vec::new(),
                        history_index: None,
                    };
                } else {
                    self.mode = AppMode::FilterManagement { selected_filter_index: selected };
                }
            }
            KeyCode::Char('c') => {
                let color_config = self.log_manager.get_filters().get(selected).and_then(|f| f.color_config.clone());
                self.filter_context = Some(selected);
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
                self.mode = AppMode::Command {
                    input: cmd,
                    cursor: len,
                    history: Vec::new(),
                    history_index: None,
                };
            }
            KeyCode::Char('i') => {
                self.mode = AppMode::Command {
                    input: "filter ".to_string(),
                    cursor: 7,
                    history: Vec::new(),
                    history_index: None,
                };
            }
            KeyCode::Char('x') => {
                self.mode = AppMode::Command {
                    input: "exclude ".to_string(),
                    cursor: 8,
                    history: Vec::new(),
                    history_index: None,
                };
            }
            _ => {
                self.mode = AppMode::FilterManagement { selected_filter_index: selected };
            }
        }
    }

    fn handle_filter_edit_mode_key(&mut self, key_code: KeyCode) {
        let (current_id, current_input) = match &self.mode {
            AppMode::FilterEdit { filter_id, filter_input } => (*filter_id, filter_input.clone()),
            _ => return,
        };

        match key_code {
            KeyCode::Enter => {
                if let Some(id) = current_id {
                    self.log_manager.edit_filter(id, current_input);
                    self.refresh_visible();
                    self.mode = AppMode::FilterManagement { selected_filter_index: 0 };
                }
            }
            KeyCode::Esc => {
                self.mode = AppMode::FilterManagement { selected_filter_index: 0 };
            }
            KeyCode::Backspace => {
                let mut input = current_input;
                input.pop();
                self.mode = AppMode::FilterEdit { filter_id: current_id, filter_input: input };
            }
            KeyCode::Char(c) => {
                let mut input = current_input;
                input.push(c);
                self.mode = AppMode::FilterEdit { filter_id: current_id, filter_input: input };
            }
            _ => {}
        }
    }

    fn handle_search_mode_key(&mut self, key_code: KeyCode) {
        if let AppMode::Search { input, forward: _ } = &mut self.mode {
            match key_code {
                KeyCode::Enter => {
                    let (search_input, forward_val) = match &mut self.mode {
                        AppMode::Search { input, forward } => (input.clone(), *forward),
                        _ => return,
                    };
                    let _ = self
                        .search
                        .search(&search_input, &self.visible_indices, &self.file_reader);
                    let search_result = if forward_val {
                        self.search.next_match()
                    } else {
                        self.search.previous_match()
                    };
                    if let Some(result) = search_result {
                        let line_idx = result.line_idx;
                        self.scroll_to_line_idx(line_idx);
                    }
                    self.mode = AppMode::Normal;
                }
                KeyCode::Esc => {
                    *input = String::new();
                    self.mode = AppMode::Normal;
                }
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Char(c) => {
                    input.push(c);
                }
                _ => {}
            }
        }
    }

    fn get_command_list(&self) -> String {
        match self.mode {
            AppMode::Normal => {
                "[NORMAL] [q]uit | : => command Mode | [f]ilter mode | [s]idebar | [m]ark Line | / => search | ? => search backward | [n]ext match | N => prev match | PgDn/Ctrl+d PgUp/Ctrl+u | Tab/Shift+Tab => switch tab".to_string()
            },
            AppMode::Command { .. } => {
                "[COMMAND] filter | exclude | set-color | export-marked | save-filters | load-filters | wrap | set-theme | level-colors | open | close-tab | Esc | Enter".to_string()
            },
            AppMode::FilterManagement { .. } => {
                "[FILTER] [i]nclude | e[x]clude | Space => toggle | [d]elete | [e]dit | set [c]olor | [J/K] move down/up | Esc => normal mode".to_string()
            },
            AppMode::FilterEdit { .. } => {
                "[FILTER EDIT] Esc => cancel | Enter => save".to_string()
            },
            AppMode::Search { .. } => {
                "[SEARCH] Esc => cancel | Enter => search".to_string()
            },
            AppMode::ConfirmRestore { .. } => {
                "[RESTORE] Restore previous session? [y]es / [n]o".to_string()
            },
        }
    }

    fn scroll_to_line_idx(&mut self, line_idx: usize) {
        if let Some(index) = self.visible_indices.iter().position(|&i| i == line_idx) {
            self.scroll_offset = index;
        }
    }

    fn get_selected_filter_index(&self) -> Option<usize> {
        match &self.mode {
            AppMode::FilterManagement {
                selected_filter_index,
            } => Some(*selected_filter_index),
            _ => None,
        }
    }

    fn get_command_input_and_cursor(&self) -> Option<(&str, usize)> {
        match &self.mode {
            AppMode::Command { input, cursor, .. } => Some((input.as_str(), *cursor)),
            _ => None,
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

    fn apply_file_context(&mut self, ctx: &FileContext) {
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

pub struct App {
    pub tabs: Vec<TabState>,
    pub active_tab: usize,
    pub theme: Theme,
    pub available_themes: Vec<String>,
    pub db: Arc<crate::db::Database>,
    pub rt: Arc<tokio::runtime::Runtime>,
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

        let mut theme_paths = vec![];
        if let Ok(entries) = std::fs::read_dir("themes") {
            for entry in entries.flatten() {
                theme_paths.push(entry.path());
            }
        }
        if let Some(config_dir) = dirs::config_dir() {
            let user_themes_path = config_dir.join("logsmith-rs/themes");
            if let Ok(entries) = std::fs::read_dir(user_themes_path) {
                for entry in entries.flatten() {
                    theme_paths.push(entry.path());
                }
            }
        }

        let mut available_themes_set = std::collections::HashSet::new();
        for path in theme_paths {
            if path.extension().and_then(|ext| ext.to_str()) == Some("json")
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            {
                available_themes_set.insert(stem.to_string());
            }
        }
        let mut available_themes: Vec<String> = available_themes_set.into_iter().collect();
        available_themes.sort();

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
                tab.mode = AppMode::ConfirmRestore { context: ctx };
            }
        }

        App {
            tabs: vec![tab],
            active_tab: 0,
            theme,
            available_themes,
            db,
            rt,
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

        let file_reader = FileReader::new(path).map_err(|e| format!("Failed to read '{}': {}", path, e))?;
        let log_manager = LogManager::new(self.db.clone(), self.rt.clone(), Some(path.to_string()));

        let title = file_path_obj
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path)
            .to_string();

        let mut tab = TabState::new(file_reader, log_manager, title);

        // Check for saved context
        if let Ok(Some(ctx)) = self.rt.block_on(self.db.load_file_context(path)) {
            tab.mode = AppMode::ConfirmRestore { context: ctx };
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
                let tab = &self.tabs[self.active_tab];
                match tab.mode {
                    AppMode::Normal => {
                        if key.code == KeyCode::Char('q') {
                            self.save_all_contexts();
                            return Ok(());
                        }
                        if key.code == KeyCode::Tab {
                            if self.tabs.len() > 1 {
                                self.active_tab = (self.active_tab + 1) % self.tabs.len();
                            }
                        } else if key.code == KeyCode::BackTab {
                            if self.tabs.len() > 1 {
                                self.active_tab = if self.active_tab == 0 {
                                    self.tabs.len() - 1
                                } else {
                                    self.active_tab - 1
                                };
                            }
                        } else if key.code == KeyCode::Char('w')
                            && key.modifiers.contains(KeyModifiers::CONTROL)
                        {
                            if self.close_tab() {
                                return Ok(());
                            }
                        } else if key.code == KeyCode::Char('t')
                            && key.modifiers.contains(KeyModifiers::CONTROL)
                        {
                            self.tabs[self.active_tab].mode = AppMode::Command {
                                input: "open ".to_string(),
                                cursor: 5,
                                history: Vec::new(),
                                history_index: None,
                            };
                        } else {
                            self.tabs[self.active_tab]
                                .handle_normal_mode_key(key.code, key.modifiers);
                        }
                    }
                    AppMode::Command { .. } => self.handle_command_mode_key(key.code),
                    AppMode::FilterManagement { .. } => {
                        self.tabs[self.active_tab].handle_filter_management_mode_key(key.code);
                    }
                    AppMode::FilterEdit { .. } => {
                        self.tabs[self.active_tab].handle_filter_edit_mode_key(key.code);
                    }
                    AppMode::Search { .. } => {
                        self.tabs[self.active_tab].handle_search_mode_key(key.code);
                    }
                    AppMode::ConfirmRestore { .. } => match key.code {
                        KeyCode::Char('y') => {
                            let tab = &mut self.tabs[self.active_tab];
                            if let AppMode::ConfirmRestore { context } =
                                std::mem::replace(&mut tab.mode, AppMode::Normal)
                            {
                                tab.apply_file_context(&context);
                            }
                        }
                        KeyCode::Char('n') | KeyCode::Esc => {
                            let tab = &mut self.tabs[self.active_tab];
                            tab.mode = AppMode::Normal;
                            tab.log_manager.clear_filters();
                            tab.log_manager.set_marks(vec![]);
                            tab.refresh_visible();
                        }
                        _ => {}
                    },
                }
            }

            if last_tick.elapsed() >= tick_rate {
                last_tick = Instant::now();
            }
        }
    }

    pub fn handle_command_mode_key(&mut self, key_code: KeyCode) {
        match key_code {
            KeyCode::Enter => {
                self.handle_command();
                let tab = &mut self.tabs[self.active_tab];
                if tab.command_error.is_none() {
                    if let AppMode::Command {
                        input,
                        cursor,
                        history,
                        history_index,
                    } = &mut tab.mode
                    {
                        if !input.is_empty() {
                            history.push(input.clone());
                        }
                        *input = String::new();
                        *cursor = 0;
                        *history_index = None;
                    }
                    if let Some(idx) = tab.filter_context.take() {
                        tab.mode = AppMode::FilterManagement {
                            selected_filter_index: idx,
                        };
                    } else {
                        tab.mode = AppMode::Normal;
                    }
                }
            }
            KeyCode::Esc => {
                let tab = &mut self.tabs[self.active_tab];
                if let AppMode::Command {
                    input,
                    cursor,
                    history_index,
                    ..
                } = &mut tab.mode
                {
                    *input = String::new();
                    *cursor = 0;
                    *history_index = None;
                }
                tab.filter_context = None;
                tab.editing_filter_id = None;
                tab.mode = AppMode::Normal;
            }
            KeyCode::Backspace => {
                let tab = &mut self.tabs[self.active_tab];
                if let AppMode::Command { input, cursor, .. } = &mut tab.mode
                    && *cursor > 0
                    && !input.is_empty()
                {
                    input.remove(*cursor - 1);
                    *cursor -= 1;
                }
            }
            KeyCode::Char(c) => {
                let tab = &mut self.tabs[self.active_tab];
                if let AppMode::Command { input, cursor, .. } = &mut tab.mode {
                    input.insert(*cursor, c);
                    *cursor += 1;
                    tab.command_error = None;
                    tab.tab_completion_index = None;
                }
            }
            KeyCode::Left => {
                let tab = &mut self.tabs[self.active_tab];
                if let AppMode::Command { cursor, .. } = &mut tab.mode
                    && *cursor > 0
                {
                    *cursor -= 1;
                }
            }
            KeyCode::Right => {
                let tab = &mut self.tabs[self.active_tab];
                if let AppMode::Command { input, cursor, .. } = &mut tab.mode
                    && *cursor < input.len()
                {
                    *cursor += 1;
                }
            }
            KeyCode::Up => {
                let tab = &mut self.tabs[self.active_tab];
                if let AppMode::Command {
                    input,
                    cursor,
                    history,
                    history_index,
                } = &mut tab.mode
                {
                    if history.is_empty() {
                        return;
                    }
                    let new_index = match history_index {
                        None => Some(history.len() - 1),
                        Some(0) => Some(0),
                        Some(i) => Some(*i - 1),
                    };
                    if let Some(i) = new_index {
                        *input = history[i].clone();
                        *cursor = input.len();
                        *history_index = Some(i);
                    }
                }
            }
            KeyCode::Down => {
                let tab = &mut self.tabs[self.active_tab];
                if let AppMode::Command {
                    input,
                    cursor,
                    history,
                    history_index,
                } = &mut tab.mode
                {
                    if history.is_empty() {
                        return;
                    }
                    let new_index = match history_index {
                        None => return,
                        Some(i) if *i + 1 >= history.len() => {
                            *input = String::new();
                            *cursor = 0;
                            *history_index = None;
                            return;
                        }
                        Some(i) => Some(*i + 1),
                    };
                    if let Some(i) = new_index {
                        *input = history[i].clone();
                        *cursor = input.len();
                        *history_index = Some(i);
                    }
                }
            }
            KeyCode::Tab => {
                let tab = &mut self.tabs[self.active_tab];
                if let AppMode::Command { input, .. } = &mut tab.mode {
                    let trimmed = input.trim().to_string();

                    // Color completion for --fg/--bg arguments
                    if let Some(partial) = extract_color_partial(&trimmed) {
                        let completions = complete_color(partial);
                        if !completions.is_empty() {
                            let tab = &mut self.tabs[self.active_tab];
                            let idx = tab.tab_completion_index.unwrap_or(0) % completions.len();
                            let color_name = completions[idx];
                            let prefix = if partial.is_empty() {
                                trimmed.clone()
                            } else {
                                trimmed[..trimmed.len() - partial.len()].to_string()
                            };
                            if let AppMode::Command { input, cursor, .. } = &mut tab.mode {
                                *input = format!("{}{}", prefix, color_name);
                                *cursor = input.len();
                            }
                            tab.tab_completion_index = Some(idx + 1);
                            return;
                        }
                    }

                    // Commands that take a file path argument
                    let file_commands = ["open", "load-filters", "save-filters", "export-marked"];
                    let file_cmd = file_commands
                        .iter()
                        .find(|cmd| trimmed.starts_with(&format!("{} ", cmd)));

                    if let Some(&cmd) = file_cmd {
                        let partial = trimmed[cmd.len()..].trim_start();
                        let completions = complete_file_path(partial);
                        if completions.is_empty() {
                            return;
                        }
                        let tab = &mut self.tabs[self.active_tab];
                        let idx = tab.tab_completion_index.unwrap_or(0) % completions.len();
                        let completed = completions[idx].clone();
                        if let AppMode::Command { input, cursor, .. } = &mut tab.mode {
                            *input = format!("{} {}", cmd, completed);
                            *cursor = input.len();
                        }
                        tab.tab_completion_index = Some(idx + 1);
                        return;
                    }

                    // set-theme completion
                    if trimmed.starts_with("set-theme ") {
                        let themes = &self.available_themes;
                        if themes.is_empty() {
                            return;
                        }
                        let tab = &mut self.tabs[self.active_tab];
                        let idx = tab.tab_completion_index.unwrap_or(0);
                        let theme_name = self.available_themes[idx % self.available_themes.len()].clone();
                        if let AppMode::Command { input, cursor, .. } = &mut tab.mode {
                            *input = format!("set-theme {}", theme_name);
                            *cursor = input.len();
                        }
                        tab.tab_completion_index = Some((idx + 1) % self.available_themes.len());
                        return;
                    }

                    // Command name completion
                    let completions = find_command_completions(&trimmed);
                    if completions.is_empty() {
                        return;
                    }
                    let tab = &mut self.tabs[self.active_tab];
                    let idx = tab.tab_completion_index.unwrap_or(0) % completions.len();
                    if let AppMode::Command { input, cursor, .. } = &mut tab.mode {
                        *input = completions[idx].to_string();
                        *cursor = input.len();
                    }
                    tab.tab_completion_index = Some(idx + 1);
                }
            }
            _ => {}
        }
    }

    pub fn handle_key_event(&mut self, key_code: KeyCode) {
        self.handle_key_event_with_modifiers(key_code, KeyModifiers::NONE);
    }

    pub fn handle_key_event_with_modifiers(&mut self, key_code: KeyCode, modifiers: KeyModifiers) {
        let mode = &self.tabs[self.active_tab].mode;
        match mode {
            AppMode::Normal => {
                self.tabs[self.active_tab].handle_normal_mode_key(key_code, modifiers)
            }
            AppMode::Command { .. } => self.handle_command_mode_key(key_code),
            AppMode::FilterManagement { .. } => {
                self.tabs[self.active_tab].handle_filter_management_mode_key(key_code);
            }
            AppMode::FilterEdit { .. } => {
                self.tabs[self.active_tab].handle_filter_edit_mode_key(key_code);
            }
            AppMode::Search { .. } => {
                self.tabs[self.active_tab].handle_search_mode_key(key_code);
            }
            AppMode::ConfirmRestore { .. } => {
                // Handled in run() loop directly
            }
        }
    }

    fn ui(&mut self, frame: &mut Frame) {
        let size = frame.size();
        frame.render_widget(Block::default().bg(self.theme.root_bg), size);

        let has_multiple_tabs = self.tabs.len() > 1;

        let mut constraints = vec![];
        if has_multiple_tabs {
            constraints.push(Constraint::Length(1)); // Tab bar
        }
        constraints.push(Constraint::Min(1)); // Main content
        let tab = &self.tabs[self.active_tab];
        let has_input_bar = matches!(tab.mode, AppMode::Command { .. })
            || matches!(tab.mode, AppMode::Search { .. });
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

        // Clamp scroll_offset
        if num_visible > 0 && self.tabs[self.active_tab].scroll_offset >= num_visible {
            self.tabs[self.active_tab].scroll_offset = num_visible - 1;
        }

        // Adjust viewport so cursor stays visible
        let scroll_offset = self.tabs[self.active_tab].scroll_offset;
        let viewport_offset = self.tabs[self.active_tab].viewport_offset;
        if scroll_offset < viewport_offset {
            self.tabs[self.active_tab].viewport_offset = scroll_offset;
        } else if visible_height > 0 && scroll_offset >= viewport_offset + visible_height {
            self.tabs[self.active_tab].viewport_offset = scroll_offset - visible_height + 1;
        }

        let start = self.tabs[self.active_tab].viewport_offset;
        let end = (start + visible_height).min(num_visible);

        // Build filter manager and styles array (256 slots: filter styles + search at index 255)
        let (filter_manager, mut styles) =
            self.tabs[self.active_tab].log_manager.build_filter_manager();
        let search_style = Style::default()
            .fg(Color::Black)
            .bg(self.theme.text_highlight);
        styles.resize(256, Style::default());
        styles[255] = search_style;

        // Build search result lookup: line_idx → &SearchResult
        let search_results = self.tabs[self.active_tab].search.get_results();
        let search_map: HashMap<usize, &crate::types::SearchResult> =
            search_results.iter().map(|r| (r.line_idx, r)).collect();

        let theme = &self.theme;
        let level_colors = self.tabs[self.active_tab].level_colors;
        let current_scroll = self.tabs[self.active_tab].scroll_offset;
        let show_line_numbers = self.tabs[self.active_tab].show_line_numbers;
        let line_number_width = if show_line_numbers {
            num_visible.max(1).to_string().len()
        } else {
            0
        };

        let log_lines: Vec<Line> = self.tabs[self.active_tab].visible_indices[start..end]
            .iter()
            .enumerate()
            .map(|(vis_idx, &line_idx)| {
                let line_bytes = self.tabs[self.active_tab].file_reader.get_line(line_idx);
                let is_current = start + vis_idx == current_scroll;
                let is_marked = self.tabs[self.active_tab].log_manager.is_marked(line_idx);

                // Evaluate filters to collect match spans
                let mut collector = filter_manager.evaluate_line(line_bytes);

                // Add search highlights with higher priority
                if let Some(sr) = search_map.get(&line_idx) {
                    collector.with_priority(1000);
                    for &(s, e) in &sr.matches {
                        collector.push(s, e, SEARCH_STYLE_ID);
                    }
                }

                // Base style from level colors and marks
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
                    let line_num_str =
                        format!("{:>width$} ", line_num, width = line_number_width);
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
            let selected_idx = self.tabs[self.active_tab]
                .get_selected_filter_index()
                .unwrap_or(0);
            let filters = self.tabs[self.active_tab].log_manager.get_filters();
            let filters_text: Vec<Line> = filters
                .iter()
                .enumerate()
                .map(|(i, filter)| {
                    let status = if filter.enabled { "[x]" } else { "[ ]" };
                    let selected_prefix = if i == selected_idx { ">" } else { " " };
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

        if matches!(self.tabs[self.active_tab].mode, AppMode::Command { .. }) {
            let input_prefix = ":";
            let (input_text, cursor_pos) = self.tabs[self.active_tab]
                .get_command_input_and_cursor()
                .unwrap_or(("", 0));
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
            } else if let Some(partial) = extract_color_partial(input_text) {
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
                } else if let Some(cmd) = find_matching_command(input_text) {
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

        if let AppMode::Search { input, forward } = &self.tabs[self.active_tab].mode {
            let prefix = if *forward { "/" } else { "?" };
            let input_clone = input.clone();
            let search_line = Paragraph::new(format!("{}{}", prefix, input_clone))
                .style(Style::default().fg(self.theme.text).bg(self.theme.border))
                .wrap(Wrap { trim: false });
            let input_area = chunks[chunk_idx];
            frame.render_widget(search_line, input_area);
            let cursor_x = input_area.x + 1 + input_clone.len() as u16;
            if cursor_x < input_area.x + input_area.width {
                frame.set_cursor(cursor_x, input_area.y);
            }

            let hint_area = chunks[chunk_idx + 1];
            let match_count = self.tabs[self.active_tab].search.get_results().len();
            let hint_text = if !input_clone.is_empty() {
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

        if matches!(
            self.tabs[self.active_tab].mode,
            AppMode::ConfirmRestore { .. }
        ) {
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

        let command_list = Paragraph::new(self.tabs[self.active_tab].get_command_list())
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

    fn handle_command(&mut self) {
        use crate::commands::{CommandLine, Commands};
        use clap::Parser;
        let input = match &self.tabs[self.active_tab].mode {
            AppMode::Command { input, .. } => input.trim().to_string(),
            _ => return,
        };
        let args = match CommandLine::try_parse_from(shell_split(&input)) {
            Ok(args) => args,
            Err(e) => {
                self.tabs[self.active_tab].command_error = Some(format!("Invalid command: {}", e));
                return;
            }
        };
        match args.command {
            Some(Commands::Filter { pattern, fg, bg, m }) => {
                if let Some(old_id) = self.tabs[self.active_tab].editing_filter_id.take() {
                    self.tabs[self.active_tab].log_manager.remove_filter(old_id);
                }
                self.tabs[self.active_tab].log_manager.add_filter_with_color(
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
                self.tabs[self.active_tab].log_manager.add_filter_with_color(
                    pattern,
                    FilterType::Exclude,
                    None,
                    None,
                    false,
                );
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
                    if let Err(e) = self.tabs[self.active_tab].log_manager.save_filters(&path) {
                        self.tabs[self.active_tab].command_error =
                            Some(format!("Failed to save filters: {}", e));
                    }
                }
            }
            Some(Commands::LoadFilters { path }) => {
                if !path.is_empty() {
                    match self.tabs[self.active_tab].log_manager.load_filters(&path) {
                        Ok(()) => self.tabs[self.active_tab].refresh_visible(),
                        Err(e) => {
                            self.tabs[self.active_tab].command_error =
                                Some(format!("Failed to load filters: {}", e));
                        }
                    }
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
                self.tabs[self.active_tab].level_colors =
                    !self.tabs[self.active_tab].level_colors;
            }
            Some(Commands::SetTheme { theme_name }) => {
                let theme_filename = format!("{}.json", theme_name.to_lowercase());
                match Theme::from_file(&theme_filename) {
                    Ok(theme) => self.theme = theme,
                    Err(e) => {
                        self.tabs[self.active_tab].command_error =
                            Some(format!("Failed to load theme '{}': {}", theme_name, e))
                    }
                }
            }
            Some(Commands::Open { path }) => {
                let old_tab = self.active_tab;
                if let Err(e) = self.open_file(&path) {
                    self.tabs[self.active_tab].command_error = Some(e);
                } else {
                    self.tabs[old_tab].mode = AppMode::Normal;
                }
            }
            Some(Commands::CloseTab) => {
                if self.tabs.len() <= 1 {
                    self.tabs[self.active_tab].command_error =
                        Some("Cannot close last tab. Use 'q' to quit.".to_string());
                } else {
                    self.tabs.remove(self.active_tab);
                    if self.active_tab >= self.tabs.len() {
                        self.active_tab = self.tabs.len() - 1;
                    }
                }
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::file_reader::FileReader;
    use crate::log_manager::LogManager;
    use crate::auto_complete::shell_split;
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
        app.tab_mut().mode = AppMode::Command {
            input: "wrap".to_string(),
            cursor: 4,
            history: Vec::new(),
            history_index: None,
        };
        app.handle_command();
        assert!(!app.tab().wrap);
        app.tab_mut().mode = AppMode::Command {
            input: "wrap".to_string(),
            cursor: 4,
            history: Vec::new(),
            history_index: None,
        };
        app.handle_command();
        assert!(app.tab().wrap);
    }

    #[test]
    fn test_add_filter_command() {
        let mut app = make_app(&["INFO something", "WARN warning", "ERROR error"]);
        app.tab_mut().mode = AppMode::Command {
            input: "filter foo".to_string(),
            cursor: 10,
            history: Vec::new(),
            history_index: None,
        };
        app.handle_command();
        let filters = app.tab().log_manager.get_filters();
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].filter_type, FilterType::Include);
        assert_eq!(filters[0].pattern, "foo");
    }

    #[test]
    fn test_add_exclude_command() {
        let mut app = make_app(&["INFO something", "WARN warning", "ERROR error"]);
        app.tab_mut().mode = AppMode::Command {
            input: "exclude bar".to_string(),
            cursor: 11,
            history: Vec::new(),
            history_index: None,
        };
        app.handle_command();
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
        app.tab_mut().mode = AppMode::Command {
            input: r#"filter "hello world""#.to_string(),
            cursor: 20,
            history: Vec::new(),
            history_index: None,
        };
        app.handle_command();
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

        // All lines visible initially
        assert_eq!(app.tab().visible_indices.len(), 3);

        // Add include filter for "INFO"
        app.tab_mut().mode = AppMode::Command {
            input: "filter INFO".to_string(),
            cursor: 11,
            history: Vec::new(),
            history_index: None,
        };
        app.handle_command();

        // Only INFO line visible
        assert_eq!(app.tab().visible_indices.len(), 1);
    }

    #[test]
    fn test_mark_toggle() {
        let lines = vec!["line0", "line1", "line2"];
        let (file_reader, log_manager) = make_tab(&lines);
        let mut app = App::new(log_manager, file_reader, Theme::default());

        app.tab_mut().scroll_offset = 0;
        app.tab_mut().handle_normal_mode_key(KeyCode::Char('m'), KeyModifiers::NONE);
        assert!(app.tab().log_manager.is_marked(0));

        app.tab_mut().handle_normal_mode_key(KeyCode::Char('m'), KeyModifiers::NONE);
        assert!(!app.tab().log_manager.is_marked(0));
    }

    #[test]
    fn test_scroll_g_key() {
        let lines: Vec<&str> = (0..20).map(|_| "line").collect();
        let (file_reader, log_manager) = make_tab(&lines);
        let mut app = App::new(log_manager, file_reader, Theme::default());

        // 'G' goes to end
        app.tab_mut().handle_normal_mode_key(KeyCode::Char('G'), KeyModifiers::NONE);
        assert_eq!(app.tab().scroll_offset, 19);

        // 'gg' goes to top
        app.tab_mut().handle_normal_mode_key(KeyCode::Char('g'), KeyModifiers::NONE);
        app.tab_mut().handle_normal_mode_key(KeyCode::Char('g'), KeyModifiers::NONE);
        assert_eq!(app.tab().scroll_offset, 0);
    }

    #[test]
    fn test_to_file_context_none_without_source() {
        let app = make_app(&["line"]);
        // No source_file set → to_file_context returns None
        assert!(app.tab().to_file_context().is_none());
    }
}
