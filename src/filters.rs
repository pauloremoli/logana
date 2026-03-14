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

impl FilterDecision {
    /// Returns true when this decision is Include or Exclude (not Neutral).
    #[inline]
    pub fn is_decided(self) -> bool {
        self != FilterDecision::Neutral
    }

    /// Convert to a visibility boolean given whether include filters exist.
    #[inline]
    pub fn to_visibility(self, has_include_filters: bool) -> bool {
        match self {
            FilterDecision::Include => true,
            FilterDecision::Exclude => false,
            FilterDecision::Neutral => !has_include_filters,
        }
    }
}

pub trait Filter: Send + Sync {
    fn evaluate(&self, line: &[u8], collector: &mut MatchCollector) -> FilterDecision;

    /// The decision this filter produces on a match (Include or Exclude).
    fn decision(&self) -> FilterDecision;

    /// Return the filter decision without collecting match spans.
    ///
    /// Used by visibility-check paths that do not need span data, avoiding the
    /// `MatchCollector` heap allocation entirely.  The default implementation
    /// delegates to [`evaluate`] with a throwaway collector; implementors should
    /// override this with a cheaper path.
    fn matches(&self, line: &[u8]) -> FilterDecision {
        let mut dummy = MatchCollector::new(line);
        self.evaluate(line, &mut dummy)
    }
}

/// Lossily convert a byte slice to an owned `String`.
#[inline]
fn slice_to_string(bytes: &[u8]) -> String {
    std::str::from_utf8(bytes).unwrap_or("").to_string()
}

/// Drain set bits from a u64 bitset into a mutable counts slice.
#[inline]
fn flush_bitset_counts(mut bits: u64, counts: &mut [usize]) {
    while bits != 0 {
        let bit = bits.trailing_zeros() as usize;
        if let Some(c) = counts.get_mut(bit) {
            *c += 1;
        }
        bits &= bits - 1;
    }
}

/// Drain set bits from a u64 bitset into an atomic counts slice.
#[inline]
fn flush_bitset_counts_atomic(mut bits: u64, counts: &[std::sync::atomic::AtomicUsize]) {
    while bits != 0 {
        let bit = bits.trailing_zeros() as usize;
        if let Some(c) = counts.get(bit) {
            c.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        bits &= bits - 1;
    }
}

/// Extract line bytes at `global` index from a contiguous data buffer.
///
/// Strips the trailing newline if present, matching `FileReader::get_line` semantics.
#[inline]
fn line_bytes_at<'a>(data: &'a [u8], line_starts: &[usize], global: usize) -> &'a [u8] {
    let start = line_starts[global];
    let end = if global + 1 < line_starts.len() {
        let next = line_starts[global + 1];
        if next > 0 && data.get(next - 1) == Some(&b'\n') {
            next - 1
        } else {
            next
        }
    } else {
        data.len()
    };
    &data[start..end]
}

/// Render a line using the collected match spans and a styles array.
///
/// For each boundary interval, `fg` and `bg` are composed independently: each attribute
/// is taken from the highest-priority active span that has that attribute set.  This lets
/// a level filter (fg only) and a text filter (bg only) both apply to the same segment.
pub fn render_line<'a>(col: &MatchCollector, styles: &[ratatui::style::Style]) -> Line<'a> {
    if col.spans.is_empty() {
        return Line::from(slice_to_string(col.line));
    }

    let line_len = col.line.len();
    let mut valid = collect_valid_spans(col, line_len);
    if valid.is_empty() {
        return Line::from(slice_to_string(col.line));
    }
    valid.sort_unstable_by_key(|&(start, _, _, _)| start);

    let boundaries = collect_boundaries(&valid, line_len);
    let events = sweep_styled_events(&valid, &boundaries, styles);
    events_to_line(col.line, line_len, &events)
}

/// Filter collector spans to valid (non-empty, in-bounds) tuples.
#[inline]
fn collect_valid_spans(col: &MatchCollector, line_len: usize) -> Vec<(usize, usize, u32, StyleId)> {
    col.spans
        .iter()
        .filter(|s| s.start < s.end && s.end <= line_len)
        .map(|s| (s.start, s.end, s.priority, s.style))
        .collect()
}

/// Collect unique sorted boundary points from valid spans plus line extents.
#[inline]
fn collect_boundaries(valid: &[(usize, usize, u32, StyleId)], line_len: usize) -> Vec<usize> {
    let mut boundaries = Vec::with_capacity(valid.len() * 2 + 2);
    boundaries.push(0);
    boundaries.push(line_len);
    for &(start, end, _, _) in valid {
        boundaries.push(start);
        boundaries.push(end);
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    boundaries
}

/// Sweep-line pass: for each boundary interval compose fg/bg from the active
/// spans and merge adjacent intervals with the same style.
fn sweep_styled_events(
    valid: &[(usize, usize, u32, StyleId)],
    boundaries: &[usize],
    styles: &[ratatui::style::Style],
) -> Vec<(usize, usize, ratatui::style::Style)> {
    let mut active: Vec<(u32, usize, StyleId)> = Vec::new();
    let mut span_idx = 0usize;
    let mut events: Vec<(usize, usize, ratatui::style::Style)> =
        Vec::with_capacity(boundaries.len());

    for w in boundaries.windows(2) {
        let (seg_s, seg_e) = (w[0], w[1]);
        if seg_s >= seg_e {
            continue;
        }
        while span_idx < valid.len() && valid[span_idx].0 <= seg_s {
            let (_, end, priority, style) = valid[span_idx];
            active.push((priority, end, style));
            span_idx += 1;
        }
        active.retain(|&(_, end, _)| end > seg_s);
        if active.is_empty() {
            continue;
        }
        let composed = compose_segment_style(&active, styles);
        if composed.fg.is_none() && composed.bg.is_none() {
            continue;
        }
        if let Some(last) = events.last_mut()
            && last.1 == seg_s
            && last.2 == composed
        {
            last.1 = seg_e;
        } else {
            events.push((seg_s, seg_e, composed));
        }
    }
    events
}

/// Build a ratatui [`Line`] from styled events, filling unstyled gaps with raw text.
fn events_to_line<'a>(
    line: &[u8],
    line_len: usize,
    events: &[(usize, usize, ratatui::style::Style)],
) -> Line<'a> {
    let mut spans: Vec<Span<'a>> = Vec::new();
    let mut pos = 0usize;

    for &(start, end, style) in events {
        if start > pos {
            let text = slice_to_string(&line[pos..start]);
            if !text.is_empty() {
                spans.push(Span::raw(text));
            }
        }
        if end > start {
            let text = slice_to_string(&line[start..end]);
            if !text.is_empty() {
                spans.push(Span::styled(text, style));
            }
        }
        pos = end.max(pos);
    }

    if pos < line_len {
        let text = slice_to_string(&line[pos..]);
        if !text.is_empty() {
            spans.push(Span::raw(text));
        }
    }
    Line::from(spans)
}

/// Compose a single [`ratatui::style::Style`] from a set of active spans.
///
/// `fg` is taken from the span with the highest priority that has `fg` set;
/// `bg` from the span with the highest priority that has `bg` set.
#[inline]
fn compose_segment_style(
    active: &[(u32, usize, StyleId)],
    styles: &[ratatui::style::Style],
) -> ratatui::style::Style {
    let mut best_fg: Option<ratatui::style::Color> = None;
    let mut best_fg_priority: u32 = 0;
    let mut best_bg: Option<ratatui::style::Color> = None;
    let mut best_bg_priority: u32 = 0;

    for &(priority, _, style_id) in active {
        let style = styles.get(style_id as usize).copied().unwrap_or_default();
        if let Some(fg) = style.fg
            && (best_fg.is_none() || priority > best_fg_priority)
        {
            best_fg_priority = priority;
            best_fg = Some(fg);
        }
        if let Some(bg) = style.bg
            && (best_bg.is_none() || priority > best_bg_priority)
        {
            best_bg_priority = priority;
            best_bg = Some(bg);
        }
    }

    let mut composed = ratatui::style::Style::default();
    if let Some(fg) = best_fg {
        composed = composed.fg(fg);
    }
    if let Some(bg) = best_bg {
        composed = composed.bg(bg);
    }
    composed
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
pub(crate) fn is_regex_pattern(pattern: &str) -> bool {
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
    /// Precomputed: push individual match spans (Include && match_only).
    push_match_spans: bool,
    /// Precomputed: push a full-line span (Include && !match_only).
    push_full_line: bool,
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
            .ok()?;
        let is_include = decision == FilterDecision::Include;
        Some(SubstringFilter {
            ac,
            decision,
            style_id,
            push_match_spans: is_include && match_only,
            push_full_line: is_include && !match_only,
        })
    }
}

impl Filter for SubstringFilter {
    #[inline]
    fn decision(&self) -> FilterDecision {
        self.decision
    }

    fn evaluate(&self, line: &[u8], collector: &mut MatchCollector) -> FilterDecision {
        let mut found = false;
        for mat in self.ac.find_iter(line) {
            found = true;
            if self.push_match_spans {
                collector.push(mat.start(), mat.end(), self.style_id);
            }
        }
        if found {
            if self.push_full_line {
                collector.push(0, line.len(), self.style_id);
            }
            self.decision
        } else {
            FilterDecision::Neutral
        }
    }

    fn matches(&self, line: &[u8]) -> FilterDecision {
        if self.ac.is_match(line) {
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
    /// Precomputed: push individual match spans (Include && match_only).
    push_match_spans: bool,
    /// Precomputed: push a full-line span (Include && !match_only).
    push_full_line: bool,
}

impl RegexFilter {
    /// Returns `None` if the pattern is not a valid regex.
    pub fn new(
        pattern: &str,
        decision: FilterDecision,
        match_only: bool,
        style_id: StyleId,
    ) -> Option<Self> {
        let is_include = decision == FilterDecision::Include;
        Regex::new(pattern).ok().map(|re| RegexFilter {
            re,
            decision,
            style_id,
            push_match_spans: is_include && match_only,
            push_full_line: is_include && !match_only,
        })
    }
}

impl Filter for RegexFilter {
    #[inline]
    fn decision(&self) -> FilterDecision {
        self.decision
    }

    fn evaluate(&self, line: &[u8], collector: &mut MatchCollector) -> FilterDecision {
        let text = match std::str::from_utf8(line) {
            Ok(s) => s,
            Err(_) => return FilterDecision::Neutral,
        };
        let mut found = false;
        for mat in self.re.find_iter(text) {
            found = true;
            if self.push_match_spans {
                collector.push(mat.start(), mat.end(), self.style_id);
            }
        }
        if found {
            if self.push_full_line {
                collector.push(0, line.len(), self.style_id);
            }
            self.decision
        } else {
            FilterDecision::Neutral
        }
    }

    fn matches(&self, line: &[u8]) -> FilterDecision {
        let text = match std::str::from_utf8(line) {
            Ok(s) => s,
            Err(_) => return FilterDecision::Neutral,
        };
        if self.re.is_match(text) {
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
    /// Per-filter decision: `filter_decisions[i]` is the `FilterDecision` for
    /// `filters[i]`. Used by whole-buffer scan paths to map filter index → decision
    /// without re-evaluating the filter.
    filter_decisions: Vec<FilterDecision>,
    /// True if any enabled Include filter exists.
    has_include_filters: bool,
    /// Combined Aho-Corasick automaton built from all literal (non-regex) patterns.
    /// `None` when fewer than 2 literal patterns exist (no benefit over per-filter scan).
    combined_ac: Option<AhoCorasick>,
    /// Maps combined AC pattern index → (filter index in `self.filters`, FilterDecision).
    combined_ac_meta: Vec<(usize, FilterDecision)>,
    /// Indices into `self.filters` that are regex-based (not covered by `combined_ac`).
    regex_filter_indices: Vec<usize>,
}

impl FilterManager {
    pub fn new(filters: Vec<Box<dyn Filter>>, has_include_filters: bool) -> Self {
        let n = filters.len();
        let filter_decisions: Vec<FilterDecision> = filters.iter().map(|f| f.decision()).collect();
        FilterManager {
            filters,
            filter_decisions,
            has_include_filters,
            combined_ac: None,
            combined_ac_meta: Vec::new(),
            // Treat all filters as needing individual evaluation when no combined AC.
            regex_filter_indices: (0..n).collect(),
        }
    }

    pub fn new_with_combined(
        filters: Vec<Box<dyn Filter>>,
        has_include_filters: bool,
        combined_ac: Option<AhoCorasick>,
        combined_ac_meta: Vec<(usize, FilterDecision)>,
        regex_filter_indices: Vec<usize>,
    ) -> Self {
        let filter_decisions: Vec<FilterDecision> = filters.iter().map(|f| f.decision()).collect();
        FilterManager {
            filters,
            filter_decisions,
            has_include_filters,
            combined_ac,
            combined_ac_meta,
            regex_filter_indices,
        }
    }

    pub fn empty() -> Self {
        FilterManager {
            filters: Vec::new(),
            filter_decisions: Vec::new(),
            has_include_filters: false,
            combined_ac: None,
            combined_ac_meta: Vec::new(),
            regex_filter_indices: Vec::new(),
        }
    }

    /// Returns true if any enabled Include filter exists.
    #[inline]
    pub fn has_include(&self) -> bool {
        self.has_include_filters
    }

    /// Evaluate text filters and return the first-match decision.
    ///
    /// Returns `Include` or `Exclude` on the first match; `Neutral` if no filter matched.
    /// This is the same as `is_visible` but returns the decision rather than a bool,
    /// allowing callers to combine it with field and date filter results.
    ///
    /// When a combined Aho-Corasick automaton is available, a single scan covers all
    /// literal patterns; regex-only filters are checked individually afterwards.
    pub fn evaluate_text(&self, line: &[u8]) -> FilterDecision {
        if let Some(ref ac) = self.combined_ac {
            let mut best: Option<(usize, FilterDecision)> = None;

            for mat in ac.find_iter(line) {
                let (filter_idx, decision) = self.combined_ac_meta[mat.pattern().as_usize()];
                if best.is_none_or(|(best_idx, _)| filter_idx < best_idx) {
                    best = Some((filter_idx, decision));
                }
            }

            for &fi in &self.regex_filter_indices {
                if let Some(filter) = self.filters.get(fi) {
                    let d = filter.matches(line);
                    if d.is_decided() && best.is_none_or(|(best_idx, _)| fi < best_idx) {
                        best = Some((fi, d));
                    }
                }
            }

            best.map(|(_, d)| d).unwrap_or(FilterDecision::Neutral)
        } else {
            for filter in &self.filters {
                let d = filter.matches(line);
                if d.is_decided() {
                    return d;
                }
            }
            FilterDecision::Neutral
        }
    }

    /// Returns true if `line` should be visible under the current filter set.
    ///
    /// Filters are evaluated top-to-bottom (index 0 = highest precedence).
    /// The first filter that matches (Include or Exclude) determines the outcome.
    /// If no filter matches, the line is visible only when there are no Include filters.
    #[inline]
    pub fn is_visible(&self, line: &[u8]) -> bool {
        self.evaluate_text(line)
            .to_visibility(self.has_include_filters)
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

    /// Returns the number of compiled text filters in this manager.
    #[inline]
    pub fn filter_count(&self) -> usize {
        self.filters.len()
    }

    /// Count all matching filters into `counts` and return the first-match [`FilterDecision`]
    /// in a single scan.
    ///
    /// Combines the work of [`count_line_matches`] and [`evaluate_text`]: the Aho-Corasick
    /// automaton (or individual filter list) is scanned exactly once per call.  All filters
    /// that match are counted; the decision of the lowest-index matching filter is returned.
    ///
    /// `counts` is a plain (non-atomic) slice — callers must use thread-local accumulators
    /// and merge them after the parallel scan.
    pub fn evaluate_and_count(&self, line: &[u8], counts: &mut [usize]) -> FilterDecision {
        if let Some(ref ac) = self.combined_ac {
            let mut best_idx = usize::MAX;
            let mut best_decision = FilterDecision::Neutral;

            if self.filters.len() <= 64 {
                let mut seen: u64 = 0;
                for m in ac.find_iter(line) {
                    let (filter_idx, decision) = self.combined_ac_meta[m.pattern().as_usize()];
                    if filter_idx < best_idx {
                        best_idx = filter_idx;
                        best_decision = decision;
                    }
                    seen |= 1u64 << filter_idx;
                }
                flush_bitset_counts(seen, counts);
            } else {
                let mut matched: Vec<usize> = ac
                    .find_iter(line)
                    .map(|m| {
                        let (filter_idx, decision) = self.combined_ac_meta[m.pattern().as_usize()];
                        if filter_idx < best_idx {
                            best_idx = filter_idx;
                            best_decision = decision;
                        }
                        filter_idx
                    })
                    .collect();
                matched.sort_unstable();
                matched.dedup();
                for filter_idx in matched {
                    if let Some(c) = counts.get_mut(filter_idx) {
                        *c += 1;
                    }
                }
            }

            for &fi in &self.regex_filter_indices {
                if let Some(filter) = self.filters.get(fi) {
                    let d = filter.matches(line);
                    if d.is_decided() {
                        if let Some(c) = counts.get_mut(fi) {
                            *c += 1;
                        }
                        if fi < best_idx {
                            best_idx = fi;
                            best_decision = d;
                        }
                    }
                }
            }

            best_decision
        } else {
            let mut result = FilterDecision::Neutral;
            let mut has_best = false;
            for (i, filter) in self.filters.iter().enumerate() {
                let d = filter.matches(line);
                if d.is_decided() {
                    if let Some(c) = counts.get_mut(i) {
                        *c += 1;
                    }
                    if !has_best {
                        result = d;
                        has_best = true;
                    }
                }
            }
            result
        }
    }

    /// Evaluate each filter independently on `line` and increment the corresponding
    /// counter in `counts` (indexed parallel to the internal filter list).
    ///
    /// Unlike `is_visible`, this does not short-circuit: every filter is evaluated
    /// so that per-filter match counts accumulate correctly across lines.
    pub fn count_line_matches(&self, line: &[u8], counts: &[std::sync::atomic::AtomicUsize]) {
        if let Some(ref ac) = self.combined_ac {
            if self.filters.len() <= 64 {
                let mut seen: u64 = 0;
                for m in ac.find_iter(line) {
                    seen |= 1u64 << self.combined_ac_meta[m.pattern().as_usize()].0;
                }
                flush_bitset_counts_atomic(seen, counts);
            } else {
                let mut matched: Vec<usize> = ac
                    .find_iter(line)
                    .map(|m| self.combined_ac_meta[m.pattern().as_usize()].0)
                    .collect();
                matched.sort_unstable();
                matched.dedup();
                for filter_idx in matched {
                    if let Some(c) = counts.get(filter_idx) {
                        c.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            }

            for &fi in &self.regex_filter_indices {
                if let Some(filter) = self.filters.get(fi)
                    && filter.matches(line).is_decided()
                    && let Some(c) = counts.get(fi)
                {
                    c.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
        } else {
            for (i, filter) in self.filters.iter().enumerate() {
                if filter.matches(line).is_decided()
                    && let Some(c) = counts.get(i)
                {
                    c.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }
    }

    /// Compute visible line indices from a `FileReader` using Rayon parallel processing.
    /// The returned indices are in ascending order.
    pub fn compute_visible(&self, reader: &crate::file_reader::FileReader) -> Vec<usize> {
        use rayon::prelude::*;
        let count = reader.line_count();
        (0..count)
            .into_par_iter()
            .filter(|&idx| self.is_visible(reader.get_line(idx)))
            .collect()
    }

    /// Returns true when a combined Aho-Corasick automaton is available and can
    /// be used for whole-buffer scanning.
    #[inline]
    pub fn has_combined_ac(&self) -> bool {
        self.combined_ac.is_some()
    }

    /// Evaluate a contiguous range of lines by scanning the raw data buffer with
    /// a single Aho-Corasick pass instead of per-line iterator calls.
    ///
    /// Returns `(visible_indices, text_counts)` where `text_counts[i]` is the
    /// number of lines matched by text filter `i` within this range.
    ///
    /// # Arguments
    /// - `data`: the contiguous file buffer (`FileReader::data()`).
    /// - `line_starts`: sorted byte offsets (`FileReader::line_starts()`).
    /// - `line_range`: the `start..end` range of line indices to evaluate.
    ///
    /// # Panics
    /// Panics if `combined_ac` is `None` — callers must check `has_combined_ac()`.
    pub fn evaluate_chunk_wholefile(
        &self,
        data: &[u8],
        line_starts: &[usize],
        line_range: std::ops::Range<usize>,
    ) -> (Vec<usize>, Vec<usize>) {
        use rayon::prelude::*;

        let ac = self
            .combined_ac
            .as_ref()
            .expect("evaluate_chunk_wholefile requires combined_ac");

        let n_filters = self.filters.len();
        let chunk_line_start = line_range.start;
        let chunk_line_end = line_range.end;
        let chunk_line_count = chunk_line_end - chunk_line_start;
        if chunk_line_count == 0 {
            return (Vec::new(), vec![0; n_filters]);
        }

        let n_threads = rayon::current_num_threads().max(1);
        let sub_chunk_len = chunk_line_count.div_ceil(n_threads);

        let results: Vec<(Vec<usize>, Vec<usize>)> = (0..n_threads)
            .into_par_iter()
            .filter_map(|t| {
                let sub_start = chunk_line_start + t * sub_chunk_len;
                let sub_end = (sub_start + sub_chunk_len).min(chunk_line_end);
                if sub_start >= sub_end {
                    return None;
                }
                Some(self.scan_sub_chunk(ac, data, line_starts, sub_start, sub_end, n_filters))
            })
            .collect();

        merge_sub_chunk_results(results, n_filters)
    }

    /// Scan a sub-chunk of lines using the AC automaton on the contiguous data
    /// buffer, then evaluate regex fallback filters, count matches, and build
    /// the visibility vec.
    fn scan_sub_chunk(
        &self,
        ac: &AhoCorasick,
        data: &[u8],
        line_starts: &[usize],
        sub_start: usize,
        sub_end: usize,
        n_filters: usize,
    ) -> (Vec<usize>, Vec<usize>) {
        let sub_line_count = sub_end - sub_start;
        let mut state = SubChunkState::new(sub_line_count, n_filters);

        self.scan_ac_with_cursor(ac, data, line_starts, sub_start, &mut state);
        self.scan_regex_fallback(data, line_starts, sub_start, &mut state);

        let tc = state.aggregate_counts(n_filters);
        let vis = self.build_visibility_from_best(&state.best, sub_start);
        (vis, tc)
    }

    /// Single AC pass over a contiguous byte range, mapping match positions
    /// to line indices with a forward cursor.
    fn scan_ac_with_cursor(
        &self,
        ac: &AhoCorasick,
        data: &[u8],
        line_starts: &[usize],
        sub_start: usize,
        state: &mut SubChunkState,
    ) {
        let sub_line_count = state.best.len();
        let sub_byte_start = line_starts[sub_start];
        let sub_end = sub_start + sub_line_count;
        let sub_byte_end = if sub_end < line_starts.len() {
            line_starts[sub_end]
        } else {
            data.len()
        };
        let sub_data = &data[sub_byte_start..sub_byte_end];

        let mut cursor: usize = 0;
        let mut cursor_byte_end = next_line_byte(line_starts, data, sub_start);

        for mat in ac.find_iter(sub_data) {
            let abs_pos = sub_byte_start + mat.start();
            while abs_pos >= cursor_byte_end && cursor + 1 < sub_line_count {
                cursor += 1;
                cursor_byte_end = next_line_byte(line_starts, data, sub_start + cursor);
            }

            let (filter_idx, _) = self.combined_ac_meta[mat.pattern().as_usize()];
            state.record(cursor, filter_idx);
        }
    }

    /// Per-line regex fallback for lines not yet decided by a higher-priority AC match.
    fn scan_regex_fallback(
        &self,
        data: &[u8],
        line_starts: &[usize],
        sub_start: usize,
        state: &mut SubChunkState,
    ) {
        if self.regex_filter_indices.is_empty() {
            return;
        }
        let sub_line_count = state.best.len();
        for local in 0..sub_line_count {
            let global = sub_start + local;
            for &fi in &self.regex_filter_indices {
                if state.best[local] != u8::MAX && (state.best[local] as usize) < fi {
                    continue;
                }
                if let Some(filter) = self.filters.get(fi) {
                    let lb = line_bytes_at(data, line_starts, global);
                    if filter.matches(lb).is_decided() {
                        state.record(local, fi);
                    }
                }
            }
        }
    }

    /// Build the visible-indices vec from the per-line best-filter array.
    #[inline]
    fn build_visibility_from_best(&self, best: &[u8], sub_start: usize) -> Vec<usize> {
        let mut vis = Vec::new();
        for (local, &b) in best.iter().enumerate() {
            let decision = if b == u8::MAX {
                FilterDecision::Neutral
            } else {
                self.filter_decisions
                    .get(b as usize)
                    .copied()
                    .unwrap_or(FilterDecision::Neutral)
            };
            if decision.to_visibility(self.has_include_filters) {
                vis.push(sub_start + local);
            }
        }
        vis
    }
}

/// Per-line tracking state for a sub-chunk during whole-buffer scanning.
struct SubChunkState {
    /// Best (lowest-index) matching filter per line; `u8::MAX` = no match.
    best: Vec<u8>,
    /// Bitset dedup for counting when `≤64` filters.
    seen_bits: Vec<u64>,
    /// Fallback dedup for `>64` filters.
    seen_set: Vec<Vec<usize>>,
    /// Whether the bitset path is in use.
    use_bitset: bool,
}

impl SubChunkState {
    fn new(sub_line_count: usize, n_filters: usize) -> Self {
        let use_bitset = n_filters <= 64;
        SubChunkState {
            best: vec![u8::MAX; sub_line_count],
            seen_bits: if use_bitset {
                vec![0u64; sub_line_count]
            } else {
                Vec::new()
            },
            seen_set: if use_bitset {
                Vec::new()
            } else {
                vec![Vec::new(); sub_line_count]
            },
            use_bitset,
        }
    }

    /// Record a filter match for the given local line index.
    #[inline]
    fn record(&mut self, local: usize, filter_idx: usize) {
        let fi8 = filter_idx as u8;
        if fi8 < self.best[local] {
            self.best[local] = fi8;
        }
        if self.use_bitset {
            self.seen_bits[local] |= 1u64 << filter_idx;
        } else {
            self.seen_set[local].push(filter_idx);
        }
    }

    /// Aggregate the seen data into a total counts vec.
    fn aggregate_counts(&mut self, n_filters: usize) -> Vec<usize> {
        let mut tc = vec![0usize; n_filters];
        if self.use_bitset {
            for &bits in &self.seen_bits {
                flush_bitset_counts(bits, &mut tc);
            }
        } else {
            for set in &mut self.seen_set {
                set.sort_unstable();
                set.dedup();
                for &fi in set.iter() {
                    tc[fi] += 1;
                }
            }
        }
        tc
    }
}

/// Byte offset where the line *after* `line_idx` begins (or data length).
#[inline]
fn next_line_byte(line_starts: &[usize], data: &[u8], line_idx: usize) -> usize {
    if line_idx + 1 < line_starts.len() {
        line_starts[line_idx + 1]
    } else {
        data.len()
    }
}

/// Merge parallel sub-chunk results into a single (visible, counts) pair.
fn merge_sub_chunk_results(
    results: Vec<(Vec<usize>, Vec<usize>)>,
    n_filters: usize,
) -> (Vec<usize>, Vec<usize>) {
    let mut visible = Vec::new();
    let mut counts = vec![0usize; n_filters];
    for (vis, tc) in results {
        visible.extend(vis);
        for (a, b) in counts.iter_mut().zip(tc) {
            *a += b;
        }
    }
    (visible, counts)
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

    #[test]
    fn test_render_line_composes_fg_and_bg_from_different_spans() {
        // One span sets fg, another sets bg on the same segment — both must apply.
        let line = b"hello world";
        let style_fg = ratatui::style::Style::default().fg(ratatui::style::Color::Yellow);
        let style_bg = ratatui::style::Style::default().bg(ratatui::style::Color::DarkGray);
        let styles = vec![style_fg, style_bg];

        let mut col = MatchCollector::new(line);
        col.with_priority(0);
        col.push(0, 5, 0); // "hello" — fg=Yellow
        col.with_priority(0);
        col.push(0, 5, 1); // "hello" — bg=DarkGray

        let rendered = render_line(&col, &styles);
        let hello_span = rendered
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "hello");
        assert!(hello_span.is_some());
        let span = hello_span.unwrap();
        assert_eq!(span.style.fg, Some(ratatui::style::Color::Yellow));
        assert_eq!(span.style.bg, Some(ratatui::style::Color::DarkGray));
    }

    #[test]
    fn test_render_line_higher_priority_fg_wins_over_lower() {
        // Two spans both set fg; the higher-priority one must win for fg.
        let line = b"hello";
        let style_lo = ratatui::style::Style::default().fg(ratatui::style::Color::Blue);
        let style_hi = ratatui::style::Style::default().fg(ratatui::style::Color::Red);
        let styles = vec![style_lo, style_hi];

        let mut col = MatchCollector::new(line);
        col.with_priority(0);
        col.push(0, 5, 0); // low priority, fg=Blue
        col.with_priority(10);
        col.push(0, 5, 1); // high priority, fg=Red

        let rendered = render_line(&col, &styles);
        let span = rendered
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "hello");
        assert!(span.is_some());
        assert_eq!(span.unwrap().style.fg, Some(ratatui::style::Color::Red));
    }

    #[test]
    fn test_render_line_higher_priority_bg_wins_independent_of_fg() {
        // High-priority span sets bg; low-priority span sets fg — both apply independently.
        let line = b"hello";
        let style_lo = ratatui::style::Style::default().fg(ratatui::style::Color::Cyan);
        let style_hi = ratatui::style::Style::default().bg(ratatui::style::Color::Red);
        let styles = vec![style_lo, style_hi];

        let mut col = MatchCollector::new(line);
        col.with_priority(0);
        col.push(0, 5, 0); // low priority, fg=Cyan
        col.with_priority(10);
        col.push(0, 5, 1); // high priority, bg=Red

        let rendered = render_line(&col, &styles);
        let span = rendered
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "hello");
        assert!(span.is_some());
        assert_eq!(span.unwrap().style.fg, Some(ratatui::style::Color::Cyan));
        assert_eq!(span.unwrap().style.bg, Some(ratatui::style::Color::Red));
    }

    #[test]
    fn test_filter_count_returns_number_of_compiled_filters() {
        let f1 = SubstringFilter::new("ERROR", FilterDecision::Include, false, 0).unwrap();
        let f2 = SubstringFilter::new("DEBUG", FilterDecision::Exclude, false, 1).unwrap();
        let fm = FilterManager::new(vec![Box::new(f1), Box::new(f2)], true);
        assert_eq!(fm.filter_count(), 2);
    }

    #[test]
    fn test_filter_count_empty() {
        let fm = FilterManager::empty();
        assert_eq!(fm.filter_count(), 0);
    }

    #[test]
    fn test_count_line_matches_independent_no_short_circuit() {
        // Both filters match — counts must increment independently even though
        // pipeline evaluation would short-circuit after the first match.
        let line = b"ERROR DEBUG both";
        let f1 = SubstringFilter::new("ERROR", FilterDecision::Include, false, 0).unwrap();
        let f2 = SubstringFilter::new("DEBUG", FilterDecision::Exclude, false, 1).unwrap();
        let fm = FilterManager::new(vec![Box::new(f1), Box::new(f2)], true);

        let counts: Vec<std::sync::atomic::AtomicUsize> = (0..2)
            .map(|_| std::sync::atomic::AtomicUsize::new(0))
            .collect();
        fm.count_line_matches(line, &counts);

        assert_eq!(counts[0].load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(counts[1].load(std::sync::atomic::Ordering::Relaxed), 1);
    }

    #[test]
    fn test_count_line_matches_only_matching_filters_increment() {
        let line = b"INFO: all good";
        let f_error = SubstringFilter::new("ERROR", FilterDecision::Include, false, 0).unwrap();
        let f_info = SubstringFilter::new("INFO", FilterDecision::Include, false, 1).unwrap();
        let fm = FilterManager::new(vec![Box::new(f_error), Box::new(f_info)], true);

        let counts: Vec<std::sync::atomic::AtomicUsize> = (0..2)
            .map(|_| std::sync::atomic::AtomicUsize::new(0))
            .collect();
        fm.count_line_matches(line, &counts);

        assert_eq!(counts[0].load(std::sync::atomic::Ordering::Relaxed), 0);
        assert_eq!(counts[1].load(std::sync::atomic::Ordering::Relaxed), 1);
    }

    #[test]
    fn test_count_line_matches_accumulates_across_lines() {
        let f = SubstringFilter::new("ERROR", FilterDecision::Include, false, 0).unwrap();
        let fm = FilterManager::new(vec![Box::new(f)], true);

        let counts: Vec<std::sync::atomic::AtomicUsize> = (0..1)
            .map(|_| std::sync::atomic::AtomicUsize::new(0))
            .collect();
        fm.count_line_matches(b"ERROR: first", &counts);
        fm.count_line_matches(b"INFO: skip", &counts);
        fm.count_line_matches(b"ERROR: second", &counts);

        assert_eq!(counts[0].load(std::sync::atomic::Ordering::Relaxed), 2);
    }

    #[test]
    fn test_substring_filter_matches_include() {
        let f = SubstringFilter::new("ERROR", FilterDecision::Include, false, 0).unwrap();
        assert_eq!(f.matches(b"ERROR: something"), FilterDecision::Include);
        assert_eq!(f.matches(b"INFO: something"), FilterDecision::Neutral);
    }

    #[test]
    fn test_substring_filter_matches_exclude() {
        let f = SubstringFilter::new("DEBUG", FilterDecision::Exclude, false, 0).unwrap();
        assert_eq!(f.matches(b"DEBUG: verbose"), FilterDecision::Exclude);
        assert_eq!(f.matches(b"INFO: important"), FilterDecision::Neutral);
    }

    #[test]
    fn test_regex_filter_matches_include() {
        let f = RegexFilter::new(r"\d{3}", FilterDecision::Include, true, 0).unwrap();
        assert_eq!(f.matches(b"status 200 OK"), FilterDecision::Include);
        assert_eq!(f.matches(b"no digits here"), FilterDecision::Neutral);
    }

    #[test]
    fn test_regex_filter_matches_exclude() {
        let f = RegexFilter::new(r"^DEBUG", FilterDecision::Exclude, false, 0).unwrap();
        assert_eq!(f.matches(b"DEBUG: noise"), FilterDecision::Exclude);
        assert_eq!(f.matches(b"INFO: keep"), FilterDecision::Neutral);
    }

    #[test]
    fn test_regex_filter_matches_invalid_utf8_returns_neutral() {
        let f = RegexFilter::new("pattern", FilterDecision::Include, false, 0).unwrap();
        assert_eq!(f.matches(b"\xff\xfe invalid"), FilterDecision::Neutral);
    }

    #[test]
    fn test_matches_consistent_with_evaluate_substring() {
        let line = b"ERROR: connection refused";
        let f = SubstringFilter::new("ERROR", FilterDecision::Include, true, 1).unwrap();
        let mut col = MatchCollector::new(line);
        let eval_decision = f.evaluate(line, &mut col);
        assert_eq!(f.matches(line), eval_decision);
    }

    #[test]
    fn test_matches_consistent_with_evaluate_regex() {
        let line = b"GET /api 200 OK";
        let f = RegexFilter::new(r"\d+", FilterDecision::Include, true, 0).unwrap();
        let mut col = MatchCollector::new(line);
        let eval_decision = f.evaluate(line, &mut col);
        assert_eq!(f.matches(line), eval_decision);
    }

    fn make_combined_fm(patterns: &[(&str, FilterDecision)], has_include: bool) -> FilterManager {
        let filters: Vec<Box<dyn Filter>> = patterns
            .iter()
            .map(|(p, d)| {
                SubstringFilter::new(p, *d, false, 0)
                    .map(|f| Box::new(f) as Box<dyn Filter>)
                    .unwrap()
            })
            .collect();
        let literal_pats: Vec<&str> = patterns.iter().map(|(p, _)| *p).collect();
        let meta: Vec<(usize, FilterDecision)> = patterns
            .iter()
            .enumerate()
            .map(|(i, (_, d))| (i, *d))
            .collect();
        let ac = AhoCorasick::builder()
            .ascii_case_insensitive(false)
            .build(&literal_pats)
            .ok();
        FilterManager::new_with_combined(filters, has_include, ac, meta, vec![])
    }

    #[test]
    fn test_combined_ac_two_include_filters_both_visible() {
        let fm = make_combined_fm(
            &[
                ("ERROR", FilterDecision::Include),
                ("WARN", FilterDecision::Include),
            ],
            true,
        );
        assert!(fm.is_visible(b"ERROR: something bad"));
        assert!(fm.is_visible(b"WARN: degraded"));
        assert!(!fm.is_visible(b"INFO: all good"));
    }

    #[test]
    fn test_combined_ac_first_match_wins_by_filter_order() {
        // Filter 0 = Include "WARN", Filter 1 = Exclude "ERROR"
        // A line matching "ERROR" should be Included because filter 0 (WARN) is checked first,
        // but "WARN ERROR" line matches both → filter 0 (idx=0) wins → Include.
        let fm = make_combined_fm(
            &[
                ("WARN", FilterDecision::Include),
                ("ERROR", FilterDecision::Exclude),
            ],
            true,
        );
        assert!(fm.is_visible(b"WARN ERROR mixed")); // filter 0 (Include) < filter 1 (Exclude)
        assert!(!fm.is_visible(b"ERROR only")); // only filter 1 matches → Exclude
        assert!(fm.is_visible(b"WARN only")); // only filter 0 matches → Include
    }

    #[test]
    fn test_combined_ac_compute_visible() {
        let (_f, reader) = make_reader(&[
            "ERROR: bad",
            "WARN: degraded",
            "INFO: ok",
            "ERROR WARN: both",
        ]);
        let fm = make_combined_fm(
            &[
                ("ERROR", FilterDecision::Include),
                ("WARN", FilterDecision::Include),
            ],
            true,
        );
        let visible = fm.compute_visible(&reader);
        assert_eq!(visible, vec![0, 1, 3]);
    }

    #[test]
    fn test_combined_ac_count_line_matches_no_double_count() {
        let fm = make_combined_fm(
            &[
                ("ERROR", FilterDecision::Include),
                ("WARN", FilterDecision::Include),
            ],
            true,
        );
        let counts: Vec<std::sync::atomic::AtomicUsize> = (0..2)
            .map(|_| std::sync::atomic::AtomicUsize::new(0))
            .collect();
        // "ERROR ERROR" — pattern "ERROR" appears twice but should count filter 0 once
        fm.count_line_matches(b"ERROR ERROR", &counts);
        assert_eq!(counts[0].load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(counts[1].load(std::sync::atomic::Ordering::Relaxed), 0);
    }

    // ── evaluate_and_count ────────────────────────────────────────────

    #[test]
    fn test_evaluate_and_count_returns_include_decision() {
        let f = SubstringFilter::new("ERROR", FilterDecision::Include, false, 0).unwrap();
        let fm = FilterManager::new(vec![Box::new(f)], true);
        let mut counts = vec![0usize];
        let dec = fm.evaluate_and_count(b"ERROR: bad", &mut counts);
        assert_eq!(dec, FilterDecision::Include);
        assert_eq!(counts[0], 1);
    }

    #[test]
    fn test_evaluate_and_count_returns_exclude_decision() {
        let f = SubstringFilter::new("DEBUG", FilterDecision::Exclude, false, 0).unwrap();
        let fm = FilterManager::new(vec![Box::new(f)], false);
        let mut counts = vec![0usize];
        let dec = fm.evaluate_and_count(b"DEBUG: noisy", &mut counts);
        assert_eq!(dec, FilterDecision::Exclude);
        assert_eq!(counts[0], 1);
    }

    #[test]
    fn test_evaluate_and_count_returns_neutral_on_no_match() {
        let f = SubstringFilter::new("ERROR", FilterDecision::Include, false, 0).unwrap();
        let fm = FilterManager::new(vec![Box::new(f)], true);
        let mut counts = vec![0usize];
        let dec = fm.evaluate_and_count(b"INFO: fine", &mut counts);
        assert_eq!(dec, FilterDecision::Neutral);
        assert_eq!(counts[0], 0);
    }

    #[test]
    fn test_evaluate_and_count_counts_all_matching_no_short_circuit() {
        let f1 = SubstringFilter::new("ERROR", FilterDecision::Include, false, 0).unwrap();
        let f2 = SubstringFilter::new("DEBUG", FilterDecision::Exclude, false, 1).unwrap();
        let fm = FilterManager::new(vec![Box::new(f1), Box::new(f2)], true);
        let mut counts = vec![0usize; 2];
        let dec = fm.evaluate_and_count(b"ERROR DEBUG both", &mut counts);
        assert_eq!(dec, FilterDecision::Include);
        assert_eq!(counts[0], 1);
        assert_eq!(counts[1], 1);
    }

    #[test]
    fn test_evaluate_and_count_first_match_wins_by_index() {
        let f1 = SubstringFilter::new("WARN", FilterDecision::Include, false, 0).unwrap();
        let f2 = SubstringFilter::new("ERROR", FilterDecision::Exclude, false, 1).unwrap();
        let fm = FilterManager::new(vec![Box::new(f1), Box::new(f2)], true);
        let mut counts = vec![0usize; 2];
        let dec = fm.evaluate_and_count(b"ERROR only", &mut counts);
        assert_eq!(dec, FilterDecision::Exclude);
        let dec2 = fm.evaluate_and_count(b"WARN ERROR both", &mut counts);
        assert_eq!(dec2, FilterDecision::Include);
    }

    #[test]
    fn test_evaluate_and_count_consistent_with_separate_calls() {
        let f1 = SubstringFilter::new("ERROR", FilterDecision::Include, false, 0).unwrap();
        let f2 = SubstringFilter::new("WARN", FilterDecision::Include, false, 1).unwrap();
        let fm = FilterManager::new(vec![Box::new(f1), Box::new(f2)], true);

        let lines: &[&[u8]] = &[
            b"ERROR: critical",
            b"WARN: degraded",
            b"INFO: fine",
            b"ERROR WARN: both",
        ];
        for line in lines {
            let mut counts_a = vec![0usize; 2];
            let counts_b: Vec<std::sync::atomic::AtomicUsize> = (0..2)
                .map(|_| std::sync::atomic::AtomicUsize::new(0))
                .collect();

            let dec_combined = fm.evaluate_and_count(line, &mut counts_a);
            fm.count_line_matches(line, &counts_b);
            let dec_separate = fm.evaluate_text(line);

            assert_eq!(
                dec_combined, dec_separate,
                "decision mismatch for {:?}",
                line
            );
            for i in 0..2 {
                assert_eq!(
                    counts_a[i],
                    counts_b[i].load(std::sync::atomic::Ordering::Relaxed),
                    "count mismatch at filter {i} for {:?}",
                    line
                );
            }
        }
    }

    #[test]
    fn test_evaluate_and_count_combined_ac_path() {
        let fm = make_combined_fm(
            &[
                ("ERROR", FilterDecision::Include),
                ("WARN", FilterDecision::Include),
            ],
            true,
        );
        let mut counts = vec![0usize; 2];
        assert_eq!(
            fm.evaluate_and_count(b"ERROR WARN line", &mut counts),
            FilterDecision::Include
        );
        assert_eq!(counts[0], 1);
        assert_eq!(counts[1], 1);
    }

    #[test]
    fn test_evaluate_and_count_no_double_count_repeated_pattern() {
        let fm = make_combined_fm(&[("ERROR", FilterDecision::Include)], true);
        let mut counts = vec![0usize];
        fm.evaluate_and_count(b"ERROR ERROR ERROR", &mut counts);
        assert_eq!(counts[0], 1);
    }

    #[test]
    fn test_combined_ac_with_regex_fallback() {
        let f_lit = SubstringFilter::new("ERROR", FilterDecision::Include, false, 0)
            .map(|f| Box::new(f) as Box<dyn Filter>)
            .unwrap();
        let f_re = RegexFilter::new(r"\d{3}", FilterDecision::Include, false, 1)
            .map(|f| Box::new(f) as Box<dyn Filter>)
            .unwrap();
        let ac = AhoCorasick::builder()
            .ascii_case_insensitive(false)
            .build(["ERROR"])
            .ok();
        let meta = vec![(0, FilterDecision::Include)];
        let fm = FilterManager::new_with_combined(
            vec![f_lit, f_re],
            true,
            ac,
            meta,
            vec![1], // regex filter at index 1
        );
        assert!(fm.is_visible(b"ERROR: bad"));
        assert!(fm.is_visible(b"status 200 OK"));
        assert!(!fm.is_visible(b"INFO: plain"));
    }

    // ── evaluate_chunk_wholefile ─────────────────────────────────────

    fn make_wholefile_data(lines: &[&str]) -> (Vec<u8>, Vec<usize>) {
        let mut data = Vec::new();
        let mut starts = vec![0usize];
        for line in lines {
            data.extend_from_slice(line.as_bytes());
            data.push(b'\n');
            starts.push(data.len());
        }
        (data, starts)
    }

    #[test]
    fn test_wholefile_include_filters_visible() {
        let fm = make_combined_fm(
            &[
                ("ERROR", FilterDecision::Include),
                ("WARN", FilterDecision::Include),
            ],
            true,
        );
        let (data, starts) =
            make_wholefile_data(&["ERROR: bad", "INFO: ok", "WARN: degraded", "DEBUG: verbose"]);
        let (visible, counts) = fm.evaluate_chunk_wholefile(&data, &starts, 0..4);
        assert_eq!(visible, vec![0, 2]);
        assert_eq!(counts[0], 1, "ERROR filter count");
        assert_eq!(counts[1], 1, "WARN filter count");
    }

    #[test]
    fn test_wholefile_exclude_filter() {
        let fm = make_combined_fm(&[("DEBUG", FilterDecision::Exclude)], false);
        let (data, starts) = make_wholefile_data(&["ERROR: bad", "DEBUG: noisy", "INFO: ok"]);
        let (visible, counts) = fm.evaluate_chunk_wholefile(&data, &starts, 0..3);
        assert_eq!(visible, vec![0, 2]);
        assert_eq!(counts[0], 1, "DEBUG matched once");
    }

    #[test]
    fn test_wholefile_no_double_count_repeated_pattern() {
        let fm = make_combined_fm(&[("ERROR", FilterDecision::Include)], true);
        let (data, starts) = make_wholefile_data(&["ERROR ERROR ERROR"]);
        let (visible, counts) = fm.evaluate_chunk_wholefile(&data, &starts, 0..1);
        assert_eq!(visible, vec![0]);
        assert_eq!(counts[0], 1, "must count line once despite 3 matches");
    }

    #[test]
    fn test_wholefile_first_match_wins_by_filter_order() {
        let fm = make_combined_fm(
            &[
                ("WARN", FilterDecision::Include),
                ("ERROR", FilterDecision::Exclude),
            ],
            true,
        );
        let (data, starts) = make_wholefile_data(&["WARN ERROR mixed", "ERROR only", "WARN only"]);
        let (visible, _) = fm.evaluate_chunk_wholefile(&data, &starts, 0..3);
        assert_eq!(
            visible,
            vec![0, 2],
            "line 0: Include wins; line 1: Exclude; line 2: Include"
        );
    }

    #[test]
    fn test_wholefile_sub_range() {
        let fm = make_combined_fm(&[("ERROR", FilterDecision::Include)], true);
        let (data, starts) = make_wholefile_data(&[
            "ERROR: first",
            "INFO: skip",
            "ERROR: second",
            "INFO: also skip",
        ]);
        let (visible, counts) = fm.evaluate_chunk_wholefile(&data, &starts, 1..3);
        assert_eq!(visible, vec![2], "only line 2 within range 1..3 matches");
        assert_eq!(counts[0], 1);
    }

    #[test]
    fn test_wholefile_empty_range() {
        let fm = make_combined_fm(&[("ERROR", FilterDecision::Include)], true);
        let (data, starts) = make_wholefile_data(&["ERROR: line"]);
        let (visible, counts) = fm.evaluate_chunk_wholefile(&data, &starts, 0..0);
        assert!(visible.is_empty());
        assert_eq!(counts[0], 0);
    }

    #[test]
    fn test_wholefile_consistent_with_per_line() {
        let fm = make_combined_fm(
            &[
                ("ERROR", FilterDecision::Include),
                ("WARN", FilterDecision::Include),
                ("DEBUG", FilterDecision::Exclude),
            ],
            true,
        );
        let lines = [
            "ERROR: critical",
            "WARN: degraded",
            "INFO: fine",
            "DEBUG: noisy",
            "ERROR WARN: both",
        ];
        let (data, starts) = make_wholefile_data(&lines);

        // Whole-file path
        let (wf_visible, wf_counts) = fm.evaluate_chunk_wholefile(&data, &starts, 0..5);

        // Per-line path
        let mut pl_counts = vec![0usize; 3];
        let mut pl_visible = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            let dec = fm.evaluate_and_count(line.as_bytes(), &mut pl_counts);
            let vis = match dec {
                FilterDecision::Include => true,
                FilterDecision::Exclude => false,
                FilterDecision::Neutral => !fm.has_include(),
            };
            if vis {
                pl_visible.push(i);
            }
        }

        assert_eq!(wf_visible, pl_visible, "visibility must match per-line");
        assert_eq!(wf_counts, pl_counts, "counts must match per-line");
    }

    #[test]
    fn test_wholefile_with_regex_fallback() {
        let f_lit = SubstringFilter::new("ERROR", FilterDecision::Include, false, 0)
            .map(|f| Box::new(f) as Box<dyn Filter>)
            .unwrap();
        let f_re = RegexFilter::new(r"\d{3}", FilterDecision::Include, false, 1)
            .map(|f| Box::new(f) as Box<dyn Filter>)
            .unwrap();
        let ac = AhoCorasick::builder()
            .ascii_case_insensitive(false)
            .build(["ERROR"])
            .ok();
        let meta = vec![(0, FilterDecision::Include)];
        let fm = FilterManager::new_with_combined(vec![f_lit, f_re], true, ac, meta, vec![1]);
        let (data, starts) = make_wholefile_data(&["ERROR: bad", "status 200 OK", "INFO: plain"]);
        let (visible, counts) = fm.evaluate_chunk_wholefile(&data, &starts, 0..3);
        assert_eq!(visible, vec![0, 1]);
        assert_eq!(counts[0], 1, "ERROR filter");
        assert_eq!(counts[1], 1, "regex digit filter");
    }
}
