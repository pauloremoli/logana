use ratatui::style::Color;
use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, serde_as};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LogEntry {
    pub id: usize,
    pub content: String,
    pub marked: bool,
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
}

#[serde_as]
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct ColorConfig {
    #[serde_as(as = "DisplayFromStr")]
    pub fg: Color,
    #[serde_as(as = "DisplayFromStr")]
    pub bg: Color,
}

#[derive(Debug, PartialEq, Clone)]
pub struct SearchResult {
    pub log_id: usize,
    pub matches: Vec<(usize, usize)>, // (start_index, end_index) of the match
}

#[derive(Debug)]
pub struct LogAnalyzer {
    pub entries: Vec<LogEntry>,
    pub filters: Vec<Filter>,
    next_filter_id: usize,
    pub color_configs: Vec<(String, ColorConfig)>,
}

impl Default for LogAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl LogAnalyzer {
    pub fn new() -> Self {
        LogAnalyzer {
            entries: Vec::new(),
            filters: Vec::new(),
            next_filter_id: 0,
            color_configs: Vec::new(),
        }
    }

    pub fn toggle_mark(&mut self, id: usize) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) {
            entry.marked = !entry.marked;
        }
    }

    pub fn ingest_file(&mut self, path: &str) -> anyhow::Result<()> {
        use std::fs::File;
        use std::io::{self, BufRead};

        let file = File::open(path)?;
        let reader = io::BufReader::new(file);

        for (id, line) in reader.lines().enumerate() {
            self.entries.push(LogEntry {
                id,
                content: line?,
                marked: false,
            });
        }
        Ok(())
    }

    pub fn ingest_reader<R: std::io::Read>(&mut self, reader: R) -> anyhow::Result<()> {
        use std::io::{self, BufRead};

        let reader = io::BufReader::new(reader);
        for (id, line) in reader.lines().enumerate() {
            self.entries.push(LogEntry {
                id,
                content: line?,
                marked: false,
            });
        }
        Ok(())
    }

    pub fn get_logs(&self) -> &Vec<LogEntry> {
        &self.entries
    }

    pub fn add_filter(&mut self, pattern: String, filter_type: FilterType) {
        self.filters.push(Filter {
            id: self.next_filter_id,
            pattern,
            filter_type,
            enabled: true,
        });
        self.next_filter_id += 1;
    }

    pub fn toggle_filter(&mut self, id: usize) {
        if let Some(filter) = self.filters.iter_mut().find(|f| f.id == id) {
            filter.enabled = !filter.enabled;
        }
    }

    pub fn remove_filter(&mut self, id: usize) {
        self.filters.retain(|f| f.id != id);
    }

    pub fn clear_filters(&mut self) {
        self.filters.clear();
    }

    pub fn edit_filter(&mut self, id: usize, new_pattern: String) {
        if let Some(filter) = self.filters.iter_mut().find(|f| f.id == id) {
            filter.pattern = new_pattern;
        }
    }

    pub fn apply_filters(&self, logs: &[LogEntry]) -> anyhow::Result<Vec<LogEntry>> {
        use regex::Regex;

        let enabled_filters: Vec<&Filter> = self.filters.iter().filter(|f| f.enabled).collect();

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
                        .any(|re| re.is_match(&log_entry.content))
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
                    .any(|re| re.is_match(&log_entry.content))
            })
            .collect();

        Ok(final_logs)
    }

    pub fn search(&self, pattern: &str) -> anyhow::Result<Vec<SearchResult>> {
        use regex::Regex;
        let re = Regex::new(pattern)?;
        let mut results = Vec::new();

        for entry in &self.entries {
            let mut matches = Vec::new();
            for mat in re.find_iter(&entry.content) {
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

    pub fn move_filter_up(&mut self, id: usize) {
        if let Some(index) = self.filters.iter().position(|f| f.id == id) {
            if index > 0 {
                self.filters.swap(index, index - 1);
            }
        }
    }

    pub fn move_filter_down(&mut self, id: usize) {
        if let Some(index) = self.filters.iter().position(|f| f.id == id) {
            if index < self.filters.len() - 1 {
                self.filters.swap(index, index + 1);
            }
        }
    }

    pub fn save_filters(&self, path: &str) -> anyhow::Result<()> {
        let json = serde_json::to_string_pretty(&self.filters)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load_filters(&mut self, path: &str) -> anyhow::Result<()> {
        let json = std::fs::read_to_string(path)?;
        self.filters = serde_json::from_str(&json)?;
        self.next_filter_id = self.filters.iter().map(|f| f.id).max().unwrap_or(0) + 1;
        Ok(())
    }

    pub fn set_color_config(&mut self, pattern: &str, fg: &str, bg: &str) {
        let fg_color = self.parse_color(fg);
        let bg_color = self.parse_color(bg);

        if let (Some(fg), Some(bg)) = (fg_color, bg_color) {
            self.color_configs
                .push((pattern.to_string(), ColorConfig { fg, bg }));
        }
    }

    pub fn get_color_config(&self, pattern: &str) -> Option<&ColorConfig> {
        self.color_configs
            .iter()
            .find(|(p, _)| p == pattern)
            .map(|(_, c)| c)
    }

    fn parse_color(&self, color_str: &str) -> Option<Color> {
        match color_str.to_lowercase().as_str() {
            "black" => Some(Color::Black),
            "red" => Some(Color::Red),
            "green" => Some(Color::Green),
            "yellow" => Some(Color::Yellow),
            "blue" => Some(Color::Blue),
            "magenta" => Some(Color::Magenta),
            "cyan" => Some(Color::Cyan),
            "white" => Some(Color::White),
            _ => None,
        }
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
        write!(f, "{}", self.content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_default_log_analyzer() {
        let analyzer = LogAnalyzer::default();
        assert!(analyzer.entries.is_empty());
        assert!(analyzer.filters.is_empty());
        assert_eq!(analyzer.next_filter_id, 0);
    }

    #[test]
    fn test_ingest_file() -> anyhow::Result<()> {
        let mut file = NamedTempFile::new()?;
        writeln!(file, "log line 1")?;
        writeln!(file, "log line 2")?;
        writeln!(file, "log line 3")?;
        let path = file.path().to_str().unwrap();

        let mut analyzer = LogAnalyzer::new();
        analyzer.ingest_file(path)?;

        assert_eq!(analyzer.entries.len(), 3);
        assert_eq!(analyzer.entries[0].id, 0);
        assert_eq!(analyzer.entries[0].content, "log line 1");
        assert!(!analyzer.entries[0].marked);
        assert_eq!(analyzer.entries[1].id, 1);
        assert_eq!(analyzer.entries[1].content, "log line 2");
        assert!(!analyzer.entries[1].marked);
        assert_eq!(analyzer.entries[2].id, 2);
        assert_eq!(analyzer.entries[2].content, "log line 3");
        assert!(!analyzer.entries[2].marked);

        Ok(())
    }

    #[test]
    fn test_ingest_reader() -> anyhow::Result<()> {
        let input = "stdin line 1\nstdin line 2\nstdin line 3\n";
        let cursor = Cursor::new(input.as_bytes());

        let mut analyzer = LogAnalyzer::new();
        analyzer.ingest_reader(cursor)?;

        assert_eq!(analyzer.entries.len(), 3);
        assert_eq!(analyzer.entries[0].id, 0);
        assert_eq!(analyzer.entries[0].content, "stdin line 1");
        assert!(!analyzer.entries[0].marked);
        assert_eq!(analyzer.entries[1].id, 1);
        assert_eq!(analyzer.entries[1].content, "stdin line 2");
        assert!(!analyzer.entries[1].marked);
        assert_eq!(analyzer.entries[2].id, 2);
        assert_eq!(analyzer.entries[2].content, "stdin line 3");
        assert!(!analyzer.entries[2].marked);

        Ok(())
    }

    #[test]
    fn test_toggle_mark() {
        let mut analyzer = LogAnalyzer::new();
        analyzer.entries.push(LogEntry {
            id: 0,
            content: "line 1".to_string(),
            marked: false,
        });
        analyzer.entries.push(LogEntry {
            id: 1,
            content: "line 2".to_string(),
            marked: false,
        });

        assert!(!analyzer.entries[0].marked);
        assert!(!analyzer.entries[1].marked);

        analyzer.toggle_mark(0);
        assert!(analyzer.entries[0].marked);
        assert!(!analyzer.entries[1].marked);

        analyzer.toggle_mark(0);
        assert!(!analyzer.entries[0].marked);
        assert!(!analyzer.entries[1].marked);

        analyzer.toggle_mark(1);
        assert!(!analyzer.entries[0].marked);
        assert!(analyzer.entries[1].marked);
    }

    #[test]
    fn test_regex_direct_match() -> anyhow::Result<()> {
        use regex::Regex;
        let re = Regex::new("test")?;
        let content = "This is a test line";
        let mut matches = Vec::new();
        for mat in re.find_iter(content) {
            matches.push((mat.start(), mat.end()));
        }
        assert_eq!(matches, vec![(10, 14)]);
        Ok(())
    }

    #[test]
    fn test_regex_crate_behavior_direct() -> anyhow::Result<()> {
        use regex::Regex;
        let re = Regex::new("test")?;
        let content = "This is a test line";
        let mut matches = Vec::new();
        for mat in re.find_iter(content) {
            matches.push((mat.start(), mat.end()));
        }
        assert_eq!(matches, vec![(10, 14)]);
        Ok(())
    }

    #[test]
    fn test_search_basic() -> anyhow::Result<()> {
        let mut analyzer = LogAnalyzer::new();
        analyzer.entries.push(LogEntry {
            id: 0,
            content: "This is a test line".to_string(),
            marked: false,
        });
        analyzer.entries.push(LogEntry {
            id: 1,
            content: "Another line".to_string(),
            marked: false,
        });

        let results = analyzer.search("test")?;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].log_id, 0);
        assert_eq!(results[0].matches, vec![(10, 14)]);
        Ok(())
    }

    #[test]
    fn test_search_multiple_matches_in_one_line() -> anyhow::Result<()> {
        let mut analyzer = LogAnalyzer::new();
        analyzer.entries.push(LogEntry {
            id: 0,
            content: "test test test".to_string(),
            marked: false,
        });

        let results = analyzer.search("test")?;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].log_id, 0);
        assert_eq!(results[0].matches, vec![(0, 4), (5, 9), (10, 14)]);
        Ok(())
    }

    #[test]
    fn test_search_no_match() -> anyhow::Result<()> {
        let mut analyzer = LogAnalyzer::new();
        analyzer.entries.push(LogEntry {
            id: 0,
            content: "This is a line".to_string(),
            marked: false,
        });

        let results = analyzer.search("nomatch")?;
        assert!(results.is_empty());
        Ok(())
    }

    #[test]
    fn test_search_case_insensitive() -> anyhow::Result<()> {
        let mut analyzer = LogAnalyzer::new();
        analyzer.entries.push(LogEntry {
            id: 0,
            content: "Test line test".to_string(),
            marked: false,
        });

        let results = analyzer.search("(?i)test")?;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].log_id, 0);
        assert_eq!(results[0].matches, vec![(0, 4), (10, 14)]);
        Ok(())
    }

    #[test]
    fn test_search_regex_special_chars() -> anyhow::Result<()> {
        let mut analyzer = LogAnalyzer::new();
        analyzer.entries.push(LogEntry {
            id: 0,
            content: "line with .*".to_string(),
            marked: false,
        });

        let results = analyzer.search(r".*")?;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].log_id, 0);
        assert_eq!(results[0].matches, vec![(0, 12)]);
        Ok(())
    }

    #[test]
    fn test_filter_management() {
        let mut analyzer = LogAnalyzer::new();
        assert_eq!(analyzer.filters.len(), 0);

        // Add
        analyzer.add_filter("error".to_string(), FilterType::Include);
        assert_eq!(analyzer.filters.len(), 1);
        assert_eq!(analyzer.filters[0].id, 0);
        assert_eq!(analyzer.filters[0].pattern, "error");
        assert_eq!(analyzer.filters[0].filter_type, FilterType::Include);
        assert!(analyzer.filters[0].enabled);

        analyzer.add_filter("info".to_string(), FilterType::Exclude);
        assert_eq!(analyzer.filters.len(), 2);
        assert_eq!(analyzer.filters[1].id, 1);

        // Toggle
        analyzer.toggle_filter(0);
        assert!(!analyzer.filters[0].enabled);
        analyzer.toggle_filter(0);
        assert!(analyzer.filters[0].enabled);

        // Edit
        analyzer.edit_filter(1, "debug".to_string());
        assert_eq!(analyzer.filters[1].pattern, "debug");

        // Remove
        analyzer.remove_filter(0);
        assert_eq!(analyzer.filters.len(), 1);
        assert_eq!(analyzer.filters[0].id, 1);

        // Clear
        analyzer.clear_filters();
        assert_eq!(analyzer.filters.len(), 0);
    }

    #[test]
    fn test_apply_filters() -> anyhow::Result<()> {
        let mut analyzer = LogAnalyzer::new();
        analyzer.entries.push(LogEntry {
            id: 0,
            content: "error: critical issue ".to_string(),
            marked: false,
        });
        analyzer.entries.push(LogEntry {
            id: 1,
            content: "info: user logged in ".to_string(),
            marked: false,
        });
        analyzer.entries.push(LogEntry {
            id: 2,
            content: "error: minor issue ".to_string(),
            marked: false,
        });
        analyzer.entries.push(LogEntry {
            id: 3,
            content: "debug: value is 5".to_string(),
            marked: false,
        });
        analyzer.entries.push(LogEntry {
            id: 4,
            content: "info: user logged out ".to_string(),
            marked: false,
        });

        // Scenario 1: No filters
        let filtered = analyzer.apply_filters(&analyzer.entries)?;
        assert_eq!(filtered.len(), 5, "Scenario 1 Failed: No filters ");

        // Scenario 2: Only include filters
        analyzer.add_filter("error".to_string(), FilterType::Include); // id 0
        let filtered = analyzer.apply_filters(&analyzer.entries)?;
        assert_eq!(filtered.len(), 2, "Scenario 2 Failed: Only include ");
        assert!(filtered.iter().any(|l| l.id == 0));
        assert!(filtered.iter().any(|l| l.id == 2));

        // Scenario 3: Only exclude filters
        analyzer.clear_filters();
        analyzer.add_filter("info".to_string(), FilterType::Exclude); // id 1
        let filtered = analyzer.apply_filters(&analyzer.entries)?;
        assert_eq!(filtered.len(), 3, "Scenario 3 Failed: Only exclude ");
        assert!(!filtered.iter().any(|l| l.id == 1 || l.id == 4));

        // Scenario 4: Include and Exclude, no overlap
        analyzer.clear_filters();
        analyzer.add_filter("error".to_string(), FilterType::Include); // id 2
        analyzer.add_filter("info".to_string(), FilterType::Exclude); // id 3
        let filtered = analyzer.apply_filters(&analyzer.entries)?;
        assert_eq!(
            filtered.len(),
            2,
            "Scenario 4 Failed: Include and Exclude, no overlap "
        );
        assert!(filtered.iter().any(|l| l.id == 0));
        assert!(filtered.iter().any(|l| l.id == 2));

        // Scenario 5: Include and Exclude, with overlap
        analyzer.clear_filters();
        analyzer.add_filter("error".to_string(), FilterType::Include); // id 4
        analyzer.add_filter("critical".to_string(), FilterType::Exclude); // id 5
        let filtered = analyzer.apply_filters(&analyzer.entries)?;
        assert_eq!(
            filtered.len(),
            1,
            "Scenario 5 Failed: Include and Exclude, with overlap "
        );
        assert_eq!(filtered[0].id, 2);

        // Scenario 6: Disabled include filter
        analyzer.toggle_filter(4); // disable 'error' include
        let filtered = analyzer.apply_filters(&analyzer.entries)?;
        assert_eq!(filtered.len(), 4, "Scenario 6 Failed: Disabled include");
        assert!(!filtered.iter().any(|l| l.id == 0)); // critical is excluded

        // Scenario 7: Disabled exclude filter
        analyzer.toggle_filter(4); // enable 'error' include
        analyzer.toggle_filter(5); // disable 'critical' exclude
        let filtered = analyzer.apply_filters(&analyzer.entries)?;
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
        let mut analyzer = LogAnalyzer::new();
        analyzer.add_filter("filter1".to_string(), FilterType::Include);
        analyzer.add_filter("filter2".to_string(), FilterType::Exclude);
        analyzer.add_filter("filter3".to_string(), FilterType::Include);

        // Move filter2 up (id 1)
        analyzer.move_filter_up(1); // filter with id 1 (filter2) moves up
        assert_eq!(analyzer.filters[0].id, 1); // filter2 is now at index 0
        assert_eq!(analyzer.filters[1].id, 0); // filter1 is now at index 1
        assert_eq!(analyzer.filters[2].id, 2); // filter3 remains at index 2

        Ok(())
    }

    #[test]
    fn test_move_filter_up_at_top() -> anyhow::Result<()> {
        let mut analyzer = LogAnalyzer::new();
        analyzer.add_filter("filter1".to_string(), FilterType::Include);
        analyzer.add_filter("filter2".to_string(), FilterType::Exclude);

        // Try to move filter1 up (id 0), which is already at the top
        analyzer.move_filter_up(0);
        assert_eq!(analyzer.filters[0].id, 0);
        assert_eq!(analyzer.filters[1].id, 1);

        Ok(())
    }

    #[test]
    fn test_move_filter_down() -> anyhow::Result<()> {
        let mut analyzer = LogAnalyzer::new();
        analyzer.add_filter("filter1".to_string(), FilterType::Include);
        analyzer.add_filter("filter2".to_string(), FilterType::Exclude);
        analyzer.add_filter("filter3".to_string(), FilterType::Include);

        // Move filter2 down (id 1)
        analyzer.move_filter_down(1); // filter with id 1 (filter2) moves down
        assert_eq!(analyzer.filters[0].id, 0); // filter1 remains at index 0
        assert_eq!(analyzer.filters[1].id, 2); // filter3 is now at index 1
        assert_eq!(analyzer.filters[2].id, 1); // filter2 is now at index 2

        Ok(())
    }

    #[test]
    fn test_move_filter_down_at_bottom() -> anyhow::Result<()> {
        let mut analyzer = LogAnalyzer::new();
        analyzer.add_filter("filter1".to_string(), FilterType::Include);
        analyzer.add_filter("filter2".to_string(), FilterType::Exclude);

        // Try to move filter2 down (id 1), which is already at the bottom
        analyzer.move_filter_down(1);
        assert_eq!(analyzer.filters[0].id, 0);
        assert_eq!(analyzer.filters[1].id, 1);

        Ok(())
    }


    #[test]
    fn test_save_and_load_filters() -> anyhow::Result<()> {
        let mut analyzer = LogAnalyzer::new();
        analyzer.add_filter("error".to_string(), FilterType::Include);
        analyzer.add_filter("info".to_string(), FilterType::Exclude);

        let file = NamedTempFile::new()?;
        let path = file.path().to_str().unwrap();

        analyzer.save_filters(path)?;

        let mut new_analyzer = LogAnalyzer::new();
        new_analyzer.load_filters(path)?;

        assert_eq!(new_analyzer.filters.len(), 2);
        assert_eq!(new_analyzer.filters[0].pattern, "error");
        assert_eq!(new_analyzer.filters[0].filter_type, FilterType::Include);
        assert_eq!(new_analyzer.filters[1].pattern, "info");
        assert_eq!(new_analyzer.filters[1].filter_type, FilterType::Exclude);

        Ok(())
    }
}
