//! Core TUI types: [`App`], [`TabState`], [`KeyResult`], and [`LoadContext`].
//!
//! [`App`] owns the tab list, global theme, and shared [`Keybindings`].
//! [`TabState`] owns the per-tab file reader, log manager, format parser,
//! visible indices, scroll state, and active mode.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use ratatui::style::Style;

use crate::config::Keybindings;
use crate::date_filter::DateFilterStyle;
use crate::db::FileContext;
use crate::file_reader::FileReader;
use crate::filters::FilterManager;
use crate::log_manager::LogManager;
use crate::mode::app_mode::Mode;
use crate::mode::normal_mode::NormalMode;
use crate::parser::{LogFormatParser, detect_format};
use crate::search::Search;
use crate::types::FieldLayout;

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
    pub detected_format: Option<Box<dyn LogFormatParser>>,
    /// When true, always scroll to the last visible line when new content arrives.
    pub tail_mode: bool,
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
    /// Monotonically increasing counter; bumped in `refresh_visible` and whenever the
    /// field layout or display mode changes. Cache entries with a stale generation are
    /// re-computed on the next render.
    pub parse_cache_gen: u64,
    /// Per-line parse cache: file-line index → (generation, CachedParsedLine).
    pub parse_cache: HashMap<usize, (u64, CachedParsedLine)>,
}

impl TabState {
    pub fn new(file_reader: FileReader, log_manager: LogManager, title: String) -> Self {
        // Sample up to 200 lines for format detection.
        let sample_limit = file_reader.line_count().min(200);
        let sample: Vec<&[u8]> = (0..sample_limit).map(|i| file_reader.get_line(i)).collect();
        let detected_format = detect_format(&sample);

        let mut tab = TabState {
            file_reader,
            log_manager,
            visible_indices: VisibleLines::default(),
            mode: Box::new(NormalMode::default()),
            scroll_offset: 0,
            viewport_offset: 0,
            show_sidebar: true,
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
            title,
            command_history: Vec::new(),
            watch_state: None,
            hidden_fields: HashSet::new(),
            field_layout: FieldLayout::default(),
            keybindings: Arc::new(Keybindings::default()),
            detected_format,
            tail_mode: false,
            raw_mode: false,
            show_keys: false,
            show_mode_bar: true,
            show_borders: true,
            filter_manager_arc: Arc::new(FilterManager::empty()),
            filter_styles: Vec::new(),
            filter_date_styles: Vec::new(),
            parse_cache_gen: 0,
            parse_cache: HashMap::new(),
        };
        tab.refresh_visible();
        tab
    }

    /// Recompute which file lines are visible under the current filters.
    pub fn refresh_visible(&mut self) {
        // Invalidate the parse cache: field layout, filters, or file content may have changed.
        self.parse_cache_gen = self.parse_cache_gen.wrapping_add(1);
        self.parse_cache.clear();

        if self.show_marks_only {
            let mut indices = self.log_manager.get_marked_indices();
            indices.retain(|&i| i < self.file_reader.line_count());
            self.visible_indices = VisibleLines::Filtered(indices);
            // Rebuild filter cache so the render path always has a valid manager.
            let (fm, styles, date_filter_styles) = self.log_manager.build_filter_manager();
            self.filter_manager_arc = Arc::new(fm);
            self.filter_styles = styles;
            self.filter_date_styles = date_filter_styles;
        } else if !self.filtering_enabled {
            // No allocation: All(n) represents identity mapping i→i.
            self.visible_indices = VisibleLines::All(self.file_reader.line_count());
            // Keep an empty manager so the render path produces no filter highlights.
            self.filter_manager_arc = Arc::new(FilterManager::empty());
            self.filter_styles = Vec::new();
            self.filter_date_styles = Vec::new();
        } else {
            // Build once: compute_visible uses the same manager the render path will clone.
            let (fm, styles, date_filter_styles) = self.log_manager.build_filter_manager();
            let visible = fm.compute_visible(&self.file_reader);
            self.filter_manager_arc = Arc::new(fm);
            self.filter_styles = styles;
            self.filter_date_styles = date_filter_styles;
            self.visible_indices = VisibleLines::Filtered(visible);
        }

        // Apply date filters as a post-processing step.
        let date_filters = crate::date_filter::extract_date_filters(self.log_manager.get_filters());
        if !date_filters.is_empty()
            && let Some(parser) = &self.detected_format
        {
            self.visible_indices.retain(|idx| {
                let line = self.file_reader.get_line(idx);
                match parser.parse_line(line) {
                    Some(parts) => match parts.timestamp {
                        Some(ts) => crate::date_filter::matches_any(&date_filters, ts),
                        None => true, // lines without timestamps pass through
                    },
                    None => true, // unparseable lines pass through
                }
            });
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

    /// Apply a new exclude filter incrementally against the currently visible lines,
    /// avoiding a full `compute_visible` scan of the entire file.
    ///
    /// Only safe for pure-text exclude additions when no include-filter-only changes are needed.
    /// The filter manager cache is rebuilt afterward so render highlights stay correct.
    pub fn apply_incremental_exclude(&mut self, pattern: &str) {
        use crate::filters::{FilterDecision, MatchCollector, build_filter};
        if let Some(filter) = build_filter(pattern, FilterDecision::Exclude, true, 0) {
            self.visible_indices.retain(|line_idx| {
                let line = self.file_reader.get_line(line_idx);
                let mut dummy = MatchCollector::new(line);
                !matches!(filter.evaluate(line, &mut dummy), FilterDecision::Exclude)
            });
        }
        // Rebuild filter manager cache so the render path sees the updated filters.
        let (fm, styles, date_filter_styles) = self.log_manager.build_filter_manager();
        self.filter_manager_arc = Arc::new(fm);
        self.filter_styles = styles;
        self.filter_date_styles = date_filter_styles;
        // Invalidate parse cache (filter change affects highlight output).
        self.parse_cache_gen = self.parse_cache_gen.wrapping_add(1);
        self.parse_cache.clear();
        // Clamp scroll.
        if self.visible_indices.is_empty() {
            self.scroll_offset = 0;
        } else {
            self.scroll_offset = self.scroll_offset.min(self.visible_indices.len() - 1);
        }
    }

    /// Bump the parse cache generation so that all cached render outputs are re-computed
    /// on the next frame. Call this whenever the field layout or display mode changes.
    pub fn invalidate_parse_cache(&mut self) {
        self.parse_cache_gen = self.parse_cache_gen.wrapping_add(1);
        self.parse_cache.clear();
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
    pub fn collect_field_names(&self) -> Vec<String> {
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
    /// Open as a new tab; continue with any remaining session-restore files.
    SessionRestoreTab {
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

    // ── list_dir_files ────────────────────────────────────────────────

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
        let tab = make_tab(&["plain text line", "another line"]).await;
        let fields = tab.collect_field_names();
        assert!(fields.is_empty());
    }

    #[tokio::test]
    async fn test_collect_field_names_json_format() {
        let tab = make_tab(&[r#"{"level":"INFO","msg":"hello"}"#]).await;
        let fields = tab.collect_field_names();
        assert!(!fields.is_empty());
        assert!(fields.contains(&"level".to_string()));
        assert!(fields.contains(&"msg".to_string()));
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

    // ── goto_line ─────────────────────────────────────────────────────

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

    // ── Opt-1: filter manager cache ───────────────────────────────────

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

    // ── Opt-4: parse cache invalidation ──────────────────────────────

    #[tokio::test]
    async fn test_refresh_visible_increments_parse_cache_gen() {
        let mut tab = make_tab(&["line"]).await;
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

    // ── Opt-5: incremental exclude ────────────────────────────────────

    #[tokio::test]
    async fn test_apply_incremental_exclude_filters_visible() {
        let mut tab = make_tab(&["error line", "info line", "error again", "debug line"]).await;
        // Start with all lines visible.
        assert_eq!(tab.visible_indices.len(), 4);
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
}
