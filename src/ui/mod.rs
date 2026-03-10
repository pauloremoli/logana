//! Core TUI types: [`App`], [`TabState`], [`KeyResult`], and [`LoadContext`].
//!
//! [`App`] owns the tab list, global theme, and shared [`Keybindings`].
//! [`TabState`] owns the per-tab file reader, log manager, format parser,
//! visible indices, scroll state, and active mode.
//!
//! ## Key `TabState` fields
//!
//! - `file_reader`: the backing log data
//! - `log_manager`: filter defs and marks
//! - `detected_format: Option<Arc<dyn LogFormatParser>>`: auto-detected parser,
//!   stored behind `Arc` so background filter tasks can clone it in O(1)
//! - `visible_indices: VisibleLines`: `All(n)` when no filters (zero allocation,
//!   O(1) access), `Filtered(Vec<usize>)` when filters or marks are active
//! - `scroll_offset`: selected line (index into `visible_indices`)
//! - `viewport_offset`: first rendered line
//! - `filter_manager_arc: Arc<FilterManager>`: cached filter manager, cloned
//!   O(1) per render frame (atomic ref-count increment)
//! - `parse_cache_gen: u64`: monotonically increasing generation counter;
//!   incremented whenever filters, field layout, display mode, or raw mode changes
//! - `parse_cache: HashMap<usize, (u64, CachedParsedLine)>`: per-line parse
//!   cache; entry valid only when stored generation equals `parse_cache_gen`
//!
//! ## Background filter computation
//!
//! `FilterHandle` (stored on `TabState` while a filter is in flight):
//! - `result_rx: oneshot::Receiver<Vec<usize>>` — resolves with new visible indices
//! - `cancel: Arc<AtomicBool>` — set to abort; checked every 10 000 lines
//! - `progress_rx: watch::Receiver<f64>` — [0.0, 1.0] shown as "Filtering…" in tab bar
//!
//! Fast paths (synchronous, O(1)): no active filters → `VisibleLines::All(n)`;
//! `show_marks_only = true` → apply marks directly; `filtering_enabled = false`
//! → `VisibleLines::All(n)`.
//!
//! `apply_incremental_exclude(pattern)`: additive fast-path for new exclude
//! filters — compiles the pattern and calls `VisibleLines::retain()` to remove
//! matching lines from the current visible set. Avoids scanning the full file
//! when only lines need to be removed.
//!
//! ## Background search
//!
//! `SearchHandle` (stored on `TabState` while a search is in flight):
//! - `result_rx: oneshot::Receiver<(Vec<SearchResult>, Regex)>`
//! - `cancel: Arc<AtomicBool>` — checked every 10 000 lines
//! - `progress_rx: watch::Receiver<f64>` — progress for the status bar
//! - `pattern: String`, `forward: bool`, `navigate: bool`
//!
//! ## Rendering pipeline (per frame)
//!
//! 1. Compute `visible_height` and `inner_width`.
//! 2. Wrap-aware viewport adjustment (fast-path for large jumps like `G`).
//! 3. Clone `tab.filter_manager_arc` (O(1)) and filter style palettes.
//! 4. Parse cache pre-population for lines in `[start..end)`.
//! 5. For each line: use cached `rendered` string, evaluate filters, overlay
//!    search spans, apply level/mark styles, compose final `Line` via
//!    `render_line`.
//! 6. Apply `colorize_known_values` to spans with no `fg` set.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ratatui::style::Style;
use ratatui::text::Line;
use tokio::sync::{oneshot, watch};

use crate::config::Keybindings;
use crate::date_filter::DateFilterStyle;
use crate::db::FileContext;
use crate::field_filter::{FieldVote, any_field_exclude_matches, field_include_vote};
use crate::file_reader::FileReader;
use crate::filters::{FilterDecision, FilterManager};
use crate::log_manager::LogManager;
use crate::mode::app_mode::Mode;
use crate::mode::normal_mode::NormalMode;
use crate::parser::{LogFormatParser, detect_format};
use crate::search::Search;
use crate::types::{FieldLayout, SearchResult};

mod app;
mod commands;
pub(crate) mod field_layout;
mod loading;
mod render;
mod render_popups;

// ---------------------------------------------------------------------------
// KeyResult
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum KeyResult {
    Handled,
    Ignored,
    ExecuteCommand(String),
    RestoreSession(Vec<String>),
    DockerAttach(String, String),
    ApplyValueColors(std::collections::HashSet<String>),
    ApplyLevelColors(std::collections::HashSet<String>),
    CopyToClipboard(String),
    OpenFiles(Vec<String>),
    ToggleModeBar,
    AlwaysRestoreFile(crate::db::FileContext),
    NeverRestoreFile,
    AlwaysRestoreSession(Vec<String>),
    NeverRestoreSession,
}

// ---------------------------------------------------------------------------
// SearchHandle
// ---------------------------------------------------------------------------

/// Handle for a background search task spawned by [`TabState::begin_search`].
pub struct SearchHandle {
    /// Receives the completed results. `None` means the search was cancelled.
    pub result_rx: oneshot::Receiver<(Vec<SearchResult>, regex::Regex)>,
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

// ---------------------------------------------------------------------------
// FilterHandle
// ---------------------------------------------------------------------------

/// Result delivered by the background filter/level-index task.
pub struct FilterComputeResult {
    pub visible: Vec<usize>,
    pub error_positions: Vec<usize>,
    pub warning_positions: Vec<usize>,
    /// Per-filter match counts unified across all filter types, indexed parallel to
    /// `filter_defs` (disabled filters get count 0). `None` when the result comes from a
    /// level-index rebuild that did not recompute filter state (counts should be left
    /// unchanged on the tab).
    pub filter_match_counts: Option<Vec<usize>>,
}

/// Handle for a background filter computation task spawned by
/// [`TabState::begin_filter_refresh`] or [`TabState::begin_level_index_rebuild`].
pub struct FilterHandle {
    /// Receives visible indices and level positions. Never sent when cancelled.
    pub result_rx: oneshot::Receiver<FilterComputeResult>,
    /// Set to `true` to abort the in-flight computation early.
    pub cancel: Arc<AtomicBool>,
    /// Live fraction-complete (0.0–1.0) polled each frame for the progress bar.
    pub progress_rx: watch::Receiver<f64>,
}

/// List the flat (non-recursive), non-hidden regular files in `path`.
/// Returns absolute path strings sorted by name.
/// Returns an empty Vec for non-existent or unreadable paths.
pub fn list_dir_files(path: &str) -> Vec<String> {
    let dir = match std::fs::read_dir(path) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    let mut files: Vec<String> = dir
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            let fname = entry.file_name();
            let name = fname.to_string_lossy();
            // Skip hidden files (dot-prefixed).
            if name.starts_with('.') {
                return false;
            }
            // Keep regular files only (no dirs, symlinks to dirs, etc.).
            entry.file_type().map(|t| t.is_file()).unwrap_or(false)
        })
        .filter_map(|entry| entry.path().to_str().map(|s| s.to_string()))
        .collect();
    files.sort();
    files
}

/// Merge three compacted per-type count vectors into a single `Vec<usize>` of length
/// `filters.len()`, indexed by position in `filter_defs`. Disabled filters get count 0.
fn merge_filter_counts(
    filters: &[crate::types::FilterDef],
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
        if f.pattern.starts_with(crate::date_filter::DATE_PREFIX) {
            out[i] = date.get(di).copied().unwrap_or(0);
            di += 1;
        } else if f.pattern.starts_with(crate::field_filter::FIELD_PREFIX) {
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
/// Text filters (compiled into `fm`) and field filters are combined with **OR** semantics
/// for includes: a line is visible if any include filter — text or field — matches it.
/// Exclude filters from either source hide the line unconditionally.
/// Date filters act as strict AND constraints on the timestamp field.
///
/// Pass-through rules (field filters only):
/// - If the line cannot be parsed (e.g. a stack-trace continuation) → field filters do not apply.
/// - If the line was parsed but the named field is absent → treated as Miss (hidden).
fn line_is_visible(
    fm: &FilterManager,
    line: &[u8],
    date_filters: &[crate::date_filter::DateFilter],
    date_counts: &[std::sync::atomic::AtomicUsize],
    inc_ff: &[crate::field_filter::FieldFilter],
    exc_ff: &[crate::field_filter::FieldFilter],
    parser: Option<&dyn LogFormatParser>,
) -> bool {
    // Step 1: text filter — fast path, no parsing needed.
    let text_dec = fm.evaluate_text(line);
    if text_dec == FilterDecision::Exclude {
        return false;
    }

    // Step 2: parse the line once for date/field evaluation.
    let parts = parser.and_then(|p| p.parse_line(line));

    // Step 3: date filter — AND constraint (timestamp must fall in range).
    if !date_filters.is_empty()
        && let Some(ref p) = parts
        && let Some(ts) = p.timestamp
    {
        for (df, count) in date_filters.iter().zip(date_counts.iter()) {
            if df.matches(ts) {
                count.fetch_add(1, Ordering::Relaxed);
            }
        }
        if !crate::date_filter::matches_any(date_filters, ts) {
            return false;
        }
    }

    // Step 4: field exclude — hides the line if any matching exclude is found.
    if any_field_exclude_matches(exc_ff, parts.as_ref()) {
        return false;
    }

    // Step 5: include resolution — text include OR field include.
    if text_dec == FilterDecision::Include {
        return true;
    }

    // text_dec is Neutral; check field includes.
    if !inc_ff.is_empty() {
        return match field_include_vote(inc_ff, parts.as_ref()) {
            FieldVote::Match => true,
            FieldVote::Miss => false,
            // Pass-through: field filters don't apply to this line; fall back to
            // text-filter-only logic (visible iff there are no text include filters).
            FieldVote::PassThrough => !fm.has_include(),
        };
    }

    // No field includes; visible iff no text include filters exist.
    !fm.has_include()
}

/// Compute `error_positions` and `warning_positions` from a set of visible file-line indices.
///
/// Designed to run inside a `spawn_blocking` thread. Uses the format parser when available,
/// falling back to byte-pattern detection. Returns `(errors, warnings)` — each a vec of
/// *visible positions* (indices into `visible`, not file line indices).
fn compute_level_positions(
    visible: &[usize],
    reader: &crate::file_reader::FileReader,
    parser: Option<&dyn LogFormatParser>,
) -> (Vec<usize>, Vec<usize>) {
    use crate::types::LogLevel;
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    for (pos, &file_idx) in visible.iter().enumerate() {
        let bytes = reader.get_line(file_idx);
        let level = if let Some(p) = parser {
            if let Some(parts) = p.parse_line(bytes) {
                if let Some(level_str) = parts.level {
                    LogLevel::parse_level(level_str)
                } else {
                    LogLevel::detect_from_bytes(bytes)
                }
            } else {
                LogLevel::detect_from_bytes(bytes)
            }
        } else {
            LogLevel::detect_from_bytes(bytes)
        };
        match level {
            LogLevel::Error | LogLevel::Fatal => errors.push(pos),
            LogLevel::Warning => warnings.push(pos),
            _ => {}
        }
    }
    (errors, warnings)
}

/// Snapshot of the filter-driven view: visible indices + filter manager + styles.
/// Saved on marks-only entry and restored on exit to avoid re-running compute_visible.
type FilterViewSnapshot = (
    VisibleLines,
    Arc<FilterManager>,
    Vec<Style>,
    Vec<DateFilterStyle>,
    Vec<crate::field_filter::FieldFilterStyle>,
);

// ---------------------------------------------------------------------------
// VisibleLines
// ---------------------------------------------------------------------------

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
            Self::Filtered(v) => v.iter().position(|&i| i == line_idx),
        }
    }

    /// Iterate file-line indices for all visible positions in order.
    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        let len = self.len();
        (0..len).map(move |i| self.get(i))
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

// ---------------------------------------------------------------------------
// CachedParsedLine
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// TabState
// ---------------------------------------------------------------------------

pub struct TabState {
    pub file_reader: FileReader,
    pub log_manager: LogManager,
    /// Which file lines are currently visible under the active filters.
    pub visible_indices: VisibleLines,
    pub mode: Box<dyn Mode>,
    pub scroll_offset: usize,
    pub viewport_offset: usize,
    pub show_sidebar: bool,
    /// Width in terminal columns of the filter sidebar. Resizable in filter management mode.
    pub sidebar_width: u16,
    pub g_key_pressed: bool,
    pub wrap: bool,
    pub show_line_numbers: bool,
    pub horizontal_scroll: usize,
    pub search: Search,
    pub command_error: Option<String>,
    /// Set of log-level keys whose colour is disabled (e.g. `"trace"`, `"error"`).
    /// An empty set means all level colours are enabled.
    pub level_colors_disabled: HashSet<String>,
    pub filtering_enabled: bool,
    pub show_marks_only: bool,
    pub filter_context: Option<usize>,
    pub editing_filter_id: Option<usize>,
    pub visible_height: usize,
    pub visible_width: usize,
    pub title: String,
    pub command_history: Vec<String>,
    /// Active file watcher for this tab (None for stdin tabs or tabs not yet watching).
    pub watch_state: Option<FileWatchState>,
    /// Field names that should be hidden from display (filter evaluation still uses raw line).
    pub hidden_fields: HashSet<String>,
    /// Field selection and ordering for display.
    pub field_layout: FieldLayout,
    /// Active keybindings for this tab (shared with App, overwritten after TabState::new).
    pub keybindings: Arc<Keybindings>,
    /// Auto-detected log format parser for structured display (None = raw bytes).
    /// Stored behind an `Arc` so it can be shared with background filter tasks (O(1) clone).
    pub detected_format: Option<Arc<dyn LogFormatParser>>,
    /// When true, always scroll to the last visible line when new content arrives.
    pub tail_mode: bool,
    /// When true, incoming data from file watchers and streams is not applied to
    /// the view.  The background watcher/stream continues running so no data is
    /// lost; resuming with `continue` replays the latest snapshot.
    pub paused: bool,
    /// When true, the format parser is bypassed and lines are shown as raw bytes.
    pub raw_mode: bool,
    /// When true, structured fields (spans, extra fields) are shown as `key=value`;
    /// when false (default), only values are shown.
    pub show_keys: bool,
    /// Whether the mode bar is shown at the bottom.
    pub show_mode_bar: bool,
    /// Whether panel borders (logs, sidebar, mode bar) are drawn.
    pub show_borders: bool,
    /// Cached filter manager, rebuilt only in `refresh_visible`. Shared via `Arc` so
    /// the render path can clone the pointer (O(1)) instead of rebuilding Aho-Corasick.
    pub filter_manager_arc: Arc<FilterManager>,
    /// Filter highlight styles parallel to `filter_manager_arc`.
    pub filter_styles: Vec<Style>,
    /// Date-filter highlight styles parallel to `filter_manager_arc`.
    pub filter_date_styles: Vec<DateFilterStyle>,
    /// Field-filter highlight styles parallel to `filter_manager_arc`.
    pub filter_field_styles: Vec<crate::field_filter::FieldFilterStyle>,
    /// Filter view saved when entering marks-only mode. Restored on exit so
    /// the O(file_size) `compute_visible` scan is not repeated.
    saved_filter_view: Option<FilterViewSnapshot>,
    /// Monotonically increasing counter; bumped in `refresh_visible` and whenever the
    /// field layout or display mode changes. Cache entries with a stale generation are
    /// re-computed on the next render.
    pub parse_cache_gen: u64,
    /// Per-line parse cache: file-line index → (generation, CachedParsedLine).
    pub parse_cache: HashMap<usize, (u64, CachedParsedLine)>,
    /// In-flight background search, if one is running.
    pub search_handle: Option<SearchHandle>,
    /// In-flight background filter computation, if one is running.
    pub filter_handle: Option<FilterHandle>,
    /// Memoized result of `collect_field_names`: (parse_cache_gen, names).
    /// Invalidated automatically when `parse_cache_gen` advances.
    pub field_names_cache: Option<(u64, Vec<String>)>,
    /// Bumped whenever rendered output could change: filter/layout changes (alongside
    /// `parse_cache_gen`), value_colors toggles, or theme changes.
    pub render_cache_gen: u64,
    /// Bumped when search results are delivered or cleared.
    pub search_result_gen: u64,
    /// Cached content lines (result of evaluate_line + render_line + colorize_known_values,
    /// before applying cursor/mark/visual style and line numbers).
    /// Key: line_idx → (render_cache_gen, search_result_gen, current_occ, Line<'static>).
    pub render_line_cache: HashMap<usize, (u64, u64, Option<usize>, Line<'static>)>,
    /// Sorted visible positions (indices into `visible_indices`) of ERROR and FATAL lines.
    pub error_positions: Vec<usize>,
    /// Sorted visible positions (indices into `visible_indices`) of WARN lines.
    pub warning_positions: Vec<usize>,
    /// Per-filter match counts unified across all filter types, indexed parallel to
    /// `filter_defs` (disabled filters get count 0). Updated after each filter computation.
    pub filter_match_counts: Vec<usize>,
}

impl TabState {
    pub fn new(file_reader: FileReader, log_manager: LogManager, title: String) -> Self {
        // Sample up to 200 lines for format detection.
        let sample_limit = file_reader.line_count().min(200);
        let sample: Vec<&[u8]> = (0..sample_limit).map(|i| file_reader.get_line(i)).collect();
        let detected_format = detect_format(&sample).map(Arc::from);

        let mut tab = TabState {
            file_reader,
            log_manager,
            visible_indices: VisibleLines::default(),
            mode: Box::new(NormalMode::default()),
            scroll_offset: 0,
            viewport_offset: 0,
            show_sidebar: true,
            sidebar_width: 30,
            g_key_pressed: false,
            wrap: true,
            show_line_numbers: true,
            horizontal_scroll: 0,
            search: Search::new(),
            command_error: None,
            level_colors_disabled: ["trace", "debug", "info", "notice"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            filtering_enabled: true,
            show_marks_only: false,
            filter_context: None,
            editing_filter_id: None,
            visible_height: 0,
            visible_width: 0,
            title,
            command_history: Vec::new(),
            watch_state: None,
            hidden_fields: HashSet::new(),
            field_layout: FieldLayout::default(),
            keybindings: Arc::new(Keybindings::default()),
            detected_format,
            tail_mode: false,
            paused: false,
            raw_mode: false,
            show_keys: false,
            show_mode_bar: true,
            show_borders: true,
            filter_manager_arc: Arc::new(FilterManager::empty()),
            filter_styles: Vec::new(),
            filter_date_styles: Vec::new(),
            filter_field_styles: Vec::new(),
            saved_filter_view: None,
            parse_cache_gen: 0,
            parse_cache: HashMap::new(),
            search_handle: None,
            filter_handle: None,
            field_names_cache: None,
            render_cache_gen: 0,
            search_result_gen: 0,
            render_line_cache: HashMap::new(),
            error_positions: Vec::new(),
            warning_positions: Vec::new(),
            filter_match_counts: Vec::new(),
        };
        tab.refresh_visible();
        tab
    }

    /// Rebuilds the sorted level-position index from the current `visible_indices`.
    ///
    /// Scans every visible line, classifies its log level (parse cache → parser →
    /// byte scan), and records the visible position in `error_positions` (for
    /// ERROR / FATAL) or `warning_positions` (for WARN).
    pub fn rebuild_level_index(&mut self) {
        use crate::types::LogLevel;
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        let len = self.visible_indices.len();
        for pos in 0..len {
            let file_idx = self.visible_indices.get(pos);
            let cached_level = self.parse_cache.get(&file_idx).and_then(|(cache_gen, c)| {
                (*cache_gen == self.parse_cache_gen)
                    .then(|| c.level.as_deref().map(LogLevel::parse_level))
                    .flatten()
            });
            let level = if let Some(l) = cached_level {
                l
            } else {
                let bytes = self.file_reader.get_line(file_idx);
                if !self.raw_mode {
                    if let Some(parser) = self.detected_format.as_ref() {
                        if let Some(parts) = parser.parse_line(bytes) {
                            if let Some(level_str) = parts.level {
                                LogLevel::parse_level(level_str)
                            } else {
                                LogLevel::detect_from_bytes(bytes)
                            }
                        } else {
                            LogLevel::detect_from_bytes(bytes)
                        }
                    } else {
                        LogLevel::detect_from_bytes(bytes)
                    }
                } else {
                    LogLevel::detect_from_bytes(bytes)
                }
            };
            match level {
                LogLevel::Error | LogLevel::Fatal => errors.push(pos),
                LogLevel::Warning => warnings.push(pos),
                _ => {}
            }
        }
        self.error_positions = errors;
        self.warning_positions = warnings;
    }

    /// Recompute which file lines are visible under the current filters.
    pub fn refresh_visible(&mut self) {
        self.refresh_visible_inner();
        self.rebuild_level_index();
    }

    fn refresh_visible_inner(&mut self) {
        // Short-circuit: with no active filters and not in marks-only mode the
        // visible set is always All(n), regardless of `filtering_enabled`.
        // Skipping cache invalidation avoids reprocessing every line needlessly.
        let has_active_filters =
            self.show_marks_only || self.log_manager.get_filters().iter().any(|f| f.enabled);
        if !has_active_filters {
            self.saved_filter_view = None;
            self.visible_indices = VisibleLines::All(self.file_reader.line_count());
            self.filter_manager_arc = Arc::new(FilterManager::empty());
            self.filter_styles = Vec::new();
            self.filter_date_styles = Vec::new();
            self.filter_field_styles = Vec::new();
            self.filter_match_counts = Vec::new();
            // Still clamp scroll so it stays valid if the file shrank.
            if self.visible_indices.is_empty() {
                self.scroll_offset = 0;
            } else {
                self.scroll_offset = self.scroll_offset.min(self.visible_indices.len() - 1);
            }
            return;
        }

        // Invalidate the parse cache: field layout, filters, or file content may have changed.
        self.parse_cache_gen = self.parse_cache_gen.wrapping_add(1);
        self.parse_cache.clear();
        self.render_cache_gen = self.render_cache_gen.wrapping_add(1);
        self.render_line_cache.clear();

        if self.show_marks_only {
            // Save the pre-marks-only filter view so we can restore it in O(1) on toggle-off,
            // avoiding a full O(file_size) compute_visible scan.
            // If saved_filter_view is already Some, a filter change fired while we were already
            // in marks-only mode — the saved view is now stale, so discard it.
            if self.saved_filter_view.is_none() {
                self.saved_filter_view = Some((
                    self.visible_indices.clone(),
                    self.filter_manager_arc.clone(),
                    self.filter_styles.clone(),
                    self.filter_date_styles.clone(),
                    self.filter_field_styles.clone(),
                ));
            } else {
                self.saved_filter_view = None;
            }
            let mut indices = self.log_manager.get_marked_indices();
            indices.retain(|&i| i < self.file_reader.line_count());
            self.visible_indices = VisibleLines::Filtered(indices);
            // Rebuild filter cache so the render path always has a valid manager.
            let (fm, styles, date_filter_styles, field_filter_styles) =
                self.log_manager.build_filter_manager();
            self.filter_manager_arc = Arc::new(fm);
            self.filter_styles = styles;
            self.filter_date_styles = date_filter_styles;
            self.filter_field_styles = field_filter_styles;
            self.filter_match_counts = Vec::new();
        } else if let Some((
            saved_visible,
            saved_fm,
            saved_styles,
            saved_date_styles,
            saved_field_styles,
        )) = self.saved_filter_view.take()
        {
            // Leaving marks-only: restore the saved filter view — O(1), no file scan.
            self.visible_indices = saved_visible;
            self.filter_manager_arc = saved_fm;
            self.filter_styles = saved_styles;
            self.filter_date_styles = saved_date_styles;
            self.filter_field_styles = saved_field_styles;
        } else if !self.filtering_enabled {
            // No allocation: All(n) represents identity mapping i→i.
            self.visible_indices = VisibleLines::All(self.file_reader.line_count());
            // Keep an empty manager so the render path produces no filter highlights.
            self.filter_manager_arc = Arc::new(FilterManager::empty());
            self.filter_styles = Vec::new();
            self.filter_date_styles = Vec::new();
            self.filter_field_styles = Vec::new();
            self.filter_match_counts = Vec::new();
        } else {
            // Unified single-pass: text + date + field filters evaluated together
            // so that include filters (text and field) combine with OR semantics.
            let (fm, styles, date_filter_styles, field_filter_styles) =
                self.log_manager.build_filter_manager();
            let date_filters =
                crate::date_filter::extract_date_filters(self.log_manager.get_filters());
            let (inc_ff, exc_ff) =
                crate::field_filter::extract_field_filters(self.log_manager.get_filters());
            let field_defs =
                crate::field_filter::extract_field_filters_ordered(self.log_manager.get_filters());
            let all_filter_defs = self.log_manager.get_filters().to_vec();
            let parser = self.detected_format.as_deref();
            use rayon::prelude::*;
            use std::sync::atomic::AtomicUsize;
            let file_reader = &self.file_reader;
            let n_text_filters = fm.filter_count();
            let filter_counts: Vec<AtomicUsize> =
                (0..n_text_filters).map(|_| AtomicUsize::new(0)).collect();
            let ff_counts: Vec<AtomicUsize> =
                (0..field_defs.len()).map(|_| AtomicUsize::new(0)).collect();
            let df_counts: Vec<AtomicUsize> = (0..date_filters.len())
                .map(|_| AtomicUsize::new(0))
                .collect();
            let visible: Vec<usize> = (0..self.file_reader.line_count())
                .into_par_iter()
                .filter(|&idx| {
                    let line = file_reader.get_line(idx);
                    fm.count_line_matches(line, &filter_counts);
                    if !field_defs.is_empty() {
                        let parts = parser.and_then(|p| p.parse_line(line));
                        crate::field_filter::count_field_filter_matches(
                            &field_defs,
                            parts.as_ref(),
                            &ff_counts,
                        );
                    }
                    line_is_visible(
                        &fm,
                        line,
                        &date_filters,
                        &df_counts,
                        &inc_ff,
                        &exc_ff,
                        parser,
                    )
                })
                .collect();
            let text_counts: Vec<usize> =
                filter_counts.into_iter().map(|c| c.into_inner()).collect();
            let field_counts: Vec<usize> = ff_counts.into_iter().map(|c| c.into_inner()).collect();
            let date_counts: Vec<usize> = df_counts.into_iter().map(|c| c.into_inner()).collect();
            self.filter_match_counts =
                merge_filter_counts(&all_filter_defs, &text_counts, &field_counts, &date_counts);
            self.filter_manager_arc = Arc::new(fm);
            self.filter_styles = styles;
            self.filter_date_styles = date_filter_styles;
            self.filter_field_styles = field_filter_styles;
            self.visible_indices = VisibleLines::Filtered(visible);
        }

        // Clamp scroll_offset so it never points past the end of the new visible set.
        if self.visible_indices.is_empty() {
            self.scroll_offset = 0;
        } else {
            self.scroll_offset = self.scroll_offset.min(self.visible_indices.len() - 1);
        }
    }

    /// Returns the text that is actually displayed for `line_idx`.
    /// For structured log lines this is the rendered column string (which omits
    /// hidden fields); for raw lines it is the UTF-8 decoded bytes.
    /// This is the text the search should match against so that hidden-field
    /// content is never counted as a hit.
    pub fn get_display_text(&self, line_idx: usize) -> String {
        let bytes = self.file_reader.get_line(line_idx);
        if let Some(parser) = &self.detected_format
            && let Some(parts) = parser.parse_line(bytes)
        {
            let cols = field_layout::apply_field_layout(
                &parts,
                &self.field_layout,
                &self.hidden_fields,
                self.show_keys,
            );
            if !cols.is_empty() {
                return cols.join(" ");
            }
        }
        String::from_utf8_lossy(bytes).into_owned()
    }

    /// Build a lookup map of display text for each index yielded by `indices`.
    /// Collecting up-front allows callers to pass the map into `Search::search`
    /// without conflicting borrows on `self.search`.
    pub fn collect_display_texts(
        &self,
        indices: impl Iterator<Item = usize>,
    ) -> std::collections::HashMap<usize, String> {
        indices.map(|li| (li, self.get_display_text(li))).collect()
    }

    pub fn scroll_to_line_idx(&mut self, line_idx: usize) {
        if let Some(index) = self.visible_indices.position_of(line_idx) {
            self.scroll_offset = index;
        }
    }

    /// Scroll vertically to the current search match and, when wrap is off,
    /// also center the match occurrence horizontally.
    pub fn scroll_to_current_search_match(&mut self) {
        let Some(result) = self.search.get_current_match() else {
            return;
        };
        let line_idx = result.line_idx;
        let occurrence_idx = self.search.get_current_occurrence_index();

        let h_scroll = if !self.wrap && self.visible_width > 0 {
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
                match_center.saturating_sub(self.visible_width / 2)
            })
        } else {
            None
        };

        self.scroll_to_line_idx(line_idx);
        if let Some(h) = h_scroll {
            self.horizontal_scroll = h;
        }
    }

    /// Start a background search for `pattern` over the current visible lines.
    ///
    /// Any in-flight search is cancelled immediately.  Results are delivered
    /// via [`SearchHandle`] and polled each frame by `App::advance_search`.
    /// When `navigate` is true the view scrolls to the first match on completion.
    pub fn begin_search(&mut self, pattern: &str, forward: bool, navigate: bool) {
        // Cancel any in-flight search.
        if let Some(ref h) = self.search_handle {
            h.cancel.store(true, Ordering::Relaxed);
        }
        self.search_handle = None;

        if pattern.is_empty() {
            self.search.clear();
            self.search_result_gen = self.search_result_gen.wrapping_add(1);
            return;
        }

        let case_sensitive = self.search.is_case_sensitive();
        let regex_str = if case_sensitive {
            pattern.to_string()
        } else {
            format!("(?i){}", pattern)
        };
        let Ok(re) = regex::Regex::new(&regex_str) else {
            return;
        };

        // Pre-set the pattern so highlights appear immediately (stale results).
        self.search.set_pattern(re.clone(), forward);

        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = cancel.clone();
        let (result_tx, result_rx) = oneshot::channel();
        let (progress_tx, progress_rx) = watch::channel(0.0_f64);

        // Clone the file reader (O(1) — just increments Arc ref-counts).
        let file_reader = self.file_reader.clone();
        let visible: Vec<usize> = self.visible_indices.iter().collect();
        let total = visible.len();

        tokio::task::spawn_blocking(move || {
            use rayon::prelude::*;
            use std::sync::atomic::AtomicUsize;
            let counter = AtomicUsize::new(0);
            let re_for_search = re.clone();
            let results: Vec<SearchResult> = visible
                .par_iter()
                .copied()
                .filter_map(|line_idx| {
                    if cancel_clone.load(Ordering::Relaxed) {
                        return None;
                    }
                    let i = counter.fetch_add(1, Ordering::Relaxed);
                    if i.is_multiple_of(10_000) && total > 0 {
                        let _ = progress_tx.send(i as f64 / total as f64);
                    }
                    let line = file_reader.get_line(line_idx);
                    let text = String::from_utf8_lossy(line);
                    let matches: Vec<(usize, usize)> = re_for_search
                        .find_iter(text.as_ref())
                        .map(|m| (m.start(), m.end()))
                        .collect();
                    if matches.is_empty() {
                        None
                    } else {
                        Some(SearchResult { line_idx, matches })
                    }
                })
                .collect();
            let _ = result_tx.send((results, re));
        });

        self.search_handle = Some(SearchHandle {
            result_rx,
            cancel,
            progress_rx,
            pattern: pattern.to_string(),
            forward,
            navigate,
        });
    }

    /// Start a background filter computation over the entire file.
    ///
    /// Fast paths (no filters, marks-only, leaving marks-only, filtering disabled) run
    /// synchronously in O(1) or O(marks). The slow path (active text/date filters over
    /// the full file) spawns a [`tokio::task::spawn_blocking`] task, stores a
    /// [`FilterHandle`], and returns immediately so the UI stays responsive.
    ///
    /// Any in-flight filter computation is cancelled before the new one starts.
    pub fn begin_filter_refresh(&mut self) {
        // Cancel any in-flight filter computation.
        if let Some(ref h) = self.filter_handle {
            h.cancel.store(true, Ordering::Relaxed);
        }
        self.filter_handle = None;

        // Invalidate parse/render caches — filters or content changed.
        self.parse_cache_gen = self.parse_cache_gen.wrapping_add(1);
        self.parse_cache.clear();
        self.render_cache_gen = self.render_cache_gen.wrapping_add(1);
        self.render_line_cache.clear();

        let has_active_filters =
            self.show_marks_only || self.log_manager.get_filters().iter().any(|f| f.enabled);

        if !has_active_filters {
            // Fast path: no filters — O(1), no allocation.
            self.saved_filter_view = None;
            self.visible_indices = VisibleLines::All(self.file_reader.line_count());
            self.filter_manager_arc = Arc::new(FilterManager::empty());
            self.filter_styles = Vec::new();
            self.filter_date_styles = Vec::new();
            self.filter_field_styles = Vec::new();
            self.filter_match_counts = Vec::new();
            if self.visible_indices.is_empty() {
                self.scroll_offset = 0;
            } else {
                self.scroll_offset = self.scroll_offset.min(self.visible_indices.len() - 1);
            }
            return;
        }

        if self.show_marks_only {
            // Marks-only: O(marks count) — sync.
            if self.saved_filter_view.is_none() {
                self.saved_filter_view = Some((
                    self.visible_indices.clone(),
                    self.filter_manager_arc.clone(),
                    self.filter_styles.clone(),
                    self.filter_date_styles.clone(),
                    self.filter_field_styles.clone(),
                ));
            } else {
                self.saved_filter_view = None;
            }
            let mut indices = self.log_manager.get_marked_indices();
            indices.retain(|&i| i < self.file_reader.line_count());
            self.visible_indices = VisibleLines::Filtered(indices);
            let (fm, styles, date_filter_styles, field_filter_styles) =
                self.log_manager.build_filter_manager();
            self.filter_manager_arc = Arc::new(fm);
            self.filter_styles = styles;
            self.filter_date_styles = date_filter_styles;
            self.filter_field_styles = field_filter_styles;
            self.filter_match_counts = Vec::new();
            if self.visible_indices.is_empty() {
                self.scroll_offset = 0;
            } else {
                self.scroll_offset = self.scroll_offset.min(self.visible_indices.len() - 1);
            }
            return;
        }

        if let Some((
            saved_visible,
            saved_fm,
            saved_styles,
            saved_date_styles,
            saved_field_styles,
        )) = self.saved_filter_view.take()
        {
            // Leaving marks-only: restore saved filter view — O(1).
            self.visible_indices = saved_visible;
            self.filter_manager_arc = saved_fm;
            self.filter_styles = saved_styles;
            self.filter_date_styles = saved_date_styles;
            self.filter_field_styles = saved_field_styles;
            if self.visible_indices.is_empty() {
                self.scroll_offset = 0;
            } else {
                self.scroll_offset = self.scroll_offset.min(self.visible_indices.len() - 1);
            }
            return;
        }

        if !self.filtering_enabled {
            // Filtering disabled: show all lines — O(1).
            self.visible_indices = VisibleLines::All(self.file_reader.line_count());
            self.filter_manager_arc = Arc::new(FilterManager::empty());
            self.filter_styles = Vec::new();
            self.filter_date_styles = Vec::new();
            self.filter_field_styles = Vec::new();
            self.filter_match_counts = Vec::new();
            if self.visible_indices.is_empty() {
                self.scroll_offset = 0;
            } else {
                self.scroll_offset = self.scroll_offset.min(self.visible_indices.len() - 1);
            }
            return;
        }

        // Slow path: active text/date filters require a full file scan.
        let (fm, styles, date_filter_styles, field_filter_styles) =
            self.log_manager.build_filter_manager();
        // Update immediately so render highlights reflect new filters before results arrive.
        self.filter_manager_arc = Arc::new(fm);
        self.filter_styles = styles;
        self.filter_date_styles = date_filter_styles;
        self.filter_field_styles = field_filter_styles;
        // Clear stale counts — the filter order may have changed (e.g. reorder) so old
        // index-based counts would map to the wrong filters until the scan completes.
        self.filter_match_counts = Vec::new();

        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = cancel.clone();
        let (result_tx, result_rx) = oneshot::channel();
        let (progress_tx, progress_rx) = watch::channel(0.0_f64);

        let file_reader = self.file_reader.clone();
        let fm_arc = self.filter_manager_arc.clone();
        let date_filters = crate::date_filter::extract_date_filters(self.log_manager.get_filters());
        let (inc_ff, exc_ff) =
            crate::field_filter::extract_field_filters(self.log_manager.get_filters());
        let field_defs =
            crate::field_filter::extract_field_filters_ordered(self.log_manager.get_filters());
        let all_filter_defs = self.log_manager.get_filters().to_vec();
        let parser = if self.raw_mode {
            None
        } else {
            self.detected_format.clone()
        };
        let line_count = self.file_reader.line_count();
        let n_text_filters = self.filter_manager_arc.filter_count();

        tokio::task::spawn_blocking(move || {
            use rayon::prelude::*;
            use std::sync::atomic::AtomicUsize;

            let counter = AtomicUsize::new(0);
            let parser_ref: Option<&dyn LogFormatParser> = parser.as_deref();
            let filter_counts: Vec<AtomicUsize> =
                (0..n_text_filters).map(|_| AtomicUsize::new(0)).collect();
            let ff_counts: Vec<AtomicUsize> =
                (0..field_defs.len()).map(|_| AtomicUsize::new(0)).collect();
            let df_counts: Vec<AtomicUsize> = (0..date_filters.len())
                .map(|_| AtomicUsize::new(0))
                .collect();

            // Unified single pass: text + date + field filters evaluated together.
            // Per-filter match counts are piggybacked on this pass at negligible cost.
            let visible: Vec<usize> = (0..line_count)
                .into_par_iter()
                .filter(|&i| {
                    if cancel_clone.load(Ordering::Relaxed) {
                        return false;
                    }
                    let n = counter.fetch_add(1, Ordering::Relaxed);
                    if n.is_multiple_of(10_000) && line_count > 0 {
                        let _ = progress_tx.send(n as f64 / line_count as f64);
                    }
                    let line = file_reader.get_line(i);
                    fm_arc.count_line_matches(line, &filter_counts);
                    if !field_defs.is_empty() {
                        let parts = parser_ref.and_then(|p| p.parse_line(line));
                        crate::field_filter::count_field_filter_matches(
                            &field_defs,
                            parts.as_ref(),
                            &ff_counts,
                        );
                    }
                    line_is_visible(
                        &fm_arc,
                        line,
                        &date_filters,
                        &df_counts,
                        &inc_ff,
                        &exc_ff,
                        parser_ref,
                    )
                })
                .collect();

            if cancel_clone.load(Ordering::Relaxed) {
                return;
            }

            // Signal 100% so the UI transitions from "X%" to "Indexing…" while
            // compute_level_positions runs — prevents appearing stuck near 99%.
            let _ = progress_tx.send(1.0);

            let text_counts: Vec<usize> =
                filter_counts.into_iter().map(|c| c.into_inner()).collect();
            let field_counts: Vec<usize> = ff_counts.into_iter().map(|c| c.into_inner()).collect();
            let date_counts: Vec<usize> = df_counts.into_iter().map(|c| c.into_inner()).collect();
            let unified =
                merge_filter_counts(&all_filter_defs, &text_counts, &field_counts, &date_counts);

            // Compute level positions from the visible set in the same background thread
            // so the event loop never does O(visible) work synchronously.
            let (error_positions, warning_positions) =
                compute_level_positions(&visible, &file_reader, parser_ref);

            if !cancel_clone.load(Ordering::Relaxed) {
                let _ = result_tx.send(FilterComputeResult {
                    visible,
                    error_positions,
                    warning_positions,
                    filter_match_counts: Some(unified),
                });
            }
        });

        self.filter_handle = Some(FilterHandle {
            result_rx,
            cancel,
            progress_rx,
        });
    }

    /// Offload the level-position computation for the current `visible_indices` to a
    /// background thread, reusing the [`FilterHandle`] mechanism.
    ///
    /// Used after the startup single-pass sets `visible_indices` synchronously so that
    /// `rebuild_level_index` never runs on the event loop for large files.
    pub fn begin_level_index_rebuild(&mut self) {
        if let Some(ref h) = self.filter_handle {
            h.cancel.store(true, Ordering::Relaxed);
        }
        self.filter_handle = None;

        let visible: Vec<usize> = self.visible_indices.iter().collect();
        let file_reader = self.file_reader.clone();
        let parser = if self.raw_mode {
            None
        } else {
            self.detected_format.clone()
        };

        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = cancel.clone();
        let (result_tx, result_rx) = oneshot::channel();
        let (_, progress_rx) = watch::channel(1.0_f64);

        tokio::task::spawn_blocking(move || {
            if cancel_clone.load(Ordering::Relaxed) {
                return;
            }
            let parser_ref: Option<&dyn LogFormatParser> = parser.as_deref();
            let (error_positions, warning_positions) =
                compute_level_positions(&visible, &file_reader, parser_ref);
            if !cancel_clone.load(Ordering::Relaxed) {
                let _ = result_tx.send(FilterComputeResult {
                    visible,
                    error_positions,
                    warning_positions,
                    filter_match_counts: None,
                });
            }
        });

        self.filter_handle = Some(FilterHandle {
            result_rx,
            cancel,
            progress_rx,
        });
    }

    /// Jump to a 1-based line number, or the closest visible line if the
    /// target is hidden by filters.  Returns an error message when the
    /// line number is invalid (zero).
    pub fn goto_line(&mut self, line_number: usize) -> Result<(), String> {
        if line_number == 0 {
            return Err("Line numbers start at 1".to_string());
        }
        if self.visible_indices.is_empty() {
            return Ok(());
        }
        let target_idx = line_number - 1; // convert to 0-based file index

        // Binary search for the target in visible_indices.
        match self.visible_indices.binary_search(target_idx) {
            Ok(pos) => {
                // Exact match — the line is visible.
                self.scroll_offset = pos;
            }
            Err(pos) => {
                // `pos` is where target_idx would be inserted.
                // Pick the closer neighbour.
                let before = if pos > 0 { Some(pos - 1) } else { None };
                let after = if pos < self.visible_indices.len() {
                    Some(pos)
                } else {
                    None
                };
                let best = match (before, after) {
                    (Some(b), Some(a)) => {
                        let dist_b = target_idx - self.visible_indices.get(b);
                        let dist_a = self.visible_indices.get(a) - target_idx;
                        if dist_b <= dist_a { b } else { a }
                    }
                    (Some(b), None) => b,
                    (None, Some(a)) => a,
                    (None, None) => unreachable!(), // visible_indices is non-empty
                };
                self.scroll_offset = best;
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
        use crate::filters::{FilterDecision, MatchCollector, build_filter};
        use rayon::prelude::*;
        if let Some(filter) = build_filter(pattern, FilterDecision::Include, true, 0) {
            let file_reader = &self.file_reader;
            let indices: Vec<usize> = self.visible_indices.iter().collect();
            let new_visible: Vec<usize> = indices
                .par_iter()
                .copied()
                .filter(|&line_idx| {
                    let line = file_reader.get_line(line_idx);
                    let mut dummy = MatchCollector::new(line);
                    matches!(filter.evaluate(line, &mut dummy), FilterDecision::Include)
                })
                .collect();
            self.visible_indices = VisibleLines::Filtered(new_visible);
        }
        // Rebuild filter manager cache so the render path sees the updated filters.
        let (fm, styles, date_filter_styles, field_filter_styles) =
            self.log_manager.build_filter_manager();
        self.filter_manager_arc = Arc::new(fm);
        self.filter_styles = styles;
        self.filter_date_styles = date_filter_styles;
        self.filter_field_styles = field_filter_styles;
        // Invalidate parse cache (filter change affects highlight output).
        self.parse_cache_gen = self.parse_cache_gen.wrapping_add(1);
        self.parse_cache.clear();
        // Clamp scroll.
        if self.visible_indices.is_empty() {
            self.scroll_offset = 0;
        } else {
            self.scroll_offset = self.scroll_offset.min(self.visible_indices.len() - 1);
        }
        // Kick off a background refresh to compute per-filter match counts and level
        // positions. The visible set computed above stays until the background result
        // arrives, at which point it is confirmed (same filters) and counts are applied.
        self.begin_filter_refresh();
    }

    /// Apply a new exclude filter incrementally against the currently visible lines,
    /// avoiding a full `compute_visible` scan of the entire file.
    ///
    /// Only safe for pure-text exclude additions when no include-filter-only changes are needed.
    /// The filter manager cache is rebuilt afterward so render highlights stay correct.
    pub fn apply_incremental_exclude(&mut self, pattern: &str) {
        use crate::filters::{FilterDecision, MatchCollector, build_filter};
        use rayon::prelude::*;
        if let Some(filter) = build_filter(pattern, FilterDecision::Exclude, true, 0) {
            let file_reader = &self.file_reader;
            let indices: Vec<usize> = self.visible_indices.iter().collect();
            let new_visible: Vec<usize> = indices
                .par_iter()
                .copied()
                .filter(|&line_idx| {
                    let line = file_reader.get_line(line_idx);
                    let mut dummy = MatchCollector::new(line);
                    !matches!(filter.evaluate(line, &mut dummy), FilterDecision::Exclude)
                })
                .collect();
            self.visible_indices = VisibleLines::Filtered(new_visible);
        }
        // Rebuild filter manager cache so the render path sees the updated filters.
        let (fm, styles, date_filter_styles, field_filter_styles) =
            self.log_manager.build_filter_manager();
        self.filter_manager_arc = Arc::new(fm);
        self.filter_styles = styles;
        self.filter_date_styles = date_filter_styles;
        self.filter_field_styles = field_filter_styles;
        // Invalidate parse cache (filter change affects highlight output).
        self.parse_cache_gen = self.parse_cache_gen.wrapping_add(1);
        self.parse_cache.clear();
        // Clamp scroll.
        if self.visible_indices.is_empty() {
            self.scroll_offset = 0;
        } else {
            self.scroll_offset = self.scroll_offset.min(self.visible_indices.len() - 1);
        }
        // Kick off a background refresh to compute per-filter match counts and level
        // positions. The visible set computed above stays until the background result
        // arrives, at which point it is confirmed (same filters) and counts are applied.
        self.begin_filter_refresh();
    }

    /// Bump the parse cache generation so that all cached render outputs are re-computed
    /// on the next frame. Call this whenever the field layout or display mode changes.
    pub fn invalidate_parse_cache(&mut self) {
        self.parse_cache_gen = self.parse_cache_gen.wrapping_add(1);
        self.parse_cache.clear();
        self.render_cache_gen = self.render_cache_gen.wrapping_add(1);
        self.render_line_cache.clear();
    }

    pub fn to_file_context(&self) -> Option<FileContext> {
        let source = self.log_manager.source_file()?;
        let marked_lines = self.log_manager.get_marked_indices();
        let comments = self.log_manager.get_comments().to_vec();
        let file_hash = LogManager::compute_file_hash(source);
        Some(FileContext {
            source_file: source.to_string(),
            scroll_offset: self.scroll_offset,
            search_query: self.search.get_pattern().unwrap_or_default().to_string(),
            wrap: self.wrap,
            level_colors_disabled: self.level_colors_disabled.clone(),
            show_sidebar: self.show_sidebar,
            horizontal_scroll: self.horizontal_scroll,
            marked_lines,
            file_hash,
            show_line_numbers: self.show_line_numbers,
            comments,
            show_mode_bar: self.show_mode_bar,
            show_borders: self.show_borders,
            show_keys: self.show_keys,
            raw_mode: self.raw_mode,
            sidebar_width: self.sidebar_width,
        })
    }

    pub fn apply_file_context(&mut self, ctx: &FileContext) {
        self.scroll_offset = ctx.scroll_offset;
        self.wrap = ctx.wrap;
        self.level_colors_disabled = ctx.level_colors_disabled.clone();
        self.show_sidebar = ctx.show_sidebar;
        self.show_line_numbers = ctx.show_line_numbers;
        self.horizontal_scroll = ctx.horizontal_scroll;
        self.show_mode_bar = ctx.show_mode_bar;
        self.show_borders = ctx.show_borders;
        self.show_keys = ctx.show_keys;
        self.raw_mode = ctx.raw_mode;
        self.sidebar_width = ctx.sidebar_width;
        if !ctx.marked_lines.is_empty() {
            self.log_manager.set_marks(ctx.marked_lines.clone());
        }
        if !ctx.comments.is_empty() {
            self.log_manager.set_comments(ctx.comments.clone());
        }
        if !ctx.search_query.is_empty() {
            let visible = self.visible_indices.clone();
            let texts = self.collect_display_texts(visible.iter());
            let _ = self.search.search(&ctx.search_query, visible.iter(), |li| {
                texts.get(&li).cloned()
            });
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
        let current_gen = self.parse_cache_gen;
        if let Some((cached_gen, ref names)) = self.field_names_cache
            && cached_gen == current_gen
        {
            return names.clone();
        }
        let names = self.compute_field_names();
        self.field_names_cache = Some((current_gen, names.clone()));
        names
    }

    fn compute_field_names(&self) -> Vec<String> {
        let parser = match &self.detected_format {
            Some(p) => p,
            None => return Vec::new(),
        };
        const SAMPLE_LIMIT: usize = 200;
        let limit = self.visible_indices.len().min(SAMPLE_LIMIT);
        let lines: Vec<&[u8]> = (0..limit)
            .map(|i| self.file_reader.get_line(self.visible_indices.get(i)))
            .collect();
        parser.collect_field_names(&lines)
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

        let Some(parser) = &self.detected_format else {
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
        let no_sample: HashSet<&str> = crate::parser::json::TIMESTAMP_KEYS
            .iter()
            .chain(crate::parser::json::MESSAGE_KEYS.iter())
            .copied()
            .collect();

        // Step 2: Scan raw lines to collect values and per-name frequency counts.
        let mut name_freq: HashMap<String, usize> = HashMap::new();
        let mut value_map: HashMap<String, HashSet<String>> = HashMap::new();

        for i in 0..limit {
            let line = self.file_reader.get_line(i);
            let Some(parts) = parser.parse_line(line) else {
                continue;
            };
            for name in &names {
                if let Some(v) = crate::field_filter::resolve_field(name, &parts) {
                    *name_freq.entry(name.clone()).or_insert(0) += 1;
                    if !no_sample.contains(name.as_str()) {
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
            .field("mode", &self.mode)
            .field("scroll_offset", &self.scroll_offset)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// FileLoadState / LoadContext
// ---------------------------------------------------------------------------

/// What to do once a background file load completes.
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
    /// Delivers the finished [`crate::file_reader::FileLoadResult`] (or error) when indexing is done.
    pub result_rx:
        tokio::sync::oneshot::Receiver<std::io::Result<crate::file_reader::FileLoadResult>>,
    pub total_bytes: u64,
    pub on_complete: LoadContext,
    /// Set to `true` to abort the in-flight indexing task early (e.g. on tab close).
    pub cancel: Arc<AtomicBool>,
}

/// Tracks an in-progress stdin stream.  Kept separate from `file_load_state`
/// so session-restore loads cannot overwrite it.
pub struct StdinLoadState {
    /// Receives snapshots of all complete lines accumulated so far.
    /// Updated every second.  When the sender is dropped stdin has closed.
    pub snapshot_rx: tokio::sync::watch::Receiver<Vec<u8>>,
}

/// Per-tab state for watching a file for new appended content.
pub struct FileWatchState {
    /// Receives stripped byte chunks whenever new lines are appended to the file.
    pub new_data_rx: tokio::sync::watch::Receiver<Vec<u8>>,
}

pub use app::App;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Database, FileContext};

    #[test]
    fn test_list_dir_files_basic() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::write(dir.join("b.log"), b"b").unwrap();
        std::fs::write(dir.join("a.log"), b"a").unwrap();
        let files = list_dir_files(dir.to_str().unwrap());
        assert_eq!(files.len(), 2);
        // sorted by name
        assert!(files[0].ends_with("a.log"));
        assert!(files[1].ends_with("b.log"));
    }

    #[test]
    fn test_list_dir_files_excludes_hidden() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::write(dir.join("visible.log"), b"v").unwrap();
        std::fs::write(dir.join(".hidden"), b"h").unwrap();
        let files = list_dir_files(dir.to_str().unwrap());
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("visible.log"));
    }

    #[test]
    fn test_list_dir_files_excludes_subdirs() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::write(dir.join("file.log"), b"f").unwrap();
        std::fs::create_dir(dir.join("subdir")).unwrap();
        let files = list_dir_files(dir.to_str().unwrap());
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("file.log"));
    }

    #[test]
    fn test_list_dir_files_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let files = list_dir_files(tmp.path().to_str().unwrap());
        assert!(files.is_empty());
    }

    #[test]
    fn test_list_dir_files_nonexistent() {
        let files = list_dir_files("/nonexistent/path/xyz123");
        assert!(files.is_empty());
    }
    use crate::file_reader::FileReader;
    use crate::log_manager::LogManager;
    use crate::types::{Comment, FilterType};
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
        assert_eq!(tab.visible_indices.len(), 5);
    }

    #[tokio::test]
    async fn test_refresh_visible_marks_only() {
        let mut tab = make_tab(&["line1", "line2", "line3", "line4", "line5"]).await;
        tab.log_manager.toggle_mark(0);
        tab.log_manager.toggle_mark(2);
        tab.show_marks_only = true;
        tab.refresh_visible();
        assert_eq!(tab.visible_indices, VisibleLines::Filtered(vec![0, 2]));
    }

    #[tokio::test]
    async fn test_refresh_visible_filtering_disabled() {
        let mut tab = make_tab(&["line1", "line2", "line3", "line4", "line5"]).await;
        tab.log_manager
            .add_filter_with_color("line1".to_string(), FilterType::Include, None, None, false)
            .await;
        tab.filtering_enabled = false;
        tab.refresh_visible();
        assert_eq!(tab.visible_indices.len(), 5);
    }

    #[tokio::test]
    async fn test_refresh_visible_empty_file() {
        let tab = make_tab(&[]).await;
        assert!(tab.visible_indices.is_empty());
        assert_eq!(tab.scroll_offset, 0);
    }

    #[tokio::test]
    async fn test_refresh_visible_clamps_scroll() {
        let mut tab = make_tab(&["line1", "line2", "line3"]).await;
        tab.scroll_offset = 10;
        tab.refresh_visible();
        assert_eq!(tab.scroll_offset, 2);
    }

    #[tokio::test]
    async fn test_scroll_to_line_idx_found() {
        let mut tab = make_tab(&["line1", "line2", "line3", "line4", "line5"]).await;
        tab.scroll_to_line_idx(2);
        assert_eq!(tab.scroll_offset, 2);
    }

    #[tokio::test]
    async fn test_scroll_to_line_idx_not_found() {
        let mut tab = make_tab(&["line1", "line2", "line3", "line4", "line5"]).await;
        tab.scroll_to_line_idx(999);
        assert_eq!(tab.scroll_offset, 0);
    }

    #[tokio::test]
    async fn test_scroll_to_current_search_match_centers_horizontally() {
        // Line with 100 leading spaces before "needle" — match starts at byte 100.
        let line = format!("{}needle", " ".repeat(100));
        let mut tab = make_tab(&[&line]).await;
        tab.wrap = false;
        tab.visible_width = 40;
        // Build search results manually and point the cursor at them.
        let visible = tab.visible_indices.clone();
        let texts = tab.collect_display_texts(visible.iter());
        tab.search
            .search("needle", visible.iter(), |li| texts.get(&li).cloned())
            .unwrap();
        tab.search.set_forward(true);
        tab.search.next_match();
        tab.scroll_to_current_search_match();
        // match_center ≈ 100 + 3 = 103 (col of "needle" start + half of 6-char width)
        // expected h_scroll = 103 - 20 = 83
        assert_eq!(tab.scroll_offset, 0);
        assert_eq!(tab.horizontal_scroll, 83);
    }

    #[tokio::test]
    async fn test_scroll_to_current_search_match_no_hscroll_when_wrapped() {
        let line = format!("{}needle", " ".repeat(100));
        let mut tab = make_tab(&[&line]).await;
        tab.wrap = true;
        tab.visible_width = 40;
        tab.horizontal_scroll = 0;
        let visible = tab.visible_indices.clone();
        let texts = tab.collect_display_texts(visible.iter());
        tab.search
            .search("needle", visible.iter(), |li| texts.get(&li).cloned())
            .unwrap();
        tab.search.set_forward(true);
        tab.search.next_match();
        tab.scroll_to_current_search_match();
        // wrap=true → horizontal scroll must not change
        assert_eq!(tab.horizontal_scroll, 0);
    }

    #[tokio::test]
    async fn test_to_file_context_with_source() {
        let tab = make_tab_with_source(&["line1", "line2", "line3"], "test.log").await;
        let ctx = tab.to_file_context();
        assert!(ctx.is_some());
        let ctx = ctx.unwrap();
        assert_eq!(ctx.source_file, "test.log");
        assert_eq!(ctx.scroll_offset, 0);
        assert!(ctx.wrap);
        let expected_disabled: std::collections::HashSet<String> =
            ["trace", "debug", "info", "notice"]
                .iter()
                .map(|s| s.to_string())
                .collect();
        assert_eq!(ctx.level_colors_disabled, expected_disabled);
        assert!(ctx.show_sidebar);
        assert!(ctx.show_line_numbers);
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
            wrap: false,
            level_colors_disabled: all_disabled.clone(),
            show_sidebar: false,
            horizontal_scroll: 5,
            marked_lines: vec![0, 2],
            file_hash: None,
            show_line_numbers: false,
            comments: vec![Comment {
                text: "test".to_string(),
                line_indices: vec![0],
            }],
            show_mode_bar: false,
            show_borders: false,
            show_keys: false,
            raw_mode: false,
            sidebar_width: 30,
        };
        tab.apply_file_context(&ctx);
        assert_eq!(tab.scroll_offset, 3);
        assert!(!tab.wrap);
        assert_eq!(tab.level_colors_disabled, all_disabled);
        assert!(!tab.show_sidebar);
        assert!(!tab.show_line_numbers);
        assert_eq!(tab.horizontal_scroll, 5);
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
            wrap: true,
            level_colors_disabled: HashSet::new(),
            show_sidebar: true,
            horizontal_scroll: 0,
            marked_lines: vec![],
            file_hash: None,
            show_line_numbers: true,
            comments: vec![],
            show_mode_bar: true,
            show_borders: true,
            show_keys: false,
            raw_mode: false,
            sidebar_width: 30,
        };
        tab.apply_file_context(&ctx);
        assert!(tab.wrap);
        assert!(tab.level_colors_disabled.is_empty());
        assert!(tab.show_sidebar);
        assert!(tab.show_line_numbers);
        assert_eq!(tab.scroll_offset, 0);
        assert_eq!(tab.horizontal_scroll, 0);
        assert!(!tab.log_manager.is_marked(0));
        assert!(!tab.log_manager.has_comment(0));
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
        assert!(fields.contains(&"msg".to_string()));
    }

    #[tokio::test]
    async fn test_collect_field_names_cached() {
        let mut tab = make_tab(&[r#"{"level":"INFO","msg":"hello"}"#]).await;
        let first = tab.collect_field_names();
        let gen_before = tab.parse_cache_gen;
        let second = tab.collect_field_names();
        // Result must be identical and the gen must not have changed (cache hit).
        assert_eq!(first, second);
        assert_eq!(tab.parse_cache_gen, gen_before);
        // After invalidating the cache the result is recomputed but still equal.
        tab.invalidate_parse_cache();
        let third = tab.collect_field_names();
        assert_eq!(first, third);
    }

    #[tokio::test]
    async fn test_new_tab_detects_format() {
        let tab = make_tab(&[r#"{"level":"INFO","msg":"hello"}"#]).await;
        assert!(tab.detected_format.is_some());
    }

    #[tokio::test]
    async fn test_new_tab_plain_text_no_format() {
        let tab = make_tab(&["just plain text", "no structure here"]).await;
        assert!(tab.detected_format.is_none());
    }

    #[tokio::test]
    async fn test_goto_line_exact_visible() {
        let mut tab = make_tab(&["a", "b", "c", "d", "e"]).await;
        // All lines visible (indices 0..5), go to line 3 (0-based idx 2)
        tab.goto_line(3).unwrap();
        assert_eq!(tab.scroll_offset, 2);
    }

    #[tokio::test]
    async fn test_goto_line_first_line() {
        let mut tab = make_tab(&["a", "b", "c"]).await;
        tab.scroll_offset = 2;
        tab.goto_line(1).unwrap();
        assert_eq!(tab.scroll_offset, 0);
    }

    #[tokio::test]
    async fn test_goto_line_last_line() {
        let mut tab = make_tab(&["a", "b", "c", "d", "e"]).await;
        tab.goto_line(5).unwrap();
        assert_eq!(tab.scroll_offset, 4);
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
        assert_eq!(tab.scroll_offset, 2); // last visible line
    }

    #[tokio::test]
    async fn test_goto_line_hidden_finds_closest() {
        let mut tab = make_tab(&["a", "b", "c", "d", "e"]).await;
        // Simulate filter hiding lines 1 and 2 (keep 0, 3, 4)
        tab.visible_indices = VisibleLines::Filtered(vec![0, 3, 4]);
        // Go to line 2 (idx 1) — hidden, closest visible is idx 0
        tab.goto_line(2).unwrap();
        assert_eq!(tab.scroll_offset, 0); // idx 0 is at position 0
    }

    #[tokio::test]
    async fn test_goto_line_hidden_prefers_closer_after() {
        let mut tab = make_tab(&["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"]).await;
        // Visible: 0, 5, 9
        tab.visible_indices = VisibleLines::Filtered(vec![0, 5, 9]);
        // Go to line 4 (idx 3) — equidistant: idx 0 (dist 3) vs idx 5 (dist 2) → pick 5
        tab.goto_line(4).unwrap();
        assert_eq!(tab.scroll_offset, 1); // idx 5 is at position 1
    }

    #[tokio::test]
    async fn test_goto_line_hidden_prefers_closer_before() {
        let mut tab = make_tab(&["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"]).await;
        tab.visible_indices = VisibleLines::Filtered(vec![0, 5, 9]);
        // Go to line 7 (idx 6) — idx 5 (dist 1) vs idx 9 (dist 3) → pick 5
        tab.goto_line(7).unwrap();
        assert_eq!(tab.scroll_offset, 1); // idx 5 is at position 1
    }

    #[tokio::test]
    async fn test_goto_line_empty_visible_indices() {
        let mut tab = make_tab(&["a", "b"]).await;
        tab.visible_indices = VisibleLines::Filtered(vec![]);
        // Should not panic, just no-op
        tab.goto_line(1).unwrap();
        assert_eq!(tab.scroll_offset, 0);
    }

    // ── show_mode_bar / show_borders ───────────────────────────────────

    #[tokio::test]
    async fn test_tabstate_show_mode_bar_default_true() {
        let tab = make_tab(&["line"]).await;
        assert!(tab.show_mode_bar);
    }

    #[tokio::test]
    async fn test_tabstate_show_borders_default_true() {
        let tab = make_tab(&["line"]).await;
        assert!(tab.show_borders);
    }

    #[tokio::test]
    async fn test_apply_file_context_restores_show_mode_bar() {
        let mut tab = make_tab_with_source(&["line"], "test.log").await;
        let ctx = FileContext {
            source_file: "test.log".to_string(),
            scroll_offset: 0,
            search_query: String::new(),
            wrap: true,
            level_colors_disabled: HashSet::new(),
            show_sidebar: true,
            horizontal_scroll: 0,
            marked_lines: vec![],
            file_hash: None,
            show_line_numbers: true,
            comments: vec![],
            show_mode_bar: false,
            show_borders: true,
            show_keys: false,
            raw_mode: false,
            sidebar_width: 30,
        };
        tab.apply_file_context(&ctx);
        assert!(!tab.show_mode_bar);
    }

    #[tokio::test]
    async fn test_apply_file_context_restores_show_borders() {
        let mut tab = make_tab_with_source(&["line"], "test.log").await;
        let ctx = FileContext {
            source_file: "test.log".to_string(),
            scroll_offset: 0,
            search_query: String::new(),
            wrap: true,
            level_colors_disabled: HashSet::new(),
            show_sidebar: true,
            horizontal_scroll: 0,
            marked_lines: vec![],
            file_hash: None,
            show_line_numbers: true,
            comments: vec![],
            show_mode_bar: true,
            show_borders: false,
            show_keys: false,
            raw_mode: false,
            sidebar_width: 30,
        };
        tab.apply_file_context(&ctx);
        assert!(!tab.show_borders);
    }

    // ── date filter integration with refresh_visible ──────────────────
    // OR combination logic is unit-tested in date_filter::tests::matches_any.
    // These tests verify that refresh_visible correctly applies date filters.

    async fn make_tab_with_date_filter(lines: &[&str], expr: &str) -> TabState {
        let mut tab = make_tab(lines).await;
        let pattern = format!("{}{}", crate::date_filter::DATE_PREFIX, expr);
        tab.log_manager
            .add_filter_with_color(pattern, FilterType::Include, None, None, true)
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
        assert_eq!(tab.visible_indices, VisibleLines::Filtered(vec![0]));
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
            let pattern = format!("{}{}", crate::date_filter::DATE_PREFIX, expr);
            tab.log_manager
                .add_filter_with_color(pattern, FilterType::Include, None, None, true)
                .await;
        }
        tab.refresh_visible();
        // Lines in either range are visible; the line between is hidden.
        assert_eq!(tab.visible_indices, VisibleLines::Filtered(vec![0, 2]));
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
        assert_eq!(tab.visible_indices.len(), 1);
        assert_eq!(tab.visible_indices.get(0), 1);
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
        assert_eq!(tab.visible_indices.len(), 1);
        assert_eq!(tab.visible_indices.get(0), 1);
    }

    #[tokio::test]
    async fn test_refresh_visible_populates_filter_cache() {
        let mut tab = make_tab(&["error line", "info line", "error again"]).await;
        tab.log_manager
            .add_filter_with_color("error".to_string(), FilterType::Include, None, None, true)
            .await;
        tab.refresh_visible();
        // Cache is set and reflects the filter.
        assert!(tab.filter_manager_arc.is_visible(b"error line"));
        assert!(!tab.filter_manager_arc.is_visible(b"info line"));
    }

    #[tokio::test]
    async fn test_filtering_disabled_cache_is_empty_manager() {
        let mut tab = make_tab(&["error line", "info line"]).await;
        tab.log_manager
            .add_filter_with_color("error".to_string(), FilterType::Include, None, None, true)
            .await;
        tab.filtering_enabled = false;
        tab.refresh_visible();
        // When filtering is disabled the cached manager is empty (everything visible).
        assert!(tab.filter_manager_arc.is_visible(b"info line"));
        assert!(tab.filter_styles.is_empty());
    }

    #[tokio::test]
    async fn test_refresh_visible_increments_parse_cache_gen() {
        let mut tab = make_tab(&["line"]).await;
        tab.log_manager
            .add_filter_with_color("line".to_string(), FilterType::Include, None, None, true)
            .await;
        let old_gen = tab.parse_cache_gen;
        tab.refresh_visible();
        assert!(tab.parse_cache_gen > old_gen);
    }

    #[tokio::test]
    async fn test_invalidate_parse_cache_increments_gen() {
        let mut tab = make_tab(&["line"]).await;
        let old_gen = tab.parse_cache_gen;
        tab.invalidate_parse_cache();
        assert!(tab.parse_cache_gen > old_gen);
        assert!(tab.parse_cache.is_empty());
    }

    #[tokio::test]
    async fn test_apply_incremental_include_narrows_visible() {
        let mut tab = make_tab(&["error line", "info line", "error again", "debug line"]).await;
        assert_eq!(tab.visible_indices.len(), 4);
        tab.log_manager
            .add_filter_with_color("error".to_string(), FilterType::Include, None, None, true)
            .await;
        // Only lines containing "error" should remain.
        tab.apply_incremental_include("error");
        assert_eq!(tab.visible_indices.len(), 2);
        assert_eq!(tab.visible_indices.get(0), 0);
        assert_eq!(tab.visible_indices.get(1), 2);
    }

    #[tokio::test]
    async fn test_apply_incremental_include_updates_filter_cache() {
        let mut tab = make_tab(&["line a", "line b"]).await;
        tab.log_manager
            .add_filter_with_color("line a".to_string(), FilterType::Include, None, None, true)
            .await;
        let old_gen = tab.parse_cache_gen;
        tab.apply_incremental_include("line a");
        // Parse cache generation must be bumped.
        assert!(tab.parse_cache_gen > old_gen);
        assert_eq!(tab.visible_indices.len(), 1);
        assert_eq!(tab.visible_indices.get(0), 0);
    }

    #[tokio::test]
    async fn test_apply_incremental_include_no_match_empty() {
        let mut tab = make_tab(&["error line", "info line"]).await;
        tab.log_manager
            .add_filter_with_color("NOMATCH".to_string(), FilterType::Include, None, None, true)
            .await;
        tab.apply_incremental_include("NOMATCH");
        assert!(tab.visible_indices.is_empty());
        assert_eq!(tab.scroll_offset, 0);
    }

    #[tokio::test]
    async fn test_apply_incremental_exclude_filters_visible() {
        let mut tab = make_tab(&["error line", "info line", "error again", "debug line"]).await;
        // Start with all lines visible.
        assert_eq!(tab.visible_indices.len(), 4);
        tab.log_manager
            .add_filter_with_color("error".to_string(), FilterType::Exclude, None, None, true)
            .await;
        // Apply incremental exclude for "error" — removes lines 0 and 2.
        tab.apply_incremental_exclude("error");
        assert_eq!(tab.visible_indices.len(), 2);
        // Remaining visible lines should be "info" and "debug".
        assert_eq!(tab.visible_indices.get(0), 1);
        assert_eq!(tab.visible_indices.get(1), 3);
    }

    #[tokio::test]
    async fn test_apply_incremental_exclude_updates_filter_cache() {
        let mut tab = make_tab(&["line a", "line b"]).await;
        tab.log_manager
            .add_filter_with_color("line".to_string(), FilterType::Include, None, None, true)
            .await;
        tab.refresh_visible();
        let old_gen = tab.parse_cache_gen;
        tab.apply_incremental_exclude("line b");
        // Parse cache generation must be bumped.
        assert!(tab.parse_cache_gen > old_gen);
        // Only "line a" remains visible.
        assert_eq!(tab.visible_indices.len(), 1);
    }

    #[tokio::test]
    async fn test_refresh_visible_bumps_render_cache_gen() {
        let mut tab = make_tab(&["line"]).await;
        tab.log_manager
            .add_filter_with_color("line".to_string(), FilterType::Include, None, None, true)
            .await;
        let old = tab.render_cache_gen;
        tab.refresh_visible();
        assert!(tab.render_cache_gen > old);
        assert!(tab.render_line_cache.is_empty());
    }

    #[tokio::test]
    async fn test_refresh_visible_no_filters_skips_cache_invalidation() {
        let mut tab = make_tab(&["line"]).await;
        // No active filters: toggling filtering_enabled must not bust the caches.
        let old_parse = tab.parse_cache_gen;
        let old_render = tab.render_cache_gen;
        tab.filtering_enabled = !tab.filtering_enabled;
        tab.refresh_visible();
        assert_eq!(tab.parse_cache_gen, old_parse);
        assert_eq!(tab.render_cache_gen, old_render);
    }

    #[tokio::test]
    async fn test_marks_only_toggle_restores_filter_view_without_rescan() {
        // Set up a tab with an active include filter so compute_visible is required
        // on the first call, but the toggle-off should NOT re-run it.
        let mut tab = make_tab(&["hello", "world", "hello world"]).await;
        tab.log_manager
            .add_filter_with_color("hello".to_string(), FilterType::Include, None, None, true)
            .await;
        tab.filtering_enabled = true;
        tab.refresh_visible();
        // Filter view: lines 0 and 2 ("hello" matches).
        assert_eq!(tab.visible_indices.len(), 2);
        let visible_before = tab.visible_indices.clone();

        // Toggle marks-only ON.
        tab.show_marks_only = true;
        tab.refresh_visible();
        // No marks → empty.
        assert_eq!(tab.visible_indices.len(), 0);
        // saved_filter_view was populated.
        assert!(tab.saved_filter_view.is_some());

        // Toggle marks-only OFF — must restore without a file scan.
        tab.show_marks_only = false;
        tab.refresh_visible();
        assert_eq!(tab.visible_indices, visible_before);
        // saved_filter_view consumed.
        assert!(tab.saved_filter_view.is_none());
    }

    #[tokio::test]
    async fn test_marks_only_filter_change_invalidates_saved_view() {
        // If a filter fires refresh_visible while already in marks-only mode,
        // the saved view must be cleared (it would be stale).
        let mut tab = make_tab(&["hello", "world"]).await;
        tab.log_manager
            .add_filter_with_color("hello".to_string(), FilterType::Include, None, None, true)
            .await;
        tab.filtering_enabled = true;
        tab.refresh_visible();

        // Enter marks-only — saves filter view.
        tab.show_marks_only = true;
        tab.refresh_visible();
        assert!(tab.saved_filter_view.is_some());

        // Simulate a filter change while in marks-only mode.
        tab.refresh_visible();
        assert!(tab.saved_filter_view.is_none());
    }

    #[tokio::test]
    async fn test_invalidate_parse_cache_bumps_render_cache_gen() {
        let mut tab = make_tab(&["line"]).await;
        let old = tab.render_cache_gen;
        tab.invalidate_parse_cache();
        assert!(tab.render_cache_gen > old);
        assert!(tab.render_line_cache.is_empty());
    }

    #[tokio::test]
    async fn test_begin_search_clear_bumps_search_result_gen() {
        let mut tab = make_tab(&["line"]).await;
        let old = tab.search_result_gen;
        tab.begin_search("", true, false);
        assert!(tab.search_result_gen > old);
    }

    #[tokio::test]
    async fn test_begin_search_nonempty_does_not_bump_search_result_gen() {
        // search_result_gen is only bumped on advance_search (when results arrive),
        // not on begin_search itself for non-empty patterns.
        let mut tab = make_tab(&["line"]).await;
        let old = tab.search_result_gen;
        tab.begin_search("line", true, false);
        // begin_search with a pattern spawns a background task; gen not bumped yet
        assert_eq!(tab.search_result_gen, old);
    }

    #[tokio::test]
    async fn test_date_filter_not_applied_when_filtering_disabled() {
        let lines = [
            r#"{"timestamp":"2024-01-01T01:30:00Z","level":"INFO","msg":"in range"}"#,
            r#"{"timestamp":"2024-01-01T05:00:00Z","level":"INFO","msg":"out of range"}"#,
        ];
        let mut tab = make_tab(&lines).await;
        let pattern = format!("{}01:00 .. 02:00", crate::date_filter::DATE_PREFIX);
        tab.log_manager
            .add_filter_with_color(pattern, FilterType::Include, None, None, true)
            .await;
        tab.filtering_enabled = false;
        tab.refresh_visible();
        // Both lines must be visible even though only the first matches the date filter.
        assert_eq!(tab.visible_indices.len(), 2);
    }

    #[tokio::test]
    async fn test_date_filter_not_applied_in_marks_only_mode() {
        let lines = [
            r#"{"timestamp":"2024-01-01T01:30:00Z","level":"INFO","msg":"in range"}"#,
            r#"{"timestamp":"2024-01-01T05:00:00Z","level":"INFO","msg":"out of range"}"#,
        ];
        let mut tab = make_tab(&lines).await;
        let pattern = format!("{}01:00 .. 02:00", crate::date_filter::DATE_PREFIX);
        tab.log_manager
            .add_filter_with_color(pattern, FilterType::Include, None, None, true)
            .await;
        // Mark both lines, including the one outside the date range.
        tab.log_manager.toggle_mark(0);
        tab.log_manager.toggle_mark(1);
        tab.show_marks_only = true;
        tab.refresh_visible();
        // Both marked lines must remain visible regardless of the date filter.
        assert_eq!(tab.visible_indices, VisibleLines::Filtered(vec![0, 1]));
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
            .add_filter_with_color("ERROR".to_string(), FilterType::Include, None, None, true)
            .await;
        tab.log_manager
            .add_filter_with_color(
                "@field:level:error".to_string(),
                FilterType::Include,
                None,
                None,
                true,
            )
            .await;
        tab.filtering_enabled = true;
        tab.refresh_visible();

        // Line 0: text Neutral + field Miss → hidden.
        // Line 1: text Neutral + field Match → visible.
        // Line 2: text Include + no excludes → visible.
        assert_eq!(tab.visible_indices, VisibleLines::Filtered(vec![1, 2]));
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
            .add_filter_with_color("ERROR".to_string(), FilterType::Include, None, None, true)
            .await;
        tab.log_manager
            .add_filter_with_color(
                "@field:level:debug".to_string(),
                FilterType::Exclude,
                None,
                None,
                true,
            )
            .await;
        tab.filtering_enabled = true;
        tab.refresh_visible();

        // Line 0: text Include but field exclude → hidden.
        // Line 1: text Include, no field exclude match → visible.
        assert_eq!(tab.visible_indices, VisibleLines::Filtered(vec![1]));
    }

    // ── begin_filter_refresh ─────────────────────────────────────────────────

    #[tokio::test]
    async fn test_begin_filter_refresh_fast_path_no_filters() {
        let mut tab = make_tab(&["a", "b", "c"]).await;
        tab.begin_filter_refresh();
        // No active filters → All(n) synchronously, no background handle.
        assert!(tab.filter_handle.is_none());
        assert_eq!(tab.visible_indices, VisibleLines::All(3));
    }

    #[tokio::test]
    async fn test_begin_filter_refresh_fast_path_filtering_disabled() {
        let mut tab = make_tab(&["a", "b", "c"]).await;
        tab.log_manager
            .add_filter_with_color("a".to_string(), FilterType::Include, None, None, true)
            .await;
        tab.filtering_enabled = false;
        tab.begin_filter_refresh();
        // Filtering disabled: All(n) synchronously.
        assert!(tab.filter_handle.is_none());
        assert_eq!(tab.visible_indices, VisibleLines::All(3));
    }

    #[tokio::test]
    async fn test_begin_filter_refresh_fast_path_marks_only() {
        let mut tab = make_tab(&["a", "b", "c"]).await;
        tab.log_manager.toggle_mark(0);
        tab.log_manager.toggle_mark(2);
        tab.show_marks_only = true;
        tab.begin_filter_refresh();
        // Marks-only: O(marks) sync, no background handle.
        assert!(tab.filter_handle.is_none());
        assert_eq!(tab.visible_indices, VisibleLines::Filtered(vec![0, 2]));
    }

    #[tokio::test]
    async fn test_begin_filter_refresh_spawns_background_for_active_filters() {
        let mut tab = make_tab(&["error line", "info line", "error again"]).await;
        tab.log_manager
            .add_filter_with_color("error".to_string(), FilterType::Include, None, None, true)
            .await;
        tab.begin_filter_refresh();
        // Slow path: background handle is present.
        assert!(tab.filter_handle.is_some());
        // Await the result and verify correctness.
        let h = tab.filter_handle.take().unwrap();
        let result = h.result_rx.await.unwrap();
        assert_eq!(result.visible, vec![0, 2]);
    }

    #[tokio::test]
    async fn test_begin_filter_refresh_cancels_previous_handle() {
        let mut tab = make_tab(&["x", "y", "z"]).await;
        tab.log_manager
            .add_filter_with_color("x".to_string(), FilterType::Include, None, None, true)
            .await;
        tab.begin_filter_refresh();
        let cancel_1 = tab.filter_handle.as_ref().unwrap().cancel.clone();
        // Trigger a second refresh — the first handle's cancel flag must be set.
        tab.begin_filter_refresh();
        assert!(
            cancel_1.load(std::sync::atomic::Ordering::Relaxed),
            "first handle's cancel should be true after second begin_filter_refresh"
        );
    }

    #[tokio::test]
    async fn test_advance_filter_computation_applies_result() {
        let mut tab = make_tab(&["foo bar", "baz", "foo baz"]).await;
        tab.log_manager
            .add_filter_with_color("foo".to_string(), FilterType::Include, None, None, true)
            .await;
        tab.begin_filter_refresh();
        assert!(tab.filter_handle.is_some());
        // Await the background task's result and simulate advance_filter_computation.
        let h = tab.filter_handle.take().unwrap();
        let result = h.result_rx.await.unwrap();
        tab.visible_indices = VisibleLines::Filtered(result.visible);
        assert_eq!(tab.visible_indices, VisibleLines::Filtered(vec![0, 2]));
    }

    // ── Level index ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_rebuild_level_index_populates_error_positions() {
        let tab = make_tab(&["INFO line", "ERROR oops", "WARN careful", "FATAL crash"]).await;
        assert_eq!(tab.error_positions, vec![1, 3]);
        assert_eq!(tab.warning_positions, vec![2]);
    }

    #[tokio::test]
    async fn test_rebuild_level_index_empty_on_no_matches() {
        let tab = make_tab(&["INFO line", "DEBUG detail"]).await;
        assert!(tab.error_positions.is_empty());
        assert!(tab.warning_positions.is_empty());
    }

    #[tokio::test]
    async fn test_rebuild_level_index_empty_file() {
        let tab = make_tab(&[]).await;
        assert!(tab.error_positions.is_empty());
        assert!(tab.warning_positions.is_empty());
    }

    #[tokio::test]
    async fn test_begin_filter_refresh_fast_path_does_not_update_level_index() {
        let mut tab = make_tab(&["INFO ok"]).await;
        tab.error_positions = vec![99];
        tab.file_reader = FileReader::from_bytes(b"ERROR bad\n".to_vec());
        tab.begin_filter_refresh();
        // Fast path must not touch level index — that is the streaming path's responsibility.
        assert!(tab.filter_handle.is_none());
        assert_eq!(tab.error_positions, vec![99]);
    }

    #[tokio::test]
    async fn test_begin_filter_refresh_slow_path_computes_level_positions() {
        let mut tab = make_tab(&["INFO ok", "ERROR bad", "WARN careful", "FATAL crash"]).await;
        tab.log_manager
            .add_filter_with_color("ok".to_string(), FilterType::Exclude, None, None, true)
            .await;
        tab.begin_filter_refresh();
        assert!(tab.filter_handle.is_some());
        let h = tab.filter_handle.take().unwrap();
        let result = h.result_rx.await.unwrap();
        // "INFO ok" excluded; remaining visible positions: ERROR=0, WARN=1, FATAL=2.
        assert_eq!(result.error_positions, vec![0, 2]);
        assert_eq!(result.warning_positions, vec![1]);
    }

    #[tokio::test]
    async fn test_begin_level_index_rebuild_offloads_computation() {
        let mut tab = make_tab(&["INFO ok", "ERROR bad", "WARN careful"]).await;
        tab.error_positions = vec![];
        tab.warning_positions = vec![];
        tab.begin_level_index_rebuild();
        assert!(tab.filter_handle.is_some());
        let h = tab.filter_handle.take().unwrap();
        let result = h.result_rx.await.unwrap();
        assert_eq!(result.visible, vec![0, 1, 2]);
        assert_eq!(result.error_positions, vec![1]);
        assert_eq!(result.warning_positions, vec![2]);
        // Level-index rebuild must not update filter counts.
        assert!(result.filter_match_counts.is_none());
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
            .add_filter_with_color("ERROR".to_string(), FilterType::Include, None, None, true)
            .await;
        tab.log_manager
            .add_filter_with_color("DEBUG".to_string(), FilterType::Exclude, None, None, true)
            .await;
        tab.begin_filter_refresh();
        assert!(tab.filter_handle.is_some());
        let h = tab.filter_handle.take().unwrap();
        let result = h.result_rx.await.unwrap();
        let counts = result.filter_match_counts.expect("counts must be Some");
        // "ERROR" matches 2 lines; "DEBUG" matches 1 line (counted independently).
        assert_eq!(counts, vec![2, 1]);
    }

    #[tokio::test]
    async fn test_filter_match_counts_updated_via_advance() {
        let mut tab = make_tab(&["ERROR line", "INFO line", "ERROR again"]).await;
        tab.log_manager
            .add_filter_with_color("ERROR".to_string(), FilterType::Include, None, None, true)
            .await;
        tab.begin_filter_refresh();
        let h = tab.filter_handle.take().unwrap();
        let result = h.result_rx.await.unwrap();
        // Simulate advance_filter_computation applying the result.
        if let Some(counts) = result.filter_match_counts {
            tab.filter_match_counts = counts;
        }
        assert_eq!(tab.filter_match_counts, vec![2]);
    }

    #[tokio::test]
    async fn test_filter_match_counts_includes_field_filters() {
        let mut tab = make_tab(&["line one", "line two", "line three"]).await;
        tab.log_manager
            .add_filter_with_color(
                "@field:level:error".to_string(),
                FilterType::Include,
                None,
                None,
                true,
            )
            .await;
        tab.begin_filter_refresh();
        let h = tab.filter_handle.take().unwrap();
        let result = h.result_rx.await.unwrap();
        // Unified vec has length equal to filter_defs (one entry), at position 0.
        // Raw text lines have no parser so count is 0.
        let counts = result
            .filter_match_counts
            .expect("filter_match_counts must be Some");
        assert_eq!(counts.len(), 1);
        assert_eq!(counts[0], 0);
    }

    #[tokio::test]
    async fn test_filter_match_counts_cleared_on_no_active_filters() {
        let mut tab = make_tab(&["line"]).await;
        tab.filter_match_counts = vec![5, 7];
        tab.begin_filter_refresh();
        assert!(tab.filter_match_counts.is_empty());
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
                None,
                None,
                true,
            )
            .await;
        tab.begin_filter_refresh();
        let h = tab.filter_handle.take().unwrap();
        let result = h.result_rx.await.unwrap();
        // Unified vec has length equal to filter_defs (one date filter at position 0).
        let counts = result
            .filter_match_counts
            .expect("filter_match_counts must be Some");
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
            index.names.contains(&"ts".to_string()),
            "ts should still appear in names"
        );
        for ts_key in crate::parser::json::TIMESTAMP_KEYS {
            assert!(
                index.values.get(*ts_key).map_or(true, |v| v.is_empty()),
                "timestamp key '{ts_key}' should have no sampled values"
            );
        }
        assert!(!index.values.get("level").unwrap_or(&vec![]).is_empty());
    }

    #[tokio::test]
    async fn test_build_field_index_no_values_for_message_fields() {
        let lines = [
            r#"{"time":"2024-01-01T00:00:00Z","level":"info","msg":"hello"}"#,
            r#"{"time":"2024-01-01T00:00:01Z","level":"warn","message":"world"}"#,
        ];
        let tab = make_tab(&lines).await;
        let index = tab.build_field_index();
        assert!(
            index.names.contains(&"msg".to_string())
                || index.names.contains(&"message".to_string()),
            "message key should still appear in names"
        );
        for msg_key in crate::parser::json::MESSAGE_KEYS {
            assert!(
                index.values.get(*msg_key).map_or(true, |v| v.is_empty()),
                "message key '{msg_key}' should have no sampled values"
            );
        }
        assert!(!index.values.get("level").unwrap_or(&vec![]).is_empty());
    }
}
