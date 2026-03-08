//! Filter pipeline: pattern matching, visibility computation, and span rendering.
//!
//! [`FilterManager`] evaluates enabled `FilterDef`s against log lines in
//! parallel via rayon. Literal patterns use Aho-Corasick; patterns with regex
//! metacharacters fall back to the `regex` crate. [`render_line`] flattens
//! overlapping styled spans into a ratatui [`Line`].

use aho_corasick::AhoCorasick;
use ratatui::text::{Line, Span};
use regex::Regex;

/// Index into the styles array passed to `render_line`.
pub type StyleId = u8;

/// Reserved StyleId for search highlights (always at the end of the styles array).
pub const SEARCH_STYLE_ID: StyleId = u8::MAX;

/// Reserved StyleId for the *current* search occurrence (one slot below SEARCH_STYLE_ID).
pub const CURRENT_SEARCH_STYLE_ID: StyleId = u8::MAX - 1;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FilterDecision {
    /// This line should be shown (matched an include filter).
    Include,
    /// This line should be hidden (matched an exclude filter).
    Exclude,
    /// This filter has no opinion about this line.
    Neutral,
}

pub trait Filter: Send + Sync {
    fn evaluate(&self, line: &[u8], collector: &mut MatchCollector) -> FilterDecision;
}

/// Render a line using the collected match spans and a styles array.
///
/// Overlapping spans are resolved by priority: higher-priority spans overwrite lower ones.
/// Uses a sweep-line with a max-heap keyed by `(priority, Reverse(start))` so each boundary
/// interval is resolved in O(log N) instead of the previous O(N) linear scan — O(N log N) total.
pub fn render_line<'a>(col: &MatchCollector, styles: &[ratatui::style::Style]) -> Line<'a> {
    if col.spans.is_empty() {
        let text = std::str::from_utf8(col.line).unwrap_or("").to_string();
        return Line::from(text);
    }

    let line_len = col.line.len();

    // Filter to valid spans and sort by start for the sweep activation step.
    let mut valid: Vec<(usize, usize, u32, StyleId)> = col
        .spans
        .iter()
        .filter(|s| s.start < s.end && s.end <= line_len)
        .map(|s| (s.start, s.end, s.priority, s.style))
        .collect();

    if valid.is_empty() {
        let text = std::str::from_utf8(col.line).unwrap_or("").to_string();
        return Line::from(text);
    }

    valid.sort_unstable_by_key(|&(start, _, _, _)| start);

    // Collect unique boundary points from valid spans.
    let mut boundaries: Vec<usize> = Vec::with_capacity(valid.len() * 2 + 2);
    boundaries.push(0);
    boundaries.push(line_len);
    for &(start, end, _, _) in &valid {
        boundaries.push(start);
        boundaries.push(end);
    }
    boundaries.sort_unstable();
    boundaries.dedup();

    // Sweep-line: heap of (priority, Reverse(start), end, style_id).
    // Max-heap on priority; Reverse(start) breaks ties in favour of the span that begins
    // earliest, matching the previous sorted.find() semantics.
    // For each interval [seg_s, seg_e): activate spans with start ≤ seg_s, lazily expire
    // those with end ≤ seg_s, then take the heap top as the winner.
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    let mut heap: BinaryHeap<(u32, Reverse<usize>, usize, StyleId)> = BinaryHeap::new();
    let mut span_idx = 0usize;

    // Merge adjacent intervals that share the same style to minimise Span count.
    let mut events: Vec<(usize, usize, StyleId)> = Vec::with_capacity(boundaries.len());
    for w in boundaries.windows(2) {
        let (seg_s, seg_e) = (w[0], w[1]);
        if seg_s >= seg_e {
            continue;
        }
        // Activate all spans whose start ≤ seg_s.
        while span_idx < valid.len() && valid[span_idx].0 <= seg_s {
            let (start, end, priority, style) = valid[span_idx];
            heap.push((priority, Reverse(start), end, style));
            span_idx += 1;
        }
        // Lazily expire spans that no longer cover this interval.
        while heap.peek().is_some_and(|&(_, _, end, _)| end <= seg_s) {
            heap.pop();
        }
        // The heap top is the highest-priority span covering [seg_s, seg_e).
        if let Some(&(_, _, _, style)) = heap.peek() {
            if let Some(last) = events.last_mut()
                && last.1 == seg_s
                && last.2 == style
            {
                last.1 = seg_e;
                continue;
            }
            events.push((seg_s, seg_e, style));
        }
    }

    // Build ratatui spans from events, filling unstyled gaps with raw text.
    let mut spans: Vec<Span<'a>> = Vec::new();
    let mut pos = 0usize;

    for (start, end, style_id) in events {
        if start > pos {
            let text = std::str::from_utf8(&col.line[pos..start])
                .unwrap_or("")
                .to_string();
            if !text.is_empty() {
                spans.push(Span::raw(text));
            }
        }
        if end > start {
            let text = std::str::from_utf8(&col.line[start..end])
                .unwrap_or("")
                .to_string();
            let style = styles.get(style_id as usize).copied().unwrap_or_default();
            if !text.is_empty() {
                spans.push(Span::styled(text, style));
            }
        }
        pos = end.max(pos);
    }

    if pos < line_len {
        let text = std::str::from_utf8(&col.line[pos..])
            .unwrap_or("")
            .to_string();
        if !text.is_empty() {
            spans.push(Span::raw(text));
        }
    }

    Line::from(spans)
}

#[derive(Debug, Clone)]
pub struct MatchSpan {
    pub start: usize,
    pub end: usize,
    pub style: StyleId,
    pub priority: u32, // higher = stronger
}

pub struct MatchCollector<'a> {
    pub line: &'a [u8],
    pub spans: Vec<MatchSpan>,
    current_priority: u32,
}

impl<'a> MatchCollector<'a> {
    pub fn new(line: &'a [u8]) -> Self {
        Self {
            line,
            spans: Vec::with_capacity(8),
            current_priority: 0,
        }
    }

    pub fn with_priority(&mut self, priority: u32) -> &mut Self {
        self.current_priority = priority;
        self
    }

    pub fn push(&mut self, start: usize, end: usize, style: StyleId) {
        self.spans.push(MatchSpan {
            start,
            end,
            style,
            priority: self.current_priority,
        });
    }
}

/// Returns true if `pattern` contains any regex metacharacters.
fn is_regex_pattern(pattern: &str) -> bool {
    pattern.chars().any(|c| {
        matches!(
            c,
            '.' | '+' | '*' | '?' | '[' | ']' | '(' | ')' | '{' | '}' | '\\' | '^' | '$' | '|'
        )
    })
}

/// Include/exclude filter using Aho-Corasick for efficient literal substring matching.
pub struct SubstringFilter {
    ac: AhoCorasick,
    decision: FilterDecision,
    style_id: StyleId,
    /// If true, only colour the matched spans rather than the whole line.
    match_only: bool,
}

impl SubstringFilter {
    pub fn new(
        pattern: &str,
        decision: FilterDecision,
        match_only: bool,
        style_id: StyleId,
    ) -> Option<Self> {
        let ac = AhoCorasick::builder()
            .ascii_case_insensitive(false)
            .build([pattern])
            .inspect_err(|e| {
                tracing::error!("Failed to build Aho-Corasick automaton: {}", e);
            })
            .ok()?;
        Some(SubstringFilter {
            ac,
            decision,
            style_id,
            match_only,
        })
    }
}

impl Filter for SubstringFilter {
    fn evaluate(&self, line: &[u8], collector: &mut MatchCollector) -> FilterDecision {
        let mut found = false;
        for mat in self.ac.find_iter(line) {
            found = true;
            if matches!(self.decision, FilterDecision::Include) && self.match_only {
                collector.push(mat.start(), mat.end(), self.style_id);
            }
        }
        if found {
            // For Include filters, add a full-line span when not match_only
            if matches!(self.decision, FilterDecision::Include) && !self.match_only {
                collector.push(0, line.len(), self.style_id);
            }
            self.decision
        } else {
            FilterDecision::Neutral
        }
    }
}

/// Include/exclude filter using Regex for pattern matching.
pub struct RegexFilter {
    re: Regex,
    decision: FilterDecision,
    style_id: StyleId,
    match_only: bool,
}

impl RegexFilter {
    /// Returns `None` if the pattern is not a valid regex.
    pub fn new(
        pattern: &str,
        decision: FilterDecision,
        match_only: bool,
        style_id: StyleId,
    ) -> Option<Self> {
        Regex::new(pattern).ok().map(|re| RegexFilter {
            re,
            decision,
            style_id,
            match_only,
        })
    }
}

impl Filter for RegexFilter {
    fn evaluate(&self, line: &[u8], collector: &mut MatchCollector) -> FilterDecision {
        let text = match std::str::from_utf8(line) {
            Ok(s) => s,
            Err(_) => return FilterDecision::Neutral,
        };
        let mut found = false;
        for mat in self.re.find_iter(text) {
            found = true;
            if matches!(self.decision, FilterDecision::Include) && self.match_only {
                collector.push(mat.start(), mat.end(), self.style_id);
            }
        }
        if found {
            if matches!(self.decision, FilterDecision::Include) && !self.match_only {
                collector.push(0, line.len(), self.style_id);
            }
            self.decision
        } else {
            FilterDecision::Neutral
        }
    }
}

/// Builds a `Box<dyn Filter>` from a pattern, choosing Aho-Corasick vs Regex automatically.
pub fn build_filter(
    pattern: &str,
    decision: FilterDecision,
    match_only: bool,
    style_id: StyleId,
) -> Option<Box<dyn Filter>> {
    if is_regex_pattern(pattern) {
        RegexFilter::new(pattern, decision, match_only, style_id)
            .map(|f| Box::new(f) as Box<dyn Filter>)
    } else {
        SubstringFilter::new(pattern, decision, match_only, style_id)
            .map(|f| Box::new(f) as Box<dyn Filter>)
    }
}

/// Orchestrates a layered pipeline of filters and provides parallel visibility computation.
pub struct FilterManager {
    filters: Vec<Box<dyn Filter>>,
    /// True if any enabled Include filter exists.
    has_include_filters: bool,
}

impl FilterManager {
    pub fn new(filters: Vec<Box<dyn Filter>>, has_include_filters: bool) -> Self {
        FilterManager {
            filters,
            has_include_filters,
        }
    }

    pub fn empty() -> Self {
        FilterManager {
            filters: Vec::new(),
            has_include_filters: false,
        }
    }

    /// Returns true if `line` should be visible under the current filter set.
    ///
    /// Filters are evaluated top-to-bottom (index 0 = highest precedence).
    /// The first filter that matches (Include or Exclude) determines the outcome.
    /// If no filter matches, the line is visible only when there are no Include filters.
    pub fn is_visible(&self, line: &[u8]) -> bool {
        let mut dummy = MatchCollector::new(line);
        for filter in &self.filters {
            match filter.evaluate(line, &mut dummy) {
                FilterDecision::Include => return true,
                FilterDecision::Exclude => return false,
                FilterDecision::Neutral => {}
            }
        }
        !self.has_include_filters
    }

    /// Run all filters on `line` and collect styling spans for rendering.
    pub fn evaluate_line<'a>(&self, line: &'a [u8]) -> MatchCollector<'a> {
        let mut collector = MatchCollector::new(line);
        for filter in &self.filters {
            filter.evaluate(line, &mut collector);
        }
        collector
    }

    /// Run all filters and add styling spans into an existing collector.
    /// Use this when you need to pre-seed the collector with other spans
    /// (e.g. process colors) before filter spans are added.
    pub fn evaluate_into(&self, collector: &mut MatchCollector<'_>) {
        let line = collector.line;
        for filter in &self.filters {
            filter.evaluate(line, collector);
        }
    }

    /// Compute visible line indices from a `FileReader` using Rayon parallel processing.
    /// The returned indices are in ascending order.
    pub fn compute_visible(&self, reader: &crate::file_reader::FileReader) -> Vec<usize> {
        use rayon::prelude::*;
        let count = reader.line_count();
        let has_include = self.has_include_filters;
        let filters = &self.filters;

        let visible: Vec<usize> = (0..count)
            .into_par_iter()
            .filter(|&idx| {
                let line = reader.get_line(idx);
                let mut dummy = MatchCollector::new(line);
                for filter in filters.iter() {
                    match filter.evaluate(line, &mut dummy) {
                        FilterDecision::Include => return true,
                        FilterDecision::Exclude => return false,
                        FilterDecision::Neutral => {}
                    }
                }
                !has_include
            })
            .collect();

        visible
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

    #[test]
    fn test_substring_filter_include() {
        let line = b"ERROR: connection refused";
        let f = SubstringFilter::new("ERROR", FilterDecision::Include, false, 0).unwrap();
        let mut col = MatchCollector::new(line);
        assert_eq!(f.evaluate(line, &mut col), FilterDecision::Include);

        let no_match = b"INFO: all good";
        let mut col2 = MatchCollector::new(no_match);
        assert_eq!(f.evaluate(no_match, &mut col2), FilterDecision::Neutral);
    }

    #[test]
    fn test_substring_filter_exclude() {
        let line = b"DEBUG: verbose output";
        let f = SubstringFilter::new("DEBUG", FilterDecision::Exclude, false, 0).unwrap();
        let mut col = MatchCollector::new(line);
        assert_eq!(f.evaluate(line, &mut col), FilterDecision::Exclude);

        let no_match = b"INFO: important";
        let mut col2 = MatchCollector::new(no_match);
        assert_eq!(f.evaluate(no_match, &mut col2), FilterDecision::Neutral);
    }

    #[test]
    fn test_substring_filter_match_only_spans() {
        let line = b"ERROR: something went wrong";
        let f = SubstringFilter::new("ERROR", FilterDecision::Include, true, 1).unwrap();
        let mut col = MatchCollector::new(line);
        f.evaluate(line, &mut col);
        assert_eq!(col.spans.len(), 1);
        assert_eq!(col.spans[0].start, 0);
        assert_eq!(col.spans[0].end, 5);
        assert_eq!(col.spans[0].style, 1);
    }

    #[test]
    fn test_regex_filter_include() {
        let line = b"GET /api/users 200 OK";
        let f = RegexFilter::new(r"\d{3}", FilterDecision::Include, true, 0).unwrap();
        let mut col = MatchCollector::new(line);
        assert_eq!(f.evaluate(line, &mut col), FilterDecision::Include);
        // Should have a span covering "200"
        assert_eq!(col.spans.len(), 1);
        assert_eq!(&line[col.spans[0].start..col.spans[0].end], b"200");
    }

    #[test]
    fn test_regex_filter_invalid_pattern() {
        assert!(RegexFilter::new("[invalid", FilterDecision::Include, false, 0).is_none());
    }

    #[test]
    fn test_build_filter_selects_substring_for_literal() {
        // Pure literal — should work (no panic)
        let f = build_filter("error", FilterDecision::Include, false, 0);
        assert!(f.is_some());
    }

    #[test]
    fn test_build_filter_selects_regex_for_pattern() {
        // Has regex chars — should use RegexFilter
        let f = build_filter(r"error\d+", FilterDecision::Include, false, 0);
        assert!(f.is_some());
    }

    #[test]
    fn test_filter_manager_no_filters_all_visible() {
        let fm = FilterManager::empty();
        assert!(fm.is_visible(b"anything"));
        assert!(fm.is_visible(b""));
    }

    #[test]
    fn test_filter_manager_include_filter() {
        let f = SubstringFilter::new("ERROR", FilterDecision::Include, false, 0).unwrap();
        let fm = FilterManager::new(vec![Box::new(f)], true);

        assert!(fm.is_visible(b"ERROR: bad things"));
        assert!(!fm.is_visible(b"INFO: all good"));
    }

    #[test]
    fn test_filter_manager_exclude_filter() {
        let f = SubstringFilter::new("DEBUG", FilterDecision::Exclude, false, 0).unwrap();
        let fm = FilterManager::new(vec![Box::new(f)], false);

        assert!(fm.is_visible(b"INFO: something"));
        assert!(!fm.is_visible(b"DEBUG: verbose"));
    }

    #[test]
    fn test_filter_manager_include_then_exclude() {
        // Exclude "minor" at top (higher precedence), Include "ERROR" below.
        // First-match-wins: a line matching the top Exclude is hidden even if
        // a lower Include filter also matches it.
        let exc = SubstringFilter::new("minor", FilterDecision::Exclude, false, 1).unwrap();
        let inc = SubstringFilter::new("ERROR", FilterDecision::Include, false, 0).unwrap();
        let fm = FilterManager::new(vec![Box::new(exc), Box::new(inc)], true);

        assert!(fm.is_visible(b"ERROR: critical failure")); // no exclude match → include matches
        assert!(!fm.is_visible(b"ERROR: minor issue")); // exclude at top wins
        assert!(!fm.is_visible(b"INFO: unrelated")); // no match at all → has include filters → hidden
    }

    #[test]
    fn test_filter_manager_compute_visible() {
        let (_f, reader) = make_reader(&[
            "ERROR: bad",
            "INFO: good",
            "ERROR: also bad",
            "DEBUG: verbose",
        ]);

        let inc = SubstringFilter::new("ERROR", FilterDecision::Include, false, 0).unwrap();
        let fm = FilterManager::new(vec![Box::new(inc)], true);
        let visible = fm.compute_visible(&reader);

        assert_eq!(visible, vec![0, 2]);
    }

    #[test]
    fn test_filter_manager_compute_visible_exclude() {
        let (_f, reader) = make_reader(&["ERROR: bad", "DEBUG: verbose", "INFO: good"]);

        let exc = SubstringFilter::new("DEBUG", FilterDecision::Exclude, false, 0).unwrap();
        let fm = FilterManager::new(vec![Box::new(exc)], false);
        let visible = fm.compute_visible(&reader);

        assert_eq!(visible, vec![0, 2]);
    }

    #[test]
    fn test_render_line_no_spans() {
        let line = b"plain text";
        let col = MatchCollector::new(line);
        let styles: Vec<ratatui::style::Style> = vec![];
        let rendered = render_line(&col, &styles);
        let text: String = rendered.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "plain text");
    }

    #[test]
    fn test_render_line_with_span() {
        let line = b"hello world";
        let mut col = MatchCollector::new(line);
        let style = ratatui::style::Style::default().fg(ratatui::style::Color::Red);
        let styles = vec![style];
        col.push(6, 11, 0); // "world"
        let rendered = render_line(&col, &styles);
        // Should have "hello " unstyled and "world" styled
        assert!(rendered.spans.len() >= 2);
    }

    #[test]
    fn test_evaluate_line_collects_spans() {
        let line = b"ERROR: connection refused to host";
        let f = SubstringFilter::new("ERROR", FilterDecision::Include, true, 0).unwrap();
        let fm = FilterManager::new(vec![Box::new(f)], true);
        let col = fm.evaluate_line(line);
        assert!(!col.spans.is_empty());
        assert_eq!(&line[col.spans[0].start..col.spans[0].end], b"ERROR");
    }

    #[test]
    fn test_render_line_overlapping_spans_priority() {
        // Two spans cover the same region. Higher priority must win.
        let line = b"hello world";
        let style_lo = ratatui::style::Style::default().fg(ratatui::style::Color::Blue);
        let style_hi = ratatui::style::Style::default().fg(ratatui::style::Color::Red);
        let styles = vec![style_lo, style_hi];

        let mut col = MatchCollector::new(line);
        col.with_priority(0);
        col.push(0, 5, 0); // "hello" — low priority, style 0 (Blue)
        col.with_priority(10);
        col.push(0, 5, 1); // "hello" — high priority, style 1 (Red)

        let rendered = render_line(&col, &styles);
        let hello_span = rendered
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "hello");
        assert!(hello_span.is_some());
        assert_eq!(
            hello_span.unwrap().style.fg,
            Some(ratatui::style::Color::Red)
        );
    }

    #[test]
    fn test_render_line_adjacent_same_style_merged() {
        // Two adjacent spans with the same style should be merged into one.
        let line = b"abcdef";
        let style = ratatui::style::Style::default().fg(ratatui::style::Color::Green);
        let styles = vec![style];

        let mut col = MatchCollector::new(line);
        col.push(0, 3, 0); // "abc"
        col.push(3, 6, 0); // "def" — same style, adjacent

        let rendered = render_line(&col, &styles);
        let styled: Vec<_> = rendered
            .spans
            .iter()
            .filter(|s| s.style.fg.is_some())
            .collect();
        assert_eq!(styled.len(), 1);
        assert_eq!(styled[0].content.as_ref(), "abcdef");
    }
}
