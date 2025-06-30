use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph},
};
use std::time::{Duration, Instant};

use crate::analyzer::{FilterType, LogAnalyzer, LogEntry};
use crate::search::Search;

pub enum AppMode {
    Normal,
    Command,
    FilterManagement,
    FilterEdit,
    Search,
}

pub struct App {
    pub analyzer: LogAnalyzer,
    pub mode: AppMode,
    pub command_input: String,
    pub scroll_offset: usize,

    pub show_sidebar: bool,
    pub selected_filter_index: usize,
    pub editing_filter_id: Option<usize>,
    pub editing_filter_input: String,

    pub search: Search,
    pub search_input: String,
    pub search_forward: bool, // true for '/', false for '?'
}

impl App {
    pub fn new(analyzer: LogAnalyzer) -> Self {
        App {
            analyzer,
            mode: AppMode::Normal,
            command_input: String::new(),
            scroll_offset: 0,

            show_sidebar: false,
            selected_filter_index: 0,
            editing_filter_id: None,
            editing_filter_input: String::new(),

            search: Search::new(),
            search_input: String::new(),
            search_forward: true,
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
                            AppMode::Normal => match key.code {
                                KeyCode::Char('q') => return Ok(()),
                                KeyCode::Char(':') => self.mode = AppMode::Command,
                                KeyCode::Char('f') => {
                                    self.mode = AppMode::FilterManagement;
                                    self.show_sidebar = true
                                }
                                KeyCode::Char('s') => self.show_sidebar = !self.show_sidebar,
                                KeyCode::Down => {
                                    self.scroll_offset = self.scroll_offset.saturating_add(1)
                                }
                                KeyCode::Up => {
                                    self.scroll_offset = self.scroll_offset.saturating_sub(1)
                                }
                                KeyCode::Char('m') => {
                                    let logs_to_display = self.get_filtered_logs();
                                    if let Some(log) = logs_to_display.get(self.scroll_offset) {
                                        self.analyzer.toggle_mark(log.id);
                                    }
                                }
                                KeyCode::Char('/') => {
                                    self.mode = AppMode::Search;
                                    self.search_input.clear();
                                    self.search_forward = true;
                                }
                                KeyCode::Char('?') => {
                                    self.mode = AppMode::Search;
                                    self.search_input.clear();
                                    self.search_forward = false;
                                }
                                KeyCode::Char('n') => {
                                    if let Some(result) = self.search.next_match() {
                                        let log_id = result.log_id;
                                        self.scroll_to_log_entry(log_id);
                                    }
                                }
                                KeyCode::Char('N') => {
                                    if let Some(result) = self.search.previous_match() {
                                        let log_id = result.log_id;
                                        self.scroll_to_log_entry(log_id);
                                    }
                                }
                                _ => {}
                            },
                            AppMode::Command => match key.code {
                                KeyCode::Enter => {
                                    self.handle_command();
                                    self.command_input.clear();
                                    self.mode = AppMode::Normal;
                                }
                                KeyCode::Esc => {
                                    self.command_input.clear();
                                    self.mode = AppMode::Normal;
                                }
                                KeyCode::Backspace => {
                                    self.command_input.pop();
                                }
                                KeyCode::Char(c) => {
                                    self.command_input.push(c);
                                }
                                _ => {}
                            },
                            AppMode::FilterManagement => match key.code {
                                KeyCode::Esc => self.mode = AppMode::Normal,
                                KeyCode::Up => {
                                    self.selected_filter_index =
                                        self.selected_filter_index.saturating_sub(1);
                                }
                                KeyCode::Down => {
                                    self.selected_filter_index =
                                        self.selected_filter_index.saturating_add(1);
                                    let num_filters = self.analyzer.filters.len();
                                    if num_filters > 0 && self.selected_filter_index >= num_filters
                                    {
                                        self.selected_filter_index = num_filters - 1;
                                    }
                                }
                                KeyCode::Char(' ') => {
                                    if let Some(filter) =
                                        self.analyzer.filters.get(self.selected_filter_index)
                                    {
                                        self.analyzer.toggle_filter(filter.id);
                                    }
                                }
                                KeyCode::Char('d') => {
                                    if let Some(filter) =
                                        self.analyzer.filters.get(self.selected_filter_index)
                                    {
                                        self.analyzer.remove_filter(filter.id);
                                        // Adjust selected_filter_index if the last filter was removed
                                        if self.selected_filter_index >= self.analyzer.filters.len()
                                            && !self.analyzer.filters.is_empty()
                                        {
                                            self.selected_filter_index =
                                                self.analyzer.filters.len() - 1;
                                        }
                                    }
                                }
                                KeyCode::Char('e') => {
                                    if let Some(filter) =
                                        self.analyzer.filters.get(self.selected_filter_index)
                                    {
                                        self.editing_filter_id = Some(filter.id);
                                        self.editing_filter_input = filter.pattern.clone();
                                        self.mode = AppMode::FilterEdit;
                                    }
                                }
                                _ => {}
                            },
                            AppMode::FilterEdit => match key.code {
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
                            },
                            AppMode::Search => match key.code {
                                KeyCode::Enter => {
                                    let _ = self.search.search(&self.search_input, &self.analyzer.entries);
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
                            },
                        }
                    }
                }
            }

            if last_tick.elapsed() >= tick_rate {
                last_tick = Instant::now();
            }
        }
    }

    pub fn handle_key_event(&mut self, key_code: KeyCode) {
        match self.mode {
            AppMode::Normal => match key_code {
                KeyCode::Char('q') => {} // Can't test exit in the same way
                KeyCode::Char(':') => self.mode = AppMode::Command,
                KeyCode::Char('f') => {
                    self.mode = AppMode::FilterManagement;
                    self.show_sidebar = true
                }
                KeyCode::Char('s') => self.show_sidebar = !self.show_sidebar,
                KeyCode::Down => self.scroll_offset = self.scroll_offset.saturating_add(1),
                KeyCode::Up => self.scroll_offset = self.scroll_offset.saturating_sub(1),
                KeyCode::Char('m') => {
                    let logs_to_display = self.get_filtered_logs();
                    if let Some(log) = logs_to_display.get(self.scroll_offset) {
                        self.analyzer.toggle_mark(log.id);
                    }
                }
                KeyCode::Char('/') => {
                    self.mode = AppMode::Search;
                    self.search_input.clear();
                    self.search_forward = true;
                }
                KeyCode::Char('?') => {
                    self.mode = AppMode::Search;
                    self.search_input.clear();
                    self.search_forward = false;
                }
                KeyCode::Char('n') => {
                    if let Some(result) = self.search.next_match() {
                        let log_id = result.log_id;
                        self.scroll_to_log_entry(log_id);
                    }
                }
                KeyCode::Char('N') => {
                    if let Some(result) = self.search.previous_match() {
                        let log_id = result.log_id;
                        self.scroll_to_log_entry(log_id);
                    }
                }
                _ => {}
            },
            AppMode::Command => match key_code {
                KeyCode::Enter => {
                    self.handle_command();
                    self.command_input.clear();
                    self.mode = AppMode::Normal;
                }
                KeyCode::Esc => {
                    self.command_input.clear();
                    self.mode = AppMode::Normal;
                }
                KeyCode::Backspace => {
                    self.command_input.pop();
                }
                KeyCode::Char(c) => {
                    self.command_input.push(c);
                }
                _ => {}
            },
            AppMode::FilterManagement => match key_code {
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
                        self.editing_filter_id = Some(filter.id);
                        self.editing_filter_input = filter.pattern.clone();
                        self.mode = AppMode::FilterEdit;
                    }
                }
                _ => {}
            },
            AppMode::FilterEdit => match key_code {
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
            },
            AppMode::Search => match key_code {
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
            },
        }
    }

    fn ui(&mut self, frame: &mut Frame) {
        let size = frame.size();
        let main_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(1), // Main content (logs, command bar, command list)
                Constraint::Length(if self.show_sidebar { 30 } else { 0 }), // Sidebar
            ])
            .split(size);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),    // Logs
                Constraint::Length(1), // Command input
                Constraint::Length(3), // Command list
            ])
            .split(main_chunks[0]);

        let logs_to_display = self.get_filtered_logs();

        let num_logs = logs_to_display.len();
        if self.scroll_offset >= num_logs && num_logs > 0 {
            self.scroll_offset = num_logs - 1;
        }

        let log_lines: Vec<Line> = logs_to_display
            .iter()
            .map(|log| {
                let prefix = if log.marked { "* " } else { "  " };
                let mut spans = vec![Span::raw(prefix)];
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
                line
            })
            .collect();

        let paragraph = Paragraph::new(log_lines)
            .block(Block::default().borders(Borders::ALL).title("Logs"))
            .scroll((self.scroll_offset as u16, 0));
        frame.render_widget(paragraph, chunks[0]);

        let input_prefix = match self.mode {
            AppMode::Command => ":",
            AppMode::Search => if self.search_forward { "/" } else { "?" },
            _ => "",
        };

        let input_text = match self.mode {
            AppMode::Command => &self.command_input,
            AppMode::Search => &self.search_input,
            _ => "",
        };

        let command_line = Paragraph::new(format!("{}{}", input_prefix, input_text))
            .style(Style::default().fg(Color::White).bg(Color::DarkGray));
        frame.render_widget(command_line, chunks[1]);

        if let AppMode::Command = self.mode {
            frame.set_cursor(
                chunks[1].x + self.command_input.len() as u16 + 1, // +1 for the ':' character
                chunks[1].y,
            );
        } else if let AppMode::Search = self.mode {
            frame.set_cursor(
                chunks[1].x + self.search_input.len() as u16 + 1, // +1 for the '/' or '?' character
                chunks[1].y,
            );
        }

        let command_list =
            Paragraph::new(self.get_command_list()).block(Block::default().borders(Borders::ALL));
        frame.render_widget(command_list, chunks[2]);

        if self.show_sidebar {
            let sidebar_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1)])
                .split(main_chunks[1]);

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
                    Line::from(format!(
                        "{}{} {}: {}",
                        selected_prefix,
                        status,
                        filter.filter_type,
                        filter.pattern
                    ))
                })
                .collect();

            let sidebar = Paragraph::new(filters_text)
                .block(Block::default().borders(Borders::ALL).title("Filters"));
            frame.render_widget(sidebar, sidebar_chunks[0]);
        }
    }

    fn handle_command(&mut self) {
        let command_parts: Vec<&str> = self.command_input.splitn(2, ' ').collect();
        if let Some(cmd) = command_parts.first() {
            match *cmd {
                "filter" => {
                    if let Some(pattern) = command_parts.get(1) {
                        self.analyzer
                            .add_filter(pattern.to_string(), FilterType::Include);
                        self.scroll_offset = 0;
                    }
                }
                "exclude" => {
                    if let Some(pattern) = command_parts.get(1) {
                        self.analyzer
                            .add_filter(pattern.to_string(), FilterType::Exclude);
                        self.scroll_offset = 0;
                    }
                }
                "export-marked" => {
                    if let Some(path) = command_parts.get(1) {
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
                            if std::fs::write(path, marked_logs_content).is_ok() {
                                // Optionally, provide feedback to the user that the export was successful
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn get_command_list(&self) -> String {
        match self.mode {
            AppMode::Normal => {
                "[NORMAL] q: Quit | : : Command Mode | Up/Down: Scroll | f: Filter Management | s: Toggle Sidebar | m: Mark Line | /: Search Forward | ?: Search Backward | n: Next Match | N: Previous Match".to_string()
            },
            AppMode::Command => {
                "[COMMAND] filter | exclude | export-marked | Esc | Enter".to_string()
            },
            AppMode::FilterManagement => {
                "[FILTER] Esc: Exit | Up/Down: Select | Space: Toggle | d: Delete | e: Edit".to_string()
            },
            AppMode::FilterEdit => {
                "[FILTER EDIT] Esc: Cancel | Enter: Save".to_string()
            },
            AppMode::Search => {
                "[SEARCH] Esc: Cancel | Enter: Search".to_string()
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
