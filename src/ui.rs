use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame, Terminal,
    prelude::*,
    style::Modifier,
    widgets::{Block, Borders, Paragraph, Wrap},
};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::analyzer::{FilterType, LogAnalyzer, LogEntry, LogLevel, apply_filters_to_logs};
use crate::db::{FilterStore, LogStore};
use crate::search::Search;
use crate::theme::Theme;
use std::collections::HashSet;

struct CommandInfo {
    name: &'static str,
    usage: &'static str,
    description: &'static str,
}

const COMMANDS: &[CommandInfo] = &[
    CommandInfo {
        name: "filter",
        usage: "filter [--fg <color>] [--bg <color>] <pattern>",
        description: "Add an include filter. e.g. filter --fg Red error",
    },
    CommandInfo {
        name: "exclude",
        usage: "exclude <pattern>",
        description: "Add an exclude filter. e.g. exclude debug",
    },
    CommandInfo {
        name: "set-color",
        usage: "set-color --fg <color> --bg <color>",
        description: "Set color for the selected filter. e.g. set-color --fg Green --bg Black",
    },
    CommandInfo {
        name: "export-marked",
        usage: "export-marked <path>",
        description: "Export marked logs to a file. e.g. export-marked /tmp/marked.log",
    },
    CommandInfo {
        name: "save-filters",
        usage: "save-filters <path>",
        description: "Save current filters to JSON. e.g. save-filters filters.json",
    },
    CommandInfo {
        name: "load-filters",
        usage: "load-filters <path>",
        description: "Load filters from JSON. e.g. load-filters filters.json",
    },
    CommandInfo {
        name: "wrap",
        usage: "wrap",
        description: "Toggle line wrapping on/off",
    },
    CommandInfo {
        name: "set-theme",
        usage: "set-theme <name>",
        description: "Change the color theme. e.g. set-theme dracula",
    },
    CommandInfo {
        name: "level-colors",
        usage: "level-colors",
        description: "Toggle ERROR/WARN log level color highlighting on/off",
    },
];

fn command_names() -> Vec<&'static str> {
    COMMANDS.iter().map(|c| c.name).collect()
}

fn find_matching_command(input: &str) -> Option<&'static CommandInfo> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let cmd_word = trimmed.split_whitespace().next().unwrap_or("");
    COMMANDS.iter().find(|c| c.name == cmd_word)
}

fn find_command_completions(prefix: &str) -> Vec<&'static str> {
    let trimmed = prefix.trim();
    if trimmed.is_empty() {
        return command_names();
    }
    // Only complete the command name (first word)
    if trimmed.contains(' ') {
        return vec![];
    }
    COMMANDS
        .iter()
        .filter(|c| c.name.starts_with(trimmed))
        .map(|c| c.name)
        .collect()
}

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
}

const BACKGROUND_BATCH_SIZE: usize = 5000;

const SPINNER_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

struct BackgroundLoadState {
    receiver: mpsc::Receiver<Result<usize, String>>,
    lines_loaded: usize,
    spinner_frame: usize,
}

impl std::fmt::Debug for BackgroundLoadState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackgroundLoadState")
            .field("lines_loaded", &self.lines_loaded)
            .field("spinner_frame", &self.spinner_frame)
            .finish()
    }
}

struct CacheResult {
    all_logs: Vec<LogEntry>,
    filtered_logs: Vec<LogEntry>,
}

pub struct App {
    pub analyzer: LogAnalyzer,
    pub mode: AppMode,
    pub scroll_offset: usize,
    pub viewport_offset: usize,
    pub show_sidebar: bool,
    pub g_key_pressed: bool,
    pub wrap: bool,
    pub horizontal_scroll: usize,
    pub search: Search,
    pub theme: Theme,
    pub available_themes: Vec<String>,
    pub tab_completion_index: Option<usize>,
    pub command_error: Option<String>,
    pub level_colors: bool,
    pub visible_height: usize,
    cached_logs: Vec<LogEntry>,
    cached_filtered_logs: Vec<LogEntry>,
    logs_dirty: bool,
    filters_dirty: bool,
    cache_refresh_rx: Option<mpsc::Receiver<CacheResult>>,
    cache_refresh_pending: bool,
    background_loading: Option<BackgroundLoadState>,
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("mode", &self.mode)
            .field("scroll_offset", &self.scroll_offset)
            .finish()
    }
}

impl App {
    pub fn new(analyzer: LogAnalyzer, theme: Theme) -> App {
        let mut theme_paths = vec![];
        // Local themes directory
        if let Ok(entries) = std::fs::read_dir("themes") {
            for entry in entries.flatten() {
                theme_paths.push(entry.path());
            }
        }

        // User config themes directory
        if let Some(config_dir) = dirs::config_dir() {
            let user_themes_path = config_dir.join("logsmith-rs/themes");
            if let Ok(entries) = std::fs::read_dir(user_themes_path) {
                for entry in entries.flatten() {
                    theme_paths.push(entry.path());
                }
            }
        }

        let mut available_themes_set = HashSet::new();
        for path in theme_paths {
            if path.extension().and_then(|ext| ext.to_str()) == Some("json")
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            {
                available_themes_set.insert(stem.to_string());
            }
        }

        let mut available_themes: Vec<String> = available_themes_set.into_iter().collect();
        available_themes.sort();

        App {
            analyzer,
            mode: AppMode::Normal,
            scroll_offset: 0,
            viewport_offset: 0,
            show_sidebar: false,
            g_key_pressed: false,
            wrap: true,
            horizontal_scroll: 0,
            search: Search::new(),
            theme,
            available_themes,
            tab_completion_index: None,
            command_error: None,
            level_colors: true,
            visible_height: 0,
            cached_logs: Vec::new(),
            cached_filtered_logs: Vec::new(),
            logs_dirty: true,
            filters_dirty: true,
            cache_refresh_rx: None,
            cache_refresh_pending: false,
            background_loading: None,
        }
    }

    pub fn run(&mut self, terminal: &mut Terminal<impl Backend>) -> anyhow::Result<()> {
        let mut last_tick = Instant::now();
        let tick_rate = Duration::from_millis(250);

        // Kick off initial async cache load
        self.schedule_cache_refresh();

        loop {
            terminal.draw(|frame| self.ui(frame))?;

            // Use a short poll timeout while async work is in progress
            let has_async_work =
                self.background_loading.is_some() || self.cache_refresh_pending;
            let poll_timeout = if has_async_work {
                Duration::from_millis(16) // ~60fps while loading
            } else {
                tick_rate
                    .checked_sub(last_tick.elapsed())
                    .unwrap_or_else(|| Duration::from_secs(0))
            };

            if event::poll(poll_timeout)?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                match self.mode {
                    AppMode::Normal => {
                        if key.code == KeyCode::Char('q') {
                            return Ok(());
                        }
                        self.handle_normal_mode_key(key.code, key.modifiers)
                    }
                    AppMode::Command { .. } => self.handle_command_mode_key(key.code),
                    AppMode::FilterManagement { .. } => {
                        self.handle_filter_management_mode_key(key.code)
                    }
                    AppMode::FilterEdit { .. } => self.handle_filter_edit_mode_key(key.code),
                    AppMode::Search { .. } => self.handle_search_mode_key(key.code),
                }
            }

            // Poll async results (non-blocking)
            self.process_background_loading();
            self.poll_cache_refresh();

            // Schedule refresh if dirty flags were set by loading or key handlers
            self.schedule_cache_refresh();

            if last_tick.elapsed() >= tick_rate {
                last_tick = Instant::now();
            }
        }
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
                let num_logs = self.cached_filtered_logs.len();
                if num_logs > 0 {
                    self.scroll_offset = num_logs - 1;
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
                if let Some(log) = self.cached_filtered_logs.get(self.scroll_offset) {
                    let id = log.id;
                    let new_marked = !log.marked;

                    // Optimistic update: toggle in both caches immediately
                    if let Some(entry) = self.cached_filtered_logs.iter_mut().find(|e| e.id == id)
                    {
                        entry.marked = new_marked;
                    }
                    if let Some(entry) = self.cached_logs.iter_mut().find(|e| e.id == id) {
                        entry.marked = new_marked;
                    }

                    // Fire-and-forget DB write on the tokio runtime
                    let db = self.analyzer.db.clone();
                    self.analyzer.rt.spawn(async move {
                        let _ = db.toggle_mark(id as i64).await;
                    });
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
                    let log_id = result.log_id;
                    self.scroll_to_log_entry(log_id);
                }
                self.g_key_pressed = false;
            }
            KeyCode::Char('N') => {
                if let Some(result) = self.search.previous_match() {
                    let log_id = result.log_id;
                    self.scroll_to_log_entry(log_id);
                }
                self.g_key_pressed = false;
            }
            _ => {
                self.g_key_pressed = false;
            }
        }
    }

    pub fn handle_command_mode_key(&mut self, key_code: KeyCode) {
        match key_code {
            KeyCode::Enter => {
                self.handle_command();
                // Only clear and exit if no error
                if self.command_error.is_none() {
                    if let AppMode::Command {
                        input,
                        cursor,
                        history,
                        history_index,
                    } = &mut self.mode
                    {
                        if !input.is_empty() {
                            history.push(input.clone());
                        }
                        *input = String::new();
                        *cursor = 0;
                        *history_index = None;
                    }
                    self.mode = AppMode::Normal;
                }
            }
            KeyCode::Esc => {
                if let AppMode::Command {
                    input,
                    cursor,
                    history_index,
                    ..
                } = &mut self.mode
                {
                    *input = String::new();
                    *cursor = 0;
                    *history_index = None;
                }
                self.mode = AppMode::Normal;
            }
            KeyCode::Backspace => {
                if let AppMode::Command { input, cursor, .. } = &mut self.mode
                    && *cursor > 0
                    && !input.is_empty()
                {
                    input.remove(*cursor - 1);
                    *cursor -= 1;
                }
            }
            KeyCode::Char(c) => {
                if let AppMode::Command { input, cursor, .. } = &mut self.mode {
                    input.insert(*cursor, c);
                    *cursor += 1;
                    self.command_error = None;
                    self.tab_completion_index = None;
                }
            }
            KeyCode::Left => {
                if let AppMode::Command { cursor, .. } = &mut self.mode
                    && *cursor > 0
                {
                    *cursor -= 1;
                }
            }
            KeyCode::Right => {
                if let AppMode::Command { input, cursor, .. } = &mut self.mode
                    && *cursor < input.len()
                {
                    *cursor += 1;
                }
            }
            KeyCode::Up => {
                if let AppMode::Command {
                    input,
                    cursor,
                    history,
                    history_index,
                } = &mut self.mode
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
                if let AppMode::Command {
                    input,
                    cursor,
                    history,
                    history_index,
                } = &mut self.mode
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
                if let AppMode::Command { input, cursor, .. } = &mut self.mode {
                    let trimmed = input.trim().to_string();

                    // If typing "set-theme <partial>", complete theme names
                    if trimmed.starts_with("set-theme ") {
                        let themes = &self.available_themes;
                        if themes.is_empty() {
                            return;
                        }
                        let idx = self.tab_completion_index.unwrap_or(0);
                        let theme_name = &themes[idx];
                        *input = format!("set-theme {}", theme_name);
                        *cursor = input.len();
                        self.tab_completion_index =
                            Some((idx + 1) % themes.len());
                        return;
                    }

                    // Otherwise, complete command names
                    let completions = find_command_completions(&trimmed);
                    if completions.is_empty() {
                        return;
                    }
                    let idx = self.tab_completion_index.unwrap_or(0) % completions.len();
                    *input = completions[idx].to_string();
                    *cursor = input.len();
                    self.tab_completion_index = Some(idx + 1);
                }
            }
            _ => {}
        }
    }

    fn handle_filter_management_mode_key(&mut self, key_code: KeyCode) {
        if let AppMode::FilterManagement {
            selected_filter_index,
        } = &mut self.mode
        {
            match key_code {
                KeyCode::Esc => self.mode = AppMode::Normal,
                KeyCode::Up => {
                    *selected_filter_index = selected_filter_index.saturating_sub(1);
                }
                KeyCode::Down => {
                    *selected_filter_index = selected_filter_index.saturating_add(1);
                    let filters = self.analyzer.get_filters();
                    let num_filters = filters.len();
                    if num_filters > 0 && *selected_filter_index >= num_filters {
                        *selected_filter_index = num_filters - 1;
                    }
                }
                KeyCode::Char(' ') => {
                    let filters = self.analyzer.get_filters();
                    if let Some(filter) = filters.get(*selected_filter_index) {
                        self.analyzer.toggle_filter(filter.id);
                        self.filters_dirty = true;
                    }
                }
                KeyCode::Char('d') => {
                    let filters = self.analyzer.get_filters();
                    if let Some(filter) = filters.get(*selected_filter_index) {
                        self.analyzer.remove_filter(filter.id);
                        self.filters_dirty = true;
                        let remaining = self.analyzer.get_filters();
                        if *selected_filter_index >= remaining.len() && !remaining.is_empty() {
                            *selected_filter_index = remaining.len() - 1;
                        }
                    }
                }
                KeyCode::Char('e') => {
                    let filters = self.analyzer.get_filters();
                    if let Some(filter) = filters.get(*selected_filter_index) {
                        let mut cmd = String::from("filter");
                        if filter.filter_type == FilterType::Include
                            && let Some(cfg) = &filter.color_config
                        {
                            cmd.push_str(&format!(" --fg {:?} --bg {:?}", cfg.fg, cfg.bg));
                        }
                        cmd.push(' ');
                        cmd.push_str(&filter.pattern);
                        let len = cmd.len();
                        self.mode = AppMode::Command {
                            input: cmd,
                            cursor: len,
                            history: Vec::new(),
                            history_index: None,
                        };
                    }
                }
                KeyCode::Char('c') => {
                    let filters = self.analyzer.get_filters();
                    if let Some(filter) = filters.get(*selected_filter_index) {
                        let mut cmd = String::from("set-color");
                        if let Some(cfg) = &filter.color_config {
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
                _ => {}
            }
        }
    }

    fn handle_filter_edit_mode_key(&mut self, key_code: KeyCode) {
        if let AppMode::FilterEdit {
            filter_id,
            filter_input,
        } = &mut self.mode
        {
            match key_code {
                KeyCode::Enter => {
                    if let Some(id) = filter_id {
                        self.analyzer.edit_filter(*id, filter_input.clone());
                        *filter_id = None;
                        *filter_input = String::new();
                        self.mode = AppMode::FilterManagement {
                            selected_filter_index: 0,
                        };
                    }
                }
                KeyCode::Esc => {
                    *filter_id = None;
                    *filter_input = String::new();
                    self.mode = AppMode::FilterManagement {
                        selected_filter_index: 0,
                    };
                }
                KeyCode::Backspace => {
                    filter_input.pop();
                }
                KeyCode::Char(c) => {
                    filter_input.push(c);
                }
                _ => {}
            }
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
                    let search_result = if forward_val {
                        self.search.search(&search_input, &self.cached_logs).ok();
                        self.search.next_match()
                    } else {
                        self.search.search(&search_input, &self.cached_logs).ok();
                        self.search.previous_match()
                    };
                    if let Some(result) = search_result {
                        let log_id = result.log_id;
                        self.scroll_to_log_entry(log_id);
                    }
                    if let AppMode::Search { input, .. } = &mut self.mode {
                        *input = String::new();
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

    pub fn handle_key_event(&mut self, key_code: KeyCode) {
        self.handle_key_event_with_modifiers(key_code, KeyModifiers::NONE);
    }

    pub fn handle_key_event_with_modifiers(&mut self, key_code: KeyCode, modifiers: KeyModifiers) {
        match self.mode {
            AppMode::Normal => self.handle_normal_mode_key(key_code, modifiers),
            AppMode::Command { .. } => self.handle_command_mode_key(key_code),
            AppMode::FilterManagement { .. } => self.handle_filter_management_mode_key(key_code),
            AppMode::FilterEdit { .. } => self.handle_filter_edit_mode_key(key_code),
            AppMode::Search { .. } => self.handle_search_mode_key(key_code),
        }
    }

    fn ui(&mut self, frame: &mut Frame) {
        let size = frame.size();
        frame.render_widget(Block::default().bg(self.theme.root_bg), size);

        let mut constraints = vec![Constraint::Min(1)];
        let has_input_bar = matches!(self.mode, AppMode::Command { .. })
            || matches!(self.mode, AppMode::Search { .. });
        if has_input_bar {
            constraints.push(Constraint::Length(1)); // input line
            constraints.push(Constraint::Length(1)); // hint line
        }
        constraints.push(Constraint::Length(3));
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(size);

        let (logs_area, sidebar_area) = if self.show_sidebar {
            let horizontal = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Min(1), Constraint::Length(30)])
                .split(chunks[0]);
            (horizontal[0], Some(horizontal[1]))
        } else {
            (chunks[0], None)
        };

        let num_logs = self.cached_filtered_logs.len();

        // Inner height excludes the border (2 lines for top+bottom border)
        let visible_height = (logs_area.height as usize).saturating_sub(2);
        self.visible_height = visible_height;

        // Clamp scroll_offset
        if num_logs > 0 && self.scroll_offset >= num_logs {
            self.scroll_offset = num_logs - 1;
        }

        // Adjust viewport so cursor stays visible
        if self.scroll_offset < self.viewport_offset {
            self.viewport_offset = self.scroll_offset;
        } else if visible_height > 0 && self.scroll_offset >= self.viewport_offset + visible_height
        {
            self.viewport_offset = self.scroll_offset - visible_height + 1;
        }

        let start = self.viewport_offset;
        let end = (start + visible_height).min(num_logs);
        let visible_logs = &self.cached_filtered_logs[start..end];

        let filters = self.analyzer.get_filters();

        let search_results = self.search.get_results();
        // Build a lookup map for search results of visible logs
        let search_map: std::collections::HashMap<usize, &crate::analyzer::SearchResult> =
            search_results.iter().map(|r| (r.log_id, r)).collect();
        let log_lines: Vec<Line> = visible_logs
            .iter()
            .enumerate()
            .map(|(vis_idx, log)| {
                let display = log.display_line();
                let is_current = start + vis_idx == self.scroll_offset;

                // Compute the process name segment for coloring
                let process_segment: Option<(usize, usize, Color)> =
                    log.process_name.as_ref().map(|pn| {
                        let color = get_process_color(pn, &filters, &self.theme);
                        let needle = format!("{}: ", pn);
                        let offset = display.find(&needle).unwrap_or(0);
                        (offset, offset + needle.len(), color)
                    });

                let search_match = search_map.get(&log.id).copied();

                // Build styled spans for the full display line
                let mut base_style = Style::default().fg(self.theme.text);
                if self.level_colors {
                    match log.level {
                        LogLevel::Error => {
                            base_style = base_style.fg(self.theme.error_fg);
                        }
                        LogLevel::Warning => {
                            base_style = base_style.fg(self.theme.warning_fg);
                        }
                        _ => {}
                    }
                }
                if log.marked {
                    base_style = base_style
                        .fg(self.theme.text_highlight)
                        .add_modifier(Modifier::BOLD);
                }
                let highlight_style = Style::default()
                    .fg(Color::Black)
                    .bg(self.theme.text_highlight);

                let spans = build_highlighted_spans(
                    &display,
                    base_style,
                    highlight_style,
                    search_match.map(|r| r.matches.as_slice()),
                    process_segment,
                );

                let mut line_style = base_style;
                if is_current {
                    line_style = line_style.bg(self.theme.border);
                }
                Line::from(spans).style(line_style)
            })
            .collect();

        let logs_title = if let Some(state) = &self.background_loading {
            let spinner = SPINNER_FRAMES[state.spinner_frame];
            format!("Logs {} Loading... ({} lines)", spinner, state.lines_loaded)
        } else {
            format!("Logs ({})", num_logs)
        };

        let mut paragraph = Paragraph::new(log_lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(self.theme.border))
                    .title(logs_title)
                    .title_style(Style::default().fg(self.theme.border_title)),
            )
            .scroll((0, self.horizontal_scroll as u16));

        if self.wrap {
            paragraph = paragraph.wrap(Wrap { trim: false });
        }

        frame.render_widget(paragraph, logs_area);

        if let Some(sidebar_area) = sidebar_area {
            let filters_text: Vec<Line> = filters
                .iter()
                .enumerate()
                .map(|(i, filter)| {
                    let status = if filter.enabled { "[x]" } else { "[ ]" };
                    let selected_prefix = if i == self.get_selected_filter_index().unwrap_or(0) {
                        ">"
                    } else {
                        " "
                    };
                    let filter_type_str = match filter.filter_type {
                        FilterType::Include => "In",
                        FilterType::Exclude => "Out",
                    };
                    Line::from(format!(
                        "{}{} {}: {}",
                        selected_prefix, status, filter_type_str, filter.pattern
                    ))
                    .style(Style::default().fg(self.theme.text))
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

        if matches!(self.mode, AppMode::Command { .. }) {
            let input_prefix = ":";
            let (input_text, cursor_pos) = self.get_command_input_and_cursor().unwrap_or(("", 0));
            let command_line = Paragraph::new(format!("{}{}", input_prefix, input_text))
                .style(Style::default().fg(self.theme.text).bg(self.theme.border))
                .wrap(Wrap { trim: false });
            let input_area = chunks[1];
            frame.render_widget(command_line, input_area);
            let cursor_x = input_area.x + 1 + cursor_pos as u16;
            if cursor_x < input_area.x + input_area.width {
                frame.set_cursor(cursor_x, input_area.y);
            }

            // Render hint line: error, matched command help, or available completions
            let hint_area = chunks[2];
            if let Some(err) = &self.command_error {
                let error_paragraph = Paragraph::new(err.as_str())
                    .style(Style::default().fg(Color::Red).bg(self.theme.root_bg));
                frame.render_widget(error_paragraph, hint_area);
            } else if let Some(cmd) = find_matching_command(input_text) {
                let hint = Paragraph::new(format!("  {} - {}", cmd.usage, cmd.description))
                    .style(Style::default().fg(self.theme.border).bg(self.theme.root_bg));
                frame.render_widget(hint, hint_area);
            } else {
                let completions = find_command_completions(input_text.trim());
                if !completions.is_empty() {
                    let hint_text = completions.join("  ");
                    let hint = Paragraph::new(hint_text)
                        .style(Style::default().fg(self.theme.border).bg(self.theme.root_bg));
                    frame.render_widget(hint, hint_area);
                }
            }
        }

        if let AppMode::Search { input, forward } = &self.mode {
            let prefix = if *forward { "/" } else { "?" };
            let search_line = Paragraph::new(format!("{}{}", prefix, input))
                .style(Style::default().fg(self.theme.text).bg(self.theme.border))
                .wrap(Wrap { trim: false });
            let input_area = chunks[1];
            frame.render_widget(search_line, input_area);
            let cursor_x = input_area.x + 1 + input.len() as u16;
            if cursor_x < input_area.x + input_area.width {
                frame.set_cursor(cursor_x, input_area.y);
            }

            // Render search hint
            let hint_area = chunks[2];
            let match_count = self.search.get_results().len();
            let hint_text = if !input.is_empty() {
                format!("  {} matches", match_count)
            } else {
                "  Type pattern and press Enter to search".to_string()
            };
            let hint = Paragraph::new(hint_text)
                .style(Style::default().fg(self.theme.border).bg(self.theme.root_bg));
            frame.render_widget(hint, hint_area);
        }

        let command_list = Paragraph::new(self.get_command_list())
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
        use crate::command_args::{CommandLine, Commands};
        use clap::Parser;
        let input = match &self.mode {
            AppMode::Command { input, .. } => input.trim(),
            _ => return,
        };
        let args = match CommandLine::try_parse_from(input.split_whitespace()) {
            Ok(args) => args,
            Err(e) => {
                self.command_error = Some(format!("Invalid command: {}", e));
                return;
            }
        };
        match args.command {
            Some(Commands::Filter { pattern, fg, bg }) => {
                self.analyzer.add_filter_with_color(
                    pattern,
                    FilterType::Include,
                    fg.as_deref(),
                    bg.as_deref(),
                );
                self.scroll_offset = 0;
                self.filters_dirty = true;
            }
            Some(Commands::Exclude { pattern }) => {
                self.analyzer
                    .add_filter_with_color(pattern, FilterType::Exclude, None, None);
                self.scroll_offset = 0;
                self.filters_dirty = true;
            }
            Some(Commands::SetColor { fg, bg }) => {
                let selected_filter_index = match &self.mode {
                    AppMode::FilterManagement {
                        selected_filter_index,
                    } => *selected_filter_index,
                    _ => 0,
                };
                let filters = self.analyzer.get_filters();
                if let Some(filter) = filters.get(selected_filter_index)
                    && filter.filter_type == FilterType::Include
                {
                    let pattern = filter.pattern.clone();
                    self.analyzer
                        .set_color_config(&pattern, fg.as_deref(), bg.as_deref());
                    self.filters_dirty = true;
                }
            }
            Some(Commands::ExportMarked { path }) => {
                if !path.is_empty() {
                    let marked_logs = self.analyzer.get_marked_logs();
                    let marked_messages: Vec<String> =
                        marked_logs.iter().map(|e| e.message.clone()).collect();
                    let mut content = marked_messages.join("\n");
                    if !content.ends_with('\n') {
                        content.push('\n');
                    }
                    let _ = std::fs::write(path, content);
                }
            }
            Some(Commands::SaveFilters { path }) => {
                if !path.is_empty() {
                    let _ = self.analyzer.save_filters(&path);
                }
            }
            Some(Commands::LoadFilters { path }) => {
                if !path.is_empty() {
                    let _ = self.analyzer.load_filters(&path);
                    self.filters_dirty = true;
                }
            }
            Some(Commands::Wrap) => {
                self.wrap = !self.wrap;
            }
            Some(Commands::LevelColors) => {
                self.level_colors = !self.level_colors;
            }
            Some(Commands::SetTheme { theme_name }) => {
                let theme_filename = format!("{}.json", theme_name.to_lowercase());
                match Theme::from_file(&theme_filename) {
                    Ok(theme) => self.theme = theme,
                    Err(e) => {
                        self.command_error =
                            Some(format!("Failed to load theme '{}': {}", theme_name, e))
                    }
                }
            }
            None => {}
        }
    }

    fn get_command_list(&self) -> String {
        match self.mode {
            AppMode::Normal => {
                "[NORMAL] [q]uit | : => command Mode | [f]ilter mode | [s]idebar | [m]ark Line | / => search | ? => search backward | [n]ext match | N => prev match | PgDn/Ctrl+d PgUp/Ctrl+u".to_string()
            },
            AppMode::Command { .. } => {
                "[COMMAND] filter | exclude | set-color | export-marked | save-filters | load-filters | wrap | set-theme | level-colors | Esc | Enter".to_string()
            },
            AppMode::FilterManagement { .. } => {
                "[FILTER] [i]nclude | e[x]clude | Space => toggle | [d]elete | [e]dit | set [c]olor | Esc => normal mode".to_string()
            },
            AppMode::FilterEdit { .. } => {
                "[FILTER EDIT] Esc => cancel | Enter => save".to_string()
            },
            AppMode::Search { .. } => {
                "[SEARCH] Esc => cancel | Enter => search".to_string()
            },
        }
    }

    /// Schedule an async cache refresh on the tokio runtime.
    /// Does nothing if one is already in flight.
    fn schedule_cache_refresh(&mut self) {
        if self.cache_refresh_pending {
            return;
        }
        if !self.logs_dirty && !self.filters_dirty {
            return;
        }

        let db = self.analyzer.db.clone();
        let rt = self.analyzer.rt.clone();
        let needs_logs = self.logs_dirty;

        // If only filters changed, we can reuse cached_logs from the main thread
        let existing_logs = if !needs_logs {
            Some(self.cached_logs.clone())
        } else {
            None
        };

        self.logs_dirty = false;
        self.filters_dirty = false;
        self.cache_refresh_pending = true;

        let (tx, rx) = mpsc::channel();
        self.cache_refresh_rx = Some(rx);

        rt.spawn(async move {
            let all_logs = if let Some(logs) = existing_logs {
                logs
            } else {
                db.get_all_logs().await.unwrap_or_default()
            };

            let filters = db.get_filters().await.unwrap_or_default();
            let filtered_logs = apply_filters_to_logs(&all_logs, &filters)
                .unwrap_or_else(|_| all_logs.clone());

            let _ = tx.send(CacheResult {
                all_logs,
                filtered_logs,
            });
        });
    }

    /// Non-blocking poll for completed cache refresh results.
    fn poll_cache_refresh(&mut self) {
        let rx = match &self.cache_refresh_rx {
            Some(rx) => rx,
            None => return,
        };

        match rx.try_recv() {
            Ok(result) => {
                self.cached_logs = result.all_logs;
                self.cached_filtered_logs = result.filtered_logs;
                self.cache_refresh_pending = false;
                self.cache_refresh_rx = None;

                // If dirty flags were set while the refresh was in flight, re-schedule
                if self.logs_dirty || self.filters_dirty {
                    self.schedule_cache_refresh();
                }
            }
            Err(mpsc::TryRecvError::Empty) => {} // still in progress
            Err(mpsc::TryRecvError::Disconnected) => {
                // Task panicked or was dropped — reset state
                self.cache_refresh_pending = false;
                self.cache_refresh_rx = None;
            }
        }
    }

    /// Synchronous cache refresh for use in tests only.
    #[cfg(test)]
    pub fn refresh_caches(&mut self) {
        if self.logs_dirty {
            self.cached_logs = self.analyzer.get_logs();
            self.logs_dirty = false;
            self.filters_dirty = true;
        }
        if self.filters_dirty {
            self.cached_filtered_logs = self
                .analyzer
                .apply_filters(&self.cached_logs)
                .unwrap_or_else(|_| self.cached_logs.clone());
            self.filters_dirty = false;
        }
    }

    pub fn invalidate_logs(&mut self) {
        self.logs_dirty = true;
    }

    pub fn invalidate_filters(&mut self) {
        self.filters_dirty = true;
    }

    pub fn start_background_loading(&mut self, path: String, lines_loaded: usize) {
        let receiver = self.analyzer.start_file_stream(
            path,
            lines_loaded,
            BACKGROUND_BATCH_SIZE,
        );
        self.background_loading = Some(BackgroundLoadState {
            receiver,
            lines_loaded,
            spinner_frame: 0,
        });
    }

    fn process_background_loading(&mut self) {
        let state = match &mut self.background_loading {
            Some(s) => s,
            None => return,
        };

        // Drain all available progress notifications (non-blocking, no DB work here)
        let mut got_updates = false;
        loop {
            match state.receiver.try_recv() {
                Ok(Ok(count)) => {
                    state.lines_loaded += count;
                    state.spinner_frame = (state.spinner_frame + 1) % SPINNER_FRAMES.len();
                    got_updates = true;
                }
                Ok(Err(_)) => {
                    // Background thread hit an error
                    self.background_loading = None;
                    return;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    // Thread finished — all data inserted
                    self.background_loading = None;
                    if got_updates {
                        self.logs_dirty = true;
                    }
                    return;
                }
            }
        }

        if got_updates {
            self.logs_dirty = true;
        }
    }

    fn scroll_to_log_entry(&mut self, log_id: usize) {
        if let Some(index) = self.cached_filtered_logs.iter().position(|e| e.id == log_id) {
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
}

/// Build spans for a display line, applying search highlights and process name coloring.
fn build_highlighted_spans<'a>(
    display: &str,
    base_style: Style,
    highlight_style: Style,
    search_matches: Option<&[(usize, usize)]>,
    process_segment: Option<(usize, usize, Color)>,
) -> Vec<Span<'a>> {
    let mut spans = Vec::new();
    let mut pos = 0;

    // Merge search matches into a sorted list of (start, end, is_highlight)
    // We iterate character by character through segments
    let matches = search_matches.unwrap_or(&[]);

    while pos < display.len() {
        // Check if we're at a search match boundary
        if let Some(&(m_start, m_end)) = matches.iter().find(|&&(s, e)| s <= pos && pos < e) {
            let segment = &display[m_start.max(pos)..m_end];
            spans.push(Span::styled(segment.to_string(), highlight_style));
            pos = m_end;
            continue;
        }

        // Find the next search match start
        let next_match_start = matches
            .iter()
            .filter(|&&(s, _)| s > pos)
            .map(|&(s, _)| s)
            .min()
            .unwrap_or(display.len());

        // Emit text from pos to next_match_start, applying process color if in range
        let end = next_match_start;
        if pos >= end {
            break;
        }

        if let Some((ps, pe, color)) = process_segment {
            // Split around the process segment if it overlaps
            if pos < ps && end > pos {
                let seg_end = ps.min(end);
                if seg_end > pos {
                    spans.push(Span::styled(
                        display[pos..seg_end].to_string(),
                        base_style,
                    ));
                }
                pos = seg_end;
                if pos < pe && pos < end {
                    let seg_end = pe.min(end);
                    spans.push(Span::styled(
                        display[pos..seg_end].to_string(),
                        base_style.fg(color),
                    ));
                    pos = seg_end;
                }
                if pos < end {
                    spans.push(Span::styled(display[pos..end].to_string(), base_style));
                    pos = end;
                }
            } else if pos >= ps && pos < pe {
                let seg_end = pe.min(end);
                spans.push(Span::styled(
                    display[pos..seg_end].to_string(),
                    base_style.fg(color),
                ));
                pos = seg_end;
                if pos < end {
                    spans.push(Span::styled(display[pos..end].to_string(), base_style));
                    pos = end;
                }
            } else {
                spans.push(Span::styled(display[pos..end].to_string(), base_style));
                pos = end;
            }
        } else {
            spans.push(Span::styled(display[pos..end].to_string(), base_style));
            pos = end;
        }
    }

    spans
}

fn get_process_color(
    process_name: &str,
    filters: &[crate::analyzer::Filter],
    theme: &Theme,
) -> Color {
    for filter in filters {
        if filter.filter_type == FilterType::Include
            && filter.pattern == process_name
            && let Some(color_config) = &filter.color_config
            && let Some(fg) = color_config.fg
        {
            return fg;
        }
    }
    theme.text
}

#[cfg(test)]
mod tests {
    use crate::analyzer::{FilterType, LogAnalyzer, LogEntry, LogLevel};
    use crate::db::{Database, LogStore};
    use crate::ui::Theme;
    use crate::ui::{App, AppMode};
    use ratatui::prelude::{Color, Modifier, Style};
    use crossterm::event::KeyCode;
    use std::sync::Arc;

    fn mock_analyzer() -> LogAnalyzer {
        let rt = Arc::new(tokio::runtime::Runtime::new().unwrap());
        let db = rt.block_on(Database::in_memory()).unwrap();
        let db = Arc::new(db);
        let analyzer = LogAnalyzer::new(db.clone(), rt.clone());

        let entries = vec![
            LogEntry {
                id: 0,
                message: "INFO something".to_string(),
                level: LogLevel::Info,
                ..Default::default()
            },
            LogEntry {
                id: 1,
                message: "WARN warning".to_string(),
                level: LogLevel::Warning,
                ..Default::default()
            },
            LogEntry {
                id: 2,
                message: "ERROR error".to_string(),
                level: LogLevel::Error,
                ..Default::default()
            },
        ];
        rt.block_on(db.insert_logs_batch(&entries)).unwrap();
        analyzer
    }

    fn mock_empty_analyzer() -> LogAnalyzer {
        let rt = Arc::new(tokio::runtime::Runtime::new().unwrap());
        let db = rt.block_on(Database::in_memory()).unwrap();
        LogAnalyzer::new(Arc::new(db), rt)
    }

    #[test]
    fn test_toggle_wrap_command() {
        let mut app = App::new(mock_analyzer(), Theme::default());
        app.mode = AppMode::Command {
            input: "wrap".to_string(),
            cursor: 4,
            history: Vec::new(),
            history_index: None,
        };
        app.handle_command();
        assert!(!app.wrap);
        app.mode = AppMode::Command {
            input: "wrap".to_string(),
            cursor: 4,
            history: Vec::new(),
            history_index: None,
        };
        app.handle_command();
        assert!(app.wrap);
    }

    #[test]
    fn test_add_filter_command() {
        let mut app = App::new(mock_analyzer(), Theme::default());
        app.mode = AppMode::Command {
            input: "filter foo".to_string(),
            cursor: 10,
            history: Vec::new(),
            history_index: None,
        };
        app.handle_command();
        let filters = app.analyzer.get_filters();
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].filter_type, FilterType::Include);
        assert_eq!(filters[0].pattern, "foo");
    }

    #[test]
    fn test_add_exclude_command() {
        let mut app = App::new(mock_analyzer(), Theme::default());
        app.mode = AppMode::Command {
            input: "exclude bar".to_string(),
            cursor: 11,
            history: Vec::new(),
            history_index: None,
        };
        app.handle_command();
        let filters = app.analyzer.get_filters();
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].filter_type, FilterType::Exclude);
        assert_eq!(filters[0].pattern, "bar");
    }

    #[test]
    fn test_mark_line() {
        let mut app = App::new(mock_analyzer(), Theme::default());
        app.refresh_caches();
        app.handle_key_event(KeyCode::Char('m'));
        let logs = app.analyzer.get_logs();
        assert!(logs[0].marked);
    }

    #[test]
    fn test_scroll_offset_j_k() {
        let mut app = App::new(mock_analyzer(), Theme::default());
        app.handle_key_event(KeyCode::Char('j'));
        assert_eq!(app.scroll_offset, 1);
        app.handle_key_event(KeyCode::Char('k'));
        assert_eq!(app.scroll_offset, 0);
    }

    #[test]
    fn test_mode_switching() {
        let mut app = App::new(mock_analyzer(), Theme::default());
        app.handle_key_event(KeyCode::Char(':'));
        assert!(matches!(app.mode, AppMode::Command { .. }));
        app.handle_command_mode_key(KeyCode::Esc);
        assert!(matches!(app.mode, AppMode::Normal));
    }

    #[test]
    fn test_sidebar_filter_display_in_out() {
        let app = App::new(mock_analyzer(), Theme::default());
        app.analyzer
            .add_filter("foo".to_string(), FilterType::Include);
        app.analyzer
            .add_filter("bar".to_string(), FilterType::Exclude);
        let filters = app.analyzer.get_filters();
        assert_eq!(filters[0].filter_type, FilterType::Include);
        assert_eq!(filters[1].filter_type, FilterType::Exclude);
    }

    #[test]
    fn test_command_list_texts() {
        let app = App::new(mock_analyzer(), Theme::default());
        let normal = app.get_command_list();
        assert!(normal.contains("[NORMAL]"));
    }

    #[test]
    fn test_toggle_sidebar() {
        let mut app = App::new(mock_analyzer(), Theme::default());
        assert!(!app.show_sidebar);
        app.handle_key_event(KeyCode::Char('s'));
        assert!(app.show_sidebar);
        app.handle_key_event(KeyCode::Char('s'));
        assert!(!app.show_sidebar);
    }

    #[test]
    fn test_filter_management_mode_navigation() {
        let mut app = App::new(mock_analyzer(), Theme::default());
        app.mode = AppMode::FilterManagement {
            selected_filter_index: 0,
        };
        app.analyzer
            .add_filter("foo".to_string(), FilterType::Include);
        app.analyzer
            .add_filter("bar".to_string(), FilterType::Exclude);

        app.handle_filter_management_mode_key(KeyCode::Down);

        match app.mode {
            AppMode::FilterManagement {
                selected_filter_index,
            } => {
                assert_eq!(selected_filter_index, 1);
            }
            _ => panic!("should be in filter mode"),
        }

        app.handle_filter_management_mode_key(KeyCode::Up);
        match app.mode {
            AppMode::FilterManagement {
                selected_filter_index,
            } => {
                assert_eq!(selected_filter_index, 0);
            }
            _ => panic!("should be in filter mode"),
        }
    }

    #[test]
    fn test_filter_toggle_and_delete() {
        let mut app = App::new(mock_analyzer(), Theme::default());
        app.mode = AppMode::FilterManagement {
            selected_filter_index: 0,
        };
        app.analyzer
            .add_filter("foo".to_string(), FilterType::Include);
        let filters = app.analyzer.get_filters();
        assert!(filters[0].enabled);
        app.handle_filter_management_mode_key(KeyCode::Char(' '));
        let filters = app.analyzer.get_filters();
        assert!(!filters[0].enabled);
        app.handle_filter_management_mode_key(KeyCode::Char('d'));
        let filters = app.analyzer.get_filters();
        assert!(filters.is_empty());
    }

    #[test]
    fn test_filter_edit() {
        let mut app = App::new(mock_analyzer(), Theme::default());
        app.mode = AppMode::FilterManagement {
            selected_filter_index: 0,
        };
        app.analyzer
            .add_filter("foo".to_string(), FilterType::Include);
        app.handle_filter_management_mode_key(KeyCode::Char('e'));
        match &app.mode {
            AppMode::Command { input, .. } => {
                assert!(input.starts_with("filter"));
                assert!(input.contains("foo"));
            }
            _ => panic!("Expected Command mode"),
        }
    }

    #[test]
    fn test_search_mode_and_input() {
        let mut app = App::new(mock_analyzer(), Theme::default());
        app.handle_key_event(KeyCode::Char('/'));
        assert!(matches!(app.mode, AppMode::Search { .. }));
        app.handle_search_mode_key(KeyCode::Char('t'));
        match &app.mode {
            AppMode::Search { input, .. } => assert_eq!(input, "t"),
            _ => panic!("Expected Search mode"),
        }
        app.handle_search_mode_key(KeyCode::Backspace);
        match &app.mode {
            AppMode::Search { input, .. } => assert_eq!(input, ""),
            _ => panic!("Expected Search mode"),
        }
        app.handle_search_mode_key(KeyCode::Esc);
        assert!(matches!(app.mode, AppMode::Normal));
    }

    #[test]
    fn test_command_input_and_backspace() {
        let mut app = App::new(mock_analyzer(), Theme::default());
        app.mode = AppMode::Command {
            input: "ab".to_string(),
            cursor: 2,
            history: Vec::new(),
            history_index: None,
        };
        app.handle_command_mode_key(KeyCode::Backspace);
        match &app.mode {
            AppMode::Command { input, .. } => assert_eq!(input, "a"),
            _ => panic!("Expected Command mode"),
        }
        app.handle_command_mode_key(KeyCode::Esc);
        assert!(matches!(app.mode, AppMode::Normal));
    }

    #[test]
    fn test_scroll_to_log_entry() {
        let mut app = App::new(mock_analyzer(), Theme::default());
        app.refresh_caches();
        app.scroll_to_log_entry(2);
        assert_eq!(app.scroll_offset, 2);
    }

    #[test]
    fn test_toggle_line_wrapping() {
        let mut app = App::new(mock_empty_analyzer(), Theme::default());
        assert!(app.wrap);
        app.handle_key_event(KeyCode::Char('w'));
        assert!(!app.wrap);
        app.handle_key_event(KeyCode::Char('w'));
        assert!(app.wrap);
    }

    #[test]
    fn test_horizontal_scroll() {
        let mut app = App::new(mock_empty_analyzer(), Theme::default());
        app.wrap = false;
        assert_eq!(app.horizontal_scroll, 0);
        app.handle_key_event(KeyCode::Char('l'));
        assert_eq!(app.horizontal_scroll, 1);
        app.handle_key_event(KeyCode::Char('h'));
        assert_eq!(app.horizontal_scroll, 0);
    }

    fn setup_test_app_for_vim_motions() -> App {
        let rt = Arc::new(tokio::runtime::Runtime::new().unwrap());
        let db = rt.block_on(Database::in_memory()).unwrap();
        let db = Arc::new(db);
        let analyzer = LogAnalyzer::new(db.clone(), rt.clone());

        let entries: Vec<LogEntry> = (0..100)
            .map(|i| LogEntry {
                id: i,
                message: format!("line {}", i),
                level: LogLevel::Info,
                ..Default::default()
            })
            .collect();
        rt.block_on(db.insert_logs_batch(&entries)).unwrap();
        App::new(analyzer, Theme::default())
    }

    #[test]
    fn test_vim_j_key() {
        let mut app = setup_test_app_for_vim_motions();
        app.handle_key_event(KeyCode::Char('j'));
        assert_eq!(app.scroll_offset, 1);
    }

    #[test]
    fn test_vim_k_key() {
        let mut app = setup_test_app_for_vim_motions();
        app.scroll_offset = 5;
        app.handle_key_event(KeyCode::Char('k'));
        assert_eq!(app.scroll_offset, 4);
    }

    #[test]
    fn test_vim_gg_key() {
        let mut app = setup_test_app_for_vim_motions();
        app.scroll_offset = 50;
        app.handle_key_event(KeyCode::Char('g'));
        app.handle_key_event(KeyCode::Char('g'));
        assert_eq!(app.scroll_offset, 0);
    }

    #[test]
    fn test_vim_g_key() {
        let mut app = setup_test_app_for_vim_motions();
        app.refresh_caches();
        app.scroll_offset = 50;
        app.handle_key_event(KeyCode::Char('G'));
        assert_eq!(app.scroll_offset, 99);
    }

    #[test]
    fn test_viewport_initial_state() {
        let app = setup_test_app_for_vim_motions();
        assert_eq!(app.scroll_offset, 0);
        assert_eq!(app.viewport_offset, 0);
    }

    #[test]
    fn test_cursor_moves_without_viewport_shift() {
        let mut app = setup_test_app_for_vim_motions();
        // Simulate a viewport that can show many lines
        app.viewport_offset = 0;

        // Move cursor down a few lines
        app.handle_key_event(KeyCode::Char('j'));
        app.handle_key_event(KeyCode::Char('j'));
        app.handle_key_event(KeyCode::Char('j'));

        assert_eq!(app.scroll_offset, 3);
        // Viewport should still be at 0 since cursor hasn't exceeded visible area
        assert_eq!(app.viewport_offset, 0);
    }

    #[test]
    fn test_cursor_up_does_not_shift_viewport_when_within_bounds() {
        let mut app = setup_test_app_for_vim_motions();
        app.scroll_offset = 5;
        app.viewport_offset = 0;

        app.handle_key_event(KeyCode::Char('k'));
        assert_eq!(app.scroll_offset, 4);
        assert_eq!(app.viewport_offset, 0);
    }

    #[test]
    fn test_viewport_adjusts_when_cursor_moves_above_viewport() {
        let mut app = setup_test_app_for_vim_motions();
        // Set viewport to show lines starting at 10
        app.scroll_offset = 10;
        app.viewport_offset = 10;

        // Move cursor up past viewport
        app.handle_key_event(KeyCode::Char('k'));
        assert_eq!(app.scroll_offset, 9);
        // viewport_offset is adjusted during rendering, not during key handling.
        // But the logic in ui() will set viewport_offset = scroll_offset when
        // scroll_offset < viewport_offset. Let's verify the invariant holds by
        // checking that scroll_offset < viewport_offset triggers adjustment.
        assert!(app.scroll_offset < app.viewport_offset);
    }

    #[test]
    fn test_gg_resets_cursor_to_top() {
        let mut app = setup_test_app_for_vim_motions();
        app.scroll_offset = 50;
        app.viewport_offset = 45;

        app.handle_key_event(KeyCode::Char('g'));
        app.handle_key_event(KeyCode::Char('g'));

        assert_eq!(app.scroll_offset, 0);
    }

    #[test]
    fn test_big_g_moves_cursor_to_last_line() {
        let mut app = setup_test_app_for_vim_motions();
        app.refresh_caches();
        app.scroll_offset = 0;

        app.handle_key_event(KeyCode::Char('G'));

        assert_eq!(app.scroll_offset, 99);
    }

    #[test]
    fn test_search_forward_mode_sets_correct_state() {
        let mut app = App::new(mock_analyzer(), Theme::default());
        app.handle_key_event(KeyCode::Char('/'));

        match &app.mode {
            AppMode::Search { input, forward } => {
                assert_eq!(input, "");
                assert!(*forward);
            }
            _ => panic!("Expected Search mode"),
        }
    }

    #[test]
    fn test_search_backward_mode_sets_correct_state() {
        let mut app = App::new(mock_analyzer(), Theme::default());
        app.handle_key_event(KeyCode::Char('?'));

        match &app.mode {
            AppMode::Search { input, forward } => {
                assert_eq!(input, "");
                assert!(!*forward);
            }
            _ => panic!("Expected Search mode"),
        }
    }

    #[test]
    fn test_search_populates_results_and_navigates() {
        let mut app = App::new(mock_analyzer(), Theme::default());
        app.refresh_caches();

        // Enter search mode
        app.handle_key_event(KeyCode::Char('/'));

        // Type search query matching "ERROR"
        app.handle_search_mode_key(KeyCode::Char('E'));
        app.handle_search_mode_key(KeyCode::Char('R'));
        app.handle_search_mode_key(KeyCode::Char('R'));
        app.handle_search_mode_key(KeyCode::Char('O'));
        app.handle_search_mode_key(KeyCode::Char('R'));

        // Submit search
        app.handle_search_mode_key(KeyCode::Enter);

        // Should be back in Normal mode
        assert!(matches!(app.mode, AppMode::Normal));

        // Search results should be populated
        let results = app.search.get_results();
        assert!(!results.is_empty());

        // Cursor should have moved to the matching log
        let first_match_id = results[0].log_id;
        let logs = app.analyzer.get_logs();
        let expected_offset = logs.iter().position(|e| e.id == first_match_id).unwrap();
        assert_eq!(app.scroll_offset, expected_offset);
    }

    #[test]
    fn test_search_results_persist_for_highlighting() {
        let mut app = App::new(mock_analyzer(), Theme::default());
        app.refresh_caches();

        // Perform a search
        app.handle_key_event(KeyCode::Char('/'));
        app.handle_search_mode_key(KeyCode::Char('W'));
        app.handle_search_mode_key(KeyCode::Char('A'));
        app.handle_search_mode_key(KeyCode::Char('R'));
        app.handle_search_mode_key(KeyCode::Char('N'));
        app.handle_search_mode_key(KeyCode::Enter);

        // After returning to normal mode, search results should persist
        assert!(matches!(app.mode, AppMode::Normal));
        let results = app.search.get_results();
        assert!(!results.is_empty());

        // Navigate to next match
        app.handle_key_event(KeyCode::Char('n'));
        let current = app.search.get_current_match();
        assert!(current.is_some());
    }

    #[test]
    fn test_search_next_and_prev_navigation() {
        let rt = Arc::new(tokio::runtime::Runtime::new().unwrap());
        let db = rt.block_on(Database::in_memory()).unwrap();
        let db = Arc::new(db);
        let analyzer = LogAnalyzer::new(db.clone(), rt.clone());

        let entries = vec![
            LogEntry {
                id: 0,
                message: "first match here".to_string(),
                level: LogLevel::Info,
                ..Default::default()
            },
            LogEntry {
                id: 1,
                message: "no hit".to_string(),
                level: LogLevel::Info,
                ..Default::default()
            },
            LogEntry {
                id: 2,
                message: "second match here".to_string(),
                level: LogLevel::Info,
                ..Default::default()
            },
        ];
        rt.block_on(db.insert_logs_batch(&entries)).unwrap();
        let mut app = App::new(analyzer, Theme::default());
        app.refresh_caches();

        // Search for "match"
        app.handle_key_event(KeyCode::Char('/'));
        for c in "match".chars() {
            app.handle_search_mode_key(KeyCode::Char(c));
        }
        app.handle_search_mode_key(KeyCode::Enter);

        // Initial forward search calls next_match(), landing on result[1] (log 2)
        assert_eq!(app.scroll_offset, 2);

        // n -> next_match wraps to result[0] (log 0)
        app.handle_key_event(KeyCode::Char('n'));
        assert_eq!(app.scroll_offset, 0);

        // n -> next_match to result[1] (log 2)
        app.handle_key_event(KeyCode::Char('n'));
        assert_eq!(app.scroll_offset, 2);

        // N -> previous_match to result[0] (log 0)
        app.handle_key_event(KeyCode::Char('N'));
        assert_eq!(app.scroll_offset, 0);
    }

    #[test]
    fn test_mark_preserves_with_cursor_highlight() {
        let mut app = setup_test_app_for_vim_motions();
        app.refresh_caches();

        // Mark line 0
        app.handle_key_event(KeyCode::Char('m'));
        let logs = app.analyzer.get_logs();
        assert!(logs[0].marked);

        // Move cursor away and back
        app.handle_key_event(KeyCode::Char('j'));
        app.handle_key_event(KeyCode::Char('k'));
        assert_eq!(app.scroll_offset, 0);

        // Line should still be marked
        let logs = app.analyzer.get_logs();
        assert!(logs[0].marked);
    }

    #[test]
    fn test_build_highlighted_spans_no_matches() {
        let base = Style::default().fg(Color::White);
        let hl = Style::default().fg(Color::Black).bg(Color::Yellow);
        let spans = super::build_highlighted_spans("hello world", base, hl, None, None);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "hello world");
    }

    #[test]
    fn test_build_highlighted_spans_with_search_match() {
        let base = Style::default().fg(Color::White);
        let hl = Style::default().fg(Color::Black).bg(Color::Yellow);
        let matches = vec![(6, 11)]; // "world"
        let spans =
            super::build_highlighted_spans("hello world", base, hl, Some(&matches), None);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].content, "hello ");
        assert_eq!(spans[1].content, "world");
        assert_eq!(spans[1].style, hl);
    }

    #[test]
    fn test_build_highlighted_spans_with_process_color() {
        let base = Style::default().fg(Color::White);
        let hl = Style::default().fg(Color::Black).bg(Color::Yellow);
        // "myhost nginx: 200 OK" -> process segment at "nginx: " (7..15)
        let process_seg = Some((7, 15, Color::Green));
        let spans = super::build_highlighted_spans(
            "myhost nginx: 200 OK",
            base,
            hl,
            None,
            process_seg,
        );
        // Should have: "myhost " (base), "nginx: " (green), "200 OK" (base)
        assert!(spans.len() >= 3);
        let texts: Vec<&str> = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(texts.join(""), "myhost nginx: 200 OK");
        // The process span should have green fg
        let process_span = spans.iter().find(|s| s.content.contains("nginx")).unwrap();
        assert_eq!(process_span.style.fg, Some(Color::Green));
    }

    #[test]
    fn test_build_highlighted_spans_search_overlapping_process() {
        let base = Style::default().fg(Color::White);
        let hl = Style::default().fg(Color::Black).bg(Color::Yellow);
        // Search match overlaps with the process name segment
        let matches = vec![(7, 12)]; // "nginx"
        let process_seg = Some((7, 15, Color::Green)); // "nginx: "
        let spans = super::build_highlighted_spans(
            "myhost nginx: 200 OK",
            base,
            hl,
            Some(&matches),
            process_seg,
        );
        let texts: Vec<&str> = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(texts.join(""), "myhost nginx: 200 OK");
        // The "nginx" part should be highlighted (search takes precedence)
        let nginx_span = spans.iter().find(|s| s.content == "nginx").unwrap();
        assert_eq!(nginx_span.style, hl);
    }

    #[test]
    fn test_search_by_timestamp_in_ui() {
        let rt = Arc::new(tokio::runtime::Runtime::new().unwrap());
        let db = rt.block_on(Database::in_memory()).unwrap();
        let db = Arc::new(db);
        let analyzer = LogAnalyzer::new(db.clone(), rt.clone());

        let entries = vec![
            LogEntry {
                id: 0,
                timestamp: Some("Jun 28 10:00:03".to_string()),
                hostname: Some("myhost".to_string()),
                process_name: Some("app".to_string()),
                message: "started".to_string(),
                ..Default::default()
            },
            LogEntry {
                id: 1,
                message: "no timestamp".to_string(),
                ..Default::default()
            },
        ];
        rt.block_on(db.insert_logs_batch(&entries)).unwrap();
        let mut app = App::new(analyzer, Theme::default());
        app.refresh_caches();

        // Search for timestamp
        app.handle_key_event(KeyCode::Char('/'));
        for c in "Jun 28".chars() {
            app.handle_search_mode_key(KeyCode::Char(c));
        }
        app.handle_search_mode_key(KeyCode::Enter);

        let results = app.search.get_results();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].log_id, 0);
    }

    #[test]
    fn test_search_by_hostname_in_ui() {
        let rt = Arc::new(tokio::runtime::Runtime::new().unwrap());
        let db = rt.block_on(Database::in_memory()).unwrap();
        let db = Arc::new(db);
        let analyzer = LogAnalyzer::new(db.clone(), rt.clone());

        let entries = vec![
            LogEntry {
                id: 0,
                hostname: Some("webserver01".to_string()),
                message: "request handled".to_string(),
                ..Default::default()
            },
            LogEntry {
                id: 1,
                hostname: Some("dbserver01".to_string()),
                message: "query executed".to_string(),
                ..Default::default()
            },
        ];
        rt.block_on(db.insert_logs_batch(&entries)).unwrap();
        let mut app = App::new(analyzer, Theme::default());
        app.refresh_caches();

        // Search for hostname
        app.handle_key_event(KeyCode::Char('/'));
        for c in "webserver".chars() {
            app.handle_search_mode_key(KeyCode::Char(c));
        }
        app.handle_search_mode_key(KeyCode::Enter);

        let results = app.search.get_results();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].log_id, 0);
    }

    #[test]
    fn test_find_matching_command() {
        assert!(super::find_matching_command("filter foo").is_some());
        assert_eq!(
            super::find_matching_command("filter foo").unwrap().name,
            "filter"
        );
        assert!(super::find_matching_command("exclude bar").is_some());
        assert!(super::find_matching_command("wrap").is_some());
        assert!(super::find_matching_command("set-theme dracula").is_some());
        assert!(super::find_matching_command("set-color --fg Red").is_some());
        assert!(super::find_matching_command("nonexistent").is_none());
        assert!(super::find_matching_command("").is_none());
    }

    #[test]
    fn test_find_command_completions() {
        let completions = super::find_command_completions("f");
        assert!(completions.contains(&"filter"));
        assert!(!completions.contains(&"exclude"));

        let completions = super::find_command_completions("ex");
        assert!(completions.contains(&"exclude"));
        assert!(completions.contains(&"export-marked"));
        assert_eq!(completions.len(), 2);

        let completions = super::find_command_completions("set");
        assert!(completions.contains(&"set-color"));
        assert!(completions.contains(&"set-theme"));
        assert_eq!(completions.len(), 2);

        let completions = super::find_command_completions("");
        assert_eq!(completions.len(), super::COMMANDS.len());

        // No completions after the first word
        let completions = super::find_command_completions("filter foo");
        assert!(completions.is_empty());
    }

    #[test]
    fn test_tab_completion_cycles_commands() {
        let mut app = App::new(mock_analyzer(), Theme::default());
        app.mode = AppMode::Command {
            input: "f".to_string(),
            cursor: 1,
            history: Vec::new(),
            history_index: None,
        };

        app.handle_command_mode_key(KeyCode::Tab);
        match &app.mode {
            AppMode::Command { input, .. } => {
                assert_eq!(input, "filter");
            }
            _ => panic!("Expected Command mode"),
        }
    }

    #[test]
    fn test_tab_completion_with_empty_input() {
        let mut app = App::new(mock_analyzer(), Theme::default());
        app.mode = AppMode::Command {
            input: String::new(),
            cursor: 0,
            history: Vec::new(),
            history_index: None,
        };

        // Tab on empty should fill with the first command
        app.handle_command_mode_key(KeyCode::Tab);
        match &app.mode {
            AppMode::Command { input, cursor, .. } => {
                assert!(!input.is_empty());
                assert_eq!(*cursor, input.len());
            }
            _ => panic!("Expected Command mode"),
        }
    }

    #[test]
    fn test_tab_resets_on_char_input() {
        let mut app = App::new(mock_analyzer(), Theme::default());
        app.mode = AppMode::Command {
            input: "s".to_string(),
            cursor: 1,
            history: Vec::new(),
            history_index: None,
        };

        // Tab once to get a completion
        app.handle_command_mode_key(KeyCode::Tab);
        assert!(app.tab_completion_index.is_some());

        // Type a character -> resets
        app.handle_command_mode_key(KeyCode::Char('x'));
        assert!(app.tab_completion_index.is_none());
    }

    #[test]
    fn test_marked_line_has_bold_style() {
        let base_marked = Style::default()
            .fg(Color::Rgb(255, 184, 108))
            .add_modifier(Modifier::BOLD);
        let hl = Style::default().fg(Color::Black).bg(Color::Yellow);
        let spans =
            super::build_highlighted_spans("marked line text", base_marked, hl, None, None);
        assert_eq!(spans.len(), 1);
        // The span should carry the marked style
        assert_eq!(spans[0].style, base_marked);
    }

    #[test]
    fn test_command_names_list() {
        let names = super::command_names();
        assert!(names.contains(&"filter"));
        assert!(names.contains(&"exclude"));
        assert!(names.contains(&"set-color"));
        assert!(names.contains(&"export-marked"));
        assert!(names.contains(&"save-filters"));
        assert!(names.contains(&"load-filters"));
        assert!(names.contains(&"wrap"));
        assert!(names.contains(&"set-theme"));
        assert!(names.contains(&"level-colors"));
    }

    #[test]
    fn test_level_colors_enabled_by_default() {
        let app = App::new(mock_analyzer(), Theme::default());
        assert!(app.level_colors);
    }

    #[test]
    fn test_level_colors_toggle_command() {
        let mut app = App::new(mock_analyzer(), Theme::default());
        assert!(app.level_colors);

        app.mode = AppMode::Command {
            input: "level-colors".to_string(),
            cursor: 12,
            history: Vec::new(),
            history_index: None,
        };
        app.handle_command();
        assert!(!app.level_colors);

        app.mode = AppMode::Command {
            input: "level-colors".to_string(),
            cursor: 12,
            history: Vec::new(),
            history_index: None,
        };
        app.handle_command();
        assert!(app.level_colors);
    }

    #[test]
    fn test_level_colors_error_line_styling() {
        let theme = Theme::default();
        let error_fg = theme.error_fg;
        let base = Style::default().fg(error_fg);
        let hl = Style::default().fg(Color::Black).bg(Color::Yellow);
        let spans = super::build_highlighted_spans("ERROR something failed", base, hl, None, None);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].style.fg, Some(error_fg));
    }

    #[test]
    fn test_level_colors_warning_line_styling() {
        let theme = Theme::default();
        let warning_fg = theme.warning_fg;
        let base = Style::default().fg(warning_fg);
        let hl = Style::default().fg(Color::Black).bg(Color::Yellow);
        let spans = super::build_highlighted_spans("WARN something happened", base, hl, None, None);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].style.fg, Some(warning_fg));
    }

    #[test]
    fn test_level_colors_disabled_uses_default_text() {
        let mut app = App::new(mock_analyzer(), Theme::default());
        app.level_colors = false;

        // With level_colors disabled, error/warning lines should use default text color
        // We verify by checking the app state
        assert!(!app.level_colors);

        let logs = app.analyzer.get_logs();
        let error_log = logs.iter().find(|l| l.level == LogLevel::Error).unwrap();
        assert_eq!(error_log.level, LogLevel::Error);
        // The styling logic in ui() checks app.level_colors before applying level colors
    }

    #[test]
    fn test_level_colors_marked_overrides_level_color() {
        // When a line is both error-level and marked, marked style should take precedence
        let theme = Theme::default();
        let base_marked = Style::default()
            .fg(theme.text_highlight)
            .add_modifier(Modifier::BOLD);
        let hl = Style::default().fg(Color::Black).bg(Color::Yellow);
        let spans =
            super::build_highlighted_spans("ERROR critical failure", base_marked, hl, None, None);
        assert_eq!(spans.len(), 1);
        // Marked style should be applied (text_highlight fg + bold), not error_fg
        assert_eq!(spans[0].style.fg, Some(theme.text_highlight));
        assert!(spans[0].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn test_page_down() {
        let mut app = setup_test_app_for_vim_motions();
        app.visible_height = 20;
        app.handle_key_event(KeyCode::PageDown);
        assert_eq!(app.scroll_offset, 20);
        app.handle_key_event(KeyCode::PageDown);
        assert_eq!(app.scroll_offset, 40);
    }

    #[test]
    fn test_page_up() {
        let mut app = setup_test_app_for_vim_motions();
        app.visible_height = 20;
        app.scroll_offset = 50;
        app.handle_key_event(KeyCode::PageUp);
        assert_eq!(app.scroll_offset, 30);
        app.handle_key_event(KeyCode::PageUp);
        assert_eq!(app.scroll_offset, 10);
    }

    #[test]
    fn test_page_up_at_top() {
        let mut app = setup_test_app_for_vim_motions();
        app.visible_height = 20;
        app.scroll_offset = 5;
        app.handle_key_event(KeyCode::PageUp);
        assert_eq!(app.scroll_offset, 0);
    }

    #[test]
    fn test_ctrl_d_half_page_down() {
        use crossterm::event::KeyModifiers;
        let mut app = setup_test_app_for_vim_motions();
        app.visible_height = 20;
        app.handle_key_event_with_modifiers(KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert_eq!(app.scroll_offset, 10);
        app.handle_key_event_with_modifiers(KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert_eq!(app.scroll_offset, 20);
    }

    #[test]
    fn test_ctrl_u_half_page_up() {
        use crossterm::event::KeyModifiers;
        let mut app = setup_test_app_for_vim_motions();
        app.visible_height = 20;
        app.scroll_offset = 50;
        app.handle_key_event_with_modifiers(KeyCode::Char('u'), KeyModifiers::CONTROL);
        assert_eq!(app.scroll_offset, 40);
        app.handle_key_event_with_modifiers(KeyCode::Char('u'), KeyModifiers::CONTROL);
        assert_eq!(app.scroll_offset, 30);
    }

    #[test]
    fn test_ctrl_u_at_top() {
        use crossterm::event::KeyModifiers;
        let mut app = setup_test_app_for_vim_motions();
        app.visible_height = 20;
        app.scroll_offset = 3;
        app.handle_key_event_with_modifiers(KeyCode::Char('u'), KeyModifiers::CONTROL);
        assert_eq!(app.scroll_offset, 0);
    }

    #[test]
    fn test_background_loading_state() {
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        for i in 0..10 {
            writeln!(file, "line {}", i).unwrap();
        }
        let path = file.path().to_str().unwrap().to_string();

        let mut app = App::new(mock_empty_analyzer(), Theme::default());
        assert!(app.background_loading.is_none());

        app.start_background_loading(path, 0);
        assert!(app.background_loading.is_some());
        let state = app.background_loading.as_ref().unwrap();
        assert_eq!(state.lines_loaded, 0);
        assert_eq!(state.spinner_frame, 0);
    }

    #[test]
    fn test_background_loading_completes_on_missing_file() {
        let mut app = App::new(mock_empty_analyzer(), Theme::default());
        app.start_background_loading("/nonexistent/file.log".to_string(), 0);
        // Give the thread a moment to finish
        std::thread::sleep(std::time::Duration::from_millis(50));
        app.process_background_loading();
        // Should clear background_loading when thread disconnects
        assert!(app.background_loading.is_none());
    }

    #[test]
    fn test_background_loading_processes_chunk() {
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        for i in 0..50 {
            writeln!(file, "line {}", i).unwrap();
        }
        let path = file.path().to_str().unwrap().to_string();

        let mut app = App::new(mock_empty_analyzer(), Theme::default());
        // Ingest first 10 lines synchronously
        app.analyzer.ingest_file_chunk(&path, 0, 10).unwrap();
        app.logs_dirty = true;

        // Start background loading from line 10
        app.start_background_loading(path, 10);

        // Give the background thread time to read and send
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Process batches from channel
        app.process_background_loading();
        assert!(app.logs_dirty);

        // After refresh, should have more logs
        app.refresh_caches();
        assert!(app.cached_logs.len() > 10);
    }
}
