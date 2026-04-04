use rayon::iter::IndexedParallelIterator;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{mpsc, watch};

use crate::db::FileContext;
use crate::db::LogManager;
use crate::filters::{FieldVote, any_field_exclude_matches, field_include_vote};
use crate::filters::{FilterDecision, FilterManager};
use crate::ingestion::FileReader;
use crate::mode::normal_mode::NormalMode;
use crate::parser::{LogFormatParser, detect_format};
use crate::search::Search;
use crate::search::SearchResult;
use crate::ui::FieldLayout;

pub mod cache_state;
pub mod display_config;
pub mod filter_state;
pub mod interaction_state;
pub mod merged;
pub mod scroll_state;
pub mod search_state;
pub mod stream_state;
pub mod year_map;

pub use cache_state::CacheState;
pub use display_config::{DisplayConfig, SidebarSide};
pub use filter_state::{FilterState, FilterViewSnapshot};
pub use interaction_state::InteractionState;
pub use merged::MergedState;
pub use scroll_state::ScrollState;
pub use search_state::SearchState;
pub use stream_state::StreamState;

#[derive(Debug)]
pub enum KeyResult {
    Handled,
    Ignored,
    ExecuteCommand(String),
    RestoreSession(Vec<String>),
    DockerAttach(String, String),
    DltAttach(String, u16, String),
    ApplyValueColors(std::collections::HashSet<String>),
    ApplyLevelColors(std::collections::HashSet<String>),
    CopyToClipboard(String),
    OpenFiles(Vec<String>),
    ToggleModeBar,
    ToggleSidebar,
    ToggleBorders,
    ToggleWrap,
    ToggleLineNumbers,
    AlwaysRestoreFile(Box<crate::db::FileContext>),
    NeverRestoreFile,
    AlwaysRestoreSession(Vec<String>),
    NeverRestoreSession,
    OpenMergeSelect,
    OpenMergedView { source_tab_indices: Vec<usize> },
}

/// Handle for a background search task spawned by [`TabState::begin_search`].
pub struct SearchHandle {
    /// Receives incremental batches of results. Channel closes when scan is done.
    pub result_rx: mpsc::Receiver<Vec<SearchResult>>,
    /// Set to `true` to cancel the in-flight search early.
    pub cancel: Arc<AtomicBool>,
    /// Live fraction-complete (0.0–1.0) updated as lines are scanned.
    pub progress_rx: watch::Receiver<f64>,
    /// Pattern string shown in the "Searching…" status bar.
    pub pattern: String,
    pub forward: bool,
    /// When `true`, scroll to the first match once results arrive.
    pub navigate: bool,
}

/// A streaming chunk delivered by the background filter task.
pub struct FilterChunk {
    /// Visible line indices for this chunk of the file.
    pub visible: Vec<usize>,
    /// Per-filter match counts unified across all filter types; `Some` only on the last chunk.
    pub filter_match_counts: Option<Vec<usize>>,
    /// `true` when this is the final chunk (no more will be sent).
    pub is_last: bool,
    /// Fraction of the file processed when this chunk was produced (0.0–1.0).
    pub progress: f64,
}

/// Handle for a background filter computation task spawned by
/// [`TabState::begin_filter_refresh`].
pub struct FilterHandle {
    /// Receives streaming chunks of visible indices.
    pub result_rx: mpsc::Receiver<FilterChunk>,
    /// Set to `true` to abort the in-flight computation early.
    pub cancel: Arc<AtomicBool>,
    /// Fraction-complete (0.0–1.0) of the last applied chunk, used for the progress bar.
    pub displayed_progress: f64,
    /// File-line index to restore scroll position to when the result arrives.
    pub scroll_anchor: Option<usize>,
    /// `true` after the first chunk has been applied to `visible_indices`.
    pub received_first_chunk: bool,
    /// Enabled filter snapshot captured at scan-start; stored here so
    /// `advance_filter_computation` can persist a `CachedScanResult` on completion.
    pub scan_fingerprint: Vec<crate::filters::FilterDef>,
    /// File line count at scan-start; part of the cache key.
    pub scan_line_count: usize,
    /// `raw_mode` value at scan-start; part of the cache key.
    pub scan_raw_mode: bool,
}

/// Cached result of a completed background filter scan.
/// Keyed by the set of enabled filters, file line count, and raw-mode flag at scan time.
/// Used by [`TabState::begin_filter_refresh`] to skip a redundant re-scan when the
/// filter state is toggled off and then back on without any changes in between.
pub struct CachedScanResult {
    pub filter_fingerprint: Vec<crate::filters::FilterDef>,
    pub line_count: usize,
    pub raw_mode: bool,
    pub view: FilterViewSnapshot,
    pub match_counts: Vec<usize>,
}

/// Merge three compacted per-type count vectors into a single `Vec<usize>` of length
/// `filters.len()`, indexed by position in `filter_defs`. Disabled filters get count 0.
pub fn merge_filter_counts(
    filters: &[crate::filters::FilterDef],
    text: &[usize],
    field: &[usize],
    date: &[usize],
) -> Vec<usize> {
    let mut out = vec![0; filters.len()];
    let (mut ti, mut fi, mut di) = (0, 0, 0);
    for (i, f) in filters.iter().enumerate() {
        if !f.enabled {
            continue;
        }
        if f.pattern.starts_with(crate::filters::DATE_PREFIX) {
            out[i] = date.get(di).copied().unwrap_or(0);
            di += 1;
        } else if f.pattern.starts_with(crate::filters::FIELD_PREFIX) {
            out[i] = field.get(fi).copied().unwrap_or(0);
            fi += 1;
        } else {
            out[i] = text.get(ti).copied().unwrap_or(0);
            ti += 1;
        }
    }
    out
}

/// Decide whether a single log line should be visible given the full set of active filters.
///
/// Accepts a pre-computed `text_dec` (from [`FilterManager::evaluate_and_count`] or
/// [`FilterManager::evaluate_text`]) and pre-parsed `parts` so both can be produced once
/// by the caller and reused here without a second scan or parse.
///
/// Text filters and field filters are combined with **OR** semantics for includes.
/// Exclude filters from either source hide the line unconditionally.
/// Date filters act as strict AND constraints on the timestamp field.
///
/// Pass-through rules (field filters only):
/// - If the line cannot be parsed (e.g. a stack-trace continuation) → field filters do not apply.
/// - If the line was parsed but the named field is absent → treated as Miss (hidden).
#[allow(clippy::too_many_arguments)]
pub fn line_is_visible(
    text_dec: FilterDecision,
    has_text_includes: bool,
    date_filters: &[crate::filters::DateFilter],
    date_counts: &mut [usize],
    inc_ff: &[crate::filters::FieldFilter],
    exc_ff: &[crate::filters::FieldFilter],
    parts: Option<&crate::parser::DisplayParts<'_>>,
    year_override: Option<i32>,
) -> bool {
    // Step 1: text filter result — fast path.
    if text_dec == FilterDecision::Exclude {
        return false;
    }

    // Step 2: date filter — AND constraint; count and check visibility in one pass.
    if !date_filters.is_empty()
        && let Some(ts) = parts.and_then(|p| p.timestamp)
    {
        let mut any_date_match = false;
        for (df, count) in date_filters.iter().zip(date_counts.iter_mut()) {
            if df.matches(ts, year_override) {
                *count += 1;
                any_date_match = true;
            }
        }
        if !any_date_match {
            return false;
        }
    }

    // Step 3: field exclude — hides the line if any matching exclude is found.
    if any_field_exclude_matches(exc_ff, parts) {
        return false;
    }

    // Step 4: include resolution — text include OR field include.
    if text_dec == FilterDecision::Include {
        return true;
    }

    // text_dec is Neutral; check field includes.
    if !inc_ff.is_empty() {
        return match field_include_vote(inc_ff, parts) {
            FieldVote::Match => true,
            FieldVote::Miss => false,
            // Pass-through: field filters don't apply to this line; fall back to
            // text-filter-only logic (visible iff there are no text include filters).
            FieldVote::PassThrough => !has_text_includes,
        };
    }

    // No field includes; visible iff no text include filters exist.
    !has_text_includes
}

/// Efficient representation of which file lines are currently visible.
///
/// `All(n)` covers the common no-filter case: every index `i` maps to itself,
/// so no allocation is needed. `Filtered` holds the explicit subset produced
/// by the filter pipeline or marks-only view.
#[derive(Clone, Debug, PartialEq)]
pub enum VisibleLines {
    /// All N file lines are visible; `visible[i] == i` for any `i < n`.
    All(usize),
    /// An explicit, sorted subset of file-line indices.
    Filtered(Vec<usize>),
}

impl VisibleLines {
    pub fn len(&self) -> usize {
        match self {
            Self::All(n) => *n,
            Self::Filtered(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// File-line index at visible position `pos`. Panics if out of bounds.
    pub fn get(&self, pos: usize) -> usize {
        match self {
            Self::All(_) => pos,
            Self::Filtered(v) => v[pos],
        }
    }

    /// File-line index at visible position `pos`, or `None` if out of bounds.
    pub fn get_opt(&self, pos: usize) -> Option<usize> {
        match self {
            Self::All(n) => {
                if pos < *n {
                    Some(pos)
                } else {
                    None
                }
            }
            Self::Filtered(v) => v.get(pos).copied(),
        }
    }

    /// Visible position of file-line `line_idx`, or `None` if not visible.
    pub fn position_of(&self, line_idx: usize) -> Option<usize> {
        match self {
            Self::All(n) => {
                if line_idx < *n {
                    Some(line_idx)
                } else {
                    None
                }
            }
            Self::Filtered(v) => v.binary_search(&line_idx).ok(),
        }
    }

    /// Visible position of the nearest visible line to `line_idx`.
    /// Returns `None` only when the visible set is empty.
    pub fn nearest_position_of(&self, line_idx: usize) -> Option<usize> {
        if self.is_empty() {
            return None;
        }
        Some(match self.binary_search(line_idx) {
            Ok(pos) => pos,
            Err(insert_pos) => {
                let before = if insert_pos > 0 {
                    Some(insert_pos - 1)
                } else {
                    None
                };
                let after = if insert_pos < self.len() {
                    Some(insert_pos)
                } else {
                    None
                };
                match (before, after) {
                    (Some(b), Some(a)) => {
                        let dist_b = line_idx - self.get(b);
                        let dist_a = self.get(a) - line_idx;
                        if dist_b <= dist_a { b } else { a }
                    }
                    (Some(b), None) => b,
                    (None, Some(a)) => a,
                    (None, None) => unreachable!(),
                }
            }
        })
    }

    /// Iterate file-line indices for all visible positions in order.
    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        let len = self.len();
        (0..len).map(move |i| self.get(i))
    }

    /// Returns `true` if file-line `idx` is in the visible set.
    pub fn contains(&self, idx: usize) -> bool {
        match self {
            Self::All(n) => idx < *n,
            Self::Filtered(v) => v.binary_search(&idx).is_ok(),
        }
    }

    /// Binary search for file-line index `target`.
    /// Returns `Ok(pos)` if found, `Err(insert_pos)` otherwise.
    pub fn binary_search(&self, target: usize) -> Result<usize, usize> {
        match self {
            Self::All(n) => {
                if target < *n {
                    Ok(target)
                } else {
                    Err(*n)
                }
            }
            Self::Filtered(v) => v.binary_search(&target),
        }
    }

    /// Retain only positions where `f(file_line_idx)` is true.
    /// Converts `All` to `Filtered` when any line is removed.
    pub fn retain(&mut self, mut f: impl FnMut(usize) -> bool) {
        match self {
            Self::All(n) => {
                let filtered: Vec<usize> = (0..*n).filter(|&i| f(i)).collect();
                *self = Self::Filtered(filtered);
            }
            Self::Filtered(v) => v.retain(|&i| f(i)),
        }
    }

    /// Collect file-line indices for visible positions `lo..=hi` into a `Vec`.
    pub fn slice_to_vec(&self, lo: usize, hi: usize) -> Vec<usize> {
        (lo..=hi).map(|i| self.get(i)).collect()
    }
}

impl Default for VisibleLines {
    fn default() -> Self {
        Self::All(0)
    }
}

/// Cached output of parsing and rendering a structured log line.
/// Keyed by file-line index; invalidated by incrementing `TabState::parse_cache_gen`.
pub struct CachedParsedLine {
    /// `apply_field_layout` columns joined with spaces; empty string when all cols are hidden.
    pub rendered: String,
    /// Parsed level string (e.g. `"INFO"`) for level-colour lookup.
    pub level: Option<String>,
    /// Parsed timestamp string for date-filter highlighting.
    pub timestamp: Option<String>,
    /// Parsed target string for process-colour assignment.
    pub target: Option<String>,
    /// Value of the `pid` extra field, for process-colour pairing.
    pub pid: Option<String>,
    /// True when `apply_field_layout` returned an empty Vec (all columns hidden).
    pub all_cols_hidden: bool,
    /// Byte offset of `target` within `rendered`; avoids repeated `str::find` on render misses.
    pub target_offset: Option<usize>,
    /// Byte offset of `pid` within `rendered`; avoids repeated `str::find` on render misses.
    pub pid_offset: Option<usize>,
    /// Byte offset of `timestamp` within `rendered`; avoids repeated `str::find` on render misses.
    pub timestamp_offset: Option<usize>,
}

pub fn display_text_for_line(
    line_idx: usize,
    file_reader: &FileReader,
    detected_format: &Option<Arc<dyn LogFormatParser>>,
    field_layout: &FieldLayout,
    hidden_fields: &HashSet<String>,
    show_keys: bool,
) -> String {
    let bytes = file_reader.get_line(line_idx);
    if let Some(parser) = detected_format
        && let Some(parts) = parser.parse_line(bytes)
    {
        let cols = super::field_layout::apply_field_layout(
            &parts,
            field_layout,
            hidden_fields,
            show_keys,
            None,
        );
        if !cols.is_empty() {
            return cols.join(" ");
        }
    }
    String::from_utf8_lossy(bytes).into_owned()
}

pub fn build_continuation_map(reader: &FileReader, parser: &dyn LogFormatParser) -> Vec<usize> {
    let count = reader.line_count();
    let chunk_size = 1024;

    // Phase 1: process chunks in parallel
    let mut chunks: Vec<(Vec<usize>, usize)> = (0..count)
        .into_par_iter()
        .step_by(chunk_size)
        .map(|start| {
            let end = (start + chunk_size).min(count);
            let mut local = Vec::with_capacity(end - start);
            let mut last_parent = start;

            for i in start..end {
                let line = reader.get_line(i);
                if !line.is_empty() && parser.parse_line(line).is_some() {
                    last_parent = i;
                }
                local.push(last_parent);
            }

            (local, last_parent)
        })
        .collect();

    // Phase 2: fix dependencies between chunks
    let mut result = Vec::with_capacity(count);
    let mut prev_last = 0;

    for (chunk, last) in chunks.iter_mut() {
        for v in chunk.iter_mut() {
            if *v < prev_last {
                *v = prev_last;
            }
        }
        prev_last = (*last).max(prev_last);
        result.extend_from_slice(chunk);
    }

    result
}

/// `has_include_filters` controls how continuation lines absent from the filter
/// result are treated:
/// - `false` (exclude-only): a line absent from `visible` was **explicitly
///   excluded** — keep it hidden even when the parent is visible.
/// - `true` (include filters exist): absence may mean the line simply didn't
///   match an include pattern (not an explicit exclude), so the continuation
///   still follows its parent's visibility to preserve stack-trace grouping.
pub fn apply_continuation_correction(
    visible: &mut VisibleLines,
    cmap: &[usize],
    has_include_filters: bool,
) {
    let indices = match visible {
        VisibleLines::All(_) => return,
        VisibleLines::Filtered(v) => v,
    };

    let n = cmap.len();

    let mut filter_visible = vec![0u8; n];
    for &idx in indices.iter() {
        filter_visible[idx] = 1;
    }

    // Each line is visible iff its parent is visible AND
    // (the line itself was not explicitly excluded OR include filters exist).
    // For parent lines (i == parent) this reduces to filter_visible[i] unchanged.
    let new_indices: Vec<usize> = (0..n)
        .into_par_iter()
        .filter(|&i| {
            filter_visible[cmap[i]] != 0 && (filter_visible[i] != 0 || has_include_filters)
        })
        .collect();

    *indices = new_indices;
}

pub struct TabState {
    pub file_reader: FileReader,
    pub log_manager: LogManager,
    pub title: String,
    pub scroll: ScrollState,
    pub filter: FilterState,
    pub search: SearchState,
    pub cache: CacheState,
    pub stream: StreamState,
    pub display: DisplayConfig,
    pub interaction: InteractionState,
    pub load_state: Option<FileLoadState>,
    /// Keeps extracted archive temp file alive for the lifetime of this tab.
    pub archive_temp: Option<tempfile::NamedTempFile>,
    /// Some(fraction 0.0–1.0) while this tab's content is being extracted from an archive.
    /// None when waiting for its turn, or after extraction completes.
    pub extraction_progress: Option<f64>,
    /// Maps each line index to the nearest preceding line index (inclusive)
    /// that the log-format parser recognised as an entry start.  `None` when
    /// no format has been detected or raw-mode is active.
    pub continuation_map: Option<Arc<Vec<usize>>>,
    /// Year map for BSD-format timestamps (syslog, journalctl).  `None` when
    /// no BSD-format timestamps were detected.
    pub year_map: Option<Arc<year_map::YearMap>>,
    /// State for a merged (interleaved) view tab.  `None` for regular tabs.
    pub merged: Option<merged::MergedState>,
}

impl TabState {
    pub fn new(file_reader: FileReader, log_manager: LogManager, title: String) -> Self {
        // Sample up to 200 lines for format detection.
        let sample_limit = file_reader.line_count().min(200);
        let sample: Vec<&[u8]> = (0..sample_limit).map(|i| file_reader.get_line(i)).collect();
        let detected_format = detect_format(&sample).map(Arc::from);

        // Apply format-specific default hidden fields (e.g. journalctl JSON hides
        // systemd-internal fields that are not visible in short output mode).
        let default_hidden: HashSet<String> = detected_format
            .as_deref()
            .map(|fmt: &dyn LogFormatParser| fmt.default_hidden_fields(&sample))
            .unwrap_or_default();
        let fields_hidden_by_default = !default_hidden.is_empty();

        let continuation_map = detected_format
            .as_deref()
            .map(|p| Arc::new(build_continuation_map(&file_reader, p)));

        let year_map = detected_format.as_deref().and_then(|p| {
            if p.timestamp_has_year() {
                return None;
            }
            use crate::filters::system_time_to_date;
            let start_year = system_time_to_date(file_reader.mtime())
                .map(|d| d.year())
                .unwrap_or_else(|| time::OffsetDateTime::now_utc().year());
            Some(Arc::new(year_map::YearMap::build(
                &file_reader,
                p,
                start_year,
            )))
        });

        let mut tab = TabState {
            file_reader,
            log_manager,
            title,
            scroll: ScrollState::default(),
            filter: FilterState::default(),
            search: SearchState::default(),
            cache: CacheState::default(),
            stream: StreamState::default(),
            display: DisplayConfig {
                format: detected_format,
                hidden_fields: default_hidden,
                level_colors_disabled: ["trace", "debug", "info", "notice"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
                ..DisplayConfig::default()
            },
            interaction: InteractionState {
                notification: if fields_hidden_by_default {
                    Some(
                        "Some fields are hidden. Use 'select-fields' to choose fields \
                         or 'show-all-fields' to show all."
                            .to_string(),
                    )
                } else {
                    None
                },
                notification_set_at: if fields_hidden_by_default {
                    Some(std::time::Instant::now())
                } else {
                    None
                },
                ..InteractionState::default()
            },
            load_state: None,
            archive_temp: None,
            extraction_progress: None,
            continuation_map,
            year_map,
            merged: None,
        };
        tab.refresh_visible();
        tab
    }

    /// Show a transient notification bar message (auto-dismisses after 10s or on Esc).
    pub fn set_notification(&mut self, msg: impl Into<String>) {
        self.interaction.notification = Some(msg.into());
        self.interaction.notification_set_at = Some(std::time::Instant::now());
    }

    /// Clear the transient notification.
    pub fn clear_notification(&mut self) {
        self.interaction.notification = None;
        self.interaction.notification_set_at = None;
    }

    /// Recompute which file lines are visible under the current filters.
    pub fn refresh_visible(&mut self) {
        self.refresh_visible_inner();
    }

    /// Compute the visible-line set when no text/date/field filters are active.
    /// When a structured format is detected, empty lines (e.g. SSE delimiters)
    /// are excluded; otherwise all lines are visible.
    fn compute_unfiltered_visible(&self) -> VisibleLines {
        let n = self.file_reader.line_count();
        if self.display.format.is_none() || self.display.raw_mode {
            return VisibleLines::All(n);
        }
        // Fast path: scan for the first blank line.  If none exist (the common
        // case for log files) return All without allocating a Vec at all.
        let first_empty = (0..n).find(|&i| self.file_reader.get_line(i).is_empty());
        let Some(first_empty) = first_empty else {
            return VisibleLines::All(n);
        };
        // Slow path: at least one blank line — collect non-empty indices.
        let mut vis = Vec::with_capacity(n - 1);
        for i in 0..n {
            if i != first_empty && !self.file_reader.get_line(i).is_empty() {
                vis.push(i);
            }
        }
        VisibleLines::Filtered(vis)
    }

    /// Scan forward from `from` (exclusive) for the next visible ERROR/FATAL line.
    pub fn next_error_position(&self, from: usize) -> Option<usize> {
        self.scan_level_forward(from, true)
    }

    /// Scan backward from `from` (exclusive) for the previous visible ERROR/FATAL line.
    pub fn prev_error_position(&self, from: usize) -> Option<usize> {
        self.scan_level_backward(from, true)
    }

    /// Scan forward from `from` (exclusive) for the next visible WARNING line.
    pub fn next_warning_position(&self, from: usize) -> Option<usize> {
        self.scan_level_forward(from, false)
    }

    /// Scan backward from `from` (exclusive) for the previous visible WARNING line.
    pub fn prev_warning_position(&self, from: usize) -> Option<usize> {
        self.scan_level_backward(from, false)
    }

    fn scan_level_forward(&self, from: usize, errors: bool) -> Option<usize> {
        let parser: Option<&dyn LogFormatParser> = if self.display.raw_mode {
            None
        } else {
            self.display.format.as_deref()
        };
        let len = self.filter.visible_indices.len();
        (from.saturating_add(1)..len).find(|&pos| self.pos_matches_level(pos, parser, errors))
    }

    fn scan_level_backward(&self, from: usize, errors: bool) -> Option<usize> {
        let parser: Option<&dyn LogFormatParser> = if self.display.raw_mode {
            None
        } else {
            self.display.format.as_deref()
        };
        (0..from)
            .rev()
            .find(|&pos| self.pos_matches_level(pos, parser, errors))
    }

    fn pos_matches_level(
        &self,
        pos: usize,
        parser: Option<&dyn LogFormatParser>,
        errors: bool,
    ) -> bool {
        use crate::parser::LogLevel;
        let file_idx = self.filter.visible_indices.get(pos);
        let bytes = self.file_reader.get_line(file_idx);
        let level = parser
            .and_then(|p| p.parse_line(bytes))
            .and_then(|parts| parts.level)
            .map(LogLevel::parse_level)
            .unwrap_or_else(|| LogLevel::detect_from_bytes(bytes));
        if errors {
            matches!(level, LogLevel::Error | LogLevel::Fatal)
        } else {
            matches!(level, LogLevel::Warning)
        }
    }

    fn refresh_visible_inner(&mut self) {
        // Short-circuit: with no active filters and not in marks-only mode the
        // visible set is always All(n), regardless of `filtering_enabled`.
        // Skipping cache invalidation avoids reprocessing every line needlessly.
        let has_active_filters =
            self.filter.show_marks_only || self.log_manager.get_filters().iter().any(|f| f.enabled);
        if !has_active_filters {
            let current_line = if self.filter.saved_view.is_some() {
                self.filter
                    .visible_indices
                    .get_opt(self.scroll.scroll_offset)
            } else {
                None
            };
            self.filter.saved_view = None;
            self.filter.visible_indices = self.compute_unfiltered_visible();
            self.filter.manager = Arc::new(FilterManager::empty());
            self.filter.text_styles = Vec::new();
            self.filter.date_styles = Vec::new();
            self.filter.field_styles = Vec::new();
            self.filter.match_counts = Vec::new();
            self.restore_scroll_to_line(current_line);
            return;
        }

        self.invalidate_parse_cache();

        let current_line = self
            .filter
            .visible_indices
            .get_opt(self.scroll.scroll_offset);

        if self.filter.show_marks_only {
            // Save the pre-marks-only filter view so we can restore it in O(1) on toggle-off,
            // avoiding a full O(file_size) compute_visible scan.
            // If saved_filter_view is already Some, a filter change fired while we were already
            // in marks-only mode — the saved view is now stale, so discard it.
            if self.filter.saved_view.is_none() {
                self.filter.saved_view = Some((
                    self.filter.visible_indices.clone(),
                    self.filter.manager.clone(),
                    self.filter.text_styles.clone(),
                    self.filter.date_styles.clone(),
                    self.filter.field_styles.clone(),
                ));
            } else {
                self.filter.saved_view = None;
            }
            let mut indices = self.log_manager.get_marked_indices();
            indices.retain(|&i| i < self.file_reader.line_count());
            self.filter.visible_indices = VisibleLines::Filtered(indices);
            self.rebuild_filter_manager_cache();
            self.filter.match_counts = Vec::new();
        } else if let Some((
            saved_visible,
            saved_fm,
            saved_styles,
            saved_date_styles,
            saved_field_styles,
        )) = self.filter.saved_view.take()
        {
            self.filter.visible_indices = saved_visible;
            self.filter.manager = saved_fm;
            self.filter.text_styles = saved_styles;
            self.filter.date_styles = saved_date_styles;
            self.filter.field_styles = saved_field_styles;
        } else if !self.filter.enabled {
            self.filter.visible_indices = VisibleLines::All(self.file_reader.line_count());
            self.filter.manager = Arc::new(FilterManager::empty());
            self.filter.text_styles = Vec::new();
            self.filter.date_styles = Vec::new();
            self.filter.field_styles = Vec::new();
            self.filter.match_counts = Vec::new();
        } else {
            // Unified single-pass: text + date + field filters evaluated together
            // so that include filters (text and field) combine with OR semantics.
            let (fm, styles, date_filter_styles, field_filter_styles) =
                self.log_manager.build_filter_manager();
            let date_filters = crate::filters::extract_date_filters(self.log_manager.get_filters());
            let (inc_ff, exc_ff) =
                crate::filters::extract_field_filters(self.log_manager.get_filters());
            let field_defs =
                crate::filters::extract_field_filters_ordered(self.log_manager.get_filters());
            let all_filter_defs = self.log_manager.get_filters().to_vec();
            let parser = self.display.format.as_deref();
            let field_layout = &self.display.field_layout;
            let hidden_fields = &self.display.hidden_fields;
            let show_keys = self.display.show_keys;
            use rayon::prelude::*;
            let file_reader = &self.file_reader;
            let year_map = self.year_map.as_deref();
            let n_text = fm.filter_count();
            let n_field = field_defs.len();
            let n_date = date_filters.len();
            let has_text_includes = fm.has_include();
            let synthetic_level = parser.is_some_and(|p| p.has_synthetic_level()) && n_text > 0;
            let needs_parse = !date_filters.is_empty()
                || !field_defs.is_empty()
                || !inc_ff.is_empty()
                || !exc_ff.is_empty()
                || synthetic_level;
            let date_only = !date_filters.is_empty()
                && inc_ff.is_empty()
                && exc_ff.is_empty()
                && !synthetic_level;
            let line_count = self.file_reader.line_count();

            // Choose scan strategy: whole-file AC when text-only filters and
            // combined AC available.
            let use_wholefile = !needs_parse && fm.has_combined_ac();

            #[cfg(unix)]
            file_reader.advise_for_scan(0..line_count);

            let (visible, text_counts, field_counts, date_counts) = if use_wholefile {
                let (vis, tc) = fm.evaluate_chunk_wholefile(
                    file_reader.data(),
                    file_reader.line_starts(),
                    0..line_count,
                );
                (vis, tc, vec![0usize; n_field], vec![0usize; n_date])
            } else {
                (0..line_count)
                    .into_par_iter()
                    .with_min_len(1024)
                    .fold(
                        || {
                            (
                                Vec::new(),
                                vec![0usize; n_text],
                                vec![0usize; n_field],
                                vec![0usize; n_date],
                            )
                        },
                        |(mut vis, mut tc, mut fc, mut dc), idx| {
                            let line = file_reader.get_line(idx);
                            if parser.is_some() && line.is_empty() {
                                return (vis, tc, fc, dc);
                            }
                            let year_override = year_map.map(|ym| ym.year_for_line(idx));
                            let mut text_dec = fm.evaluate_and_count(line, &mut tc);
                            let can_skip = text_dec == FilterDecision::Exclude
                                || (text_dec == FilterDecision::Neutral
                                    && has_text_includes
                                    && inc_ff.is_empty()
                                    && !synthetic_level);
                            let visible = if date_only && !can_skip {
                                parser
                                    .and_then(|p| p.parse_timestamp(line))
                                    .map(|ts| {
                                        let mut any = false;
                                        for (df, cnt) in date_filters.iter().zip(dc.iter_mut()) {
                                            if df.matches(ts, year_override) {
                                                *cnt += 1;
                                                any = true;
                                            }
                                        }
                                        any
                                    })
                                    .unwrap_or(true)
                            } else {
                                let parts = if needs_parse && !can_skip {
                                    parser.and_then(|p| p.parse_line(line))
                                } else {
                                    None
                                };
                                if text_dec == FilterDecision::Neutral
                                    && synthetic_level
                                    && let Some(p) = parts.as_ref()
                                {
                                    let display = crate::ui::field_layout::apply_field_layout(
                                        p,
                                        field_layout,
                                        hidden_fields,
                                        show_keys,
                                        None,
                                    )
                                    .join(" ");
                                    let dec = fm.evaluate_and_count(display.as_bytes(), &mut tc);
                                    if dec != FilterDecision::Neutral {
                                        text_dec = dec;
                                    }
                                }
                                if !field_defs.is_empty() {
                                    crate::filters::count_field_filter_matches(
                                        &field_defs,
                                        parts.as_ref(),
                                        &mut fc,
                                    );
                                }
                                line_is_visible(
                                    text_dec,
                                    has_text_includes,
                                    &date_filters,
                                    &mut dc,
                                    &inc_ff,
                                    &exc_ff,
                                    parts.as_ref(),
                                    year_override,
                                )
                            };
                            if visible {
                                vis.push(idx);
                            }
                            (vis, tc, fc, dc)
                        },
                    )
                    .reduce(
                        || {
                            (
                                Vec::new(),
                                vec![0usize; n_text],
                                vec![0usize; n_field],
                                vec![0usize; n_date],
                            )
                        },
                        |(mut va, mut ta, mut fa, mut da), (vb, tb, fb, db)| {
                            va.extend(vb);
                            for (a, b) in ta.iter_mut().zip(tb) {
                                *a += b;
                            }
                            for (a, b) in fa.iter_mut().zip(fb) {
                                *a += b;
                            }
                            for (a, b) in da.iter_mut().zip(db) {
                                *a += b;
                            }
                            (va, ta, fa, da)
                        },
                    )
            };
            self.filter.match_counts =
                merge_filter_counts(&all_filter_defs, &text_counts, &field_counts, &date_counts);
            self.filter.manager = Arc::new(fm);
            self.filter.text_styles = styles;
            self.filter.date_styles = date_filter_styles;
            self.filter.field_styles = field_filter_styles;
            self.filter.visible_indices = VisibleLines::Filtered(visible);
            // Apply continuation-line grouping: continuation lines (those whose
            // parser returned None) inherit their parent's filter visibility so
            // they are hidden when the parent is hidden by a date or exclude filter.
            if let Some(cmap) = self.continuation_map.as_deref() {
                apply_continuation_correction(
                    &mut self.filter.visible_indices,
                    cmap,
                    has_text_includes,
                );
            }
        }

        self.restore_scroll_to_line(current_line);
    }

    /// Returns the text that is actually displayed for `line_idx`.
    /// For structured log lines this is the rendered column string (which omits
    /// hidden fields); for raw lines it is the UTF-8 decoded bytes.
    /// This is the text the search should match against so that hidden-field
    /// content is never counted as a hit.
    pub fn get_display_text(&self, line_idx: usize) -> String {
        let format = if self.display.raw_mode {
            None
        } else {
            self.display.format.clone()
        };
        display_text_for_line(
            line_idx,
            &self.file_reader,
            &format,
            &self.display.field_layout,
            &self.display.hidden_fields,
            self.display.show_keys,
        )
    }

    /// Build a lookup map of display text for each index yielded by `indices`.
    /// Collecting up-front allows callers to pass the map into `Search::search`
    /// without conflicting borrows on `self.search.query`.
    pub fn collect_display_texts(
        &self,
        indices: impl Iterator<Item = usize>,
    ) -> std::collections::HashMap<usize, String> {
        indices.map(|li| (li, self.get_display_text(li))).collect()
    }

    pub fn scroll_to_line_idx(&mut self, line_idx: usize) {
        if let Some(index) = self.filter.visible_indices.position_of(line_idx) {
            self.scroll.scroll_offset = index;
        }
    }

    /// Adjusts `horizontal_scroll` so that `cursor_col` (a char index into
    /// `line_text`) stays within the visible horizontal viewport with some
    /// padding from the edges.
    /// No-op when wrap is enabled or `visible_width` is not yet known.
    pub fn scroll_char_cursor_into_view(&mut self, cursor_col: usize, line_text: &str) {
        const PADDING: usize = 3;

        if self.display.wrap || self.scroll.visible_width == 0 {
            return;
        }
        let prefix: String = line_text.chars().take(cursor_col).collect();
        let cursor_display_col = unicode_width::UnicodeWidthStr::width(prefix.as_str());

        // Cap padding so it never exceeds half the viewport (prevents oscillation on narrow views).
        let pad = PADDING.min(self.scroll.visible_width.saturating_sub(1) / 2);

        let padded_right = cursor_display_col.saturating_add(1).saturating_add(pad);
        if padded_right > self.scroll.horizontal_scroll + self.scroll.visible_width {
            self.scroll.horizontal_scroll = padded_right - self.scroll.visible_width;
        } else if cursor_display_col < self.scroll.horizontal_scroll.saturating_add(pad) {
            self.scroll.horizontal_scroll = cursor_display_col.saturating_sub(pad);
        }
    }

    /// Scroll vertically to the current search match and, when wrap is off,
    /// also center the match occurrence horizontally.
    pub fn scroll_to_current_search_match(&mut self) {
        let Some(result) = self.search.query.get_current_match() else {
            return;
        };
        let line_idx = result.line_idx;
        let occurrence_idx = self.search.query.get_current_occurrence_index();

        let h_scroll = if !self.display.wrap && self.scroll.visible_width > 0 {
            result.matches.get(occurrence_idx).map(|&(start, end)| {
                let line = self.file_reader.get_line(line_idx);
                let prefix_bytes = &line[..start.min(line.len())];
                let col = unicode_width::UnicodeWidthStr::width(
                    std::str::from_utf8(prefix_bytes).unwrap_or(""),
                );
                let match_bytes = &line[start.min(line.len())..end.min(line.len())];
                let match_width = unicode_width::UnicodeWidthStr::width(
                    std::str::from_utf8(match_bytes).unwrap_or(""),
                );
                let match_center = col + match_width / 2;
                match_center.saturating_sub(self.scroll.visible_width / 2)
            })
        } else {
            None
        };

        self.scroll_to_line_idx(line_idx);
        if let Some(h) = h_scroll {
            self.scroll.horizontal_scroll = h;
        }
    }

    /// Cancel any in-flight search, clear results, and invalidate the render cache.
    pub fn cancel_search(&mut self) {
        if let Some(ref h) = self.search.handle {
            h.cancel.store(true, Ordering::Relaxed);
        }
        self.search.handle = None;
        self.search.query.clear();
        self.cache.search_result_gen = self.cache.search_result_gen.wrapping_add(1);
    }

    /// Start a background search for `pattern` over the current visible lines.
    ///
    /// Any in-flight search is cancelled immediately.  Results are delivered
    /// via [`SearchHandle`] and polled each frame by `App::advance_search`.
    /// When `navigate` is true the view scrolls to the first match on completion.
    pub fn begin_search(&mut self, pattern: &str, forward: bool, navigate: bool) {
        if pattern.is_empty() {
            self.cancel_search();
            return;
        }

        if let Some(ref h) = self.search.handle {
            h.cancel.store(true, Ordering::Relaxed);
        }
        self.search.handle = None;

        let case_sensitive = self.search.query.is_case_sensitive();
        let regex_str = if case_sensitive {
            pattern.to_string()
        } else {
            format!("(?i){}", pattern)
        };
        let Ok(re) = regex::Regex::new(&regex_str) else {
            return;
        };

        // Set pattern and clear results immediately so highlights appear and
        // stale results from the previous search don't linger.
        self.search.query.set_results(vec![], re.clone());
        self.search.query.set_forward(forward);

        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = cancel.clone();
        let (result_tx, result_rx) = mpsc::channel::<Vec<SearchResult>>(32);
        let (progress_tx, progress_rx) = watch::channel(0.0_f64);

        // Clone the file reader (O(1) — just increments Arc ref-counts).
        let file_reader = self.file_reader.clone();
        let total = self.filter.visible_indices.len();
        // O(1) for All(n); clones the Vec only for Filtered — avoids blocking the
        // main thread with a potentially large allocation before the task starts.
        let visible_indices = self.filter.visible_indices.clone();
        // Clone display context so the background task searches displayed text only.
        // In raw mode the parser is bypassed so search must run against raw bytes.
        let detected_format = if self.display.raw_mode {
            None
        } else {
            self.display.format.clone()
        };
        let field_layout = self.display.field_layout.clone();
        let hidden_fields = self.display.hidden_fields.clone();
        let show_keys = self.display.show_keys;

        // Use Aho-Corasick for literal (non-regex) patterns — much faster.
        let pattern_str = pattern.to_string();
        let use_ac = !crate::filters::is_regex_pattern(&pattern_str);
        let ac = use_ac.then(|| {
            aho_corasick::AhoCorasick::builder()
                .ascii_case_insensitive(!case_sensitive)
                .build([&pattern_str])
                .unwrap()
        });

        const INITIAL_SEARCH_CHUNK: usize = 5_000;
        const MAX_SEARCH_CHUNK: usize = 500_000;

        tokio::task::spawn_blocking(move || {
            use rayon::prelude::*;

            let re_for_search = re;
            let ac_ref = ac.as_ref();
            let mut chunk: Vec<usize> = Vec::with_capacity(INITIAL_SEARCH_CHUNK);
            let mut processed = 0usize;
            let mut chunk_size = INITIAL_SEARCH_CHUNK;

            let mut iter = visible_indices.iter();
            loop {
                if cancel_clone.load(Ordering::Relaxed) {
                    break;
                }

                chunk.clear();
                while chunk.len() < chunk_size {
                    match iter.next() {
                        Some(idx) => chunk.push(idx),
                        None => break,
                    }
                }
                if chunk.is_empty() {
                    break;
                }

                let mut batch: Vec<SearchResult> = chunk
                    .par_iter()
                    .filter_map(|&line_idx| {
                        if cancel_clone.load(Ordering::Relaxed) {
                            return None;
                        }
                        let text = display_text_for_line(
                            line_idx,
                            &file_reader,
                            &detected_format,
                            &field_layout,
                            &hidden_fields,
                            show_keys,
                        );
                        let matches: Vec<(usize, usize)> = if let Some(ac) = ac_ref {
                            ac.find_iter(&text).map(|m| (m.start(), m.end())).collect()
                        } else {
                            re_for_search
                                .find_iter(&text)
                                .map(|m| (m.start(), m.end()))
                                .collect()
                        };
                        if matches.is_empty() {
                            None
                        } else {
                            Some(SearchResult { line_idx, matches })
                        }
                    })
                    .collect();

                // par_iter doesn't preserve order within the chunk — sort by line_idx.
                batch.sort_unstable_by_key(|r| r.line_idx);

                processed += chunk.len();
                if total > 0 {
                    let _ = progress_tx.send(processed as f64 / total as f64);
                }

                if result_tx.blocking_send(batch).is_err() {
                    break;
                }

                chunk_size = (chunk_size * 4).min(MAX_SEARCH_CHUNK);
            }
            // Channel closes here, signalling completion to advance_search.
        });

        self.search.handle = Some(SearchHandle {
            result_rx,
            cancel,
            progress_rx,
            pattern: pattern.to_string(),
            forward,
            navigate,
        });
    }

    /// Start a background filter computation over the entire file.
    /// Any in-flight filter computation is cancelled before the new one starts.
    pub fn begin_filter_refresh(&mut self) {
        if let Some(ref h) = self.filter.handle {
            h.cancel.store(true, Ordering::Relaxed);
        }
        self.filter.handle = None;

        self.invalidate_parse_cache();

        let has_active_filters =
            self.filter.show_marks_only || self.log_manager.get_filters().iter().any(|f| f.enabled);

        if !has_active_filters {
            let current_line = if self.filter.saved_view.is_some() {
                self.filter
                    .visible_indices
                    .get_opt(self.scroll.scroll_offset)
            } else {
                None
            };
            self.filter.saved_view = None;
            self.filter.visible_indices = self.compute_unfiltered_visible();
            self.filter.manager = Arc::new(FilterManager::empty());
            self.filter.text_styles = Vec::new();
            self.filter.date_styles = Vec::new();
            self.filter.field_styles = Vec::new();
            self.filter.match_counts = Vec::new();
            self.restore_scroll_to_line(current_line);
            return;
        }

        if self.filter.show_marks_only {
            let current_line = self
                .filter
                .visible_indices
                .get_opt(self.scroll.scroll_offset);
            if self.filter.saved_view.is_none() {
                self.filter.saved_view = Some((
                    self.filter.visible_indices.clone(),
                    self.filter.manager.clone(),
                    self.filter.text_styles.clone(),
                    self.filter.date_styles.clone(),
                    self.filter.field_styles.clone(),
                ));
            } else {
                self.filter.saved_view = None;
            }
            let mut indices = self.log_manager.get_marked_indices();
            indices.retain(|&i| i < self.file_reader.line_count());
            self.filter.visible_indices = VisibleLines::Filtered(indices);
            self.rebuild_filter_manager_cache();
            self.filter.match_counts = Vec::new();
            self.restore_scroll_to_line(current_line);
            return;
        }

        if let Some((
            saved_visible,
            saved_fm,
            saved_styles,
            saved_date_styles,
            saved_field_styles,
        )) = self.filter.saved_view.take()
        {
            let current_line = self
                .filter
                .visible_indices
                .get_opt(self.scroll.scroll_offset);
            self.filter.visible_indices = saved_visible;
            self.filter.manager = saved_fm;
            self.filter.text_styles = saved_styles;
            self.filter.date_styles = saved_date_styles;
            self.filter.field_styles = saved_field_styles;
            self.restore_scroll_to_line(current_line);
            return;
        }

        if !self.filter.enabled {
            let current_line = self
                .filter
                .visible_indices
                .get_opt(self.scroll.scroll_offset);
            self.filter.visible_indices = VisibleLines::All(self.file_reader.line_count());
            self.filter.manager = Arc::new(FilterManager::empty());
            self.filter.text_styles = Vec::new();
            self.filter.date_styles = Vec::new();
            self.filter.field_styles = Vec::new();
            self.filter.match_counts = Vec::new();
            self.restore_scroll_to_line(current_line);
            return;
        }

        let desired_fingerprint: Vec<crate::filters::FilterDef> = self
            .log_manager
            .get_filters()
            .iter()
            .filter(|f| f.enabled)
            .cloned()
            .collect();
        let current_line_count = self.file_reader.line_count();
        if let Some(cached) = &self.filter.cached_scan
            && cached.filter_fingerprint == desired_fingerprint
            && cached.line_count == current_line_count
            && cached.raw_mode == self.display.raw_mode
        {
            let current_line = self
                .filter
                .visible_indices
                .get_opt(self.scroll.scroll_offset);
            let (saved_visible, saved_fm, saved_styles, saved_date_styles, saved_field_styles) =
                cached.view.clone();
            self.filter.visible_indices = saved_visible;
            self.filter.manager = saved_fm;
            self.filter.text_styles = saved_styles;
            self.filter.date_styles = saved_date_styles;
            self.filter.field_styles = saved_field_styles;
            self.filter.match_counts = cached.match_counts.clone();
            self.restore_scroll_to_line(current_line);
            return;
        }

        let scroll_anchor = self
            .filter
            .visible_indices
            .get_opt(self.scroll.scroll_offset);
        self.rebuild_filter_manager_cache();
        // Clear stale counts — the filter order may have changed (e.g. reorder) so old
        // index-based counts would map to the wrong filters until the scan completes.
        self.filter.match_counts = Vec::new();

        const INITIAL_CHUNK_SIZE: usize = 5_000;
        const MAX_CHUNK_SIZE: usize = 500_000;

        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = cancel.clone();
        let channel_capacity = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        let (result_tx, result_rx) = mpsc::channel::<FilterChunk>(channel_capacity);

        let file_reader = self.file_reader.clone();
        let fm_arc = self.filter.manager.clone();
        let date_filters = crate::filters::extract_date_filters(self.log_manager.get_filters());
        let (inc_ff, exc_ff) =
            crate::filters::extract_field_filters(self.log_manager.get_filters());
        let field_defs =
            crate::filters::extract_field_filters_ordered(self.log_manager.get_filters());
        let all_filter_defs = self.log_manager.get_filters().to_vec();
        let raw_mode = self.display.raw_mode;
        let parser = if raw_mode {
            None
        } else {
            self.display.format.clone()
        };
        let field_layout = self.display.field_layout.clone();
        let hidden_fields = self.display.hidden_fields.clone();
        let show_keys = self.display.show_keys;
        let line_count = self.file_reader.line_count();
        let n_text_filters = self.filter.manager.filter_count();
        let year_map = self.year_map.clone();
        let is_merged_reader = self.merged.is_some();

        tokio::task::spawn_blocking(move || {
            use rayon::prelude::*;

            let parser_ref: Option<&dyn LogFormatParser> = parser.as_deref();
            let n_text = n_text_filters;
            let n_field = field_defs.len();
            let n_date = date_filters.len();

            let has_text_includes = fm_arc.has_include();
            let synthetic_level = parser_ref.is_some_and(|p| p.has_synthetic_level()) && n_text > 0;
            let needs_parse = !date_filters.is_empty()
                || !field_defs.is_empty()
                || !inc_ff.is_empty()
                || !exc_ff.is_empty()
                || synthetic_level;
            let date_only = !date_filters.is_empty()
                && inc_ff.is_empty()
                && exc_ff.is_empty()
                && !synthetic_level;

            // Whole-file AC scan: single contiguous pass per chunk instead of
            // per-line iterator calls.  Used when only text filters are active
            // and a combined Aho-Corasick automaton is available.
            // Disabled for merged readers: their data() is empty and line_starts
            // are dummy sequential indices, not byte offsets.
            let use_wholefile = !needs_parse && fm_arc.has_combined_ac() && !is_merged_reader;

            let mut total_text_counts = vec![0usize; n_text];
            let mut total_field_counts = vec![0usize; n_field];
            let mut total_date_counts = vec![0usize; n_date];

            let mut chunk_start = 0;
            let mut chunk_size = INITIAL_CHUNK_SIZE;
            while chunk_start < line_count {
                if cancel_clone.load(Ordering::Relaxed) {
                    return;
                }

                let chunk_end = (chunk_start + chunk_size).min(line_count);
                let is_last = chunk_end == line_count;
                let progress = if is_last {
                    1.0
                } else {
                    chunk_start as f64 / line_count as f64
                };

                #[cfg(unix)]
                file_reader.advise_for_scan(chunk_start..chunk_end);

                let (visible, text_counts, field_counts, date_counts) = if use_wholefile {
                    // Fast path: whole-buffer AC scan with rayon sub-chunking.
                    let (vis, tc) = fm_arc.evaluate_chunk_wholefile(
                        file_reader.data(),
                        file_reader.line_starts(),
                        chunk_start..chunk_end,
                    );
                    (vis, tc, vec![0usize; n_field], vec![0usize; n_date])
                } else {
                    // Per-line path: needed for date/field filters or regex-only pipelines.
                    (chunk_start..chunk_end)
                        .into_par_iter()
                        .with_min_len(1024)
                        .fold(
                            || {
                                (
                                    Vec::new(),
                                    vec![0usize; n_text],
                                    vec![0usize; n_field],
                                    vec![0usize; n_date],
                                )
                            },
                            |(mut vis, mut tc, mut fc, mut dc), i| {
                                let line = file_reader.get_line(i);
                                let year_override =
                                    year_map.as_deref().map(|ym| ym.year_for_line(i));
                                let mut text_dec = fm_arc.evaluate_and_count(line, &mut tc);
                                let can_skip = text_dec == FilterDecision::Exclude
                                    || (text_dec == FilterDecision::Neutral
                                        && has_text_includes
                                        && inc_ff.is_empty()
                                        && !synthetic_level);
                                let visible = if date_only && !can_skip {
                                    parser_ref
                                        .and_then(|p| p.parse_timestamp(line))
                                        .map(|ts| {
                                            let mut any = false;
                                            for (df, cnt) in date_filters.iter().zip(dc.iter_mut())
                                            {
                                                if df.matches(ts, year_override) {
                                                    *cnt += 1;
                                                    any = true;
                                                }
                                            }
                                            any
                                        })
                                        .unwrap_or(true)
                                } else {
                                    let parts = if needs_parse && !can_skip {
                                        parser_ref.and_then(|p| p.parse_line(line))
                                    } else {
                                        None
                                    };
                                    if text_dec == FilterDecision::Neutral
                                        && synthetic_level
                                        && let Some(p) = parts.as_ref()
                                    {
                                        let display = crate::ui::field_layout::apply_field_layout(
                                            p,
                                            &field_layout,
                                            &hidden_fields,
                                            show_keys,
                                            None,
                                        )
                                        .join(" ");
                                        let dec =
                                            fm_arc.evaluate_and_count(display.as_bytes(), &mut tc);
                                        if dec != FilterDecision::Neutral {
                                            text_dec = dec;
                                        }
                                    }
                                    if !field_defs.is_empty() {
                                        crate::filters::count_field_filter_matches(
                                            &field_defs,
                                            parts.as_ref(),
                                            &mut fc,
                                        );
                                    }
                                    line_is_visible(
                                        text_dec,
                                        has_text_includes,
                                        &date_filters,
                                        &mut dc,
                                        &inc_ff,
                                        &exc_ff,
                                        parts.as_ref(),
                                        year_override,
                                    )
                                };
                                if visible {
                                    vis.push(i);
                                }
                                (vis, tc, fc, dc)
                            },
                        )
                        .reduce(
                            || {
                                (
                                    Vec::new(),
                                    vec![0usize; n_text],
                                    vec![0usize; n_field],
                                    vec![0usize; n_date],
                                )
                            },
                            |(mut va, mut ta, mut fa, mut da), (vb, tb, fb, db)| {
                                va.extend(vb);
                                for (a, b) in ta.iter_mut().zip(tb) {
                                    *a += b;
                                }
                                for (a, b) in fa.iter_mut().zip(fb) {
                                    *a += b;
                                }
                                for (a, b) in da.iter_mut().zip(db) {
                                    *a += b;
                                }
                                (va, ta, fa, da)
                            },
                        )
                };

                if cancel_clone.load(Ordering::Relaxed) {
                    return;
                }

                for (a, b) in total_text_counts.iter_mut().zip(&text_counts) {
                    *a += b;
                }
                for (a, b) in total_field_counts.iter_mut().zip(&field_counts) {
                    *a += b;
                }
                for (a, b) in total_date_counts.iter_mut().zip(&date_counts) {
                    *a += b;
                }

                let filter_match_counts = if is_last {
                    Some(merge_filter_counts(
                        &all_filter_defs,
                        &total_text_counts,
                        &total_field_counts,
                        &total_date_counts,
                    ))
                } else {
                    None
                };

                if result_tx
                    .blocking_send(FilterChunk {
                        visible,
                        filter_match_counts,
                        is_last,
                        progress,
                    })
                    .is_err()
                {
                    return;
                }

                chunk_start = chunk_end;
                chunk_size = (chunk_size * 4).min(MAX_CHUNK_SIZE);
            }

            // Guard: if the file is empty the loop body never runs; send one empty final chunk.
            if line_count == 0 {
                let _ = result_tx.blocking_send(FilterChunk {
                    visible: Vec::new(),
                    filter_match_counts: Some(merge_filter_counts(
                        &all_filter_defs,
                        &total_text_counts,
                        &total_field_counts,
                        &total_date_counts,
                    )),
                    is_last: true,
                    progress: 1.0,
                });
            }
        });

        self.filter.handle = Some(FilterHandle {
            result_rx,
            cancel,
            displayed_progress: 0.0,
            scroll_anchor,
            received_first_chunk: false,
            scan_fingerprint: desired_fingerprint,
            scan_line_count: current_line_count,
            scan_raw_mode: self.display.raw_mode,
        });
    }

    /// Incrementally filter only the newly appended lines (from `old_line_count`
    /// to the current line count). Used by streaming/watch to avoid a full
    /// rescan that would cause the "Filtering…" indicator to flicker.
    pub fn filter_new_lines(&mut self, old_line_count: usize) {
        self.invalidate_parse_cache();

        let new_count = self.file_reader.line_count();
        if new_count <= old_line_count {
            return;
        }

        // Extend the continuation map for the newly-appended lines.
        if let (Some(cmap), Some(parser)) = (
            self.continuation_map.as_mut(),
            self.display
                .format
                .as_deref()
                .filter(|_| !self.display.raw_mode),
        ) {
            let map = Arc::make_mut(cmap);
            let mut last_parent = map.last().copied().unwrap_or(0);
            for i in old_line_count..new_count {
                let line = self.file_reader.get_line(i);
                if !line.is_empty() && parser.parse_line(line).is_some() {
                    last_parent = i;
                }
                map.push(last_parent);
            }
        }

        let has_active_filters =
            self.filter.show_marks_only || self.log_manager.get_filters().iter().any(|f| f.enabled);

        if !has_active_filters {
            let skip_empty = self.display.format.is_some() && !self.display.raw_mode;
            if skip_empty {
                let new_vis: Vec<usize> = (old_line_count..new_count)
                    .filter(|&i| !self.file_reader.get_line(i).is_empty())
                    .collect();
                match &mut self.filter.visible_indices {
                    VisibleLines::All(n) => {
                        if new_vis.len() == new_count - old_line_count {
                            *n = new_count;
                        } else {
                            let mut all: Vec<usize> = (0..*n).collect();
                            all.extend(new_vis);
                            self.filter.visible_indices = VisibleLines::Filtered(all);
                        }
                    }
                    VisibleLines::Filtered(v) => v.extend(new_vis),
                }
            } else {
                match &mut self.filter.visible_indices {
                    VisibleLines::All(n) => *n = new_count,
                    VisibleLines::Filtered(_) => {
                        self.filter.visible_indices = VisibleLines::All(new_count);
                    }
                }
            }
            return;
        }

        if !self.filter.enabled {
            match &mut self.filter.visible_indices {
                VisibleLines::All(n) => *n = new_count,
                VisibleLines::Filtered(_) => {
                    self.filter.visible_indices = VisibleLines::All(new_count);
                }
            }
            return;
        }

        if self.filter.show_marks_only {
            let mut indices = self.log_manager.get_marked_indices();
            indices.retain(|&i| i < new_count);
            self.filter.visible_indices = VisibleLines::Filtered(indices);
            return;
        }

        let date_filters = crate::filters::extract_date_filters(self.log_manager.get_filters());
        let (inc_ff, exc_ff) =
            crate::filters::extract_field_filters(self.log_manager.get_filters());
        let has_text_includes = self.filter.manager.has_include();
        let parser: Option<&dyn crate::parser::LogFormatParser> = if self.display.raw_mode {
            None
        } else {
            self.display.format.as_deref()
        };
        let synthetic_level = parser.is_some_and(|p| p.has_synthetic_level())
            && self.filter.manager.filter_count() > 0;
        let needs_parse =
            !date_filters.is_empty() || !inc_ff.is_empty() || !exc_ff.is_empty() || synthetic_level;
        let date_only =
            !date_filters.is_empty() && inc_ff.is_empty() && exc_ff.is_empty() && !synthetic_level;

        let mut new_visible = Vec::new();
        let mut dummy_date_counts = vec![0usize; date_filters.len()];
        let mut dummy_text_counts = vec![0usize; self.filter.manager.filter_count()];

        for i in old_line_count..new_count {
            let line = self.file_reader.get_line(i);
            if parser.is_some() && line.is_empty() {
                continue;
            }
            let mut text_dec = self
                .filter
                .manager
                .evaluate_and_count(line, &mut dummy_text_counts);
            let can_skip = text_dec == FilterDecision::Exclude
                || (text_dec == FilterDecision::Neutral
                    && has_text_includes
                    && inc_ff.is_empty()
                    && !synthetic_level);
            let year_override = self.year_map.as_deref().map(|ym| ym.year_for_line(i));
            let visible = if date_only && !can_skip {
                parser
                    .and_then(|p| p.parse_timestamp(line))
                    .map(|ts| {
                        let mut any = false;
                        for (df, cnt) in date_filters.iter().zip(dummy_date_counts.iter_mut()) {
                            if df.matches(ts, year_override) {
                                *cnt += 1;
                                any = true;
                            }
                        }
                        any
                    })
                    .unwrap_or(true)
            } else {
                let parts = if needs_parse && !can_skip {
                    parser.and_then(|p| p.parse_line(line))
                } else {
                    None
                };
                if text_dec == FilterDecision::Neutral
                    && synthetic_level
                    && let Some(p) = parts.as_ref()
                {
                    let display = crate::ui::field_layout::apply_field_layout(
                        p,
                        &self.display.field_layout,
                        &self.display.hidden_fields,
                        self.display.show_keys,
                        None,
                    )
                    .join(" ");
                    let dec = self
                        .filter
                        .manager
                        .evaluate_and_count(display.as_bytes(), &mut dummy_text_counts);
                    if dec != FilterDecision::Neutral {
                        text_dec = dec;
                    }
                }
                line_is_visible(
                    text_dec,
                    has_text_includes,
                    &date_filters,
                    &mut dummy_date_counts,
                    &inc_ff,
                    &exc_ff,
                    parts.as_ref(),
                    year_override,
                )
            };
            if visible {
                new_visible.push(i);
            }
        }

        // Apply continuation semantics: continuation lines inherit their parent's
        // filter decision. A parent in this batch uses `new_visible`; a parent
        // from earlier uses `visible_indices`.
        if let Some(cmap) = self.continuation_map.clone() {
            let existing = &self.filter.visible_indices;
            let new_vis_set: std::collections::HashSet<usize> =
                new_visible.iter().copied().collect();
            new_visible.retain(|&i| {
                let parent = cmap.get(i).copied().unwrap_or(i);
                if parent == i {
                    true
                } else if parent >= old_line_count {
                    new_vis_set.contains(&parent)
                } else {
                    existing.contains(parent)
                }
            });
        }

        match &mut self.filter.visible_indices {
            VisibleLines::All(n) => {
                *n = new_count;
            }
            VisibleLines::Filtered(v) => {
                v.extend(new_visible);
            }
        }
    }

    /// Jump to a 1-based line number, or the closest visible line if the
    /// target is hidden by filters.  Returns an error message when the
    /// line number is invalid (zero).
    pub fn goto_line(&mut self, line_number: usize) -> Result<(), String> {
        if line_number == 0 {
            return Err("Line numbers start at 1".to_string());
        }
        if self.filter.visible_indices.is_empty() {
            return Ok(());
        }
        let target_idx = line_number - 1; // convert to 0-based file index

        // Binary search for the target in visible_indices.
        match self.filter.visible_indices.binary_search(target_idx) {
            Ok(pos) => {
                // Exact match — the line is visible.
                self.scroll.scroll_offset = pos;
            }
            Err(pos) => {
                // `pos` is where target_idx would be inserted.
                // Pick the closer neighbour.
                let before = if pos > 0 { Some(pos - 1) } else { None };
                let after = if pos < self.filter.visible_indices.len() {
                    Some(pos)
                } else {
                    None
                };
                let best = match (before, after) {
                    (Some(b), Some(a)) => {
                        let dist_b = target_idx - self.filter.visible_indices.get(b);
                        let dist_a = self.filter.visible_indices.get(a) - target_idx;
                        if dist_b <= dist_a { b } else { a }
                    }
                    (Some(b), None) => b,
                    (None, Some(a)) => a,
                    (None, None) => unreachable!(), // visible_indices is non-empty
                };
                self.scroll.scroll_offset = best;
            }
        }
        Ok(())
    }

    /// Apply the first include filter incrementally against the currently visible lines,
    /// avoiding a full `compute_visible` scan of the entire file.
    ///
    /// Only safe when there are no pre-existing enabled include filters — in that case
    /// the visible set is "all lines minus excludes" and retaining the matching subset
    /// is equivalent to a full recompute (O(visible) instead of O(all)).
    /// The filter manager cache is rebuilt afterward so render highlights stay correct.
    pub fn apply_incremental_include(&mut self, pattern: &str) {
        self.apply_incremental_filter(pattern, FilterDecision::Include, |dec| {
            matches!(dec, FilterDecision::Include)
        });
    }

    /// Apply a new exclude filter incrementally against the currently visible lines,
    /// avoiding a full `compute_visible` scan of the entire file.
    ///
    /// Only safe for pure-text exclude additions when no include-filter-only changes are needed.
    /// The filter manager cache is rebuilt afterward so render highlights stay correct.
    pub fn apply_incremental_exclude(&mut self, pattern: &str) {
        self.apply_incremental_filter(pattern, FilterDecision::Exclude, |dec| {
            !matches!(dec, FilterDecision::Exclude)
        });
    }

    fn apply_incremental_filter(
        &mut self,
        pattern: &str,
        decision: FilterDecision,
        keep_fn: impl Fn(FilterDecision) -> bool + Sync,
    ) {
        use crate::filters::{MatchCollector, build_filter};
        use rayon::prelude::*;
        let current_line = self
            .filter
            .visible_indices
            .get_opt(self.scroll.scroll_offset);
        if let Some(filter) = build_filter(pattern, decision, true, 0, false) {
            let file_reader = &self.file_reader;
            let indices: Vec<usize> = self.filter.visible_indices.iter().collect();
            let new_visible: Vec<usize> = indices
                .par_iter()
                .copied()
                .filter(|&line_idx| {
                    let line = file_reader.get_line(line_idx);
                    let mut dummy = MatchCollector::new(line);
                    keep_fn(filter.evaluate(line, &mut dummy))
                })
                .collect();
            self.filter.visible_indices = VisibleLines::Filtered(new_visible);
        }
        self.rebuild_filter_manager_cache();
        self.cache.parse_gen = self.cache.parse_gen.wrapping_add(1);
        self.cache.parse.clear();
        self.restore_scroll_to_line(current_line);
        self.begin_filter_refresh();
    }

    /// Rebuild filter styles after a color-only change.
    ///
    /// Visible lines are unchanged, so no file scan is needed. Only the render
    /// cache is invalidated so the next frame picks up the new highlight colors.
    pub fn refresh_filter_colors(&mut self) {
        self.rebuild_filter_manager_cache();
        self.cache.render_gen = self.cache.render_gen.wrapping_add(1);
        self.cache.render_line.clear();
    }

    /// Clamp `scroll_offset` so it stays within the visible set.
    #[inline]
    pub fn clamp_scroll_offset(&mut self) {
        if self.filter.visible_indices.is_empty() {
            self.scroll.scroll_offset = 0;
        } else {
            self.scroll.scroll_offset = self
                .scroll
                .scroll_offset
                .min(self.filter.visible_indices.len() - 1);
        }
    }

    /// Rebuild the compiled filter manager and highlight styles from the current
    /// `LogManager` filter definitions.
    #[inline]
    pub fn rebuild_filter_manager_cache(&mut self) {
        let (fm, styles, date_filter_styles, field_filter_styles) =
            self.log_manager.build_filter_manager();
        self.filter.manager = Arc::new(fm);
        self.filter.text_styles = styles;
        self.filter.date_styles = date_filter_styles;
        self.filter.field_styles = field_filter_styles;
    }

    /// Try to restore `scroll_offset` to the nearest visible position to `line_idx`.
    /// Falls back to clamping when `line_idx` is `None` or the visible set is empty.
    #[inline]
    pub fn restore_scroll_to_line(&mut self, line_idx: Option<usize>) {
        if let Some(idx) = line_idx
            && let Some(pos) = self.filter.visible_indices.nearest_position_of(idx)
        {
            self.scroll.scroll_offset = pos;
        } else {
            self.clamp_scroll_offset();
        }
    }

    /// Bump the parse cache generation so that all cached render outputs are re-computed
    /// on the next frame. Call this whenever the field layout or display mode changes.
    pub fn invalidate_parse_cache(&mut self) {
        self.cache.parse_gen = self.cache.parse_gen.wrapping_add(1);
        self.cache.parse.clear();
        self.cache.render_gen = self.cache.render_gen.wrapping_add(1);
        self.cache.render_line.clear();
    }

    pub fn reset_tab_state(&mut self) {
        self.log_manager.reset_in_memory();
        self.scroll.scroll_offset = 0;
        self.scroll.horizontal_scroll = 0;
        self.display.show_sidebar = true;
        self.display.sidebar_width = 30;
        self.display.wrap = true;
        self.display.show_line_numbers = true;
        self.display.show_keys = false;
        self.display.raw_mode = false;
        self.stream.tail_mode = false;
        self.stream.paused = false;
        self.filter.enabled = true;
        self.filter.show_marks_only = false;
        self.filter.filter_context = None;
        self.filter.editing_filter_id = None;
        self.interaction.mode = Box::new(NormalMode::default());
        self.display.hidden_fields.clear();
        self.display.field_layout = FieldLayout::default();
        self.search.query = Search::new();
        self.interaction.command_error = None;
        self.filter.saved_view = None;
        self.display.level_colors_disabled = ["trace", "debug", "info", "notice"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        if let Some(ref h) = self.search.handle {
            h.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.search.handle = None;
        if let Some(ref h) = self.filter.handle {
            h.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.filter.handle = None;
        self.cache.parse.clear();
        self.cache.render_line.clear();
        self.cache.field_names = None;
        self.filter.manager = Arc::new(FilterManager::empty());
        self.filter.text_styles.clear();
        self.filter.date_styles.clear();
        self.filter.field_styles.clear();
        self.filter.match_counts.clear();
        self.begin_filter_refresh();
    }

    /// Detect log format from the first lines of the file and store it.
    /// Also applies format-specific default hidden fields when the tab has not
    /// yet had any hidden fields configured (e.g. streaming stdin tabs that
    /// start empty and detect their format once the first data arrives).
    #[inline]
    pub fn detect_and_apply_format(&mut self) {
        let limit = self.file_reader.line_count().min(200);
        if limit > 0 {
            let sample: Vec<&[u8]> = (0..limit).map(|j| self.file_reader.get_line(j)).collect();
            let fmt: Option<Arc<dyn LogFormatParser>> = detect_format(&sample).map(Arc::from);
            // Apply default hidden fields only when the tab currently has none
            // (first detection for a streaming source, or non-streaming source
            // whose preview was too short for detection).
            if self.display.hidden_fields.is_empty()
                && let Some(f) = &fmt
            {
                let defaults = f.default_hidden_fields(&sample);
                if !defaults.is_empty() {
                    self.display.hidden_fields = defaults;
                    self.invalidate_parse_cache();
                    const FIELDS_HIDDEN_MSG: &str = "Some fields are hidden. Use 'select-fields' to choose fields \
                             or 'show-all-fields' to show all.";
                    self.set_notification(FIELDS_HIDDEN_MSG);
                }
            }
            self.display.format = fmt;
            self.continuation_map = self
                .display
                .format
                .as_deref()
                .map(|p| Arc::new(build_continuation_map(&self.file_reader, p)));
            self.year_map = self.display.format.as_deref().and_then(|p| {
                if p.timestamp_has_year() {
                    return None;
                }
                use crate::filters::system_time_to_date;
                let start_year = system_time_to_date(self.file_reader.mtime())
                    .map(|d| d.year())
                    .unwrap_or_else(|| time::OffsetDateTime::now_utc().year());
                Some(Arc::new(year_map::YearMap::build(
                    &self.file_reader,
                    p,
                    start_year,
                )))
            });
        }
    }

    pub fn to_file_context(&self) -> Option<FileContext> {
        let source = self.log_manager.source_file()?;
        let marked_lines = self.log_manager.get_marked_indices();
        let comments = self.log_manager.get_comments().to_vec();
        let file_hash = LogManager::compute_file_hash(source);
        Some(FileContext {
            source_file: source.to_string(),
            // Save the absolute file-line index, not the visible position.
            // The position is an index into the current filtered set and would
            // be meaningless once filters are reapplied on next session start.
            // get_opt converts visible-position → file-line; on All views they
            // are identical, so this is always a no-op for unfiltered sessions.
            scroll_offset: self
                .filter
                .visible_indices
                .get_opt(self.scroll.scroll_offset)
                .unwrap_or(self.scroll.scroll_offset),
            search_query: String::new(),
            level_colors_disabled: self.display.level_colors_disabled.clone(),
            horizontal_scroll: self.scroll.horizontal_scroll,
            marked_lines,
            file_hash,
            comments,
            show_keys: self.display.show_keys,
            raw_mode: self.display.raw_mode,
            sidebar_width: self.display.sidebar_width,
            hidden_fields: self.display.hidden_fields.clone(),
            field_layout_columns: self.display.field_layout.columns.clone(),
            filtering_enabled: self.filter.enabled,
        })
    }

    pub fn apply_file_context(&mut self, ctx: &FileContext) {
        self.scroll.scroll_offset = ctx.scroll_offset;
        self.display.level_colors_disabled = ctx.level_colors_disabled.clone();
        self.scroll.horizontal_scroll = ctx.horizontal_scroll;
        self.display.show_keys = ctx.show_keys;
        self.display.raw_mode = ctx.raw_mode;
        self.display.sidebar_width = ctx.sidebar_width;
        // Only restore hidden_fields from the saved context when it is non-empty.
        // An empty set in the DB either means the context was saved before
        // format-specific defaults were introduced, or the session was first
        // saved with no customisation yet — either way, keep the defaults
        // that TabState::new applied. Non-empty sets are the user's explicit
        // field selection and always take precedence over defaults.
        if !ctx.hidden_fields.is_empty() {
            self.display.hidden_fields = ctx.hidden_fields.clone();
        }
        // Keep the "fields hidden" notice in sync with the current hidden_fields.
        const FIELDS_HIDDEN_MSG: &str = "Some fields are hidden. Use 'select-fields' to choose fields \
             or 'show-all-fields' to show all.";
        if self.display.hidden_fields.is_empty() {
            if self.interaction.notification.as_deref() == Some(FIELDS_HIDDEN_MSG) {
                self.clear_notification();
            }
        } else {
            self.set_notification(FIELDS_HIDDEN_MSG);
        }
        self.display.field_layout.columns = ctx.field_layout_columns.clone();
        self.filter.enabled = ctx.filtering_enabled;
        if !ctx.marked_lines.is_empty() {
            self.log_manager.set_marks(ctx.marked_lines.clone());
        }
        if !ctx.comments.is_empty() {
            self.log_manager.set_comments(ctx.comments.clone());
        }
    }

    /// Sample visible lines and collect unique field names from the detected
    /// format parser. Returns canonical names first, then extras sorted
    /// alphabetically. For JSON, container fields (`fields`, `span`) are
    /// expanded into dotted sub-field names.
    ///
    /// Results are memoized per `parse_cache_gen` so repeated calls within the
    /// same filter/layout state (e.g. rapid tab-completions) pay only a clone.
    pub fn collect_field_names(&mut self) -> Vec<String> {
        let current_gen = self.cache.parse_gen;
        if let Some((cached_gen, ref names)) = self.cache.field_names
            && cached_gen == current_gen
        {
            return names.clone();
        }
        let names = self.compute_field_names();
        self.cache.field_names = Some((current_gen, names.clone()));
        names
    }

    fn compute_field_names(&self) -> Vec<String> {
        let mut names = if let Some(parser) = &self.display.format {
            const SAMPLE_LIMIT: usize = 200;
            let limit = self.filter.visible_indices.len().min(SAMPLE_LIMIT);
            let lines: Vec<&[u8]> = (0..limit)
                .map(|i| {
                    self.file_reader
                        .get_line(self.filter.visible_indices.get(i))
                })
                .collect();
            parser.collect_field_names(&lines)
        } else {
            Vec::new()
        };
        if self.merged.is_some() {
            names.insert(0, "source".to_string());
        }
        names
    }

    /// Collect unique field names and their observed values from raw file lines for autocomplete.
    ///
    /// - Names use canonical dotted notation (`span.method`, `fields.order_id`, …) matching the
    ///   Select Fields modal, discovered via `collect_field_names`.
    /// - Values and frequency counts are collected from **all raw file lines** (not the filtered
    ///   visible set) so that available values are not limited by the current filter state.
    /// - Names are returned sorted by frequency (fields present in the most lines first), with
    ///   ties broken alphabetically, so the most universal fields appear first in autocomplete.
    pub fn build_field_index(&self) -> crate::auto_complete::FieldIndex {
        use std::collections::HashSet;

        let Some(parser) = &self.display.format else {
            return crate::auto_complete::FieldIndex::default();
        };

        const SAMPLE_LIMIT: usize = 5_000;
        let total = self.file_reader.line_count();
        let limit = total.min(SAMPLE_LIMIT);

        // Step 1: Discover canonical names from raw file lines.
        const NAME_SAMPLE: usize = 200;
        let name_sample = total.min(NAME_SAMPLE);
        let name_lines: Vec<&[u8]> = (0..name_sample)
            .map(|i| self.file_reader.get_line(i))
            .collect();
        let names = parser.collect_field_names(&name_lines);
        // Step 2: Scan raw lines to collect values and per-name frequency counts.
        let mut name_freq: HashMap<String, usize> = HashMap::new();
        let mut value_map: HashMap<String, HashSet<String>> = HashMap::new();

        for i in 0..limit {
            let line = self.file_reader.get_line(i);
            let Some(parts) = parser.parse_line(line) else {
                continue;
            };
            for name in &names {
                if let Some(v) = crate::filters::resolve_field(name, &parts) {
                    *name_freq.entry(name.clone()).or_insert(0) += 1;
                    let skip = matches!(name.as_str(), "timestamp" | "message");
                    if !skip {
                        value_map
                            .entry(name.clone())
                            .or_default()
                            .insert(v.to_string());
                    }
                }
            }
        }

        // Step 3: Sort names by frequency descending (universal fields first), then alphabetically.
        let mut sorted_names = names;
        sorted_names.sort_by(|a, b| {
            let fa = name_freq.get(a).copied().unwrap_or(0);
            let fb = name_freq.get(b).copied().unwrap_or(0);
            fb.cmp(&fa).then(a.cmp(b))
        });

        let mut values: HashMap<String, Vec<String>> = HashMap::new();
        for (k, set) in value_map {
            let mut v: Vec<String> = set.into_iter().collect();
            v.sort();
            values.insert(k, v);
        }

        crate::auto_complete::FieldIndex {
            names: sorted_names,
            values,
        }
    }
}

impl std::fmt::Debug for TabState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TabState")
            .field("title", &self.title)
            .field("mode", &self.interaction.mode)
            .field("scroll_offset", &self.scroll.scroll_offset)
            .finish()
    }
}

/// What to do once a background file load completes.
#[derive(Clone)]
pub enum LoadContext {
    /// Replace the placeholder file_reader in the initial tab (startup).
    ReplaceInitialTab,
    /// Replace the file_reader of an existing tab created with a preview.
    ReplaceTab { tab_idx: usize },
    /// Update the preview tab at `tab_idx` with the full reader; continue session restore.
    SessionRestoreTab {
        tab_idx: usize,
        remaining: VecDeque<String>,
        total: usize,
        initial_tab_idx: usize,
    },
}

/// Tracks a single in-progress background file load.
pub struct FileLoadState {
    pub path: String,
    /// Current progress fraction (0.0–1.0); updated by the background task.
    pub progress_rx: tokio::sync::watch::Receiver<f64>,
    /// Delivers the finished [`crate::ingestion::FileLoadResult`] (or error) when indexing is done.
    pub result_rx:
        tokio::sync::oneshot::Receiver<std::io::Result<crate::ingestion::FileLoadResult>>,
    pub total_bytes: u64,
    pub on_complete: LoadContext,
    /// Set to `true` to abort the in-flight indexing task early (e.g. on tab close).
    pub cancel: Arc<AtomicBool>,
}

/// Tracks an in-progress stdin stream.  Kept separate from `file_load_state`
/// so session-restore loads cannot overwrite it.
pub struct StdinLoadState {
    /// Fires each time new data has been appended to `temp_path`.
    /// When the sender is dropped stdin has closed.
    pub snapshot_rx: tokio::sync::watch::Receiver<()>,
    /// Path to the temp file on disk.
    pub temp_path: std::path::PathBuf,
    /// Keeps the temp file alive; dropped after the final mmap is established.
    #[allow(dead_code)]
    pub temp_file: tempfile::NamedTempFile,
}

/// Tracks an in-progress background archive extraction.
pub struct ArchiveExtractionState {
    /// Per-file extraction progress updates.
    pub progress_rx: tokio::sync::watch::Receiver<crate::ingestion::ArchiveExtractionProgress>,
    /// Delivers all `ExtractedFile`s (or error) when extraction finishes.
    pub result_rx:
        tokio::sync::oneshot::Receiver<Result<Vec<crate::ingestion::ExtractedFile>, String>>,
}

/// Per-tab state for watching a file for new appended content.
pub struct FileWatchState {
    /// Fires each time new data has been appended to `reader_path`.
    pub snapshot_rx: tokio::sync::watch::Receiver<()>,
    /// Path to mmap for incremental updates.
    /// - For growing files: the original file path (grows in-place).
    /// - For streams (docker/dlt/stdin): a temp file path (all data written here).
    pub reader_path: std::path::PathBuf,
    /// Keeps the temp file alive for stream sources. `None` for file watchers.
    #[allow(dead_code)]
    pub temp_file: Option<tempfile::NamedTempFile>,
}

/// The result of a successful stream connection: a notification channel and
/// the temp file that receives all stream data.
pub type StreamConnection = (tokio::sync::watch::Receiver<()>, tempfile::NamedTempFile);

pub type ConnectFn = Arc<
    dyn Fn() -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<StreamConnection, String>> + Send>,
        > + Send
        + Sync,
>;

pub struct StreamRetryState {
    pub attempt: u32,
    pub last_error: String,
    pub retry_rx: Option<mpsc::Receiver<Result<StreamConnection, String>>>,
    /// `true` while the connection is up after a successful retry.
    /// The retry state is kept alive so the attempt counter survives reconnect
    /// cycles and the backoff keeps increasing on repeated drops.
    pub connected: bool,
    pub connect: ConnectFn,
}

impl StreamRetryState {
    pub fn new(connect: ConnectFn, error: String) -> Self {
        let mut state = Self {
            attempt: 0,
            last_error: error,
            retry_rx: None,
            connected: false,
            connect,
        };
        state.schedule_retry();
        state
    }

    pub fn schedule_retry(&mut self) {
        self.attempt += 1;
        let (tx, rx) = mpsc::channel(1);
        let delay_secs = self.retry_delay_secs();
        let connect = self.connect.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;
            let result = connect().await;
            let _ = tx.send(result).await;
        });
        self.retry_rx = Some(rx);
    }

    fn retry_delay_secs(&self) -> u64 {
        match self.attempt {
            1 => 2,
            2 => 5,
            _ => 10,
        }
    }
}

pub fn dlt_connect_fn(host: String, port: u16) -> ConnectFn {
    Arc::new(move || {
        let h = host.clone();
        let p = port;
        Box::pin(async move {
            FileReader::spawn_dlt_tcp_stream(h, p)
                .await
                .map_err(|e| e.to_string())
        })
    })
}

pub fn docker_connect_fn(container: String) -> ConnectFn {
    Arc::new(move || {
        let c = container.clone();
        Box::pin(async move {
            FileReader::spawn_process_stream("docker", &["logs", "-f", &c], true)
                .await
                .map_err(|e| e.to_string())
        })
    })
}

pub fn run_connect_fn(program: String, args: Vec<String>) -> ConnectFn {
    Arc::new(move || {
        let prog = program.clone();
        let a = args.clone();
        Box::pin(async move {
            let a_refs: Vec<&str> = a.iter().map(|s| s.as_str()).collect();
            FileReader::spawn_process_stream(&prog, &a_refs, true)
                .await
                .map_err(|e| e.to_string())
        })
    })
}

pub fn otlp_connect_fn(port: u16) -> ConnectFn {
    Arc::new(move || {
        Box::pin(async move {
            crate::ingestion::spawn_otlp_http_receiver(port)
                .await
                .map_err(|e| e.to_string())
        })
    })
}

pub fn otlp_grpc_connect_fn(port: u16) -> ConnectFn {
    Arc::new(move || {
        Box::pin(async move {
            crate::ingestion::spawn_otlp_grpc_receiver(port)
                .await
                .map_err(|e| e.to_string())
        })
    })
}

/// Construct a `FileWatchState` for a stream (docker/dlt/stdin) connection.
/// The temp file holds all stream data and must stay alive until the tab is closed.
pub fn watch_state_from_connection(conn: StreamConnection) -> FileWatchState {
    let (snapshot_rx, temp_file) = conn;
    let reader_path = temp_file.path().to_owned();
    FileWatchState {
        snapshot_rx,
        reader_path,
        temp_file: Some(temp_file),
    }
}

/// Construct a `FileWatchState` for a growing file on disk.
/// The reader will mmap the original file directly as it grows.
pub fn watch_state_from_file(
    snapshot_rx: tokio::sync::watch::Receiver<()>,
    path: String,
) -> FileWatchState {
    FileWatchState {
        snapshot_rx,
        reader_path: std::path::PathBuf::from(path),
        temp_file: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::LogManager;
    use crate::db::{AppSettingsStore, Database, FileContext};
    use crate::filters::{FilterOptions, FilterType};
    use crate::ingestion::FileReader;
    use crate::types::Comment;
    use std::sync::Arc;

    async fn make_tab(lines: &[&str]) -> TabState {
        let data: Vec<u8> = lines.join("\n").into_bytes();
        let file_reader = FileReader::from_bytes(data);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        TabState::new(file_reader, log_manager, "test".to_string())
    }

    async fn make_tab_with_source(lines: &[&str], source: &str) -> TabState {
        let data: Vec<u8> = lines.join("\n").into_bytes();
        let file_reader = FileReader::from_bytes(data);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, Some(source.to_string())).await;
        TabState::new(file_reader, log_manager, "test".to_string())
    }

    #[tokio::test]
    async fn test_refresh_visible_all_lines() {
        let tab = make_tab(&["line1", "line2", "line3", "line4", "line5"]).await;
        assert_eq!(tab.filter.visible_indices.len(), 5);
    }

    #[tokio::test]
    async fn test_refresh_visible_marks_only() {
        let mut tab = make_tab(&["line1", "line2", "line3", "line4", "line5"]).await;
        tab.log_manager.toggle_mark(0);
        tab.log_manager.toggle_mark(2);
        tab.filter.show_marks_only = true;
        tab.refresh_visible();
        assert_eq!(
            tab.filter.visible_indices,
            VisibleLines::Filtered(vec![0, 2])
        );
    }

    #[tokio::test]
    async fn test_marks_only_toggle_keeps_selected_marked_line() {
        let mut tab = make_tab(&["line0", "line1", "line2", "line3", "line4"]).await;
        tab.log_manager.toggle_mark(1);
        tab.log_manager.toggle_mark(3);
        tab.scroll.scroll_offset = 3;
        tab.filter.show_marks_only = true;
        tab.refresh_visible();
        assert_eq!(
            tab.filter.visible_indices,
            VisibleLines::Filtered(vec![1, 3])
        );
        assert_eq!(tab.scroll.scroll_offset, 1);
    }

    #[tokio::test]
    async fn test_marks_only_toggle_off_keeps_selected_line() {
        let mut tab = make_tab(&["line0", "line1", "line2", "line3", "line4"]).await;
        tab.log_manager.toggle_mark(1);
        tab.log_manager.toggle_mark(3);
        tab.filter.show_marks_only = true;
        tab.refresh_visible();
        tab.scroll.scroll_offset = 1;
        tab.filter.show_marks_only = false;
        tab.refresh_visible();
        assert_eq!(tab.filter.visible_indices.len(), 5);
        assert_eq!(tab.scroll.scroll_offset, 3);
    }

    #[tokio::test]
    async fn test_marks_only_toggle_unselected_line_clamps_offset() {
        let mut tab = make_tab(&["line0", "line1", "line2", "line3", "line4"]).await;
        tab.log_manager.toggle_mark(4);
        tab.scroll.scroll_offset = 2;
        tab.filter.show_marks_only = true;
        tab.refresh_visible();
        assert_eq!(tab.filter.visible_indices, VisibleLines::Filtered(vec![4]));
        assert_eq!(tab.scroll.scroll_offset, 0);
    }

    #[tokio::test]
    async fn test_refresh_visible_filtering_disabled() {
        let mut tab = make_tab(&["line1", "line2", "line3", "line4", "line5"]).await;
        tab.log_manager
            .add_filter_with_color(
                "line1".to_string(),
                FilterType::Include,
                FilterOptions::default().line_mode(),
            )
            .await;
        tab.filter.enabled = false;
        tab.refresh_visible();
        assert_eq!(tab.filter.visible_indices.len(), 5);
    }

    #[tokio::test]
    async fn test_filtering_disabled_keeps_selected_line() {
        let mut tab = make_tab(&["line0", "line1", "line2", "line3", "line4"]).await;
        tab.log_manager
            .add_filter_with_color(
                "line".to_string(),
                FilterType::Include,
                FilterOptions::default().line_mode(),
            )
            .await;
        tab.refresh_visible();
        tab.scroll.scroll_offset = 3;
        tab.filter.enabled = false;
        tab.refresh_visible();
        assert_eq!(tab.filter.visible_indices.len(), 5);
        assert_eq!(tab.scroll.scroll_offset, 3);
    }

    #[tokio::test]
    async fn test_filtering_reenabled_keeps_selected_line_if_visible() {
        let mut tab = make_tab(&["line0", "line1", "line2", "line3", "line4"]).await;
        tab.log_manager
            .add_filter_with_color(
                "line2".to_string(),
                FilterType::Include,
                FilterOptions::default().line_mode(),
            )
            .await;
        tab.filter.enabled = false;
        tab.refresh_visible();
        tab.scroll.scroll_offset = 2;
        tab.filter.enabled = true;
        tab.refresh_visible();
        assert_eq!(tab.filter.visible_indices, VisibleLines::Filtered(vec![2]));
        assert_eq!(tab.scroll.scroll_offset, 0);
    }

    #[tokio::test]
    async fn test_filtering_reenabled_clamps_when_selected_line_hidden() {
        let mut tab = make_tab(&["line0", "line1", "line2", "line3", "line4"]).await;
        tab.log_manager
            .add_filter_with_color(
                "line4".to_string(),
                FilterType::Include,
                FilterOptions::default().line_mode(),
            )
            .await;
        tab.filter.enabled = false;
        tab.refresh_visible();
        tab.scroll.scroll_offset = 2;
        tab.filter.enabled = true;
        tab.refresh_visible();
        assert_eq!(tab.filter.visible_indices, VisibleLines::Filtered(vec![4]));
        assert_eq!(tab.scroll.scroll_offset, 0);
    }

    #[tokio::test]
    async fn test_refresh_visible_empty_file() {
        let tab = make_tab(&[]).await;
        assert!(tab.filter.visible_indices.is_empty());
        assert_eq!(tab.scroll.scroll_offset, 0);
    }

    #[tokio::test]
    async fn test_refresh_visible_clamps_scroll() {
        let mut tab = make_tab(&["line1", "line2", "line3"]).await;
        tab.scroll.scroll_offset = 10;
        tab.refresh_visible();
        assert_eq!(tab.scroll.scroll_offset, 2);
    }

    #[tokio::test]
    async fn test_scroll_to_line_idx_found() {
        let mut tab = make_tab(&["line1", "line2", "line3", "line4", "line5"]).await;
        tab.scroll_to_line_idx(2);
        assert_eq!(tab.scroll.scroll_offset, 2);
    }

    #[tokio::test]
    async fn test_scroll_to_line_idx_not_found() {
        let mut tab = make_tab(&["line1", "line2", "line3", "line4", "line5"]).await;
        tab.scroll_to_line_idx(999);
        assert_eq!(tab.scroll.scroll_offset, 0);
    }

    #[tokio::test]
    async fn test_scroll_to_current_search_match_centers_horizontally() {
        // Line with 100 leading spaces before "needle" — match starts at byte 100.
        let line = format!("{}needle", " ".repeat(100));
        let mut tab = make_tab(&[&line]).await;
        tab.display.wrap = false;
        tab.scroll.visible_width = 40;
        // Build search results manually and point the cursor at them.
        let visible = tab.filter.visible_indices.clone();
        let texts = tab.collect_display_texts(visible.iter());
        tab.search
            .query
            .search("needle", visible.iter(), |li| texts.get(&li).cloned())
            .unwrap();
        tab.search.query.set_forward(true);
        tab.search.query.next_match();
        tab.scroll_to_current_search_match();
        // match_center ≈ 100 + 3 = 103 (col of "needle" start + half of 6-char width)
        // expected h_scroll = 103 - 20 = 83
        assert_eq!(tab.scroll.scroll_offset, 0);
        assert_eq!(tab.scroll.horizontal_scroll, 83);
    }

    #[tokio::test]
    async fn test_scroll_to_current_search_match_no_hscroll_when_wrapped() {
        let line = format!("{}needle", " ".repeat(100));
        let mut tab = make_tab(&[&line]).await;
        tab.display.wrap = true;
        tab.scroll.visible_width = 40;
        tab.scroll.horizontal_scroll = 0;
        let visible = tab.filter.visible_indices.clone();
        let texts = tab.collect_display_texts(visible.iter());
        tab.search
            .query
            .search("needle", visible.iter(), |li| texts.get(&li).cloned())
            .unwrap();
        tab.search.query.set_forward(true);
        tab.search.query.next_match();
        tab.scroll_to_current_search_match();
        // wrap=true → horizontal scroll must not change
        assert_eq!(tab.scroll.horizontal_scroll, 0);
    }

    #[tokio::test]
    async fn test_to_file_context_with_source() {
        let tab = make_tab_with_source(&["line1", "line2", "line3"], "test.log").await;
        let ctx = tab.to_file_context();
        assert!(ctx.is_some());
        let ctx = ctx.unwrap();
        assert_eq!(ctx.source_file, "test.log");
        assert_eq!(ctx.scroll_offset, 0);
        let expected_disabled: std::collections::HashSet<String> =
            ["trace", "debug", "info", "notice"]
                .iter()
                .map(|s| s.to_string())
                .collect();
        assert_eq!(ctx.level_colors_disabled, expected_disabled);
    }

    #[tokio::test]
    async fn test_to_file_context_no_source() {
        let tab = make_tab(&["line1", "line2", "line3"]).await;
        let ctx = tab.to_file_context();
        assert!(ctx.is_none());
    }

    #[tokio::test]
    async fn test_apply_file_context_full() {
        let mut tab =
            make_tab_with_source(&["line1", "line2", "line3", "line4", "line5"], "test.log").await;
        let all_disabled: std::collections::HashSet<String> = [
            "trace", "debug", "info", "notice", "warning", "error", "fatal",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let ctx = FileContext {
            source_file: "test.log".to_string(),
            scroll_offset: 3,
            search_query: "line".to_string(),
            level_colors_disabled: all_disabled.clone(),
            horizontal_scroll: 5,
            marked_lines: vec![0, 2],
            file_hash: None,
            comments: vec![Comment {
                text: "test".to_string(),
                line_indices: vec![0],
            }],
            show_keys: false,
            raw_mode: false,
            sidebar_width: 30,
            hidden_fields: HashSet::new(),
            field_layout_columns: None,
            filtering_enabled: true,
        };
        tab.apply_file_context(&ctx);
        assert_eq!(tab.scroll.scroll_offset, 3);
        assert_eq!(tab.display.level_colors_disabled, all_disabled);
        assert_eq!(tab.scroll.horizontal_scroll, 5);
        assert!(tab.log_manager.is_marked(0));
        assert!(tab.log_manager.is_marked(2));
        assert!(tab.log_manager.has_comment(0));
    }

    #[tokio::test]
    async fn test_apply_file_context_empty() {
        let mut tab = make_tab_with_source(&["line1", "line2", "line3"], "test.log").await;
        let ctx = FileContext {
            source_file: "test.log".to_string(),
            scroll_offset: 0,
            search_query: String::new(),
            level_colors_disabled: HashSet::new(),
            horizontal_scroll: 0,
            marked_lines: vec![],
            file_hash: None,
            comments: vec![],
            show_keys: false,
            raw_mode: false,
            sidebar_width: 30,
            hidden_fields: HashSet::new(),
            field_layout_columns: None,
            filtering_enabled: true,
        };
        tab.apply_file_context(&ctx);
        assert!(tab.display.level_colors_disabled.is_empty());
        assert_eq!(tab.scroll.scroll_offset, 0);
        assert_eq!(tab.scroll.horizontal_scroll, 0);
        assert!(!tab.log_manager.is_marked(0));
        assert!(!tab.log_manager.has_comment(0));
    }

    #[tokio::test]
    async fn test_apply_file_context_restores_filtering_enabled_false() {
        let mut tab = make_tab_with_source(&["line1", "line2"], "test.log").await;
        assert!(tab.filter.enabled);
        let ctx = FileContext {
            source_file: "test.log".to_string(),
            scroll_offset: 0,
            search_query: String::new(),
            level_colors_disabled: HashSet::new(),
            horizontal_scroll: 0,
            marked_lines: vec![],
            file_hash: None,
            comments: vec![],
            show_keys: false,
            raw_mode: false,
            sidebar_width: 30,
            hidden_fields: HashSet::new(),
            field_layout_columns: None,
            filtering_enabled: false,
        };
        tab.apply_file_context(&ctx);
        assert!(!tab.filter.enabled);
    }

    #[tokio::test]
    async fn test_to_file_context_captures_filtering_enabled() {
        let mut tab = make_tab_with_source(&["line1", "line2"], "test.log").await;
        tab.filter.enabled = false;
        let ctx = tab.to_file_context().expect("should produce context");
        assert!(!ctx.filtering_enabled);

        tab.filter.enabled = true;
        let ctx2 = tab.to_file_context().expect("should produce context");
        assert!(ctx2.filtering_enabled);
    }

    #[tokio::test]
    async fn test_collect_field_names_no_format() {
        let mut tab = make_tab(&["plain text line", "another line"]).await;
        let fields = tab.collect_field_names();
        assert!(fields.is_empty());
    }

    #[tokio::test]
    async fn test_collect_field_names_json_format() {
        let mut tab = make_tab(&[r#"{"level":"INFO","msg":"hello"}"#]).await;
        let fields = tab.collect_field_names();
        assert!(!fields.is_empty());
        assert!(fields.contains(&"level".to_string()));
        assert!(fields.contains(&"message".to_string()));
    }

    #[tokio::test]
    async fn test_collect_field_names_cached() {
        let mut tab = make_tab(&[r#"{"level":"INFO","msg":"hello"}"#]).await;
        let first = tab.collect_field_names();
        let gen_before = tab.cache.parse_gen;
        let second = tab.collect_field_names();
        // Result must be identical and the gen must not have changed (cache hit).
        assert_eq!(first, second);
        assert_eq!(tab.cache.parse_gen, gen_before);
        // After invalidating the cache the result is recomputed but still equal.
        tab.invalidate_parse_cache();
        let third = tab.collect_field_names();
        assert_eq!(first, third);
    }

    #[tokio::test]
    async fn test_new_tab_detects_format() {
        let tab = make_tab(&[r#"{"level":"INFO","msg":"hello"}"#]).await;
        assert!(tab.display.format.is_some());
    }

    #[tokio::test]
    async fn test_new_tab_plain_text_no_format() {
        let tab = make_tab(&["just plain text", "no structure here"]).await;
        assert!(tab.display.format.is_none());
    }

    #[tokio::test]
    async fn test_goto_line_exact_visible() {
        let mut tab = make_tab(&["a", "b", "c", "d", "e"]).await;
        // All lines visible (indices 0..5), go to line 3 (0-based idx 2)
        tab.goto_line(3).unwrap();
        assert_eq!(tab.scroll.scroll_offset, 2);
    }

    #[tokio::test]
    async fn test_goto_line_first_line() {
        let mut tab = make_tab(&["a", "b", "c"]).await;
        tab.scroll.scroll_offset = 2;
        tab.goto_line(1).unwrap();
        assert_eq!(tab.scroll.scroll_offset, 0);
    }

    #[tokio::test]
    async fn test_goto_line_last_line() {
        let mut tab = make_tab(&["a", "b", "c", "d", "e"]).await;
        tab.goto_line(5).unwrap();
        assert_eq!(tab.scroll.scroll_offset, 4);
    }

    #[tokio::test]
    async fn test_goto_line_zero_returns_error() {
        let mut tab = make_tab(&["a", "b", "c"]).await;
        let result = tab.goto_line(0);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("start at 1"));
    }

    #[tokio::test]
    async fn test_goto_line_beyond_file_jumps_to_last() {
        let mut tab = make_tab(&["a", "b", "c"]).await;
        tab.goto_line(999).unwrap();
        assert_eq!(tab.scroll.scroll_offset, 2); // last visible line
    }

    #[tokio::test]
    async fn test_goto_line_hidden_finds_closest() {
        let mut tab = make_tab(&["a", "b", "c", "d", "e"]).await;
        // Simulate filter hiding lines 1 and 2 (keep 0, 3, 4)
        tab.filter.visible_indices = VisibleLines::Filtered(vec![0, 3, 4]);
        // Go to line 2 (idx 1) — hidden, closest visible is idx 0
        tab.goto_line(2).unwrap();
        assert_eq!(tab.scroll.scroll_offset, 0); // idx 0 is at position 0
    }

    #[tokio::test]
    async fn test_goto_line_hidden_prefers_closer_after() {
        let mut tab = make_tab(&["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"]).await;
        // Visible: 0, 5, 9
        tab.filter.visible_indices = VisibleLines::Filtered(vec![0, 5, 9]);
        // Go to line 4 (idx 3) — equidistant: idx 0 (dist 3) vs idx 5 (dist 2) → pick 5
        tab.goto_line(4).unwrap();
        assert_eq!(tab.scroll.scroll_offset, 1); // idx 5 is at position 1
    }

    #[tokio::test]
    async fn test_goto_line_hidden_prefers_closer_before() {
        let mut tab = make_tab(&["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"]).await;
        tab.filter.visible_indices = VisibleLines::Filtered(vec![0, 5, 9]);
        // Go to line 7 (idx 6) — idx 5 (dist 1) vs idx 9 (dist 3) → pick 5
        tab.goto_line(7).unwrap();
        assert_eq!(tab.scroll.scroll_offset, 1); // idx 5 is at position 1
    }

    #[tokio::test]
    async fn test_goto_line_empty_visible_indices() {
        let mut tab = make_tab(&["a", "b"]).await;
        tab.filter.visible_indices = VisibleLines::Filtered(vec![]);
        // Should not panic, just no-op
        tab.goto_line(1).unwrap();
        assert_eq!(tab.scroll.scroll_offset, 0);
    }

    // ── show_mode_bar / show_borders ───────────────────────────────────

    #[tokio::test]
    async fn test_tabstate_show_mode_bar_default_true() {
        let tab = make_tab(&["line"]).await;
        assert!(tab.display.show_mode_bar);
    }

    #[tokio::test]
    async fn test_tabstate_show_borders_default_true() {
        let tab = make_tab(&["line"]).await;
        assert!(tab.display.show_borders);
    }

    #[tokio::test]
    async fn test_scroll_char_cursor_into_view_scrolls_right() {
        let mut tab = make_tab(&["hello"]).await;
        tab.display.wrap = false;
        tab.scroll.visible_width = 20;
        tab.scroll.horizontal_scroll = 0;
        // cursor_display_col=18, padded_right=18+1+3=22 > 0+20 → scroll to 22-20=2
        tab.scroll_char_cursor_into_view(18, "abcdefghijklmnopqrstuvwxyz");
        assert_eq!(tab.scroll.horizontal_scroll, 2);
    }

    #[tokio::test]
    async fn test_scroll_char_cursor_into_view_scrolls_left() {
        let mut tab = make_tab(&["hello"]).await;
        tab.display.wrap = false;
        tab.scroll.visible_width = 20;
        tab.scroll.horizontal_scroll = 15;
        // cursor_display_col=5, 5 < 15+3=18 → scroll to 5-3=2
        tab.scroll_char_cursor_into_view(5, "abcdefghijklmnopqrstuvwxyz");
        assert_eq!(tab.scroll.horizontal_scroll, 2);
    }

    #[tokio::test]
    async fn test_scroll_char_cursor_into_view_no_change_when_visible() {
        let mut tab = make_tab(&["hello"]).await;
        tab.display.wrap = false;
        tab.scroll.visible_width = 20;
        tab.scroll.horizontal_scroll = 5;
        // cursor_display_col=10, padded_right=10+1+3=14 <= 5+20=25, 10 >= 5+3=8 → no change
        tab.scroll_char_cursor_into_view(10, "abcdefghijklmnopqrstuvwxyz");
        assert_eq!(tab.scroll.horizontal_scroll, 5);
    }

    #[tokio::test]
    async fn test_scroll_char_cursor_into_view_noop_when_wrap() {
        let mut tab = make_tab(&["hello"]).await;
        tab.display.wrap = true;
        tab.scroll.visible_width = 5;
        tab.scroll.horizontal_scroll = 0;
        tab.scroll_char_cursor_into_view(7, "abcdefgh");
        assert_eq!(tab.scroll.horizontal_scroll, 0);
    }

    #[tokio::test]
    async fn test_scroll_char_cursor_into_view_noop_when_width_zero() {
        let mut tab = make_tab(&["hello"]).await;
        tab.display.wrap = false;
        tab.scroll.visible_width = 0;
        tab.scroll.horizontal_scroll = 0;
        tab.scroll_char_cursor_into_view(7, "abcdefgh");
        assert_eq!(tab.scroll.horizontal_scroll, 0);
    }

    #[tokio::test]
    async fn test_config_priority_show_mode_bar_overrides_db() {
        use std::sync::Arc;
        let db = Arc::new(crate::db::Database::in_memory().await.unwrap());
        db.save_app_setting(crate::db::SettingsKey::ShowModeBar, "false")
            .await
            .unwrap();
        let fr = crate::ingestion::FileReader::from_bytes(b"line\n".to_vec());
        let lm = crate::db::LogManager::new(db, None).await;
        let app = crate::ui::App::builder(
            lm,
            fr,
            crate::theme::Theme::default(),
            Arc::new(crate::config::Keybindings::default()),
        )
        .show_mode_bar(Some(true))
        .build()
        .await;
        assert!(
            app.show_mode_bar,
            "config Some(true) should override DB false"
        );
    }

    #[tokio::test]
    async fn test_config_priority_wrap_overrides_db() {
        use std::sync::Arc;
        let db = Arc::new(crate::db::Database::in_memory().await.unwrap());
        db.save_app_setting(crate::db::SettingsKey::Wrap, "false")
            .await
            .unwrap();
        let fr = crate::ingestion::FileReader::from_bytes(b"line\n".to_vec());
        let lm = crate::db::LogManager::new(db, None).await;
        let app = crate::ui::App::builder(
            lm,
            fr,
            crate::theme::Theme::default(),
            Arc::new(crate::config::Keybindings::default()),
        )
        .wrap(Some(true))
        .build()
        .await;
        assert!(app.wrap, "config Some(true) should override DB false");
    }

    // ── date filter integration with refresh_visible ──────────────────
    // OR combination logic is unit-tested in date_filter::tests::matches_any.
    // These tests verify that refresh_visible correctly applies date filters.

    async fn make_tab_with_date_filter(lines: &[&str], expr: &str) -> TabState {
        let mut tab = make_tab(lines).await;
        let pattern = format!("{}{}", crate::filters::DATE_PREFIX, expr);
        tab.log_manager
            .add_filter_with_color(pattern, FilterType::Include, FilterOptions::default())
            .await;
        tab.refresh_visible();
        tab
    }

    #[tokio::test]
    async fn test_date_filter_keeps_matching_lines() {
        let lines = [
            r#"{"timestamp":"2024-01-01T01:30:00Z","level":"INFO","msg":"in range"}"#,
            r#"{"timestamp":"2024-01-01T05:00:00Z","level":"INFO","msg":"out of range"}"#,
        ];
        let tab = make_tab_with_date_filter(&lines, "01:00 .. 02:00").await;
        assert_eq!(tab.filter.visible_indices, VisibleLines::Filtered(vec![0]));
    }

    #[tokio::test]
    async fn test_date_filter_two_non_overlapping_ranges_union() {
        let lines = [
            r#"{"timestamp":"2024-01-01T01:30:00Z","level":"INFO","msg":"first range"}"#,
            r#"{"timestamp":"2024-01-01T02:30:00Z","level":"INFO","msg":"between"}"#,
            r#"{"timestamp":"2024-01-01T03:30:00Z","level":"INFO","msg":"second range"}"#,
        ];
        let mut tab = make_tab(&lines).await;
        for expr in &["01:00 .. 02:00", "03:00 .. 04:00"] {
            let pattern = format!("{}{}", crate::filters::DATE_PREFIX, expr);
            tab.log_manager
                .add_filter_with_color(pattern, FilterType::Include, FilterOptions::default())
                .await;
        }
        tab.refresh_visible();
        // Lines in either range are visible; the line between is hidden.
        assert_eq!(
            tab.filter.visible_indices,
            VisibleLines::Filtered(vec![0, 2])
        );
    }

    #[tokio::test]
    async fn test_date_filter_bsd_bound_against_iso_timestamps() {
        // BSD-format bound ("Jan 23") has year 0000. ISO timestamps have a real
        // year (e.g. 2024). Without year-stripping, "2024-01-20..." > "0000-01-23..."
        // is always true, causing dates before Jan 23 to pass incorrectly.
        let lines = [
            r#"{"timestamp":"2024-01-20T10:00:00Z","level":"INFO","msg":"before"}"#,
            r#"{"timestamp":"2024-01-25T10:00:00Z","level":"INFO","msg":"after"}"#,
        ];
        let tab = make_tab_with_date_filter(&lines, "> Jan 23").await;
        // Only the Jan 25 line should be visible.
        assert_eq!(tab.filter.visible_indices.len(), 1);
        assert_eq!(tab.filter.visible_indices.get(0), 1);
    }

    #[tokio::test]
    async fn test_date_filter_bsd_range_against_iso_timestamps() {
        let lines = [
            r#"{"timestamp":"2024-01-19T10:00:00Z","level":"INFO","msg":"before range"}"#,
            r#"{"timestamp":"2024-01-21T10:00:00Z","level":"INFO","msg":"in range"}"#,
            r#"{"timestamp":"2024-01-25T10:00:00Z","level":"INFO","msg":"after range"}"#,
        ];
        let tab = make_tab_with_date_filter(&lines, "Jan 20 .. Jan 23").await;
        // Only the Jan 21 line is within the range.
        assert_eq!(tab.filter.visible_indices.len(), 1);
        assert_eq!(tab.filter.visible_indices.get(0), 1);
    }

    #[tokio::test]
    async fn test_refresh_visible_populates_filter_cache() {
        let mut tab = make_tab(&["error line", "info line", "error again"]).await;
        tab.log_manager
            .add_filter_with_color(
                "error".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        tab.refresh_visible();
        // Cache is set and reflects the filter.
        assert!(tab.filter.manager.is_visible(b"error line"));
        assert!(!tab.filter.manager.is_visible(b"info line"));
    }

    #[tokio::test]
    async fn test_filtering_disabled_cache_is_empty_manager() {
        let mut tab = make_tab(&["error line", "info line"]).await;
        tab.log_manager
            .add_filter_with_color(
                "error".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        tab.filter.enabled = false;
        tab.refresh_visible();
        // When filtering is disabled the cached manager is empty (everything visible).
        assert!(tab.filter.manager.is_visible(b"info line"));
        assert!(tab.filter.text_styles.is_empty());
    }

    #[tokio::test]
    async fn test_refresh_visible_increments_parse_cache_gen() {
        let mut tab = make_tab(&["line"]).await;
        tab.log_manager
            .add_filter_with_color(
                "line".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        let old_gen = tab.cache.parse_gen;
        tab.refresh_visible();
        assert!(tab.cache.parse_gen > old_gen);
    }

    #[tokio::test]
    async fn test_invalidate_parse_cache_increments_gen() {
        let mut tab = make_tab(&["line"]).await;
        let old_gen = tab.cache.parse_gen;
        tab.invalidate_parse_cache();
        assert!(tab.cache.parse_gen > old_gen);
        assert!(tab.cache.parse.is_empty());
    }

    #[tokio::test]
    async fn test_apply_incremental_include_narrows_visible() {
        let mut tab = make_tab(&["error line", "info line", "error again", "debug line"]).await;
        assert_eq!(tab.filter.visible_indices.len(), 4);
        tab.log_manager
            .add_filter_with_color(
                "error".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        // Only lines containing "error" should remain.
        tab.apply_incremental_include("error");
        assert_eq!(tab.filter.visible_indices.len(), 2);
        assert_eq!(tab.filter.visible_indices.get(0), 0);
        assert_eq!(tab.filter.visible_indices.get(1), 2);
    }

    #[tokio::test]
    async fn test_apply_incremental_include_updates_filter_cache() {
        let mut tab = make_tab(&["line a", "line b"]).await;
        tab.log_manager
            .add_filter_with_color(
                "line a".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        let old_gen = tab.cache.parse_gen;
        tab.apply_incremental_include("line a");
        // Parse cache generation must be bumped.
        assert!(tab.cache.parse_gen > old_gen);
        assert_eq!(tab.filter.visible_indices.len(), 1);
        assert_eq!(tab.filter.visible_indices.get(0), 0);
    }

    #[tokio::test]
    async fn test_apply_incremental_include_no_match_empty() {
        let mut tab = make_tab(&["error line", "info line"]).await;
        tab.log_manager
            .add_filter_with_color(
                "NOMATCH".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        tab.apply_incremental_include("NOMATCH");
        assert!(tab.filter.visible_indices.is_empty());
        assert_eq!(tab.scroll.scroll_offset, 0);
    }

    #[tokio::test]
    async fn test_apply_incremental_exclude_filters_visible() {
        let mut tab = make_tab(&["error line", "info line", "error again", "debug line"]).await;
        // Start with all lines visible.
        assert_eq!(tab.filter.visible_indices.len(), 4);
        tab.log_manager
            .add_filter_with_color(
                "error".to_string(),
                FilterType::Exclude,
                FilterOptions::default(),
            )
            .await;
        // Apply incremental exclude for "error" — removes lines 0 and 2.
        tab.apply_incremental_exclude("error");
        assert_eq!(tab.filter.visible_indices.len(), 2);
        // Remaining visible lines should be "info" and "debug".
        assert_eq!(tab.filter.visible_indices.get(0), 1);
        assert_eq!(tab.filter.visible_indices.get(1), 3);
    }

    #[tokio::test]
    async fn test_apply_incremental_exclude_regex_pattern_with_dot() {
        // Pattern contains '.' which triggers regex mode; the regex should still
        // match the literal substring in the log line.
        let line0 = "2019-01-26 20:29:10.000 5.120.204.67 19642 200 GET / HTTP/1.1";
        let line1 = "2019-01-26 20:29:12.000 5.120.204.67 4120 200 GET /other HTTP/1.1";
        let mut tab = make_tab(&[line0, line1]).await;
        assert_eq!(tab.filter.visible_indices.len(), 2);
        tab.log_manager
            .add_filter_with_color(
                "20:29:10.000".to_string(),
                FilterType::Exclude,
                FilterOptions::default(),
            )
            .await;
        tab.apply_incremental_exclude("20:29:10.000");
        // Line 0 contains "20:29:10.000" and must be excluded; line 1 must remain.
        assert_eq!(tab.filter.visible_indices.len(), 1);
        assert_eq!(tab.filter.visible_indices.get(0), 1);
    }

    #[tokio::test]
    async fn test_apply_incremental_exclude_updates_filter_cache() {
        let mut tab = make_tab(&["line a", "line b"]).await;
        tab.log_manager
            .add_filter_with_color(
                "line".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        tab.refresh_visible();
        let old_gen = tab.cache.parse_gen;
        tab.apply_incremental_exclude("line b");
        // Parse cache generation must be bumped.
        assert!(tab.cache.parse_gen > old_gen);
        // Only "line a" remains visible.
        assert_eq!(tab.filter.visible_indices.len(), 1);
    }

    #[tokio::test]
    async fn test_refresh_visible_bumps_render_cache_gen() {
        let mut tab = make_tab(&["line"]).await;
        tab.log_manager
            .add_filter_with_color(
                "line".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        let old = tab.cache.render_gen;
        tab.refresh_visible();
        assert!(tab.cache.render_gen > old);
        assert!(tab.cache.render_line.is_empty());
    }

    #[tokio::test]
    async fn test_refresh_visible_no_filters_skips_cache_invalidation() {
        let mut tab = make_tab(&["line"]).await;
        // No active filters: toggling filtering_enabled must not bust the caches.
        let old_parse = tab.cache.parse_gen;
        let old_render = tab.cache.render_gen;
        tab.filter.enabled = !tab.filter.enabled;
        tab.refresh_visible();
        assert_eq!(tab.cache.parse_gen, old_parse);
        assert_eq!(tab.cache.render_gen, old_render);
    }

    #[tokio::test]
    async fn test_marks_only_toggle_restores_filter_view_without_rescan() {
        // Set up a tab with an active include filter so compute_visible is required
        // on the first call, but the toggle-off should NOT re-run it.
        let mut tab = make_tab(&["hello", "world", "hello world"]).await;
        tab.log_manager
            .add_filter_with_color(
                "hello".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        tab.filter.enabled = true;
        tab.refresh_visible();
        // Filter view: lines 0 and 2 ("hello" matches).
        assert_eq!(tab.filter.visible_indices.len(), 2);
        let visible_before = tab.filter.visible_indices.clone();

        // Toggle marks-only ON.
        tab.filter.show_marks_only = true;
        tab.refresh_visible();
        // No marks → empty.
        assert_eq!(tab.filter.visible_indices.len(), 0);
        // saved_filter_view was populated.
        assert!(tab.filter.saved_view.is_some());

        // Toggle marks-only OFF — must restore without a file scan.
        tab.filter.show_marks_only = false;
        tab.refresh_visible();
        assert_eq!(tab.filter.visible_indices, visible_before);
        // saved_filter_view consumed.
        assert!(tab.filter.saved_view.is_none());
    }

    #[tokio::test]
    async fn test_marks_only_filter_change_invalidates_saved_view() {
        // If a filter fires refresh_visible while already in marks-only mode,
        // the saved view must be cleared (it would be stale).
        let mut tab = make_tab(&["hello", "world"]).await;
        tab.log_manager
            .add_filter_with_color(
                "hello".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        tab.filter.enabled = true;
        tab.refresh_visible();

        // Enter marks-only — saves filter view.
        tab.filter.show_marks_only = true;
        tab.refresh_visible();
        assert!(tab.filter.saved_view.is_some());

        // Simulate a filter change while in marks-only mode.
        tab.refresh_visible();
        assert!(tab.filter.saved_view.is_none());
    }

    #[tokio::test]
    async fn test_invalidate_parse_cache_bumps_render_cache_gen() {
        let mut tab = make_tab(&["line"]).await;
        let old = tab.cache.render_gen;
        tab.invalidate_parse_cache();
        assert!(tab.cache.render_gen > old);
        assert!(tab.cache.render_line.is_empty());
    }

    #[tokio::test]
    async fn test_refresh_filter_colors_updates_styles_without_rescan() {
        let mut tab = make_tab(&["INFO hello", "WARN world"]).await;
        tab.log_manager
            .add_filter_with_color(
                "INFO".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        tab.refresh_visible();
        let old_parse_gen = tab.cache.parse_gen;
        let old_render_gen = tab.cache.render_gen;
        let visible_before = tab.filter.visible_indices.len();

        let filter_id = tab.log_manager.get_filters()[0].id;
        tab.log_manager
            .set_color_config(filter_id, Some("red"), None, true)
            .await;
        tab.refresh_filter_colors();

        // Visible lines must be unchanged — no rescan occurred.
        assert_eq!(tab.filter.visible_indices.len(), visible_before);
        // Parse cache must be untouched.
        assert_eq!(tab.cache.parse_gen, old_parse_gen);
        // Render cache must be invalidated so the next frame picks up new colors.
        assert!(tab.cache.render_gen > old_render_gen);
        assert!(tab.cache.render_line.is_empty());
        // Styles must be updated.
        assert!(!tab.filter.text_styles.is_empty());
    }

    #[tokio::test]
    async fn test_cancel_search_bumps_search_result_gen() {
        let mut tab = make_tab(&["line"]).await;
        tab.begin_search("line", true, false);
        assert!(tab.search.query.get_pattern().is_some());
        let old = tab.cache.search_result_gen;
        tab.cancel_search();
        assert!(tab.search.query.get_pattern().is_none());
        assert!(tab.search.handle.is_none());
        assert!(tab.cache.search_result_gen > old);
    }

    #[tokio::test]
    async fn test_begin_search_clear_bumps_search_result_gen() {
        let mut tab = make_tab(&["line"]).await;
        let old = tab.cache.search_result_gen;
        tab.begin_search("", true, false);
        assert!(tab.cache.search_result_gen > old);
    }

    #[tokio::test]
    async fn test_begin_search_nonempty_does_not_bump_search_result_gen() {
        // search_result_gen is only bumped on advance_search (when results arrive),
        // not on begin_search itself for non-empty patterns.
        let mut tab = make_tab(&["line"]).await;
        let old = tab.cache.search_result_gen;
        tab.begin_search("line", true, false);
        // begin_search with a pattern spawns a background task; gen not bumped yet
        assert_eq!(tab.cache.search_result_gen, old);
    }

    async fn drain_search(tab: &mut TabState) {
        if let Some(mut h) = tab.search.handle.take() {
            let forward = h.forward;
            let navigate = h.navigate;
            while let Some(batch) = h.result_rx.recv().await {
                tab.search.query.extend_results(batch);
                tab.cache.search_result_gen = tab.cache.search_result_gen.wrapping_add(1);
            }
            if navigate && !tab.search.query.get_results().is_empty() {
                let current = tab
                    .filter
                    .visible_indices
                    .get_opt(tab.scroll.scroll_offset)
                    .unwrap_or(0);
                tab.search.query.set_position_for_search(current, forward);
                if forward {
                    tab.search.query.next_match();
                } else {
                    tab.search.query.previous_match();
                }
                tab.scroll_to_current_search_match();
            }
        }
    }

    #[tokio::test]
    async fn test_begin_search_uses_display_text_not_raw() {
        // JSON line where the key "secret_key" should be hidden.
        let line =
            r#"{"ts":"2024-01-01T00:00:00Z","level":"info","msg":"hello","secret_key":"needle"}"#;
        let line_bytes = line.as_bytes();
        let mut tab = make_tab(&[line]).await;
        tab.filter.visible_indices = VisibleLines::Filtered(vec![0]);
        // Detect format so the parser kicks in.
        tab.display.format = crate::parser::detect_format(&[line_bytes]).map(Arc::from);
        // Hide the "secret_key" field so "needle" is not displayed.
        tab.display.hidden_fields.insert("secret_key".to_string());

        tab.begin_search("needle", true, false);
        drain_search(&mut tab).await;
        // The search must find no results because "needle" is in a hidden field.
        assert!(
            tab.search.query.get_results().is_empty(),
            "hidden field content must not be matched"
        );
    }

    #[tokio::test]
    async fn test_begin_search_visible_field_is_matched() {
        let line = r#"{"ts":"2024-01-01T00:00:00Z","level":"info","msg":"needle here"}"#;
        let line_bytes = line.as_bytes();
        let mut tab = make_tab(&[line]).await;
        tab.filter.visible_indices = VisibleLines::Filtered(vec![0]);
        tab.display.format = crate::parser::detect_format(&[line_bytes]).map(Arc::from);

        tab.begin_search("needle", true, false);
        drain_search(&mut tab).await;
        assert_eq!(tab.search.query.get_results().len(), 1);
    }

    #[tokio::test]
    async fn test_begin_search_raw_mode_matches_against_raw_bytes() {
        // In raw mode the parser is bypassed, so search offsets must be byte
        // positions within the raw line, not positions in the parsed/rendered text.
        let line = r#"{"ts":"2024-01-01T00:00:00Z","level":"info","msg":"needle here"}"#;
        let line_bytes = line.as_bytes();
        let mut tab = make_tab(&[line]).await;
        tab.filter.visible_indices = VisibleLines::Filtered(vec![0]);
        tab.display.format = crate::parser::detect_format(&[line_bytes]).map(Arc::from);
        tab.display.raw_mode = true;

        tab.begin_search("needle", true, false);
        drain_search(&mut tab).await;

        assert_eq!(tab.search.query.get_results().len(), 1);
        let expected_start = line.find("needle").unwrap();
        assert_eq!(
            tab.search.query.get_results()[0].matches[0].0,
            expected_start,
            "match offset must be a raw byte position"
        );
    }

    #[tokio::test]
    async fn test_search_first_chunk_size() {
        // Build a 10_000-line file where every line contains "match".
        let lines: Vec<String> = (0..10_000).map(|i| format!("match line {i}")).collect();
        let line_refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        let mut tab = make_tab(&line_refs).await;

        tab.begin_search("match", true, false);

        // Poll a single batch without consuming the rest.
        let batch = {
            let h = tab.search.handle.as_mut().unwrap();
            h.result_rx.recv().await.unwrap()
        };

        // The first batch must not exceed INITIAL_SEARCH_CHUNK (5_000).
        assert!(
            batch.len() <= 5_000,
            "first batch size {} exceeds INITIAL_SEARCH_CHUNK 5_000",
            batch.len()
        );
    }

    #[tokio::test]
    async fn test_get_display_text_raw_mode_returns_raw_bytes() {
        let line = r#"{"ts":"2024-01-01T00:00:00Z","level":"info","msg":"hello"}"#;
        let line_bytes = line.as_bytes();
        let mut tab = make_tab(&[line]).await;
        tab.display.format = crate::parser::detect_format(&[line_bytes]).map(Arc::from);
        tab.display.raw_mode = true;

        let text = tab.get_display_text(0);
        assert_eq!(
            text, line,
            "raw mode must return the raw line, not parsed text"
        );
    }

    #[tokio::test]
    async fn test_date_filter_not_applied_when_filtering_disabled() {
        let lines = [
            r#"{"timestamp":"2024-01-01T01:30:00Z","level":"INFO","msg":"in range"}"#,
            r#"{"timestamp":"2024-01-01T05:00:00Z","level":"INFO","msg":"out of range"}"#,
        ];
        let mut tab = make_tab(&lines).await;
        let pattern = format!("{}01:00 .. 02:00", crate::filters::DATE_PREFIX);
        tab.log_manager
            .add_filter_with_color(pattern, FilterType::Include, FilterOptions::default())
            .await;
        tab.filter.enabled = false;
        tab.refresh_visible();
        // Both lines must be visible even though only the first matches the date filter.
        assert_eq!(tab.filter.visible_indices.len(), 2);
    }

    #[tokio::test]
    async fn test_date_filter_not_applied_in_marks_only_mode() {
        let lines = [
            r#"{"timestamp":"2024-01-01T01:30:00Z","level":"INFO","msg":"in range"}"#,
            r#"{"timestamp":"2024-01-01T05:00:00Z","level":"INFO","msg":"out of range"}"#,
        ];
        let mut tab = make_tab(&lines).await;
        let pattern = format!("{}01:00 .. 02:00", crate::filters::DATE_PREFIX);
        tab.log_manager
            .add_filter_with_color(pattern, FilterType::Include, FilterOptions::default())
            .await;
        // Mark both lines, including the one outside the date range.
        tab.log_manager.toggle_mark(0);
        tab.log_manager.toggle_mark(1);
        tab.filter.show_marks_only = true;
        tab.refresh_visible();
        // Both marked lines must remain visible regardless of the date filter.
        assert_eq!(
            tab.filter.visible_indices,
            VisibleLines::Filtered(vec![0, 1])
        );
    }

    // ── field filter OR semantics with text filters ───────────────────────────

    #[tokio::test]
    async fn test_field_include_or_with_text_include() {
        // Field include and text include should be OR: a line visible if EITHER matches.
        let lines = [
            r#"{"level":"info","msg":"regular info"}"#, // no match
            r#"{"level":"error","msg":"structured error"}"#, // field include matches
            r#"{"level":"info","msg":"contains ERROR text"}"#, // text include matches
        ];
        let mut tab = make_tab(&lines).await;

        // Add text include for "ERROR" and field include for level=error.
        tab.log_manager
            .add_filter_with_color(
                "ERROR".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        tab.log_manager
            .add_filter_with_color(
                "@field:level:error".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        tab.filter.enabled = true;
        tab.refresh_visible();

        // Line 0: text Neutral + field Miss → hidden.
        // Line 1: text Neutral + field Match → visible.
        // Line 2: text Include + no excludes → visible.
        assert_eq!(
            tab.filter.visible_indices,
            VisibleLines::Filtered(vec![1, 2])
        );
    }

    #[tokio::test]
    async fn test_field_exclude_hides_despite_text_include() {
        // A field exclude should hide a line even if a text include matches it.
        let lines = [
            r#"{"level":"debug","msg":"ERROR in debug path"}"#, // text include + field exclude
            r#"{"level":"info","msg":"ERROR in info path"}"#,   // text include only → visible
        ];
        let mut tab = make_tab(&lines).await;

        tab.log_manager
            .add_filter_with_color(
                "ERROR".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        tab.log_manager
            .add_filter_with_color(
                "@field:level:debug".to_string(),
                FilterType::Exclude,
                FilterOptions::default(),
            )
            .await;
        tab.filter.enabled = true;
        tab.refresh_visible();

        // Line 0: text Include but field exclude → hidden.
        // Line 1: text Include, no field exclude match → visible.
        assert_eq!(tab.filter.visible_indices, VisibleLines::Filtered(vec![1]));
    }

    // ── begin_filter_refresh ─────────────────────────────────────────────────

    #[tokio::test]
    async fn test_begin_filter_refresh_fast_path_no_filters() {
        let mut tab = make_tab(&["a", "b", "c"]).await;
        tab.begin_filter_refresh();
        // No active filters → All(n) synchronously, no background handle.
        assert!(tab.filter.handle.is_none());
        assert_eq!(tab.filter.visible_indices, VisibleLines::All(3));
    }

    #[tokio::test]
    async fn test_begin_filter_refresh_fast_path_filtering_disabled() {
        let mut tab = make_tab(&["a", "b", "c"]).await;
        tab.log_manager
            .add_filter_with_color(
                "a".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        tab.filter.enabled = false;
        tab.begin_filter_refresh();
        // Filtering disabled: All(n) synchronously.
        assert!(tab.filter.handle.is_none());
        assert_eq!(tab.filter.visible_indices, VisibleLines::All(3));
    }

    #[tokio::test]
    async fn test_begin_filter_refresh_fast_path_marks_only() {
        let mut tab = make_tab(&["a", "b", "c"]).await;
        tab.log_manager.toggle_mark(0);
        tab.log_manager.toggle_mark(2);
        tab.filter.show_marks_only = true;
        tab.begin_filter_refresh();
        // Marks-only: O(marks) sync, no background handle.
        assert!(tab.filter.handle.is_none());
        assert_eq!(
            tab.filter.visible_indices,
            VisibleLines::Filtered(vec![0, 2])
        );
    }

    #[tokio::test]
    async fn test_begin_filter_refresh_spawns_background_for_active_filters() {
        let mut tab = make_tab(&["error line", "info line", "error again"]).await;
        tab.log_manager
            .add_filter_with_color(
                "error".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        tab.begin_filter_refresh();
        // Slow path: background handle is present.
        assert!(tab.filter.handle.is_some());
        // Drain all chunks and verify the combined visible indices.
        let mut h = tab.filter.handle.take().unwrap();
        let mut all_visible = Vec::new();
        while let Some(chunk) = h.result_rx.recv().await {
            all_visible.extend(chunk.visible);
            if chunk.is_last {
                break;
            }
        }
        assert_eq!(all_visible, vec![0, 2]);
    }

    #[tokio::test]
    async fn test_begin_filter_refresh_cancels_previous_handle() {
        let mut tab = make_tab(&["x", "y", "z"]).await;
        tab.log_manager
            .add_filter_with_color(
                "x".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        tab.begin_filter_refresh();
        let cancel_1 = tab.filter.handle.as_ref().unwrap().cancel.clone();
        // Trigger a second refresh — the first handle's cancel flag must be set.
        tab.begin_filter_refresh();
        assert!(
            cancel_1.load(std::sync::atomic::Ordering::Relaxed),
            "first handle's cancel should be true after second begin_filter_refresh"
        );
    }

    #[tokio::test]
    async fn test_begin_filter_refresh_cache_hit_skips_scan() {
        let mut tab = make_tab(&["error line", "info line", "error again"]).await;
        let filter_id = tab
            .log_manager
            .add_filter_with_color(
                "error".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        // Seed the cache as if a completed scan had run.
        let fingerprint: Vec<crate::filters::FilterDef> = tab
            .log_manager
            .get_filters()
            .iter()
            .filter(|f| f.enabled)
            .cloned()
            .collect();
        tab.filter.cached_scan = Some(CachedScanResult {
            filter_fingerprint: fingerprint,
            line_count: tab.file_reader.line_count(),
            raw_mode: false,
            view: (
                VisibleLines::Filtered(vec![0, 2]),
                tab.filter.manager.clone(),
                tab.filter.text_styles.clone(),
                tab.filter.date_styles.clone(),
                tab.filter.field_styles.clone(),
            ),
            match_counts: vec![2],
        });
        tab.begin_filter_refresh();
        // Cache hit: no background scan spawned.
        assert!(tab.filter.handle.is_none());
        assert_eq!(
            tab.filter.visible_indices,
            VisibleLines::Filtered(vec![0, 2])
        );
        assert_eq!(tab.filter.match_counts, vec![2]);
        let _ = filter_id;
    }

    #[tokio::test]
    async fn test_begin_filter_refresh_cache_miss_on_line_count_change() {
        let mut tab = make_tab(&["error line", "info line"]).await;
        tab.log_manager
            .add_filter_with_color(
                "error".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        let fingerprint: Vec<crate::filters::FilterDef> = tab
            .log_manager
            .get_filters()
            .iter()
            .filter(|f| f.enabled)
            .cloned()
            .collect();
        // Cache records a stale line count.
        tab.filter.cached_scan = Some(CachedScanResult {
            filter_fingerprint: fingerprint,
            line_count: 999,
            raw_mode: false,
            view: (
                VisibleLines::Filtered(vec![0]),
                tab.filter.manager.clone(),
                tab.filter.text_styles.clone(),
                tab.filter.date_styles.clone(),
                tab.filter.field_styles.clone(),
            ),
            match_counts: vec![1],
        });
        tab.begin_filter_refresh();
        // Cache miss: background scan is spawned.
        assert!(tab.filter.handle.is_some());
    }

    #[tokio::test]
    async fn test_begin_filter_refresh_cache_miss_on_filter_change() {
        let mut tab = make_tab(&["error line", "info line"]).await;
        tab.log_manager
            .add_filter_with_color(
                "error".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        // Cache built for a different enabled filter.
        let stale_filter = crate::filters::FilterDef {
            id: 99,
            pattern: "other".to_string(),
            filter_type: FilterType::Include,
            enabled: true,
            color_config: None,
            use_regex: false,
        };
        tab.filter.cached_scan = Some(CachedScanResult {
            filter_fingerprint: vec![stale_filter],
            line_count: tab.file_reader.line_count(),
            raw_mode: false,
            view: (
                VisibleLines::Filtered(vec![]),
                tab.filter.manager.clone(),
                tab.filter.text_styles.clone(),
                tab.filter.date_styles.clone(),
                tab.filter.field_styles.clone(),
            ),
            match_counts: vec![0],
        });
        tab.begin_filter_refresh();
        assert!(tab.filter.handle.is_some());
    }

    #[tokio::test]
    async fn test_begin_filter_refresh_cache_miss_on_raw_mode_change() {
        let mut tab = make_tab(&["error line", "info line"]).await;
        tab.log_manager
            .add_filter_with_color(
                "error".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        let fingerprint: Vec<crate::filters::FilterDef> = tab
            .log_manager
            .get_filters()
            .iter()
            .filter(|f| f.enabled)
            .cloned()
            .collect();
        // Cache was built with raw_mode=true, but tab has raw_mode=false.
        tab.filter.cached_scan = Some(CachedScanResult {
            filter_fingerprint: fingerprint,
            line_count: tab.file_reader.line_count(),
            raw_mode: true,
            view: (
                VisibleLines::Filtered(vec![0]),
                tab.filter.manager.clone(),
                tab.filter.text_styles.clone(),
                tab.filter.date_styles.clone(),
                tab.filter.field_styles.clone(),
            ),
            match_counts: vec![1],
        });
        tab.begin_filter_refresh();
        assert!(tab.filter.handle.is_some());
    }

    #[tokio::test]
    async fn test_advance_filter_computation_applies_result() {
        let mut tab = make_tab(&["foo bar", "baz", "foo baz"]).await;
        tab.log_manager
            .add_filter_with_color(
                "foo".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        tab.begin_filter_refresh();
        assert!(tab.filter.handle.is_some());
        let mut h = tab.filter.handle.take().unwrap();
        let mut all_visible = Vec::new();
        while let Some(chunk) = h.result_rx.recv().await {
            all_visible.extend(chunk.visible);
            if chunk.is_last {
                break;
            }
        }
        tab.filter.visible_indices = VisibleLines::Filtered(all_visible);
        assert_eq!(
            tab.filter.visible_indices,
            VisibleLines::Filtered(vec![0, 2])
        );
    }

    // ── Lazy level scan ──────────────────────────────────────────────────

    #[tokio::test]
    async fn test_next_error_position_finds_forward() {
        let tab = make_tab(&["INFO line", "ERROR oops", "WARN careful", "FATAL crash"]).await;
        assert_eq!(tab.next_error_position(0), Some(1));
        assert_eq!(tab.next_error_position(1), Some(3));
        assert_eq!(tab.next_error_position(3), None);
    }

    #[tokio::test]
    async fn test_prev_error_position_finds_backward() {
        let tab = make_tab(&["INFO line", "ERROR oops", "WARN careful", "FATAL crash"]).await;
        assert_eq!(tab.prev_error_position(3), Some(1));
        assert_eq!(tab.prev_error_position(1), None);
    }

    #[tokio::test]
    async fn test_next_warning_position_finds_forward() {
        let tab = make_tab(&["INFO line", "ERROR oops", "WARN careful", "FATAL crash"]).await;
        assert_eq!(tab.next_warning_position(0), Some(2));
        assert_eq!(tab.next_warning_position(2), None);
    }

    #[tokio::test]
    async fn test_prev_warning_position_finds_backward() {
        let tab = make_tab(&["INFO line", "ERROR oops", "WARN careful", "FATAL crash"]).await;
        assert_eq!(tab.prev_warning_position(3), Some(2));
        assert_eq!(tab.prev_warning_position(2), None);
    }

    #[tokio::test]
    async fn test_scan_level_empty_file() {
        let tab = make_tab(&[]).await;
        assert_eq!(tab.next_error_position(0), None);
        assert_eq!(tab.prev_error_position(0), None);
    }

    #[tokio::test]
    async fn test_scan_level_no_matches() {
        let tab = make_tab(&["INFO line", "DEBUG detail"]).await;
        assert_eq!(tab.next_error_position(0), None);
        assert_eq!(tab.next_warning_position(0), None);
    }

    #[tokio::test]
    async fn test_begin_filter_refresh_delivers_match_counts() {
        let mut tab = make_tab(&[
            "ERROR: first",
            "INFO: skip",
            "ERROR: second",
            "DEBUG: verbose",
        ])
        .await;
        tab.log_manager
            .add_filter_with_color(
                "ERROR".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        tab.log_manager
            .add_filter_with_color(
                "DEBUG".to_string(),
                FilterType::Exclude,
                FilterOptions::default(),
            )
            .await;
        tab.begin_filter_refresh();
        assert!(tab.filter.handle.is_some());
        let mut h = tab.filter.handle.take().unwrap();
        let mut final_counts = None;
        while let Some(chunk) = h.result_rx.recv().await {
            if chunk.is_last {
                final_counts = chunk.filter_match_counts;
                break;
            }
        }
        let counts = final_counts.expect("counts must be Some");
        // "ERROR" matches 2 lines; "DEBUG" matches 1 line (counted independently).
        assert_eq!(counts, vec![2, 1]);
    }

    #[tokio::test]
    async fn test_begin_filter_refresh_exclude_regex_dot_pattern() {
        let line0 = "2019-01-26 20:29:10.000 5.120.204.67 200 GET / HTTP/1.1";
        let line1 = "2019-01-26 20:29:12.000 5.120.204.67 200 GET /other HTTP/1.1";
        let mut tab = make_tab(&[line0, line1]).await;
        tab.log_manager
            .add_filter_with_color(
                "20:29:10.000".to_string(),
                FilterType::Exclude,
                FilterOptions::default(),
            )
            .await;
        tab.begin_filter_refresh();
        assert!(tab.filter.handle.is_some());
        let mut h = tab.filter.handle.take().unwrap();
        let mut visible = Vec::new();
        let mut final_counts = None;
        while let Some(chunk) = h.result_rx.recv().await {
            visible.extend(chunk.visible);
            if chunk.is_last {
                final_counts = chunk.filter_match_counts;
                break;
            }
        }
        // Line 0 contains "20:29:10.000" and must be excluded.
        assert_eq!(visible, vec![1]);
        // The exclude filter matched exactly 1 line.
        assert_eq!(final_counts, Some(vec![1]));
    }

    /// Regression test: exclude filter must stay excluded after continuation correction.
    ///
    /// When a log file has a detected format, lines that the parser cannot parse
    /// (e.g. access-log lines after structured log lines) are treated as
    /// "continuation" lines. The old `apply_continuation_correction` logic
    /// unconditionally set a continuation line's visibility to its parent's —
    /// so an explicitly-excluded continuation was made visible again if its
    /// parent was still visible.
    #[tokio::test]
    async fn test_continuation_correction_respects_exclude_filter() {
        // Lines 0–1 parse as generic common-log (ISO timestamp + level).
        // Lines 2–3 are access-log style: datetime timestamp but no level keyword
        // → CommonLogParser returns None → they map to parent=1 in the continuation map.
        let parsed0 = "2024-07-24T10:00:00Z INFO request processed";
        let parsed1 = "2024-07-24T10:00:01Z INFO another request";
        let access2 = "2019-01-26 20:29:10.000 5.120.204.67 200 GET / HTTP/1.1";
        let access3 = "2019-01-26 20:29:11.000 5.120.204.68 200 GET /api HTTP/1.1";

        let mut tab = make_tab(&[parsed0, parsed1, access2, access3]).await;

        // Verify format was detected and continuation map built.
        assert!(
            tab.continuation_map.is_some(),
            "format must be detected for this test to exercise the correction path"
        );
        // Lines 2 & 3 must map to parent 1 (the last parseable line before them).
        {
            let cmap = tab.continuation_map.as_ref().unwrap();
            assert_eq!(cmap[2], 1, "access line 2 must map to parsed parent 1");
            assert_eq!(cmap[3], 1, "access line 3 must map to parsed parent 1");
        }

        // Add an exclude filter that matches line 2 (the first access-log line).
        tab.log_manager
            .add_filter_with_color(
                "20:29:10.000".to_string(),
                FilterType::Exclude,
                FilterOptions::default(),
            )
            .await;

        // Run the background scan and collect all chunks.
        tab.begin_filter_refresh();
        let mut h = tab.filter.handle.take().unwrap();
        let has_include = tab.filter.manager.has_include();
        let mut all_visible = Vec::new();
        while let Some(chunk) = h.result_rx.recv().await {
            all_visible.extend(chunk.visible);
            if chunk.is_last {
                break;
            }
        }
        tab.filter.visible_indices = VisibleLines::Filtered(all_visible);

        // Apply continuation correction — this is what advance_filter_computation does.
        let cmap = tab.continuation_map.clone().unwrap();
        apply_continuation_correction(&mut tab.filter.visible_indices, &cmap, has_include);

        // Line 2 was explicitly excluded and must remain excluded even though its
        // parent (line 1) is visible.
        let visible: Vec<usize> = tab.filter.visible_indices.iter().collect();
        assert!(
            !visible.contains(&2),
            "explicitly excluded line 2 must not be restored by continuation correction; got {visible:?}"
        );
        // Lines 0, 1, 3 must be visible.
        assert!(visible.contains(&0), "line 0 must be visible");
        assert!(visible.contains(&1), "line 1 must be visible");
        assert!(visible.contains(&3), "line 3 must be visible");
    }

    #[tokio::test]
    async fn test_filter_match_counts_updated_via_advance() {
        let mut tab = make_tab(&["ERROR line", "INFO line", "ERROR again"]).await;
        tab.log_manager
            .add_filter_with_color(
                "ERROR".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        tab.begin_filter_refresh();
        let mut h = tab.filter.handle.take().unwrap();
        while let Some(chunk) = h.result_rx.recv().await {
            if let Some(counts) = chunk.filter_match_counts {
                tab.filter.match_counts = counts;
            }
            if chunk.is_last {
                break;
            }
        }
        assert_eq!(tab.filter.match_counts, vec![2]);
    }

    #[tokio::test]
    async fn test_filter_match_counts_includes_field_filters() {
        let mut tab = make_tab(&["line one", "line two", "line three"]).await;
        tab.log_manager
            .add_filter_with_color(
                "@field:level:error".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        tab.begin_filter_refresh();
        let mut h = tab.filter.handle.take().unwrap();
        let mut final_counts: Option<Vec<usize>> = None;
        while let Some(chunk) = h.result_rx.recv().await {
            if chunk.is_last {
                final_counts = chunk.filter_match_counts;
                break;
            }
        }
        // Unified vec has length equal to filter_defs (one entry), at position 0.
        // Raw text lines have no parser so count is 0.
        let counts = final_counts.expect("filter_match_counts must be Some");
        assert_eq!(counts.len(), 1);
        assert_eq!(counts[0], 0);
    }

    #[tokio::test]
    async fn test_filter_match_counts_cleared_on_no_active_filters() {
        let mut tab = make_tab(&["line"]).await;
        tab.filter.match_counts = vec![5, 7];
        tab.begin_filter_refresh();
        assert!(tab.filter.match_counts.is_empty());
    }

    #[tokio::test]
    async fn test_filter_match_counts_includes_date_filters() {
        let lines = [
            r#"{"ts":"2024-01-01T01:00:00","level":"info","msg":"in range"}"#,
            r#"{"ts":"2024-01-01T03:00:00","level":"info","msg":"out of range"}"#,
            r#"{"ts":"2024-01-01T01:30:00","level":"info","msg":"in range 2"}"#,
        ];
        let mut tab = make_tab(&lines).await;
        tab.log_manager
            .add_filter_with_color(
                "@date:01:00:00 .. 02:00:00".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        tab.begin_filter_refresh();
        let mut h = tab.filter.handle.take().unwrap();
        let mut final_counts: Option<Vec<usize>> = None;
        while let Some(chunk) = h.result_rx.recv().await {
            if chunk.is_last {
                final_counts = chunk.filter_match_counts;
                break;
            }
        }
        // Unified vec has length equal to filter_defs (one date filter at position 0).
        let counts = final_counts.expect("filter_match_counts must be Some");
        assert_eq!(counts.len(), 1);
        assert_eq!(counts[0], 2, "two lines fall within the date range");
    }

    #[tokio::test]
    async fn test_build_field_index_no_values_for_timestamp_fields() {
        let lines = [
            r#"{"ts":"2024-01-01T00:00:00Z","level":"info","msg":"hello"}"#,
            r#"{"ts":"2024-01-01T00:00:01Z","level":"warn","msg":"world"}"#,
        ];
        let tab = make_tab(&lines).await;
        let index = tab.build_field_index();
        assert!(
            index.names.contains(&"timestamp".to_string()),
            "ts should be normalised to canonical 'timestamp' in field names"
        );
        assert!(
            index.values.get("timestamp").map_or(true, |v| v.is_empty()),
            "timestamp should have no sampled values"
        );
        assert!(!index.values.get("level").unwrap_or(&vec![]).is_empty());
    }

    #[tokio::test]
    async fn test_build_field_index_no_values_for_message_fields() {
        let lines = [
            r#"{"time":"2024-01-01T00:00:00Z","level":"info","msg":"hello"}"#,
            r#"{"time":"2024-01-01T00:00:01Z","level":"warn","msg":"world"}"#,
        ];
        let tab = make_tab(&lines).await;
        let index = tab.build_field_index();
        assert!(
            index.names.contains(&"message".to_string()),
            "msg should be normalised to canonical 'message' in field names"
        );
        assert!(
            index.values.get("message").map_or(true, |v| v.is_empty()),
            "message should have no sampled values"
        );
        assert!(!index.values.get("level").unwrap_or(&vec![]).is_empty());
    }

    // ── skip-parse optimisation ──────────────────────────────────────────────

    #[tokio::test]
    async fn test_refresh_visible_skip_parse_for_neutral_with_text_include() {
        // text include + date filter: lines not matching the include are Neutral
        // and should be hidden without parse_line being called (no timestamp → invisible).
        let lines = [
            r#"{"ts":"2024-01-01T01:00:00","msg":"GET /api"}"#,
            r#"{"ts":"2024-01-01T01:00:00","msg":"POST /api"}"#,
            "plain line without timestamp",
        ];
        let mut tab = make_tab(&lines).await;
        tab.log_manager
            .add_filter_with_color(
                "GET".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        tab.log_manager
            .add_filter_with_color(
                "@date:00:00:00 .. 23:59:59".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        tab.refresh_visible();
        // Only the GET line matches the text include; the others are hidden.
        assert_eq!(tab.filter.visible_indices.len(), 1);
    }

    #[tokio::test]
    async fn test_refresh_visible_skip_parse_for_exclude() {
        // text exclude filter: matching lines are hidden without needing parse_line.
        let lines = ["DEBUG: verbose", "INFO: keep", "DEBUG: more noise"];
        let mut tab = make_tab(&lines).await;
        tab.log_manager
            .add_filter_with_color(
                "DEBUG".to_string(),
                FilterType::Exclude,
                FilterOptions::default(),
            )
            .await;
        tab.refresh_visible();
        assert_eq!(tab.filter.visible_indices.len(), 1);
        assert_eq!(tab.filter.visible_indices.get(0), 1);
    }

    #[tokio::test]
    async fn test_begin_filter_refresh_skip_parse_for_exclude() {
        // Exclude filter in background path: matching lines hidden, parse skipped.
        let lines = ["DEBUG: verbose", "INFO: keep", "DEBUG: more noise"];
        let mut tab = make_tab(&lines).await;
        tab.log_manager
            .add_filter_with_color(
                "DEBUG".to_string(),
                FilterType::Exclude,
                FilterOptions::default(),
            )
            .await;
        tab.begin_filter_refresh();
        let mut h = tab.filter.handle.take().unwrap();
        let mut all_visible = Vec::new();
        while let Some(chunk) = h.result_rx.recv().await {
            all_visible.extend(chunk.visible);
            if chunk.is_last {
                break;
            }
        }
        assert_eq!(all_visible, vec![1]);
    }

    // ── date-only fast path (parse_timestamp instead of parse_line) ──────────

    const CLF_IN: &str = r#"127.0.0.1 - - [10/Oct/2000:13:00:00 -0700] "GET /a HTTP/1.0" 200 100"#;
    const CLF_OUT: &str = r#"127.0.0.1 - - [10/Oct/2000:20:00:00 -0700] "GET /b HTTP/1.0" 200 200"#;

    #[tokio::test]
    async fn test_refresh_visible_date_only_clf_fast_path() {
        let mut tab = make_tab(&[CLF_IN, CLF_OUT]).await;
        let pattern = format!("{}12:00:00 .. 14:00:00", crate::filters::DATE_PREFIX);
        tab.log_manager
            .add_filter_with_color(pattern, FilterType::Include, FilterOptions::default())
            .await;
        tab.refresh_visible();
        assert_eq!(tab.filter.visible_indices, VisibleLines::Filtered(vec![0]));
    }

    #[tokio::test]
    async fn test_begin_filter_refresh_date_only_clf_fast_path() {
        let lines = [CLF_IN, CLF_OUT];
        let mut tab = make_tab(&lines).await;
        let pattern = format!("{}12:00:00 .. 14:00:00", crate::filters::DATE_PREFIX);
        tab.log_manager
            .add_filter_with_color(pattern, FilterType::Include, FilterOptions::default())
            .await;
        tab.begin_filter_refresh();
        let mut h = tab.filter.handle.take().unwrap();
        let mut all_visible = Vec::new();
        while let Some(chunk) = h.result_rx.recv().await {
            all_visible.extend(chunk.visible);
            if chunk.is_last {
                break;
            }
        }
        assert_eq!(all_visible, vec![0]);
    }

    #[tokio::test]
    async fn test_filter_new_lines_date_only_clf_fast_path() {
        let mut tab = make_tab(&[CLF_IN]).await;
        let pattern = format!("{}12:00:00 .. 14:00:00", crate::filters::DATE_PREFIX);
        tab.log_manager
            .add_filter_with_color(pattern, FilterType::Include, FilterOptions::default())
            .await;
        tab.refresh_visible();
        assert_eq!(
            tab.filter.visible_indices,
            VisibleLines::Filtered(vec![0]),
            "CLF_IN should match"
        );

        let old_count = tab.file_reader.line_count();
        tab.file_reader
            .append_bytes(format!("\n{}\n{}", CLF_OUT, CLF_IN).as_bytes());
        tab.filter_new_lines(old_count);

        match &tab.filter.visible_indices {
            VisibleLines::Filtered(v) => {
                assert!(v.contains(&0), "original CLF_IN visible: {:?}", v);
                assert!(!v.contains(&1), "CLF_OUT should be hidden: {:?}", v);
                assert!(v.contains(&2), "new CLF_IN should be visible: {:?}", v);
            }
            other => panic!("expected Filtered, got {:?}", other),
        }
    }

    // CLF line: in date range 12:00..14:00, path /other (does NOT match "/a")
    const CLF_IN_RANGE_OTHER: &str =
        r#"127.0.0.1 - - [10/Oct/2000:13:30:00 -0700] "GET /other HTTP/1.0" 200 200"#;

    #[tokio::test]
    async fn test_begin_filter_refresh_text_include_with_date_hides_non_matching() {
        let mut tab = make_tab(&[CLF_IN, CLF_IN_RANGE_OTHER]).await;
        let date_pat = format!("{}12:00:00 .. 14:00:00", crate::filters::DATE_PREFIX);
        tab.log_manager
            .add_filter_with_color(date_pat, FilterType::Include, FilterOptions::default())
            .await;
        tab.log_manager
            .add_filter_with_color(
                "/a".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        tab.begin_filter_refresh();
        let mut h = tab.filter.handle.take().unwrap();
        let mut all_visible = Vec::new();
        while let Some(chunk) = h.result_rx.recv().await {
            all_visible.extend(chunk.visible);
            if chunk.is_last {
                break;
            }
        }
        assert_eq!(
            all_visible,
            vec![0],
            "line matching date but not text include must be hidden"
        );
    }

    #[tokio::test]
    async fn test_begin_filter_refresh_regex_include_with_date_hides_non_matching() {
        let mut tab = make_tab(&[CLF_IN, CLF_IN_RANGE_OTHER]).await;
        let date_pat = format!("{}12:00:00 .. 14:00:00", crate::filters::DATE_PREFIX);
        tab.log_manager
            .add_filter_with_color(date_pat, FilterType::Include, FilterOptions::default())
            .await;
        // regex pattern: matches "/a" but not "/other"
        tab.log_manager
            .add_filter_with_color(
                r"/a\b".to_string(),
                FilterType::Include,
                FilterOptions::default().regex(),
            )
            .await;
        tab.begin_filter_refresh();
        let mut h = tab.filter.handle.take().unwrap();
        let mut all_visible = Vec::new();
        while let Some(chunk) = h.result_rx.recv().await {
            all_visible.extend(chunk.visible);
            if chunk.is_last {
                break;
            }
        }
        assert_eq!(
            all_visible,
            vec![0],
            "line matching date but not regex include must be hidden"
        );
    }

    #[tokio::test]
    async fn test_filter_new_lines_text_include_with_date_hides_non_matching() {
        let mut tab = make_tab(&[CLF_IN]).await;
        let date_pat = format!("{}12:00:00 .. 14:00:00", crate::filters::DATE_PREFIX);
        tab.log_manager
            .add_filter_with_color(date_pat, FilterType::Include, FilterOptions::default())
            .await;
        tab.log_manager
            .add_filter_with_color(
                "/a".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        tab.refresh_visible();
        assert_eq!(tab.filter.visible_indices, VisibleLines::Filtered(vec![0]));

        let old = tab.file_reader.line_count();
        tab.file_reader
            .append_bytes(format!("\n{}", CLF_IN_RANGE_OTHER).as_bytes());
        tab.filter_new_lines(old);

        match &tab.filter.visible_indices {
            VisibleLines::Filtered(v) => {
                assert!(v.contains(&0), "CLF_IN should remain visible");
                assert!(!v.contains(&1), "CLF_IN_RANGE_OTHER must be hidden");
            }
            other => panic!("expected Filtered, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_filter_new_lines_regex_include_with_date_hides_non_matching() {
        let mut tab = make_tab(&[CLF_IN]).await;
        let date_pat = format!("{}12:00:00 .. 14:00:00", crate::filters::DATE_PREFIX);
        tab.log_manager
            .add_filter_with_color(date_pat, FilterType::Include, FilterOptions::default())
            .await;
        tab.log_manager
            .add_filter_with_color(
                r"/a\b".to_string(),
                FilterType::Include,
                FilterOptions::default().regex(),
            )
            .await;
        tab.refresh_visible();
        assert_eq!(tab.filter.visible_indices, VisibleLines::Filtered(vec![0]));

        let old = tab.file_reader.line_count();
        tab.file_reader
            .append_bytes(format!("\n{}", CLF_IN_RANGE_OTHER).as_bytes());
        tab.filter_new_lines(old);

        match &tab.filter.visible_indices {
            VisibleLines::Filtered(v) => {
                assert!(v.contains(&0), "CLF_IN should remain visible");
                assert!(!v.contains(&1), "CLF_IN_RANGE_OTHER must be hidden");
            }
            other => panic!("expected Filtered, got {:?}", other),
        }
    }

    // ── line_is_visible ──────────────────────────────────────────────────────

    fn make_fm_include(pattern: &str) -> FilterManager {
        let f = crate::filters::SubstringFilter::new(pattern, FilterDecision::Include, false, 0)
            .unwrap();
        FilterManager::new(vec![Box::new(f)], true)
    }

    fn make_fm_exclude(pattern: &str) -> FilterManager {
        let f = crate::filters::SubstringFilter::new(pattern, FilterDecision::Exclude, false, 0)
            .unwrap();
        FilterManager::new(vec![Box::new(f)], false)
    }

    #[test]
    fn test_line_is_visible_text_include_matches() {
        let fm = make_fm_include("ERROR");
        let dec = fm.evaluate_text(b"ERROR: bad");
        assert!(line_is_visible(
            dec,
            fm.has_include(),
            &[],
            &mut [],
            &[],
            &[],
            None,
            None,
        ));
    }

    #[test]
    fn test_line_is_visible_text_include_no_match_hidden() {
        let fm = make_fm_include("ERROR");
        let dec = fm.evaluate_text(b"INFO: fine");
        assert!(!line_is_visible(
            dec,
            fm.has_include(),
            &[],
            &mut [],
            &[],
            &[],
            None,
            None,
        ));
    }

    #[test]
    fn test_line_is_visible_text_exclude_hides() {
        let fm = make_fm_exclude("DEBUG");
        let dec = fm.evaluate_text(b"DEBUG: noisy");
        assert!(!line_is_visible(
            dec,
            fm.has_include(),
            &[],
            &mut [],
            &[],
            &[],
            None,
            None,
        ));
    }

    #[test]
    fn test_line_is_visible_text_exclude_non_matching_visible() {
        let fm = make_fm_exclude("DEBUG");
        let dec = fm.evaluate_text(b"INFO: keep");
        assert!(line_is_visible(
            dec,
            fm.has_include(),
            &[],
            &mut [],
            &[],
            &[],
            None,
            None,
        ));
    }

    #[test]
    fn test_line_is_visible_no_filters_always_visible() {
        assert!(line_is_visible(
            FilterDecision::Neutral,
            false,
            &[],
            &mut [],
            &[],
            &[],
            None,
            None,
        ));
    }

    #[test]
    fn test_line_is_visible_date_filter_match_passes() {
        use crate::filters::parse_date_filter;
        use crate::parser::DisplayParts;
        let df = parse_date_filter("01:00 .. 02:00").unwrap();
        let mut counts = vec![0usize];
        let parts = DisplayParts {
            timestamp: Some("2024-01-01T01:30:00Z"),
            ..Default::default()
        };
        assert!(line_is_visible(
            FilterDecision::Neutral,
            false,
            &[df],
            &mut counts,
            &[],
            &[],
            Some(&parts),
            None,
        ));
        assert_eq!(counts[0], 1);
    }

    #[test]
    fn test_line_is_visible_date_filter_no_match_hidden() {
        use crate::filters::parse_date_filter;
        use crate::parser::DisplayParts;
        let df = parse_date_filter("01:00 .. 02:00").unwrap();
        let mut counts = vec![0usize];
        let parts = DisplayParts {
            timestamp: Some("2024-01-01T03:00:00Z"),
            ..Default::default()
        };
        assert!(!line_is_visible(
            FilterDecision::Neutral,
            false,
            &[df],
            &mut counts,
            &[],
            &[],
            Some(&parts),
            None,
        ));
        assert_eq!(counts[0], 0);
    }

    #[test]
    fn test_line_is_visible_date_filter_no_timestamp_passes_through() {
        use crate::filters::parse_date_filter;
        use crate::parser::DisplayParts;
        let df = parse_date_filter("01:00 .. 02:00").unwrap();
        let mut counts = vec![0usize];
        let parts = DisplayParts {
            timestamp: None,
            ..Default::default()
        };
        // No timestamp → date filter does not apply → line passes through.
        assert!(line_is_visible(
            FilterDecision::Neutral,
            false,
            &[df],
            &mut counts,
            &[],
            &[],
            Some(&parts),
            None,
        ));
    }

    #[test]
    fn test_line_is_visible_date_filter_counts_all_matching() {
        use crate::filters::parse_date_filter;
        use crate::parser::DisplayParts;
        let df1 = parse_date_filter("01:00 .. 02:00").unwrap();
        let df2 = parse_date_filter("00:00 .. 03:00").unwrap();
        let mut counts = vec![0usize; 2];
        let parts = DisplayParts {
            timestamp: Some("2024-01-01T01:30:00Z"),
            ..Default::default()
        };
        assert!(line_is_visible(
            FilterDecision::Neutral,
            false,
            &[df1, df2],
            &mut counts,
            &[],
            &[],
            Some(&parts),
            None,
        ));
        assert_eq!(counts[0], 1);
        assert_eq!(counts[1], 1);
    }

    #[test]
    fn test_line_is_visible_field_exclude_hides() {
        use crate::filters::FieldFilter;
        use crate::parser::DisplayParts;
        let exc = FieldFilter {
            field: "level".to_string(),
            pattern: "debug".to_string(),
            decision: FilterDecision::Exclude,
        };
        let parts = DisplayParts {
            level: Some("debug"),
            ..Default::default()
        };
        assert!(!line_is_visible(
            FilterDecision::Neutral,
            false,
            &[],
            &mut [],
            &[],
            &[exc],
            Some(&parts),
            None,
        ));
    }

    #[test]
    fn test_line_is_visible_field_include_match_visible() {
        use crate::filters::FieldFilter;
        use crate::parser::DisplayParts;
        let inc = FieldFilter {
            field: "level".to_string(),
            pattern: "error".to_string(),
            decision: FilterDecision::Include,
        };
        let parts = DisplayParts {
            level: Some("error"),
            ..Default::default()
        };
        assert!(line_is_visible(
            FilterDecision::Neutral,
            false,
            &[],
            &mut [],
            &[inc],
            &[],
            Some(&parts),
            None,
        ));
    }

    #[test]
    fn test_line_is_visible_field_include_miss_hidden() {
        use crate::filters::FieldFilter;
        use crate::parser::DisplayParts;
        let inc = FieldFilter {
            field: "level".to_string(),
            pattern: "error".to_string(),
            decision: FilterDecision::Include,
        };
        let parts = DisplayParts {
            level: Some("info"),
            ..Default::default()
        };
        assert!(!line_is_visible(
            FilterDecision::Neutral,
            false,
            &[],
            &mut [],
            &[inc],
            &[],
            Some(&parts),
            None,
        ));
    }

    #[test]
    fn test_line_is_visible_text_include_beats_field_include_miss() {
        use crate::filters::FieldFilter;
        use crate::parser::DisplayParts;
        // Text include matched; field include miss doesn't override.
        let inc = FieldFilter {
            field: "level".to_string(),
            pattern: "error".to_string(),
            decision: FilterDecision::Include,
        };
        let parts = DisplayParts {
            level: Some("info"),
            ..Default::default()
        };
        assert!(line_is_visible(
            FilterDecision::Include,
            true,
            &[],
            &mut [],
            &[inc],
            &[],
            Some(&parts),
            None,
        ));
    }

    #[test]
    fn test_line_is_visible_field_passthrough_when_unparseable() {
        use crate::filters::FieldFilter;
        fn make_inc() -> FieldFilter {
            FieldFilter {
                field: "level".to_string(),
                pattern: "error".to_string(),
                decision: FilterDecision::Include,
            }
        }
        // parts=None → field filters do not apply → falls back to text-only logic.
        // has_text_includes=false → visible (no include filter applies)
        assert!(line_is_visible(
            FilterDecision::Neutral,
            false,
            &[],
            &mut [],
            &[make_inc()],
            &[],
            None,
            None,
        ));
        // has_text_includes=true → hidden (include filter present but nothing matched)
        assert!(!line_is_visible(
            FilterDecision::Neutral,
            true,
            &[],
            &mut [],
            &[make_inc()],
            &[],
            None,
            None,
        ));
    }

    // ── filter_new_lines ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_filter_new_lines_no_filters_updates_all() {
        let data = b"a\nb\n".to_vec();
        let file_reader = FileReader::from_bytes(data);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        let mut tab = TabState::new(file_reader, log_manager, "test".to_string());
        let old = tab.file_reader.line_count();
        assert_eq!(old, 2);
        tab.file_reader.append_bytes(b"c\nd\n");
        assert_eq!(tab.file_reader.line_count(), 4);
        tab.filter_new_lines(old);
        assert_eq!(tab.filter.visible_indices, VisibleLines::All(4));
    }

    #[tokio::test]
    async fn test_filter_new_lines_with_include_filter() {
        let data = b"INFO keep\nDEBUG skip\n".to_vec();
        let file_reader = FileReader::from_bytes(data);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        let mut tab = TabState::new(file_reader, log_manager, "test".to_string());
        tab.log_manager
            .add_filter_with_color(
                "INFO".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        tab.begin_filter_refresh();
        let mut h = tab.filter.handle.take().unwrap();
        let mut all_visible = Vec::new();
        while let Some(chunk) = h.result_rx.recv().await {
            all_visible.extend(chunk.visible);
            if chunk.is_last {
                break;
            }
        }
        tab.filter.visible_indices = VisibleLines::Filtered(all_visible);
        tab.rebuild_filter_manager_cache();

        let old_count = tab.file_reader.line_count();
        tab.file_reader.append_bytes(b"INFO new\nDEBUG noise\n");
        tab.filter_new_lines(old_count);

        match &tab.filter.visible_indices {
            VisibleLines::Filtered(v) => {
                assert!(
                    v.contains(&0),
                    "line 0 (INFO keep) should be visible: {:?}",
                    v
                );
                assert!(
                    !v.contains(&1),
                    "line 1 (DEBUG skip) should be hidden: {:?}",
                    v
                );
                assert!(
                    v.contains(&old_count),
                    "new INFO line should be visible: {:?}",
                    v
                );
                assert!(
                    !v.contains(&(old_count + 1)),
                    "new DEBUG line should be hidden: {:?}",
                    v
                );
            }
            _ => panic!("expected Filtered variant"),
        }
        assert!(tab.filter.handle.is_none());
    }

    #[tokio::test]
    async fn test_filter_new_lines_filtering_disabled() {
        let data = b"a\nb\n".to_vec();
        let file_reader = FileReader::from_bytes(data);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        let mut tab = TabState::new(file_reader, log_manager, "test".to_string());
        tab.log_manager
            .add_filter_with_color(
                "a".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        tab.filter.enabled = false;
        let old = tab.file_reader.line_count();
        tab.file_reader.append_bytes(b"c\n");
        tab.filter_new_lines(old);
        assert_eq!(
            tab.filter.visible_indices,
            VisibleLines::All(tab.file_reader.line_count())
        );
    }
}
