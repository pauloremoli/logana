use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph, Wrap},
};
use std::time::{Duration, Instant};

use crate::analyzer::{FilterType, LogAnalyzer, LogEntry};
use crate::search::Search;

#[derive(Debug, PartialEq)]
pub enum AppMode {
    Normal,
    Command,
    FilterManagement,
    FilterEdit,
    Search,
}

#[derive(Debug)]
pub struct App {
    pub analyzer: LogAnalyzer,
    pub mode: AppMode,
    pub command_input: String,
    pub command_cursor: usize,
    pub command_history: Vec<String>,
    pub command_history_index: Option<usize>,
    pub scroll_offset: usize,

    pub show_sidebar: bool,
    pub selected_filter_index: usize,
    pub editing_filter_id: Option<usize>,
    pub editing_filter_input: String,

    pub search: Search,
    pub search_input: String,
    pub search_forward: bool, // true for '/', false for '?'
    pub g_key_pressed: bool,
    pub wrap: bool,
    pub horizontal_scroll: usize,
}

impl App {
    pub fn new(analyzer: LogAnalyzer) -> Self {
        App {
            analyzer,
            mode: AppMode::Normal,
            command_input: String::new(),
            command_cursor: 0,
            command_history: Vec::new(),
            command_history_index: None,
            scroll_offset: 0,

            show_sidebar: false,
            selected_filter_index: 0,
            editing_filter_id: None,
            editing_filter_input: String::new(),

            search: Search::new(),
            search_input: String::new(),
            search_forward: true,
            g_key_pressed: false,
            wrap: true,
            horizontal_scroll: 0,
        }
    }

    pub fn run(&mut self, terminal: &mut Terminal<impl Backend>) -> anyhow::Result<()> {
        let mut last_tick = Instant::now();
        let tick_rate = Duration::from_millis(250);

        loop {
            terminal.draw(|frame| self.ui(frame))?;

            let timeout = tick_rate
                .checked_sub(last_tick.elapsed())
                .unwrap_or_else(|| Duration::from_secs(0));
            if event::poll(timeout)? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        match self.mode {
                            AppMode::Normal => {
                                if key.code == KeyCode::Char('q') {
                                    return Ok(());
                                }
                                self.handle_normal_mode_key(key.code)
                            }
                            AppMode::Command => self.handle_command_mode_key(key.code),
                            AppMode::FilterManagement => {
                                self.handle_filter_management_mode_key(key.code)
                            }
                            AppMode::FilterEdit => self.handle_filter_edit_mode_key(key.code),
                            AppMode::Search => self.handle_search_mode_key(key.code),
                        }
                    }
                }
            }

            if last_tick.elapsed() >= tick_rate {
                last_tick = Instant::now();
            }
        }
    }

    fn handle_normal_mode_key(&mut self, key_code: KeyCode) {
        match key_code {
            KeyCode::Char('q') => {} // handled in run()
            KeyCode::Char(':') => self.mode = AppMode::Command,
            KeyCode::Char('f') => {
                self.mode = AppMode::FilterManagement;
                self.show_sidebar = true
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
                let num_logs = self.get_filtered_logs().len();
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
                let logs_to_display = self.get_filtered_logs();
                if let Some(log) = logs_to_display.get(self.scroll_offset) {
                    self.analyzer.toggle_mark(log.id);
                }
                self.g_key_pressed = false;
            }
            KeyCode::Char('/') => {
                self.mode = AppMode::Search;
                self.search_input.clear();
                self.search_forward = true;
                self.g_key_pressed = false;
            }
            KeyCode::Char('?') => {
                self.mode = AppMode::Search;
                self.search_input.clear();
                self.search_forward = false;
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

    fn handle_command_mode_key(&mut self, key_code: KeyCode) {
        match key_code {
            KeyCode::Enter => {
                self.handle_command();
                if !self.command_input.is_empty() {
                    self.command_history.push(self.command_input.clone());
                }
                self.command_input.clear();
                self.command_cursor = 0;
                self.command_history_index = None;
                self.mode = AppMode::Normal;
            }
            KeyCode::Esc => {
                self.command_input.clear();
                self.command_cursor = 0;
                self.command_history_index = None;
                self.mode = AppMode::Normal;
            }
            KeyCode::Backspace => {
                if self.command_cursor > 0 && !self.command_input.is_empty() {
                    self.command_input.remove(self.command_cursor - 1);
                    self.command_cursor -= 1;
                }
            }
            KeyCode::Char(c) => {
                self.command_input.insert(self.command_cursor, c);
                self.command_cursor += 1;
            }
            KeyCode::Left => {
                if self.command_cursor > 0 {
                    self.command_cursor -= 1;
                }
            }
            KeyCode::Right => {
                if self.command_cursor < self.command_input.len() {
                    self.command_cursor += 1;
                }
            }
            KeyCode::Up => {
                if self.command_history.is_empty() {
                    return;
                }
                let new_index = match self.command_history_index {
                    None => Some(self.command_history.len() - 1),
                    Some(0) => Some(0),
                    Some(i) => Some(i - 1),
                };
                if let Some(i) = new_index {
                    self.command_input = self.command_history[i].clone();
                    self.command_cursor = self.command_input.len();
                    self.command_history_index = Some(i);
                }
            }
            KeyCode::Down => {
                if self.command_history.is_empty() {
                    return;
                }
                let new_index = match self.command_history_index {
                    None => return,
                    Some(i) if i + 1 >= self.command_history.len() => {
                        self.command_input.clear();
                        self.command_cursor = 0;
                        self.command_history_index = None;
                        return;
                    }
                    Some(i) => Some(i + 1),
                };
                if let Some(i) = new_index {
                    self.command_input = self.command_history[i].clone();
                    self.command_cursor = self.command_input.len();
                    self.command_history_index = Some(i);
                }
            }
            _ => {}
        }
    }

    fn handle_filter_management_mode_key(&mut self, key_code: KeyCode) {
        match key_code {
            KeyCode::Esc => self.mode = AppMode::Normal,
            KeyCode::Up => {
                self.selected_filter_index = self.selected_filter_index.saturating_sub(1);
            }
            KeyCode::Down => {
                self.selected_filter_index = self.selected_filter_index.saturating_add(1);
                let num_filters = self.analyzer.filters.len();
                if num_filters > 0 && self.selected_filter_index >= num_filters {
                    self.selected_filter_index = num_filters - 1;
                }
            }
            KeyCode::Char(' ') => {
                if let Some(filter) = self.analyzer.filters.get(self.selected_filter_index) {
                    self.analyzer.toggle_filter(filter.id);
                }
            }
            KeyCode::Char('d') => {
                if let Some(filter) = self.analyzer.filters.get(self.selected_filter_index) {
                    self.analyzer.remove_filter(filter.id);
                    if self.selected_filter_index >= self.analyzer.filters.len()
                        && !self.analyzer.filters.is_empty()
                    {
                        self.selected_filter_index = self.analyzer.filters.len() - 1;
                    }
                }
            }
            KeyCode::Char('e') => {
                if let Some(filter) = self.analyzer.filters.get(self.selected_filter_index) {
                    use crate::analyzer::FilterType;
                    let mut cmd = String::from("filter");
                    if filter.filter_type == FilterType::Include {
                        if let Some(cfg) = &filter.color_config {
                            cmd.push_str(&format!(" --fg {:?} --bg {:?}", cfg.fg, cfg.bg));
                        }
                    }
                    // Always add a space before the pattern
                    cmd.push(' ');
                    cmd.push_str(&filter.pattern);
                    self.mode = AppMode::Command;
                    self.command_input = cmd;
                }
            }
            KeyCode::Char('c') => {
                // Enter command mode with set-color command prefilled for the selected filter
                if let Some(filter) = self.analyzer.filters.get(self.selected_filter_index) {
                    let mut cmd = String::from("set-color");
                    if let Some(cfg) = &filter.color_config {
                        if let Some(fg) = cfg.fg {
                            cmd.push_str(&format!(" --fg {:?}", fg));
                        }
                        if let Some(bg) = cfg.bg {
                            cmd.push_str(&format!(" --bg {:?}", bg));
                        }
                    }
                    self.mode = AppMode::Command;
                    self.command_input = cmd;
                }
            }
            KeyCode::Char('i') => {
                // Prompt for include filter pattern and color
                self.mode = AppMode::Command;
                self.command_input = "filter ".to_string();
            }
            KeyCode::Char('x') => {
                // Prompt for exclude filter pattern
                self.mode = AppMode::Command;
                self.command_input = "exclude ".to_string();
            }
            _ => {}
        }
    }

    fn handle_filter_edit_mode_key(&mut self, key_code: KeyCode) {
        match key_code {
            KeyCode::Enter => {
                if let Some(id) = self.editing_filter_id {
                    self.analyzer
                        .edit_filter(id, self.editing_filter_input.clone());
                    self.editing_filter_id = None;
                    self.editing_filter_input.clear();
                    self.mode = AppMode::FilterManagement;
                }
            }
            KeyCode::Esc => {
                self.editing_filter_id = None;
                self.editing_filter_input.clear();
                self.mode = AppMode::FilterManagement;
            }
            KeyCode::Backspace => {
                self.editing_filter_input.pop();
            }
            KeyCode::Char(c) => {
                self.editing_filter_input.push(c);
            }
            _ => {}
        }
    }

    fn handle_search_mode_key(&mut self, key_code: KeyCode) {
        match key_code {
            KeyCode::Enter => {
                let _ = self
                    .search
                    .search(&self.search_input, &self.analyzer.entries);
                if self.search_forward {
                    if let Some(result) = self.search.next_match() {
                        let log_id = result.log_id;
                        self.scroll_to_log_entry(log_id);
                    }
                } else if let Some(result) = self.search.previous_match() {
                    let log_id = result.log_id;
                    self.scroll_to_log_entry(log_id);
                }
                self.mode = AppMode::Normal;
                self.search_input.clear();
            }
            KeyCode::Esc => {
                self.search_input.clear();
                self.mode = AppMode::Normal;
            }
            KeyCode::Backspace => {
                self.search_input.pop();
            }
            KeyCode::Char(c) => {
                self.search_input.push(c);
            }
            _ => {}
        }
    }

    pub fn handle_key_event(&mut self, key_code: KeyCode) {
        match self.mode {
            AppMode::Normal => self.handle_normal_mode_key(key_code),
            AppMode::Command => self.handle_command_mode_key(key_code),
            AppMode::FilterManagement => self.handle_filter_management_mode_key(key_code),
            AppMode::FilterEdit => self.handle_filter_edit_mode_key(key_code),
            AppMode::Search => self.handle_search_mode_key(key_code),
        }
    }

    fn ui(&mut self, frame: &mut Frame) {
        let size = frame.size();
        // Split vertically: logs, command bar (if needed), command list (full width)
        let mut constraints = vec![Constraint::Min(1)];
        let show_command_bar = matches!(self.mode, AppMode::Command);
        if show_command_bar {
            constraints.push(Constraint::Length(1));
        }
        constraints.push(Constraint::Length(3));
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(size);

        // Now, if sidebar is shown, split only the logs area horizontally
        let (logs_area, sidebar_area) = if self.show_sidebar {
            let horizontal = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Min(1), Constraint::Length(30)])
                .split(chunks[0]);
            (horizontal[0], Some(horizontal[1]))
        } else {
            (chunks[0], None)
        };

        let logs_to_display = self.get_filtered_logs();
        let num_logs = logs_to_display.len();
        if self.scroll_offset >= num_logs && num_logs > 0 {
            self.scroll_offset = num_logs - 1;
        }

        let log_lines: Vec<Line> = logs_to_display
            .iter()
            .map(|log| {
                let mut spans = Vec::new();
                let mut last_end = 0;

                if let Some(search_result) = self
                    .search
                    .get_results()
                    .iter()
                    .find(|r| r.log_id == log.id)
                {
                    for (start, end) in &search_result.matches {
                        spans.push(Span::raw(&log.content[last_end..*start]));
                        spans.push(Span::styled(
                            &log.content[*start..*end],
                            Style::default().fg(Color::Black).bg(Color::Yellow),
                        ));
                        last_end = *end;
                    }
                }
                spans.push(Span::raw(&log.content[last_end..]));

                let mut line = Line::from(spans);
                if log.marked {
                    line = line.bg(Color::DarkGray);
                }

                // Color by log level (only for ERROR and WARN)
                if log.content.contains("ERROR") {
                    line = line.fg(Color::Red);
                } else if log.content.contains("WARN") {
                    line = line.fg(Color::Yellow);
                }

                // Custom color configs override log level coloring
                if let Some(filter) = self
                    .analyzer
                    .filters
                    .iter()
                    .find(|f| log.content.contains(&f.pattern))
                {
                    if let Some(config) = &filter.color_config {
                        if let Some(fg) = config.fg {
                            line = line.fg(fg);
                        }
                        if let Some(bg) = config.bg {
                            line = line.bg(bg);
                        }
                    }
                }

                line
            })
            .collect();

        let mut paragraph = Paragraph::new(log_lines)
            .block(Block::default().borders(Borders::ALL).title("Logs"))
            .scroll((self.scroll_offset as u16, self.horizontal_scroll as u16));

        if self.wrap {
            paragraph = paragraph.wrap(Wrap { trim: false });
        }

        frame.render_widget(paragraph, logs_area);

        if let Some(sidebar_area) = sidebar_area {
            let filters_text: Vec<Line> = self
                .analyzer
                .filters
                .iter()
                .enumerate()
                .map(|(i, filter)| {
                    let status = if filter.enabled { "[x]" } else { "[ ]" };
                    let selected_prefix = if i == self.selected_filter_index {
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
                })
                .collect();

            let sidebar = Paragraph::new(filters_text)
                .block(Block::default().borders(Borders::ALL).title("Filters"));
            frame.render_widget(sidebar, sidebar_area);
        }

        // Command bar (full width, only in command mode)
        if show_command_bar {
            let input_prefix = ":";
            let input_text = &self.command_input;
            let command_line = Paragraph::new(format!("{}{}", input_prefix, input_text))
                .style(Style::default().fg(Color::White).bg(Color::DarkGray))
                .wrap(Wrap { trim: false });
            // Limit the command bar to 5 lines
            let mut area = chunks[1];
            if area.height > 5 {
                area.height = 5;
            }
            frame.render_widget(command_line, area);
            // Set cursor at the correct position
            let cursor_x = chunks[1].x + 1 + self.command_cursor as u16;
            if cursor_x < chunks[1].x + chunks[1].width && area.height == 1 {
                frame.set_cursor(cursor_x, chunks[1].y);
            }
        }

        // Command list (full width, always last chunk)
        let command_list = Paragraph::new(self.get_command_list())
            .block(Block::default().borders(Borders::ALL))
            .wrap(Wrap { trim: true }); // Enable word wrapping
        let mut area = *chunks.last().unwrap();
        if area.height > 5 {
            area.height = 5;
        }
        frame.render_widget(command_list, area);
    }

    fn handle_command(&mut self) {
        use crate::command_args::{CommandLine, Commands};
        use clap::Parser;
        let input = self.command_input.trim();
        let args = match CommandLine::try_parse_from(input.split_whitespace()) {
            Ok(args) => args,
            Err(_) => return, // Optionally show error
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
            }
            Some(Commands::Exclude { pattern }) => {
                self.analyzer
                    .add_filter_with_color(pattern, FilterType::Exclude, None, None);
                self.scroll_offset = 0;
            }
            Some(Commands::SetColor { fg, bg }) => {
                if let Some(filter) = self.analyzer.filters.get(self.selected_filter_index) {
                    if filter.filter_type == FilterType::Include {
                        let pattern = filter.pattern.clone();
                        self.analyzer
                            .set_color_config(&pattern, fg.as_deref(), bg.as_deref());
                    }
                }
            }
            Some(Commands::ExportMarked { path }) => {
                if !path.is_empty() {
                    let marked_logs: Vec<String> = self
                        .analyzer
                        .entries
                        .iter()
                        .filter(|e| e.marked)
                        .map(|e| e.content.clone())
                        .collect();
                    let mut marked_logs_content = marked_logs.join("\n");
                    if !marked_logs_content.ends_with("\n") {
                        marked_logs_content.push('\n');
                    }
                    let _ = std::fs::write(path, marked_logs_content);
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
                }
            }
            Some(Commands::Wrap) => {
                self.wrap = !self.wrap;
            }
            None => {}
        }
    }

    fn get_command_list(&self) -> String {
        match self.mode {
            AppMode::Normal => {
                "[NORMAL] [Q]uit | : => command Mode | [f]ilter mode | [s]idebar | [m]ark Line | / => Search Forward | ? => Search Backward | [n]ext match | N => previous match".to_string()
            },
            AppMode::Command => {
                "[COMMAND] filter | exclude | set-color | export-marked | save-filters | load-filters | wrap | Esc | Enter".to_string()
            },
            AppMode::FilterManagement => {
                "[FILTER] [i]nclude | e[x]clude | Space: toggle | [d]elete | [e]dit | set [c]olor | Esc => normal mode".to_string()
            },
            AppMode::FilterEdit => {
                "[FILTER EDIT] Esc => Cancel | Enter => Save".to_string()
            },
            AppMode::Search => {
                "[SEARCH] Esc => Cancel | Enter => Search".to_string()
            },
        }
    }

    fn get_filtered_logs(&self) -> Vec<LogEntry> {
        self.analyzer
            .apply_filters(self.analyzer.get_logs())
            .unwrap_or_else(|_| self.analyzer.get_logs().clone())
    }

    fn scroll_to_log_entry(&mut self, log_id: usize) {
        if let Some(index) = self.analyzer.entries.iter().position(|e| e.id == log_id) {
            self.scroll_offset = index;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::analyzer::{FilterType, LogAnalyzer, LogEntry};
    use crate::ui::{App, AppMode};
    use crossterm::event::KeyCode;

    fn mock_analyzer() -> LogAnalyzer {
        let mut analyzer = LogAnalyzer::new();
        analyzer.entries = vec![
            LogEntry {
                id: 0,
                content: "INFO something".to_string(),
                marked: false,
            },
            LogEntry {
                id: 1,
                content: "WARN warning".to_string(),
                marked: false,
            },
            LogEntry {
                id: 2,
                content: "ERROR error".to_string(),
                marked: false,
            },
        ];
        analyzer
    }

    #[test]
    fn test_toggle_wrap_command() {
        let mut app = App::new(mock_analyzer());
        app.mode = AppMode::Command;
        app.command_input = "wrap".to_string();
        app.handle_command();
        assert!(!app.wrap);
        app.command_input = "wrap".to_string();
        app.handle_command();
        assert!(app.wrap);
    }

    #[test]
    fn test_add_filter_command() {
        let mut app = App::new(mock_analyzer());
        app.mode = AppMode::Command;
        app.command_input = "filter foo".to_string();
        app.handle_command();
        assert_eq!(app.analyzer.filters.len(), 1);
        assert_eq!(app.analyzer.filters[0].filter_type, FilterType::Include);
        assert_eq!(app.analyzer.filters[0].pattern, "foo");
    }

    #[test]
    fn test_add_exclude_command() {
        let mut app = App::new(mock_analyzer());
        app.mode = AppMode::Command;
        app.command_input = "exclude bar".to_string();
        app.handle_command();
        assert_eq!(app.analyzer.filters.len(), 1);
        assert_eq!(app.analyzer.filters[0].filter_type, FilterType::Exclude);
        assert_eq!(app.analyzer.filters[0].pattern, "bar");
    }

    #[test]
    fn test_mark_line() {
        let mut app = App::new(mock_analyzer());
        app.handle_normal_mode_key(KeyCode::Char('m'));
        assert!(app.analyzer.entries[0].marked);
    }

    #[test]
    fn test_scroll_offset_j_k() {
        let mut app = App::new(mock_analyzer());
        app.handle_normal_mode_key(KeyCode::Char('j'));
        assert_eq!(app.scroll_offset, 1);
        app.handle_normal_mode_key(KeyCode::Char('k'));
        assert_eq!(app.scroll_offset, 0);
    }

    #[test]
    fn test_mode_switching() {
        let mut app = App::new(mock_analyzer());
        app.handle_normal_mode_key(KeyCode::Char(':'));
        assert!(matches!(app.mode, AppMode::Command));
        app.handle_command_mode_key(KeyCode::Esc);
        assert!(matches!(app.mode, AppMode::Normal));
    }

    #[test]
    fn test_sidebar_filter_display_in_out() {
        let mut app = App::new(mock_analyzer());
        app.analyzer
            .add_filter("foo".to_string(), FilterType::Include);
        app.analyzer
            .add_filter("bar".to_string(), FilterType::Exclude);
        let filters = &app.analyzer.filters;
        assert_eq!(filters[0].filter_type, FilterType::Include);
        assert_eq!(filters[1].filter_type, FilterType::Exclude);
        // The UI rendering is not tested here, but the filter types are correct for sidebar display
    }

    #[test]
    fn test_command_list_texts() {
        let app = App::new(mock_analyzer());
        let normal = app.get_command_list();
        assert!(normal.contains("[NORMAL]"));
    }

    #[test]
    fn test_toggle_sidebar() {
        let mut app = App::new(mock_analyzer());
        assert!(!app.show_sidebar);
        app.handle_normal_mode_key(KeyCode::Char('s'));
        assert!(app.show_sidebar);
        app.handle_normal_mode_key(KeyCode::Char('s'));
        assert!(!app.show_sidebar);
    }

    #[test]
    fn test_filter_management_mode_navigation() {
        let mut app = App::new(mock_analyzer());
        app.mode = AppMode::FilterManagement;
        app.analyzer
            .add_filter("foo".to_string(), FilterType::Include);
        app.analyzer
            .add_filter("bar".to_string(), FilterType::Exclude);
        app.selected_filter_index = 1;
        app.handle_filter_management_mode_key(KeyCode::Up);
        assert_eq!(app.selected_filter_index, 0);
        app.handle_filter_management_mode_key(KeyCode::Down);
        assert_eq!(app.selected_filter_index, 1);
    }

    #[test]
    fn test_filter_toggle_and_delete() {
        let mut app = App::new(mock_analyzer());
        app.mode = AppMode::FilterManagement;
        app.analyzer
            .add_filter("foo".to_string(), FilterType::Include);
        assert!(app.analyzer.filters[0].enabled);
        app.handle_filter_management_mode_key(KeyCode::Char(' '));
        assert!(!app.analyzer.filters[0].enabled);
        app.handle_filter_management_mode_key(KeyCode::Char('d'));
        assert!(app.analyzer.filters.is_empty());
    }

    #[test]
    fn test_filter_edit() {
        let mut app = App::new(mock_analyzer());
        app.mode = AppMode::FilterManagement;
        app.analyzer
            .add_filter("foo".to_string(), FilterType::Include);
        // Simulate pressing 'e' in filter management, which now enters command mode with prefilled command
        app.handle_filter_management_mode_key(KeyCode::Char('e'));
        assert_eq!(app.mode, AppMode::Command);
        assert!(app.command_input.starts_with("filter"));
        assert!(app.command_input.contains("foo"));
    }

    #[test]
    fn test_search_mode_and_input() {
        let mut app = App::new(mock_analyzer());
        app.handle_normal_mode_key(KeyCode::Char('/'));
        assert_eq!(app.mode, AppMode::Search);
        app.handle_search_mode_key(KeyCode::Char('t'));
        assert_eq!(app.search_input, "t");
        app.handle_search_mode_key(KeyCode::Backspace);
        assert_eq!(app.search_input, "");
        app.handle_search_mode_key(KeyCode::Esc);
        assert_eq!(app.mode, AppMode::Normal);
    }

    #[test]
    fn test_command_input_and_backspace() {
        let mut app = App::new(mock_analyzer());
        app.mode = AppMode::Command;
        app.handle_command_mode_key(KeyCode::Char('a'));
        app.handle_command_mode_key(KeyCode::Char('b'));
        assert_eq!(app.command_input, "ab");
        app.handle_command_mode_key(KeyCode::Backspace);
        assert_eq!(app.command_input, "a");
        app.handle_command_mode_key(KeyCode::Esc);
        assert_eq!(app.command_input, "");
        assert_eq!(app.mode, AppMode::Normal);
    }

    #[test]
    fn test_scroll_to_log_entry() {
        let mut app = App::new(mock_analyzer());
        app.scroll_to_log_entry(2);
        assert_eq!(app.scroll_offset, 2);
    }

    #[test]
    fn test_toggle_line_wrapping() {
        let mut app = App::new(Default::default());
        assert!(app.wrap);
        app.handle_key_event(KeyCode::Char('w'));
        assert!(!app.wrap);
        app.handle_key_event(KeyCode::Char('w'));
        assert!(app.wrap);
    }

    #[test]
    fn test_horizontal_scroll() {
        let mut app = App::new(Default::default());
        app.wrap = false;
        assert_eq!(app.horizontal_scroll, 0);
        app.handle_key_event(KeyCode::Char('l'));
        assert_eq!(app.horizontal_scroll, 1);
        app.handle_key_event(KeyCode::Char('h'));
        assert_eq!(app.horizontal_scroll, 0);
    }

    fn setup_test_app_for_vim_motions() -> App {
        let mut analyzer = LogAnalyzer::new();
        for i in 0..100 {
            analyzer.entries.push(LogEntry {
                id: i,
                content: format!("line {}", i),
                marked: false,
            });
        }
        App::new(analyzer)
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
        app.scroll_offset = 50;
        app.handle_key_event(KeyCode::Char('G'));
        assert_eq!(app.scroll_offset, 99);
    }
}
