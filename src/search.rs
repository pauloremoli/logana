use crate::analyzer::{LogEntry, SearchResult};
use regex::Regex;

#[derive(Debug, Clone)]
pub struct Search {
    pattern: Option<Regex>,
    results: Vec<SearchResult>,
    current_match_index: usize,
    case_sensitive: bool,
}

impl Default for Search {
    fn default() -> Self {
        Self::new()
    }
}

impl Search {
    pub fn new() -> Self {
        Search {
            pattern: None,
            results: Vec::new(),
            current_match_index: 0,
            case_sensitive: true, // Default to case-sensitive
        }
    }

    pub fn search(&mut self, pattern_str: &str, logs: &[LogEntry]) -> anyhow::Result<()> {
        let pattern = if self.case_sensitive {
            Regex::new(pattern_str)?
        } else {
            Regex::new(&format!("(?i){}", pattern_str))?
        };
        self.pattern = Some(pattern.clone());
        self.results.clear();
        self.current_match_index = 0;

        for entry in logs {
            let mut matches = Vec::new();
            for mat in pattern.find_iter(&entry.content) {
                matches.push((mat.start(), mat.end()));
            }
            if !matches.is_empty() {
                self.results.push(SearchResult {
                    log_id: entry.id,
                    matches,
                });
            }
        }
        Ok(())
    }

    pub fn next_match(&mut self) -> Option<&SearchResult> {
        if self.results.is_empty() {
            return None;
        }
        self.current_match_index = (self.current_match_index + 1) % self.results.len();
        Some(&self.results[self.current_match_index])
    }

    pub fn previous_match(&mut self) -> Option<&SearchResult> {
        if self.results.is_empty() {
            return None;
        }
        if self.current_match_index == 0 {
            self.current_match_index = self.results.len() - 1;
        } else {
            self.current_match_index -= 1;
        }
        Some(&self.results[self.current_match_index])
    }

    pub fn get_current_match(&self) -> Option<&SearchResult> {
        if self.results.is_empty() {
            None
        } else {
            Some(&self.results[self.current_match_index])
        }
    }

    pub fn get_results(&self) -> &Vec<SearchResult> {
        &self.results
    }

    pub fn set_case_sensitive(&mut self, case_sensitive: bool) {
        self.case_sensitive = case_sensitive;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::LogEntry;

    fn create_test_logs() -> Vec<LogEntry> {
        vec![
            LogEntry {
                id: 0,
                content: "This is a test line".to_string(),
                marked: false,
            },
            LogEntry {
                id: 1,
                content: "Another line with Test".to_string(),
                marked: false,
            },
            LogEntry {
                id: 2,
                content: "No match here".to_string(),
                marked: false,
            },
            LogEntry {
                id: 3,
                content: "test test test".to_string(),
                marked: false,
            },
        ]
    }

    #[test]
    fn test_search_basic() -> anyhow::Result<()> {
        let mut search = Search::new();
        let logs = create_test_logs();
        search.search("test", &logs)?;

        let results = search.get_results();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].log_id, 0);
        assert_eq!(results[0].matches, vec![(10, 14)]);
        assert_eq!(results[1].log_id, 3);
        assert_eq!(results[1].matches, vec![(0, 4), (5, 9), (10, 14)]);
        Ok(())
    }

    #[test]
    fn test_search_case_insensitive() -> anyhow::Result<()> {
        let mut search = Search::new();
        search.set_case_sensitive(false);
        let logs = create_test_logs();
        search.search("test", &logs)?;

        let results = search.get_results();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].log_id, 0);
        assert_eq!(results[0].matches, vec![(10, 14)]);
        assert_eq!(results[1].log_id, 1);
        assert_eq!(results[1].matches, vec![(18, 22)]);
        assert_eq!(results[2].log_id, 3);
        assert_eq!(results[2].matches, vec![(0, 4), (5, 9), (10, 14)]);
        Ok(())
    }

    #[test]
    fn test_search_no_match() -> anyhow::Result<()> {
        let mut search = Search::new();
        let logs = create_test_logs();
        search.search("nomatch", &logs)?;
        assert!(search.get_results().is_empty());
        Ok(())
    }

    #[test]
    fn test_next_previous_match() -> anyhow::Result<()> {
        let mut search = Search::new();
        let logs = create_test_logs();
        search.search("test", &logs)?;

        // Initial state, current_match_index is 0
        assert_eq!(search.get_current_match().unwrap().log_id, 0);

        // Next
        search.next_match();
        assert_eq!(search.get_current_match().unwrap().log_id, 3);
        search.next_match(); // Wraps around
        assert_eq!(search.get_current_match().unwrap().log_id, 0);

        // Previous
        search.previous_match();
        assert_eq!(search.get_current_match().unwrap().log_id, 3);
        search.previous_match(); // Wraps around
        assert_eq!(search.get_current_match().unwrap().log_id, 0);
        Ok(())
    }

    #[test]
    fn test_empty_search() {
        let mut search = Search::new();
        assert!(search.next_match().is_none());
        assert!(search.previous_match().is_none());
        assert!(search.get_current_match().is_none());
    }

    #[test]
    fn test_default() {
        let search = Search::default();
        assert!(search.pattern.is_none());
        assert!(search.results.is_empty());
        assert_eq!(search.current_match_index, 0);
        assert!(search.case_sensitive);
    }
}
