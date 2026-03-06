//! Regex-based search over visible log lines with wrapping navigation.
//!
//! [`Search`] operates on `visible_indices` only (respects active filters).
//! Builds a [`Vec<SearchResult>`] with byte-position match spans, then
//! provides wrapping [`Search::next_match`] / [`Search::previous_match`].

use crate::types::SearchResult;
use regex::Regex;

#[derive(Debug, Clone)]
pub struct Search {
    pattern: Option<Regex>,
    results: Vec<SearchResult>,
    /// Index into `results` (which line).
    current_result_index: usize,
    /// Index into `results[current_result_index].matches` (which occurrence on that line).
    current_occurrence_index: usize,
    case_sensitive: bool,
    /// Direction of the last confirmed search. `true` = forward (`/`), `false` = backward (`?`).
    /// Determines whether `n` continues forward or backward (vim semantics).
    forward: bool,
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
            current_result_index: 0,
            current_occurrence_index: 0,
            case_sensitive: true,
            forward: true,
        }
    }

    /// Search `visible_indices` lines for `pattern_str`.
    ///
    /// `get_text` maps a `line_idx` to the string that should be searched —
    /// callers should supply the *displayed* text (after field filtering) so
    /// that hidden fields are never counted as matches.
    pub fn search(
        &mut self,
        pattern_str: &str,
        visible_indices: impl Iterator<Item = usize>,
        get_text: impl Fn(usize) -> Option<String>,
    ) -> anyhow::Result<()> {
        let pattern = if self.case_sensitive {
            Regex::new(pattern_str)?
        } else {
            Regex::new(&format!("(?i){}", pattern_str))?
        };
        self.pattern = Some(pattern.clone());
        self.results.clear();
        self.current_result_index = 0;
        self.current_occurrence_index = 0;

        for line_idx in visible_indices {
            let text = match get_text(line_idx) {
                Some(t) => t,
                None => continue,
            };
            let matches: Vec<(usize, usize)> = pattern
                .find_iter(&text)
                .map(|m| (m.start(), m.end()))
                .collect();
            if !matches.is_empty() {
                self.results.push(SearchResult { line_idx, matches });
            }
        }
        Ok(())
    }

    /// Advance to the next occurrence. Stays on the same line if it has more
    /// occurrences; wraps to the first occurrence of the next line otherwise.
    pub fn next_match(&mut self) -> Option<&SearchResult> {
        if self.results.is_empty() {
            return None;
        }
        let current_line_matches = self.results[self.current_result_index].matches.len();
        if self.current_occurrence_index + 1 < current_line_matches {
            self.current_occurrence_index += 1;
        } else {
            self.current_result_index = (self.current_result_index + 1) % self.results.len();
            self.current_occurrence_index = 0;
        }
        Some(&self.results[self.current_result_index])
    }

    /// Go back to the previous occurrence. Stays on the same line if not at the
    /// first occurrence; wraps to the last occurrence of the previous line otherwise.
    pub fn previous_match(&mut self) -> Option<&SearchResult> {
        if self.results.is_empty() {
            return None;
        }
        if self.current_occurrence_index > 0 {
            self.current_occurrence_index -= 1;
        } else {
            if self.current_result_index == 0 {
                self.current_result_index = self.results.len() - 1;
            } else {
                self.current_result_index -= 1;
            }
            self.current_occurrence_index = self.results[self.current_result_index]
                .matches
                .len()
                .saturating_sub(1);
        }
        Some(&self.results[self.current_result_index])
    }

    pub fn get_current_match(&self) -> Option<&SearchResult> {
        if self.results.is_empty() {
            None
        } else {
            Some(&self.results[self.current_result_index])
        }
    }

    pub fn get_results(&self) -> &[SearchResult] {
        &self.results
    }

    pub fn set_case_sensitive(&mut self, case_sensitive: bool) {
        self.case_sensitive = case_sensitive;
    }

    /// Store the search direction so that `go_next` / `go_prev` respect vim semantics:
    /// `n` repeats in the original direction, `N` reverses it.
    pub fn set_forward(&mut self, forward: bool) {
        self.forward = forward;
    }

    pub fn is_forward(&self) -> bool {
        self.forward
    }

    /// Move to the next occurrence in the **search direction** (`n` in vim).
    pub fn go_next(&mut self) -> Option<&SearchResult> {
        if self.forward {
            self.next_match()
        } else {
            self.previous_match()
        }
    }

    /// Move to the previous occurrence in the **search direction** (`N` in vim).
    pub fn go_prev(&mut self) -> Option<&SearchResult> {
        if self.forward {
            self.previous_match()
        } else {
            self.next_match()
        }
    }

    /// Total number of individual match occurrences across all lines.
    pub fn get_total_match_count(&self) -> usize {
        self.results.iter().map(|r| r.matches.len()).sum()
    }

    /// 1-based global occurrence number across all lines.
    pub fn get_current_occurrence_number(&self) -> usize {
        if self.results.is_empty() {
            return 0;
        }
        self.results[..self.current_result_index]
            .iter()
            .map(|r| r.matches.len())
            .sum::<usize>()
            + self.current_occurrence_index
            + 1
    }

    pub fn get_current_match_index(&self) -> usize {
        self.current_result_index
    }

    pub fn get_current_occurrence_index(&self) -> usize {
        self.current_occurrence_index
    }

    /// Position the cursor so that the next call to [`next_match`] / [`previous_match`]
    /// lands on the first occurrence strictly after (`forward=true`) or strictly before
    /// (`forward=false`) `line_idx`, wrapping around when necessary.
    pub fn set_position_for_search(&mut self, line_idx: usize, forward: bool) {
        if self.results.is_empty() {
            return;
        }
        let last = self.results.len() - 1;
        if forward {
            // pos = index of first result with line_idx >= current line (inclusive).
            // Position cursor at pos-1 so next_match() lands on pos (which may be
            // the current line itself, so matches on the current line are included).
            let pos = self.results.partition_point(|r| r.line_idx < line_idx);
            if pos == 0 {
                // All results are at/after line_idx; wrap by seating at last result.
                self.current_result_index = last;
            } else {
                self.current_result_index = pos - 1;
            }
            self.current_occurrence_index = self.results[self.current_result_index]
                .matches
                .len()
                .saturating_sub(1);
        } else {
            // pos = index of first result with line_idx >= current line.
            // Position cursor at pos so previous_match() retreats to pos-1.
            let pos = self.results.partition_point(|r| r.line_idx < line_idx);
            if pos == 0 {
                // All results are at/after line_idx; wrap by seating at first result.
                self.current_result_index = 0;
                self.current_occurrence_index = 0;
            } else {
                self.current_result_index = pos;
                self.current_occurrence_index = 0;
            }
        }
    }

    /// If `line_idx` is the line that holds the current match, returns the
    /// index of the current occurrence within that line's match list.
    pub fn get_current_occurrence_for_line(&self, line_idx: usize) -> Option<usize> {
        if self.results.is_empty() {
            return None;
        }
        let current = &self.results[self.current_result_index];
        if current.line_idx == line_idx {
            Some(self.current_occurrence_index)
        } else {
            None
        }
    }

    pub fn clear(&mut self) {
        self.pattern = None;
        self.results.clear();
        self.current_result_index = 0;
        self.current_occurrence_index = 0;
        self.forward = true;
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
    use crate::file_reader::FileReader;
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

    /// Helper: build a `get_text` closure from a `FileReader` (raw bytes).
    fn raw_text(reader: &FileReader) -> impl Fn(usize) -> Option<String> + '_ {
        |line_idx| Some(String::from_utf8_lossy(reader.get_line(line_idx)).into_owned())
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
        search.search("test", all.iter().copied(), raw_text(&reader))?;

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
        search.search("test", all.iter().copied(), raw_text(&reader))?;

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
        search.search("nomatch", all.iter().copied(), raw_text(&reader))?;
        assert!(search.get_results().is_empty());
        Ok(())
    }

    #[test]
    fn test_next_previous_match() -> anyhow::Result<()> {
        let (_f, reader) = make_reader(&["test line", "nothing", "another test"]);
        let all: Vec<usize> = (0..reader.line_count()).collect();
        let mut search = Search::new();
        search.search("test", all.iter().copied(), raw_text(&reader))?;

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
        let (_f, reader) = make_reader(&["ERROR: bad", "INFO: good", "ERROR: also bad"]);
        // Only search visible lines (0 and 2 pass the filter)
        let visible = vec![0usize, 2];
        let mut search = Search::new();
        search.search("bad", visible.iter().copied(), raw_text(&reader))?;

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
    fn test_next_match_stays_on_same_line_for_multiple_occurrences() -> anyhow::Result<()> {
        // line 0 has 3 occurrences, line 1 has 1
        let (_f, reader) = make_reader(&["foo foo foo", "foo"]);
        let all: Vec<usize> = (0..reader.line_count()).collect();
        let mut search = Search::new();
        search.search("foo", all.iter().copied(), raw_text(&reader))?;

        assert_eq!(search.get_current_match().unwrap().line_idx, 0); // occ 0
        search.next_match();
        assert_eq!(search.get_current_match().unwrap().line_idx, 0); // occ 1 – same line
        search.next_match();
        assert_eq!(search.get_current_match().unwrap().line_idx, 0); // occ 2 – still same line
        search.next_match();
        assert_eq!(search.get_current_match().unwrap().line_idx, 1); // exhausted → line 1
        Ok(())
    }

    #[test]
    fn test_previous_match_stays_on_same_line_for_multiple_occurrences() -> anyhow::Result<()> {
        let (_f, reader) = make_reader(&["foo foo", "foo"]);
        let all: Vec<usize> = (0..reader.line_count()).collect();
        let mut search = Search::new();
        search.search("foo", all.iter().copied(), raw_text(&reader))?;
        // advance to line 1
        search.next_match();
        search.next_match();
        assert_eq!(search.get_current_match().unwrap().line_idx, 1);
        search.previous_match();
        assert_eq!(search.get_current_match().unwrap().line_idx, 0); // last occ of line 0
        search.previous_match();
        assert_eq!(search.get_current_match().unwrap().line_idx, 0); // first occ of line 0
        Ok(())
    }

    #[test]
    fn test_get_total_match_count() -> anyhow::Result<()> {
        let (_f, reader) = make_reader(&["test test", "nothing", "test"]);
        let all: Vec<usize> = (0..reader.line_count()).collect();
        let mut search = Search::new();
        search.search("test", all.iter().copied(), raw_text(&reader))?;
        // line 0: 2 occurrences, line 2: 1 occurrence → total 3
        assert_eq!(search.get_total_match_count(), 3);
        Ok(())
    }

    #[test]
    fn test_get_total_match_count_empty() {
        let search = Search::new();
        assert_eq!(search.get_total_match_count(), 0);
    }

    #[test]
    fn test_get_current_occurrence_number() -> anyhow::Result<()> {
        // line 0: 2 occurrences ("test test"), line 1: 1 occurrence ("test")
        let (_f, reader) = make_reader(&["test test", "test"]);
        let all: Vec<usize> = (0..reader.line_count()).collect();
        let mut search = Search::new();
        search.search("test", all.iter().copied(), raw_text(&reader))?;
        assert_eq!(search.get_current_occurrence_number(), 1); // result=0, occ=0
        search.next_match(); // stays on line 0, advances to occ=1
        assert_eq!(search.get_current_occurrence_number(), 2);
        search.next_match(); // line 0 exhausted → advances to line 1, occ=0
        assert_eq!(search.get_current_occurrence_number(), 3);
        Ok(())
    }

    #[test]
    fn test_get_current_occurrence_number_empty() {
        let search = Search::new();
        assert_eq!(search.get_current_occurrence_number(), 0);
    }

    #[test]
    fn test_get_current_match_index() -> anyhow::Result<()> {
        let (_f, reader) = make_reader(&["test a", "test b", "test c"]);
        let all: Vec<usize> = (0..reader.line_count()).collect();
        let mut search = Search::new();
        search.search("test", all.iter().copied(), raw_text(&reader))?;
        assert_eq!(search.get_current_match_index(), 0);
        search.next_match();
        assert_eq!(search.get_current_match_index(), 1);
        search.next_match();
        assert_eq!(search.get_current_match_index(), 2);
        Ok(())
    }

    #[test]
    fn test_clear_resets_state() -> anyhow::Result<()> {
        let (_f, reader) = make_reader(&["test line"]);
        let all: Vec<usize> = (0..reader.line_count()).collect();
        let mut search = Search::new();
        search.search("test", all.iter().copied(), raw_text(&reader))?;
        assert!(!search.get_results().is_empty());
        assert!(search.get_pattern().is_some());
        search.clear();
        assert!(search.get_results().is_empty());
        assert!(search.get_pattern().is_none());
        assert_eq!(search.get_current_match_index(), 0);
        Ok(())
    }

    #[test]
    fn test_default() {
        let search = Search::default();
        assert!(search.pattern.is_none());
        assert!(search.results.is_empty());
        assert_eq!(search.current_result_index, 0);
        assert_eq!(search.current_occurrence_index, 0);
        assert!(search.case_sensitive);
        assert!(search.forward);
    }

    #[test]
    fn test_go_next_forward_search() -> anyhow::Result<()> {
        // With forward=true, go_next advances (next_match) and go_prev retreats (previous_match).
        let (_f, reader) = make_reader(&["foo", "bar", "foo"]);
        let all: Vec<usize> = (0..reader.line_count()).collect();
        let mut search = Search::new();
        search.search("foo", all.iter().copied(), raw_text(&reader))?;
        search.set_forward(true);
        // start at result 0 (line 0)
        assert_eq!(search.get_current_match().unwrap().line_idx, 0);
        assert_eq!(search.go_next().unwrap().line_idx, 2); // advances
        assert_eq!(search.go_prev().unwrap().line_idx, 0); // retreats
        Ok(())
    }

    #[test]
    fn test_go_next_backward_search() -> anyhow::Result<()> {
        // With forward=false, go_next retreats (previous_match) and go_prev advances (next_match).
        let (_f, reader) = make_reader(&["foo", "bar", "foo"]);
        let all: Vec<usize> = (0..reader.line_count()).collect();
        let mut search = Search::new();
        search.search("foo", all.iter().copied(), raw_text(&reader))?;
        search.set_forward(false);
        // start at result 0 (line 0)
        assert_eq!(search.get_current_match().unwrap().line_idx, 0);
        // go_next wraps to the last result (backward direction)
        assert_eq!(search.go_next().unwrap().line_idx, 2);
        // go_prev advances back to line 0
        assert_eq!(search.go_prev().unwrap().line_idx, 0);
        Ok(())
    }

    #[test]
    fn test_clear_resets_forward() -> anyhow::Result<()> {
        let (_f, reader) = make_reader(&["foo"]);
        let all: Vec<usize> = (0..reader.line_count()).collect();
        let mut search = Search::new();
        search.search("foo", all.iter().copied(), raw_text(&reader))?;
        search.set_forward(false);
        assert!(!search.is_forward());
        search.clear();
        assert!(search.is_forward());
        Ok(())
    }

    #[test]
    fn test_set_position_for_search_forward() -> anyhow::Result<()> {
        // results at lines 0, 2, 4
        let (_f, reader) = make_reader(&["foo", "bar", "foo", "bar", "foo"]);
        let all: Vec<usize> = (0..reader.line_count()).collect();
        let mut search = Search::new();
        search.search("foo", all.iter().copied(), raw_text(&reader))?;

        // From line 1: first result at/after line 1 → line 2.
        search.set_position_for_search(1, true);
        assert_eq!(search.next_match().unwrap().line_idx, 2);

        // From line 4 (at a result): current line counts → line 4.
        search.set_position_for_search(4, true);
        assert_eq!(search.next_match().unwrap().line_idx, 4);

        // From line 0 (at a result): current line counts → line 0.
        search.set_position_for_search(0, true);
        assert_eq!(search.next_match().unwrap().line_idx, 0);
        Ok(())
    }

    #[test]
    fn test_set_position_for_search_backward() -> anyhow::Result<()> {
        // results at lines 0, 2, 4
        let (_f, reader) = make_reader(&["foo", "bar", "foo", "bar", "foo"]);
        let all: Vec<usize> = (0..reader.line_count()).collect();
        let mut search = Search::new();
        search.search("foo", all.iter().copied(), raw_text(&reader))?;

        // From line 3: last result strictly before line 3 → line 2.
        search.set_position_for_search(3, false);
        assert_eq!(search.previous_match().unwrap().line_idx, 2);

        // From line 0 (first result): no result before line 0 → wraps to line 4.
        search.set_position_for_search(0, false);
        assert_eq!(search.previous_match().unwrap().line_idx, 4);

        // From line 4 (last result): last result strictly before line 4 → line 2.
        search.set_position_for_search(4, false);
        assert_eq!(search.previous_match().unwrap().line_idx, 2);
        Ok(())
    }

    #[test]
    fn test_get_current_occurrence_index() -> anyhow::Result<()> {
        let (_f, reader) = make_reader(&["foo foo", "foo"]);
        let all: Vec<usize> = (0..reader.line_count()).collect();
        let mut search = Search::new();
        search.search("foo", all.iter().copied(), raw_text(&reader))?;
        assert_eq!(search.get_current_occurrence_index(), 0);
        search.next_match();
        assert_eq!(search.get_current_occurrence_index(), 1);
        search.next_match(); // moves to line 1, occ 0
        assert_eq!(search.get_current_occurrence_index(), 0);
        Ok(())
    }

    #[test]
    fn test_get_current_occurrence_for_line() -> anyhow::Result<()> {
        let (_f, reader) = make_reader(&["foo foo", "foo"]);
        let all: Vec<usize> = (0..reader.line_count()).collect();
        let mut search = Search::new();
        search.search("foo", all.iter().copied(), raw_text(&reader))?;
        // current is line 0, occ 0
        assert_eq!(search.get_current_occurrence_for_line(0), Some(0));
        assert_eq!(search.get_current_occurrence_for_line(1), None);
        search.next_match(); // line 0, occ 1
        assert_eq!(search.get_current_occurrence_for_line(0), Some(1));
        assert_eq!(search.get_current_occurrence_for_line(1), None);
        search.next_match(); // line 1, occ 0
        assert_eq!(search.get_current_occurrence_for_line(0), None);
        assert_eq!(search.get_current_occurrence_for_line(1), Some(0));
        Ok(())
    }

    #[test]
    fn test_get_current_occurrence_for_line_empty() {
        let search = Search::new();
        assert_eq!(search.get_current_occurrence_for_line(0), None);
    }
}
