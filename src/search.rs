use crate::file_reader::FileReader;
use crate::types::SearchResult;
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
            case_sensitive: true,
        }
    }

    /// Search `visible_indices` lines in `reader` for `pattern_str`.
    pub fn search(
        &mut self,
        pattern_str: &str,
        visible_indices: &[usize],
        reader: &FileReader,
    ) -> anyhow::Result<()> {
        let pattern = if self.case_sensitive {
            Regex::new(pattern_str)?
        } else {
            Regex::new(&format!("(?i){}", pattern_str))?
        };
        self.pattern = Some(pattern.clone());
        self.results.clear();
        self.current_match_index = 0;

        for &line_idx in visible_indices {
            let line = reader.get_line(line_idx);
            let text = match std::str::from_utf8(line) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let mut matches = Vec::new();
            for mat in pattern.find_iter(text) {
                matches.push((mat.start(), mat.end()));
            }
            if !matches.is_empty() {
                self.results.push(SearchResult { line_idx, matches });
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

    pub fn get_results(&self) -> &[SearchResult] {
        &self.results
    }

    pub fn set_case_sensitive(&mut self, case_sensitive: bool) {
        self.case_sensitive = case_sensitive;
    }

    pub fn get_pattern(&self) -> Option<&str> {
        self.pattern.as_ref().map(|p| p.as_str())
    }

    pub fn get_compiled_pattern(&self) -> Option<&Regex> {
        self.pattern.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn make_reader(lines: &[&str]) -> (NamedTempFile, FileReader) {
        let mut f = NamedTempFile::new().unwrap();
        for line in lines {
            writeln!(f, "{}", line).unwrap();
        }
        let path = f.path().to_str().unwrap().to_string();
        let reader = FileReader::new(&path).unwrap();
        (f, reader)
    }

    #[test]
    fn test_search_basic() -> anyhow::Result<()> {
        let (_f, reader) = make_reader(&[
            "This is a test line",
            "Another line with Test",
            "No match here",
            "test test test",
        ]);
        let all: Vec<usize> = (0..reader.line_count()).collect();
        let mut search = Search::new();
        search.search("test", &all, &reader)?;

        let results = search.get_results();
        assert_eq!(results.len(), 2); // lines 0 and 3
        assert_eq!(results[0].line_idx, 0);
        assert_eq!(results[0].matches, vec![(10, 14)]);
        assert_eq!(results[1].line_idx, 3);
        assert_eq!(results[1].matches, vec![(0, 4), (5, 9), (10, 14)]);
        Ok(())
    }

    #[test]
    fn test_search_case_insensitive() -> anyhow::Result<()> {
        let (_f, reader) = make_reader(&[
            "This is a test line",
            "Another line with Test",
            "No match here",
            "test test test",
        ]);
        let all: Vec<usize> = (0..reader.line_count()).collect();
        let mut search = Search::new();
        search.set_case_sensitive(false);
        search.search("test", &all, &reader)?;

        let results = search.get_results();
        assert_eq!(results.len(), 3); // lines 0, 1, 3
        assert_eq!(results[0].line_idx, 0);
        assert_eq!(results[1].line_idx, 1);
        assert_eq!(results[2].line_idx, 3);
        Ok(())
    }

    #[test]
    fn test_search_no_match() -> anyhow::Result<()> {
        let (_f, reader) = make_reader(&["This is a line"]);
        let all: Vec<usize> = (0..reader.line_count()).collect();
        let mut search = Search::new();
        search.search("nomatch", &all, &reader)?;
        assert!(search.get_results().is_empty());
        Ok(())
    }

    #[test]
    fn test_next_previous_match() -> anyhow::Result<()> {
        let (_f, reader) = make_reader(&[
            "test line",
            "nothing",
            "another test",
        ]);
        let all: Vec<usize> = (0..reader.line_count()).collect();
        let mut search = Search::new();
        search.search("test", &all, &reader)?;

        // starts at index 0 → line_idx 0
        assert_eq!(search.get_current_match().unwrap().line_idx, 0);

        search.next_match();
        assert_eq!(search.get_current_match().unwrap().line_idx, 2);

        // wrap around
        search.next_match();
        assert_eq!(search.get_current_match().unwrap().line_idx, 0);

        search.previous_match();
        assert_eq!(search.get_current_match().unwrap().line_idx, 2);
        Ok(())
    }

    #[test]
    fn test_search_only_visible() -> anyhow::Result<()> {
        let (_f, reader) = make_reader(&[
            "ERROR: bad",
            "INFO: good",
            "ERROR: also bad",
        ]);
        // Only search visible lines (0 and 2 pass the filter)
        let visible = vec![0usize, 2];
        let mut search = Search::new();
        search.search("bad", &visible, &reader)?;

        let results = search.get_results();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].line_idx, 0);
        assert_eq!(results[1].line_idx, 2);
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
