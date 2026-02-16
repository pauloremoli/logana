use ratatui::style::Color;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, serde_as};
use std::sync::Arc;

use std::sync::mpsc;

use crate::db::{Database, FilterStore, LogStore};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub enum LogLevel {
    Info,
    Warning,
    Error,
    Debug,
    #[default]
    Unknown,
}

impl LogLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Info => "INFO",
            LogLevel::Warning => "WARN",
            LogLevel::Error => "ERROR",
            LogLevel::Debug => "DEBUG",
            LogLevel::Unknown => "",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct LogEntry {
    pub id: usize,
    pub timestamp: Option<String>,
    pub hostname: Option<String>,
    pub process_name: Option<String>,
    pub pid: Option<u32>,
    pub level: LogLevel,
    pub message: String,
    pub marked: bool,
    pub source_file: Option<String>,
}

impl LogEntry {
    /// Returns the full display representation of this log entry,
    /// matching what is shown in the UI.
    pub fn display_line(&self) -> String {
        let mut line = String::new();
        if let Some(timestamp) = &self.timestamp {
            line.push_str(timestamp);
            line.push(' ');
        }
        if let Some(hostname) = &self.hostname {
            line.push_str(hostname);
            line.push(' ');
        }
        if let Some(process_name) = &self.process_name {
            line.push_str(process_name);
            line.push_str(": ");
        }
        if self.level != LogLevel::Unknown {
            line.push_str(self.level.as_str());
            line.push_str(": ");
        }
        line.push_str(&self.message);
        line
    }
}

#[derive(Debug)]
pub struct LogParser {
    regexes: Vec<Regex>,
}

impl LogParser {
    pub fn new() -> Self {
        let regexes = vec![
            // Syslog/journalctl: timestamp hostname process_name[pid]: LEVEL: message
            Regex::new(r"^(?P<timestamp>\w{3} \d{1,2} \d{2}:\d{2}:\d{2}(?:\.\d{1,6})?) (?P<hostname>\S+) (?P<process_name>[^\[:]+)\[(?P<pid>\d+)\]: (?P<level>\w+): (?P<message>.*)$").unwrap(),
            // Syslog/journalctl: timestamp hostname process_name[pid]: message (no level)
            Regex::new(r"^(?P<timestamp>\w{3} \d{1,2} \d{2}:\d{2}:\d{2}(?:\.\d{1,6})?) (?P<hostname>\S+) (?P<process_name>[^\[:]+)\[(?P<pid>\d+)\]: (?P<message>.*)$").unwrap(),
            // Syslog/journalctl: timestamp hostname process_name: LEVEL: message (no pid)
            Regex::new(r"^(?P<timestamp>\w{3} \d{1,2} \d{2}:\d{2}:\d{2}(?:\.\d{1,6})?) (?P<hostname>\S+) (?P<process_name>[^:]+): (?P<level>\w+): (?P<message>.*)$").unwrap(),
            // Syslog/journalctl: timestamp hostname process_name: message (no pid, no level)
            Regex::new(r"^(?P<timestamp>\w{3} \d{1,2} \d{2}:\d{2}:\d{2}(?:\.\d{1,6})?) (?P<hostname>\S+) (?P<process_name>[^:]+): (?P<message>.*)$").unwrap(),
        ];
        Self { regexes }
    }

    pub fn parse(&self, id: usize, log_content: &str) -> LogEntry {
        for re in &self.regexes {
            if let Some(caps) = re.captures(log_content) {
                let mut level = caps
                    .name("level")
                    .map_or(LogLevel::Unknown, |m| Self::parse_level_str(m.as_str()));
                if level == LogLevel::Unknown {
                    level = Self::detect_level_from_content(log_content);
                }

                return LogEntry {
                    id,
                    timestamp: caps.name("timestamp").map(|m| m.as_str().to_string()),
                    hostname: caps.name("hostname").map(|m| m.as_str().to_string()),
                    process_name: caps.name("process_name").map(|m| m.as_str().to_string()),
                    pid: caps.name("pid").and_then(|m| m.as_str().parse().ok()),
                    level,
                    message: caps
                        .name("message")
                        .map_or(log_content.to_string(), |m| m.as_str().to_string()),
                    marked: false,
                    source_file: None,
                };
            }
        }

        LogEntry {
            id,
            timestamp: None,
            hostname: None,
            process_name: None,
            pid: None,
            level: Self::detect_level_from_content(log_content),
            message: log_content.to_string(),
            marked: false,
            source_file: None,
        }
    }

    fn parse_level_str(s: &str) -> LogLevel {
        match s.to_lowercase().as_str() {
            "info" => LogLevel::Info,
            "warn" | "warning" => LogLevel::Warning,
            "error" | "err" => LogLevel::Error,
            "debug" => LogLevel::Debug,
            _ => LogLevel::Unknown,
        }
    }

    fn detect_level_from_content(content: &str) -> LogLevel {
        let upper = content.to_uppercase();
        if upper.contains("ERROR") || upper.contains(" ERR ") {
            LogLevel::Error
        } else if upper.contains("WARN") {
            LogLevel::Warning
        } else if upper.contains("INFO") {
            LogLevel::Info
        } else if upper.contains("DEBUG") {
            LogLevel::Debug
        } else {
            LogLevel::Unknown
        }
    }
}

impl Default for LogParser {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum FilterType {
    Include,
    Exclude,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Filter {
    pub id: usize,
    pub pattern: String,
    pub filter_type: FilterType,
    pub enabled: bool,
    pub color_config: Option<ColorConfig>,
}

#[serde_as]
#[derive(Debug, PartialEq, Serialize, Deserialize, Clone)]
pub struct ColorConfig {
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub fg: Option<Color>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub bg: Option<Color>,
}

#[derive(Debug, PartialEq, Clone)]
pub struct SearchResult {
    pub log_id: usize,
    pub matches: Vec<(usize, usize)>,
}

/// Standalone filter application — can be called from async tasks without &self.
pub fn apply_filters_to_logs(
    logs: &[LogEntry],
    filters: &[Filter],
) -> anyhow::Result<Vec<LogEntry>> {
    let enabled_filters: Vec<&Filter> = filters.iter().filter(|f| f.enabled).collect();

    let include_filters: Vec<Regex> = enabled_filters
        .iter()
        .filter(|f| f.filter_type == FilterType::Include)
        .map(|f| Regex::new(&f.pattern))
        .collect::<Result<Vec<_>, _>>()?;

    let exclude_filters: Vec<Regex> = enabled_filters
        .iter()
        .filter(|f| f.filter_type == FilterType::Exclude)
        .map(|f| Regex::new(&f.pattern))
        .collect::<Result<Vec<_>, _>>()?;

    let potentially_included_logs: Vec<LogEntry> = if include_filters.is_empty() {
        logs.to_vec()
    } else {
        logs.iter()
            .filter(|log_entry| {
                include_filters
                    .iter()
                    .any(|re| re.is_match(&log_entry.message))
            })
            .cloned()
            .collect()
    };

    if exclude_filters.is_empty() {
        return Ok(potentially_included_logs);
    }

    let final_logs = potentially_included_logs
        .into_iter()
        .filter(|log_entry| {
            !exclude_filters
                .iter()
                .any(|re| re.is_match(&log_entry.message))
        })
        .collect();

    Ok(final_logs)
}

#[derive(Debug)]
pub struct LogAnalyzer {
    pub(crate) db: Arc<Database>,
    pub(crate) rt: Arc<tokio::runtime::Runtime>,
    log_parser: LogParser,
    source_file: Option<String>,
}

impl LogAnalyzer {
    pub fn new(db: Arc<Database>, rt: Arc<tokio::runtime::Runtime>) -> Self {
        LogAnalyzer {
            db,
            rt,
            log_parser: LogParser::new(),
            source_file: None,
        }
    }

    pub fn set_source_file(&mut self, source: Option<String>) {
        self.source_file = source;
    }

    pub fn source_file(&self) -> Option<&str> {
        self.source_file.as_deref()
    }

    pub fn clear_logs(&self) {
        let _ = self.rt.block_on(self.db.clear_logs());
    }

    pub fn toggle_mark(&self, id: usize) {
        let _ = self.rt.block_on(self.db.toggle_mark(id as i64));
    }

    pub fn ingest_file(&self, path: &str) -> anyhow::Result<()> {
        use std::fs::File;
        use std::io::{self, BufRead};

        let file = File::open(path)?;
        let reader = io::BufReader::new(file);

        let source = Some(path.to_string());
        let mut entries = Vec::new();

        for (id, line) in reader.lines().enumerate() {
            let content = line?;
            let mut entry = self.log_parser.parse(id, &content);
            entry.source_file = source.clone();
            entries.push(entry);
        }

        self.rt.block_on(self.db.insert_logs_batch(&entries))?;
        Ok(())
    }

    /// Ingest a chunk of lines from a file, starting at `start_line`, up to `max_lines`.
    /// Returns the number of lines actually ingested (0 means EOF or past end).
    pub fn ingest_file_chunk(
        &self,
        path: &str,
        start_line: usize,
        max_lines: usize,
    ) -> anyhow::Result<usize> {
        use std::fs::File;
        use std::io::{self, BufRead};

        let file = File::open(path)?;
        let reader = io::BufReader::new(file);

        let source = Some(path.to_string());
        let mut entries = Vec::new();

        for (id, line) in reader.lines().enumerate().skip(start_line).take(max_lines) {
            let content = line?;
            let mut entry = self.log_parser.parse(id, &content);
            entry.source_file = source.clone();
            entries.push(entry);
        }

        let count = entries.len();
        if count > 0 {
            self.rt.block_on(self.db.insert_logs_batch(&entries))?;
        }
        Ok(count)
    }

    /// Spawn a background thread that reads the file sequentially (starting at `start_line`),
    /// parses lines, and inserts them into the database — all off the main thread.
    /// Returns a receiver of progress updates: `Ok(count)` for each batch inserted,
    /// or `Err(msg)` on failure. The channel disconnects when loading is complete.
    pub fn start_file_stream(
        &self,
        path: String,
        start_line: usize,
        batch_size: usize,
    ) -> mpsc::Receiver<Result<usize, String>> {
        let (tx, rx) = mpsc::channel();
        let parser = LogParser::new();
        let source = Some(path.clone());
        let db = self.db.clone();
        let rt = self.rt.clone();

        std::thread::spawn(move || {
            use std::fs::File;
            use std::io::{self, BufRead};

            let file = match File::open(&path) {
                Ok(f) => f,
                Err(e) => {
                    let _ = tx.send(Err(e.to_string()));
                    return;
                }
            };
            let reader = io::BufReader::new(file);
            let mut batch = Vec::with_capacity(batch_size);

            for (id, line) in reader.lines().enumerate().skip(start_line) {
                let content = match line {
                    Ok(c) => c,
                    Err(_) => break,
                };
                let mut entry = parser.parse(id, &content);
                entry.source_file = source.clone();
                batch.push(entry);

                if batch.len() >= batch_size {
                    let count = batch.len();
                    let to_insert = std::mem::replace(&mut batch, Vec::with_capacity(batch_size));
                    if let Err(e) = rt.block_on(db.insert_logs_batch(&to_insert)) {
                        let _ = tx.send(Err(e.to_string()));
                        return;
                    }
                    if tx.send(Ok(count)).is_err() {
                        return; // receiver dropped
                    }
                }
            }

            // Insert and send remaining entries
            if !batch.is_empty() {
                let count = batch.len();
                if let Err(e) = rt.block_on(db.insert_logs_batch(&batch)) {
                    let _ = tx.send(Err(e.to_string()));
                    return;
                }
                let _ = tx.send(Ok(count));
            }
            // tx drops here, signaling completion
        });

        rx
    }

    pub fn ingest_reader<R: std::io::Read>(&self, reader: R) -> anyhow::Result<()> {
        use std::io::{self, BufRead};

        let reader = io::BufReader::new(reader);
        let mut entries = Vec::new();

        for (id, line) in reader.lines().enumerate() {
            let content = line?;
            let entry = self.log_parser.parse(id, &content);
            entries.push(entry);
        }

        self.rt.block_on(self.db.insert_logs_batch(&entries))?;
        Ok(())
    }

    pub fn get_logs(&self) -> Vec<LogEntry> {
        self.rt.block_on(self.db.get_all_logs()).unwrap_or_default()
    }

    pub fn get_filters(&self) -> Vec<Filter> {
        if let Some(source) = &self.source_file {
            self.rt
                .block_on(self.db.get_filters_for_source(source))
                .unwrap_or_default()
        } else {
            self.rt.block_on(self.db.get_filters()).unwrap_or_default()
        }
    }

    pub fn add_filter(&self, pattern: String, filter_type: FilterType) {
        let _ = self.rt.block_on(self.db.insert_filter(
            &pattern,
            &filter_type,
            true,
            None,
            self.source_file.as_deref(),
        ));
    }

    pub fn add_filter_with_color(
        &self,
        pattern: String,
        filter_type: FilterType,
        fg: Option<&str>,
        bg: Option<&str>,
    ) {
        let color_config = match filter_type {
            FilterType::Include => {
                let fg_color = fg.and_then(|s| self.parse_color(s));
                let bg_color = bg.and_then(|s| self.parse_color(s));
                if fg_color.is_some() || bg_color.is_some() {
                    Some(ColorConfig {
                        fg: fg_color,
                        bg: bg_color,
                    })
                } else {
                    None
                }
            }
            FilterType::Exclude => None,
        };
        let _ = self.rt.block_on(self.db.insert_filter(
            &pattern,
            &filter_type,
            true,
            color_config.as_ref(),
            self.source_file.as_deref(),
        ));
    }

    pub fn toggle_filter(&self, id: usize) {
        let _ = self.rt.block_on(self.db.toggle_filter(id as i64));
    }

    pub fn remove_filter(&self, id: usize) {
        let _ = self.rt.block_on(self.db.delete_filter(id as i64));
    }

    pub fn clear_filters(&self) {
        let _ = self.rt.block_on(self.db.clear_filters());
    }

    pub fn edit_filter(&self, id: usize, new_pattern: String) {
        let _ = self
            .rt
            .block_on(self.db.update_filter_pattern(id as i64, &new_pattern));
    }

    pub fn apply_filters(&self, logs: &[LogEntry]) -> anyhow::Result<Vec<LogEntry>> {
        let filters = self.get_filters();
        apply_filters_to_logs(logs, &filters)
    }

    pub fn search(&self, pattern: &str) -> anyhow::Result<Vec<SearchResult>> {
        let re = Regex::new(pattern)?;
        let entries = self.get_logs();
        let mut results = Vec::new();

        for entry in &entries {
            let display = entry.display_line();
            let mut matches = Vec::new();
            for mat in re.find_iter(&display) {
                matches.push((mat.start(), mat.end()));
            }
            if !matches.is_empty() {
                results.push(SearchResult {
                    log_id: entry.id,
                    matches,
                });
            }
        }
        Ok(results)
    }

    pub fn move_filter_up(&self, id: usize) {
        let filters = self.get_filters();
        if let Some(index) = filters.iter().position(|f| f.id == id)
            && index > 0
        {
            let other_id = filters[index - 1].id;
            let _ = self
                .rt
                .block_on(self.db.swap_filter_order(id as i64, other_id as i64));
        }
    }

    pub fn move_filter_down(&self, id: usize) {
        let filters = self.get_filters();
        if let Some(index) = filters.iter().position(|f| f.id == id)
            && index < filters.len() - 1
        {
            let other_id = filters[index + 1].id;
            let _ = self
                .rt
                .block_on(self.db.swap_filter_order(id as i64, other_id as i64));
        }
    }

    pub fn save_filters(&self, path: &str) -> anyhow::Result<()> {
        let filters = self.get_filters();
        let json = serde_json::to_string_pretty(&filters)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load_filters(&self, path: &str) -> anyhow::Result<()> {
        let json = std::fs::read_to_string(path)?;
        let filters: Vec<Filter> = serde_json::from_str(&json)?;
        self.rt.block_on(
            self.db
                .replace_all_filters(&filters, self.source_file.as_deref()),
        )?;
        Ok(())
    }

    pub fn set_color_config(&self, pattern: &str, fg: Option<&str>, bg: Option<&str>) {
        let fg_color = fg.and_then(|s| self.parse_color(s));
        let bg_color = bg.and_then(|s| self.parse_color(s));
        if fg_color.is_none() && bg_color.is_none() {
            return;
        }
        let filters = self.get_filters();
        if let Some(filter) = filters
            .iter()
            .find(|f| f.pattern == pattern && f.filter_type == FilterType::Include)
        {
            let cc = ColorConfig {
                fg: fg_color,
                bg: bg_color,
            };
            let _ = self
                .rt
                .block_on(self.db.update_filter_color(filter.id as i64, Some(&cc)));
        }
    }

    pub fn get_marked_logs(&self) -> Vec<LogEntry> {
        self.rt
            .block_on(self.db.get_marked_logs())
            .unwrap_or_default()
    }

    pub fn has_logs_for_source(&self, source: &str) -> bool {
        self.rt
            .block_on(self.db.has_logs_for_source(source))
            .unwrap_or(false)
    }

    pub fn parse_fg_bg_args(args: &str) -> (Option<String>, Option<String>) {
        let mut fg = None;
        let mut bg = None;
        let mut tokens = args.split_whitespace().peekable();
        while let Some(token) = tokens.next() {
            match token {
                "--fg" => {
                    if let Some(val) = tokens.next() {
                        fg = Some(val.to_string());
                    }
                }
                "--bg" => {
                    if let Some(val) = tokens.next() {
                        bg = Some(val.to_string());
                    }
                }
                _ => {}
            }
        }
        (fg, bg)
    }

    pub fn parse_color(&self, color_str: &str) -> Option<Color> {
        color_str.parse::<Color>().ok()
    }

    pub fn extract_process_name(&self, log_content: &str) -> Option<String> {
        self.log_parser.parse(0, log_content).process_name
    }
}

impl std::fmt::Display for FilterType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FilterType::Include => write!(f, "Include"),
            FilterType::Exclude => write!(f, "Exclude"),
        }
    }
}

impl std::fmt::Display for LogEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn setup_analyzer() -> LogAnalyzer {
        let rt = Arc::new(tokio::runtime::Runtime::new().unwrap());
        let db = rt.block_on(Database::in_memory()).unwrap();
        LogAnalyzer::new(Arc::new(db), rt)
    }

    #[test]
    fn test_log_parser() {
        let parser = LogParser::new();
        let log1 = "Jun 28 10:00:03 myhost myapp[1234]: INFO: Application started successfully.";
        let entry1 = parser.parse(0, log1);
        assert_eq!(entry1.timestamp, Some("Jun 28 10:00:03".to_string()));
        assert_eq!(entry1.hostname, Some("myhost".to_string()));
        assert_eq!(entry1.process_name, Some("myapp".to_string()));
        assert_eq!(entry1.pid, Some(1234));
        assert_eq!(entry1.level, LogLevel::Info);
        assert_eq!(entry1.message, "Application started successfully.");

        let log2 = "Jun 28 10:00:02 myhost kernel: Linux version 6.8.0-31-generic";
        let entry2 = parser.parse(1, log2);
        assert_eq!(entry2.timestamp, Some("Jun 28 10:00:02".to_string()));
        assert_eq!(entry2.hostname, Some("myhost".to_string()));
        assert_eq!(entry2.process_name, Some("kernel".to_string()));
        assert_eq!(entry2.pid, None);
        assert_eq!(entry2.level, LogLevel::Unknown);
        assert_eq!(entry2.message, "Linux version 6.8.0-31-generic");
    }

    #[test]
    fn test_default_log_analyzer() {
        let analyzer = setup_analyzer();
        assert!(analyzer.get_logs().is_empty());
        assert!(analyzer.get_filters().is_empty());
    }

    #[test]
    fn test_ingest_file() -> anyhow::Result<()> {
        let mut file = NamedTempFile::new()?;
        writeln!(file, "log line 1")?;
        writeln!(file, "log line 2")?;
        writeln!(file, "log line 3")?;
        let path = file.path().to_str().unwrap();

        let analyzer = setup_analyzer();
        analyzer.ingest_file(path)?;

        let entries = analyzer.get_logs();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].id, 0);
        assert_eq!(entries[0].message, "log line 1");
        assert!(!entries[0].marked);
        assert_eq!(entries[1].id, 1);
        assert_eq!(entries[1].message, "log line 2");
        assert!(!entries[1].marked);
        assert_eq!(entries[2].id, 2);
        assert_eq!(entries[2].message, "log line 3");
        assert!(!entries[2].marked);

        Ok(())
    }

    #[test]
    fn test_ingest_reader() -> anyhow::Result<()> {
        let input = "stdin line 1\nstdin line 2\nstdin line 3\n";
        let cursor = Cursor::new(input.as_bytes());

        let analyzer = setup_analyzer();
        analyzer.ingest_reader(cursor)?;

        let entries = analyzer.get_logs();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].id, 0);
        assert_eq!(entries[0].message, "stdin line 1");
        assert!(!entries[0].marked);
        assert_eq!(entries[1].id, 1);
        assert_eq!(entries[1].message, "stdin line 2");
        assert!(!entries[1].marked);
        assert_eq!(entries[2].id, 2);
        assert_eq!(entries[2].message, "stdin line 3");
        assert!(!entries[2].marked);

        Ok(())
    }

    #[test]
    fn test_toggle_mark() {
        let analyzer = setup_analyzer();
        let entries = vec![
            LogEntry {
                id: 0,
                message: "line 1".to_string(),
                level: LogLevel::Info,
                ..Default::default()
            },
            LogEntry {
                id: 1,
                message: "line 2".to_string(),
                level: LogLevel::Info,
                ..Default::default()
            },
        ];
        analyzer
            .rt
            .block_on(analyzer.db.insert_logs_batch(&entries))
            .unwrap();

        let logs = analyzer.get_logs();
        assert!(!logs[0].marked);
        assert!(!logs[1].marked);

        analyzer.toggle_mark(0);
        let logs = analyzer.get_logs();
        assert!(logs[0].marked);
        assert!(!logs[1].marked);

        analyzer.toggle_mark(0);
        let logs = analyzer.get_logs();
        assert!(!logs[0].marked);
        assert!(!logs[1].marked);

        analyzer.toggle_mark(1);
        let logs = analyzer.get_logs();
        assert!(!logs[0].marked);
        assert!(logs[1].marked);
    }

    #[test]
    fn test_search_basic() -> anyhow::Result<()> {
        let analyzer = setup_analyzer();
        let entries = vec![
            LogEntry {
                id: 0,
                message: "This is a test line".to_string(),
                ..Default::default()
            },
            LogEntry {
                id: 1,
                message: "Another line".to_string(),
                level: LogLevel::Info,
                ..Default::default()
            },
        ];
        analyzer
            .rt
            .block_on(analyzer.db.insert_logs_batch(&entries))
            .unwrap();

        let results = analyzer.search("test")?;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].log_id, 0);
        assert_eq!(results[0].matches, vec![(10, 14)]);
        Ok(())
    }

    #[test]
    fn test_search_multiple_matches_in_one_line() -> anyhow::Result<()> {
        let analyzer = setup_analyzer();
        let entries = vec![LogEntry {
            id: 0,
            message: "test test test".to_string(),
            level: LogLevel::Info,
            ..Default::default()
        }];
        analyzer
            .rt
            .block_on(analyzer.db.insert_logs_batch(&entries))
            .unwrap();

        let results = analyzer.search("test")?;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].log_id, 0);
        assert_eq!(results[0].matches, vec![(6, 10), (11, 15), (16, 20)]);
        Ok(())
    }

    #[test]
    fn test_search_no_match() -> anyhow::Result<()> {
        let analyzer = setup_analyzer();
        let entries = vec![LogEntry {
            id: 0,
            message: "This is a line".to_string(),
            level: LogLevel::Info,
            ..Default::default()
        }];
        analyzer
            .rt
            .block_on(analyzer.db.insert_logs_batch(&entries))
            .unwrap();

        let results = analyzer.search("nomatch")?;
        assert!(results.is_empty());
        Ok(())
    }

    #[test]
    fn test_search_case_insensitive() -> anyhow::Result<()> {
        let analyzer = setup_analyzer();
        let entries = vec![LogEntry {
            id: 0,
            message: "Test line test".to_string(),
            level: LogLevel::Info,
            ..Default::default()
        }];
        analyzer
            .rt
            .block_on(analyzer.db.insert_logs_batch(&entries))
            .unwrap();

        let results = analyzer.search("(?i)test")?;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].log_id, 0);
        assert_eq!(results[0].matches, vec![(6, 10), (16, 20)]);
        Ok(())
    }

    #[test]
    fn test_search_regex_special_chars() -> anyhow::Result<()> {
        let analyzer = setup_analyzer();
        let entries = vec![LogEntry {
            id: 0,
            message: "line with .*".to_string(),
            level: LogLevel::Info,
            ..Default::default()
        }];
        analyzer
            .rt
            .block_on(analyzer.db.insert_logs_batch(&entries))
            .unwrap();

        let results = analyzer.search(r".*")?;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].log_id, 0);
        assert_eq!(results[0].matches, vec![(0, 18)]);
        Ok(())
    }

    #[test]
    fn test_filter_management() {
        let analyzer = setup_analyzer();
        assert_eq!(analyzer.get_filters().len(), 0);

        // Add
        analyzer.add_filter("error".to_string(), FilterType::Include);
        let filters = analyzer.get_filters();
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].pattern, "error");
        assert_eq!(filters[0].filter_type, FilterType::Include);
        assert!(filters[0].enabled);

        analyzer.add_filter("info".to_string(), FilterType::Exclude);
        let filters = analyzer.get_filters();
        assert_eq!(filters.len(), 2);

        // Toggle
        let id = filters[0].id;
        analyzer.toggle_filter(id);
        let filters = analyzer.get_filters();
        assert!(!filters[0].enabled);
        analyzer.toggle_filter(id);
        let filters = analyzer.get_filters();
        assert!(filters[0].enabled);

        // Edit
        let id2 = filters[1].id;
        analyzer.edit_filter(id2, "debug".to_string());
        let filters = analyzer.get_filters();
        assert_eq!(filters[1].pattern, "debug");

        // Remove
        analyzer.remove_filter(id);
        let filters = analyzer.get_filters();
        assert_eq!(filters.len(), 1);

        // Clear
        analyzer.clear_filters();
        let filters = analyzer.get_filters();
        assert_eq!(filters.len(), 0);
    }

    #[test]
    fn test_apply_filters() -> anyhow::Result<()> {
        let analyzer = setup_analyzer();
        let entries = vec![
            LogEntry {
                id: 0,
                message: "error: critical issue ".to_string(),
                ..Default::default()
            },
            LogEntry {
                id: 1,
                message: "info: user logged in ".to_string(),
                ..Default::default()
            },
            LogEntry {
                id: 2,
                message: "error: minor issue ".to_string(),
                ..Default::default()
            },
            LogEntry {
                id: 3,
                message: "debug: value is 5".to_string(),
                ..Default::default()
            },
            LogEntry {
                id: 4,
                message: "info: user logged out ".to_string(),
                ..Default::default()
            },
        ];
        analyzer
            .rt
            .block_on(analyzer.db.insert_logs_batch(&entries))
            .unwrap();

        // Scenario 1: No filters
        let all_logs = analyzer.get_logs();
        let filtered = analyzer.apply_filters(&all_logs)?;
        assert_eq!(filtered.len(), 5, "Scenario 1 Failed: No filters ");

        // Scenario 2: Only include filters
        analyzer.add_filter("error".to_string(), FilterType::Include);
        let all_logs = analyzer.get_logs();
        let filtered = analyzer.apply_filters(&all_logs)?;
        assert_eq!(filtered.len(), 2, "Scenario 2 Failed: Only include ");
        assert!(filtered.iter().any(|l| l.id == 0));
        assert!(filtered.iter().any(|l| l.id == 2));

        // Scenario 3: Only exclude filters
        analyzer.clear_filters();
        analyzer.add_filter("info".to_string(), FilterType::Exclude);
        let all_logs = analyzer.get_logs();
        let filtered = analyzer.apply_filters(&all_logs)?;
        assert_eq!(filtered.len(), 3, "Scenario 3 Failed: Only exclude ");
        assert!(!filtered.iter().any(|l| l.id == 1 || l.id == 4));

        // Scenario 4: Include and Exclude, no overlap
        analyzer.clear_filters();
        analyzer.add_filter("error".to_string(), FilterType::Include);
        analyzer.add_filter("info".to_string(), FilterType::Exclude);
        let all_logs = analyzer.get_logs();
        let filtered = analyzer.apply_filters(&all_logs)?;
        assert_eq!(
            filtered.len(),
            2,
            "Scenario 4 Failed: Include and Exclude, no overlap "
        );
        assert!(filtered.iter().any(|l| l.id == 0));
        assert!(filtered.iter().any(|l| l.id == 2));

        // Scenario 5: Include and Exclude, with overlap
        analyzer.clear_filters();
        analyzer.add_filter("error".to_string(), FilterType::Include);
        analyzer.add_filter("critical".to_string(), FilterType::Exclude);
        let all_logs = analyzer.get_logs();
        let filtered = analyzer.apply_filters(&all_logs)?;
        assert_eq!(
            filtered.len(),
            1,
            "Scenario 5 Failed: Include and Exclude, with overlap "
        );
        assert_eq!(filtered[0].id, 2);

        // Scenario 6: Disabled include filter
        let filters = analyzer.get_filters();
        let include_id = filters.iter().find(|f| f.pattern == "error").unwrap().id;
        analyzer.toggle_filter(include_id);
        let all_logs = analyzer.get_logs();
        let filtered = analyzer.apply_filters(&all_logs)?;
        assert_eq!(filtered.len(), 4, "Scenario 6 Failed: Disabled include");
        assert!(!filtered.iter().any(|l| l.id == 0));

        // Scenario 7: Disabled exclude filter
        analyzer.toggle_filter(include_id); // re-enable
        let filters = analyzer.get_filters();
        let exclude_id = filters.iter().find(|f| f.pattern == "critical").unwrap().id;
        analyzer.toggle_filter(exclude_id);
        let all_logs = analyzer.get_logs();
        let filtered = analyzer.apply_filters(&all_logs)?;
        assert_eq!(filtered.len(), 2, "Scenario 7 Failed: Disabled exclude");
        assert!(filtered.iter().any(|l| l.id == 0));
        assert!(filtered.iter().any(|l| l.id == 2));

        analyzer.clear_filters();
        Ok(())
    }

    #[test]
    fn test_regex_crate_dot_star_behavior() -> anyhow::Result<()> {
        use regex::Regex;
        let re = Regex::new(r".*")?;
        let test_string = "hello world";
        let mut matches = Vec::new();
        for mat in re.find_iter(test_string) {
            matches.push((mat.start(), mat.end()));
        }
        assert_eq!(matches, vec![(0, 11)]);
        Ok(())
    }

    #[test]
    fn test_move_filter_up() -> anyhow::Result<()> {
        let analyzer = setup_analyzer();
        analyzer.add_filter("filter1".to_string(), FilterType::Include);
        analyzer.add_filter("filter2".to_string(), FilterType::Exclude);
        analyzer.add_filter("filter3".to_string(), FilterType::Include);

        let filters = analyzer.get_filters();
        let id2 = filters[1].id;

        analyzer.move_filter_up(id2);
        let filters = analyzer.get_filters();
        assert_eq!(filters[0].pattern, "filter2");
        assert_eq!(filters[1].pattern, "filter1");
        assert_eq!(filters[2].pattern, "filter3");

        Ok(())
    }

    #[test]
    fn test_move_filter_up_at_top() -> anyhow::Result<()> {
        let analyzer = setup_analyzer();
        analyzer.add_filter("filter1".to_string(), FilterType::Include);
        analyzer.add_filter("filter2".to_string(), FilterType::Exclude);

        let filters = analyzer.get_filters();
        let id1 = filters[0].id;

        analyzer.move_filter_up(id1);
        let filters = analyzer.get_filters();
        assert_eq!(filters[0].pattern, "filter1");
        assert_eq!(filters[1].pattern, "filter2");

        Ok(())
    }

    #[test]
    fn test_move_filter_down() -> anyhow::Result<()> {
        let analyzer = setup_analyzer();
        analyzer.add_filter("filter1".to_string(), FilterType::Include);
        analyzer.add_filter("filter2".to_string(), FilterType::Exclude);
        analyzer.add_filter("filter3".to_string(), FilterType::Include);

        let filters = analyzer.get_filters();
        let id2 = filters[1].id;

        analyzer.move_filter_down(id2);
        let filters = analyzer.get_filters();
        assert_eq!(filters[0].pattern, "filter1");
        assert_eq!(filters[1].pattern, "filter3");
        assert_eq!(filters[2].pattern, "filter2");

        Ok(())
    }

    #[test]
    fn test_move_filter_down_at_bottom() -> anyhow::Result<()> {
        let analyzer = setup_analyzer();
        analyzer.add_filter("filter1".to_string(), FilterType::Include);
        analyzer.add_filter("filter2".to_string(), FilterType::Exclude);

        let filters = analyzer.get_filters();
        let id2 = filters[1].id;

        analyzer.move_filter_down(id2);
        let filters = analyzer.get_filters();
        assert_eq!(filters[0].pattern, "filter1");
        assert_eq!(filters[1].pattern, "filter2");

        Ok(())
    }

    #[test]
    fn test_save_and_load_filters() -> anyhow::Result<()> {
        let analyzer = setup_analyzer();
        analyzer.add_filter("error".to_string(), FilterType::Include);
        analyzer.add_filter("info".to_string(), FilterType::Exclude);

        let file = NamedTempFile::new()?;
        let path = file.path().to_str().unwrap();

        analyzer.save_filters(path)?;

        // Create a new analyzer and load filters
        let analyzer2 = setup_analyzer();
        analyzer2.load_filters(path)?;

        let filters = analyzer2.get_filters();
        assert_eq!(filters.len(), 2);
        assert_eq!(filters[0].pattern, "error");
        assert_eq!(filters[0].filter_type, FilterType::Include);
        assert_eq!(filters[1].pattern, "info");
        assert_eq!(filters[1].filter_type, FilterType::Exclude);

        Ok(())
    }

    #[test]
    fn test_has_logs_for_source() {
        let analyzer = setup_analyzer();
        let entries = vec![LogEntry {
            id: 0,
            message: "test".to_string(),
            source_file: Some("myfile.log".to_string()),
            ..Default::default()
        }];
        analyzer
            .rt
            .block_on(analyzer.db.insert_logs_batch(&entries))
            .unwrap();

        assert!(analyzer.has_logs_for_source("myfile.log"));
        assert!(!analyzer.has_logs_for_source("other.log"));
    }

    #[test]
    fn test_get_marked_logs() {
        let analyzer = setup_analyzer();
        let entries = vec![
            LogEntry {
                id: 0,
                message: "line 1".to_string(),
                ..Default::default()
            },
            LogEntry {
                id: 1,
                message: "line 2".to_string(),
                ..Default::default()
            },
        ];
        analyzer
            .rt
            .block_on(analyzer.db.insert_logs_batch(&entries))
            .unwrap();

        analyzer.toggle_mark(0);
        let marked = analyzer.get_marked_logs();
        assert_eq!(marked.len(), 1);
        assert_eq!(marked[0].id, 0);
    }

    #[test]
    fn test_clear_logs() {
        let analyzer = setup_analyzer();
        let entries = vec![
            LogEntry {
                id: 0,
                message: "line 1".to_string(),
                ..Default::default()
            },
            LogEntry {
                id: 1,
                message: "line 2".to_string(),
                ..Default::default()
            },
        ];
        analyzer
            .rt
            .block_on(analyzer.db.insert_logs_batch(&entries))
            .unwrap();
        assert_eq!(analyzer.get_logs().len(), 2);

        analyzer.clear_logs();
        assert_eq!(analyzer.get_logs().len(), 0);
    }

    #[test]
    fn test_clear_logs_preserves_filters() {
        let analyzer = setup_analyzer();
        let entries = vec![LogEntry {
            id: 0,
            message: "line 1".to_string(),
            ..Default::default()
        }];
        analyzer
            .rt
            .block_on(analyzer.db.insert_logs_batch(&entries))
            .unwrap();
        analyzer.add_filter("error".to_string(), FilterType::Include);
        analyzer.add_filter("debug".to_string(), FilterType::Exclude);

        analyzer.clear_logs();

        assert_eq!(analyzer.get_logs().len(), 0);
        let filters = analyzer.get_filters();
        assert_eq!(filters.len(), 2);
        assert_eq!(filters[0].pattern, "error");
        assert_eq!(filters[1].pattern, "debug");
    }

    #[test]
    fn test_clear_logs_then_reingest() -> anyhow::Result<()> {
        let analyzer = setup_analyzer();

        // Ingest first batch with ids 0, 1
        let entries1 = vec![
            LogEntry {
                id: 0,
                message: "file1 line 1".to_string(),
                source_file: Some("file1.log".to_string()),
                ..Default::default()
            },
            LogEntry {
                id: 1,
                message: "file1 line 2".to_string(),
                source_file: Some("file1.log".to_string()),
                ..Default::default()
            },
        ];
        analyzer
            .rt
            .block_on(analyzer.db.insert_logs_batch(&entries1))?;
        assert_eq!(analyzer.get_logs().len(), 2);

        // Clear and re-ingest with same ids (simulates opening a different file)
        analyzer.clear_logs();

        let entries2 = vec![
            LogEntry {
                id: 0,
                message: "file2 line 1".to_string(),
                source_file: Some("file2.log".to_string()),
                ..Default::default()
            },
            LogEntry {
                id: 1,
                message: "file2 line 2".to_string(),
                source_file: Some("file2.log".to_string()),
                ..Default::default()
            },
        ];
        analyzer
            .rt
            .block_on(analyzer.db.insert_logs_batch(&entries2))?;

        let logs = analyzer.get_logs();
        assert_eq!(logs.len(), 2);
        assert_eq!(logs[0].message, "file2 line 1");
        assert_eq!(logs[1].message, "file2 line 2");
        Ok(())
    }

    #[test]
    fn test_display_line_all_fields() {
        let entry = LogEntry {
            id: 0,
            timestamp: Some("Jun 28 10:00:03".to_string()),
            hostname: Some("myhost".to_string()),
            process_name: Some("myapp".to_string()),
            pid: Some(1234),
            level: LogLevel::Info,
            message: "Application started".to_string(),
            ..Default::default()
        };
        assert_eq!(
            entry.display_line(),
            "Jun 28 10:00:03 myhost myapp: INFO: Application started"
        );
    }

    #[test]
    fn test_display_line_message_only() {
        let entry = LogEntry {
            id: 0,
            message: "plain log line".to_string(),
            ..Default::default()
        };
        assert_eq!(entry.display_line(), "plain log line");
    }

    #[test]
    fn test_display_line_partial_fields() {
        let entry = LogEntry {
            id: 0,
            timestamp: Some("Jun 28 10:00:03".to_string()),
            message: "some message".to_string(),
            ..Default::default()
        };
        assert_eq!(entry.display_line(), "Jun 28 10:00:03 some message");
    }

    #[test]
    fn test_search_matches_timestamp() -> anyhow::Result<()> {
        let analyzer = setup_analyzer();
        let entries = vec![LogEntry {
            id: 0,
            timestamp: Some("Jun 28 10:00:03".to_string()),
            hostname: Some("myhost".to_string()),
            process_name: Some("myapp".to_string()),
            message: "Application started".to_string(),
            ..Default::default()
        }];
        analyzer
            .rt
            .block_on(analyzer.db.insert_logs_batch(&entries))?;

        let results = analyzer.search("Jun 28")?;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].log_id, 0);
        assert_eq!(results[0].matches[0], (0, 6));
        Ok(())
    }

    #[test]
    fn test_search_matches_hostname() -> anyhow::Result<()> {
        let analyzer = setup_analyzer();
        let entries = vec![LogEntry {
            id: 0,
            timestamp: Some("Jun 28 10:00:03".to_string()),
            hostname: Some("myhost".to_string()),
            message: "Application started".to_string(),
            ..Default::default()
        }];
        analyzer
            .rt
            .block_on(analyzer.db.insert_logs_batch(&entries))?;

        let results = analyzer.search("myhost")?;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].log_id, 0);
        Ok(())
    }

    #[test]
    fn test_search_matches_process_name() -> anyhow::Result<()> {
        let analyzer = setup_analyzer();
        let entries = vec![LogEntry {
            id: 0,
            hostname: Some("myhost".to_string()),
            process_name: Some("myapp".to_string()),
            message: "Application started".to_string(),
            ..Default::default()
        }];
        analyzer
            .rt
            .block_on(analyzer.db.insert_logs_batch(&entries))?;

        let results = analyzer.search("myapp")?;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].log_id, 0);
        Ok(())
    }

    #[test]
    fn test_search_spans_across_fields() -> anyhow::Result<()> {
        let analyzer = setup_analyzer();
        let entries = vec![LogEntry {
            id: 0,
            hostname: Some("server1".to_string()),
            process_name: Some("nginx".to_string()),
            message: "200 OK".to_string(),
            ..Default::default()
        }];
        analyzer
            .rt
            .block_on(analyzer.db.insert_logs_batch(&entries))?;

        // Search a pattern that spans hostname and process_name boundary
        let results = analyzer.search("server1 nginx")?;
        assert_eq!(results.len(), 1);
        Ok(())
    }

    #[test]
    fn test_detect_level_from_message_content() {
        let parser = LogParser::new();

        // Plain lines with level keywords in the message
        let entry = parser.parse(0, "ERROR: something failed");
        assert_eq!(entry.level, LogLevel::Error);

        let entry = parser.parse(1, "WARN: disk space low");
        assert_eq!(entry.level, LogLevel::Warning);

        let entry = parser.parse(2, "INFO: service started");
        assert_eq!(entry.level, LogLevel::Info);

        let entry = parser.parse(3, "DEBUG: dumping state");
        assert_eq!(entry.level, LogLevel::Debug);

        let entry = parser.parse(4, "just a plain log line");
        assert_eq!(entry.level, LogLevel::Unknown);
    }

    #[test]
    fn test_detect_level_fallback_in_syslog_format() {
        let parser = LogParser::new();

        // Syslog format where the regex captures a non-level word as "level",
        // but the message contains the actual level keyword
        let entry = parser.parse(0, "Jun 28 10:00:03 myhost kernel: ERROR: something broke");
        assert_eq!(entry.level, LogLevel::Error);

        let entry = parser.parse(1, "Jun 28 10:00:03 myhost app: WARNING: disk full");
        assert_eq!(entry.level, LogLevel::Warning);
    }

    #[test]
    fn test_detect_level_case_insensitive() {
        let parser = LogParser::new();

        let entry = parser.parse(0, "error happened here");
        assert_eq!(entry.level, LogLevel::Error);

        let entry = parser.parse(1, "Warning: check this");
        assert_eq!(entry.level, LogLevel::Warning);
    }

    #[test]
    fn test_ingest_file_chunk_first_chunk() -> anyhow::Result<()> {
        let mut file = NamedTempFile::new()?;
        for i in 0..100 {
            writeln!(file, "line {}", i)?;
        }
        let path = file.path().to_str().unwrap();

        let analyzer = setup_analyzer();
        let count = analyzer.ingest_file_chunk(path, 0, 10)?;
        assert_eq!(count, 10);
        let logs = analyzer.get_logs();
        assert_eq!(logs.len(), 10);
        assert_eq!(logs[0].message, "line 0");
        assert_eq!(logs[9].message, "line 9");
        Ok(())
    }

    #[test]
    fn test_ingest_file_chunk_subsequent_chunk() -> anyhow::Result<()> {
        let mut file = NamedTempFile::new()?;
        for i in 0..100 {
            writeln!(file, "line {}", i)?;
        }
        let path = file.path().to_str().unwrap();

        let analyzer = setup_analyzer();
        let count1 = analyzer.ingest_file_chunk(path, 0, 10)?;
        assert_eq!(count1, 10);
        let count2 = analyzer.ingest_file_chunk(path, 10, 10)?;
        assert_eq!(count2, 10);
        let logs = analyzer.get_logs();
        assert_eq!(logs.len(), 20);
        assert_eq!(logs[10].message, "line 10");
        assert_eq!(logs[19].message, "line 19");
        Ok(())
    }

    #[test]
    fn test_ingest_file_chunk_past_eof() -> anyhow::Result<()> {
        let mut file = NamedTempFile::new()?;
        for i in 0..5 {
            writeln!(file, "line {}", i)?;
        }
        let path = file.path().to_str().unwrap();

        let analyzer = setup_analyzer();
        let count = analyzer.ingest_file_chunk(path, 0, 100)?;
        assert_eq!(count, 5);

        // Past end returns 0
        let count2 = analyzer.ingest_file_chunk(path, 5, 100)?;
        assert_eq!(count2, 0);
        Ok(())
    }
}
