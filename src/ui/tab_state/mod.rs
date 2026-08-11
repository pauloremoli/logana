use rayon::iter::IndexedParallelIterator;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{mpsc, watch};

use crate::db::CommentManager;
use crate::db::FileContext;
use crate::db::LogManager;
use crate::db::MarkManager;
use crate::filters::{FieldVote, any_field_exclude_matches, field_include_vote};
use crate::filters::{FilterDecision, FilterManager};
use crate::ingestion::{ArchiveTree, FileReader, NodeId};
use crate::mode::normal_mode::NormalMode;
use crate::parser::{LogFormatParser, detect_format};
use crate::ui::FieldLayout;
use crate::utils::search::Search;
use crate::utils::search::SearchResult;

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
    ToggleRelativeLineNumbers,
    ToggleGroupsPanel,
    AlwaysRestoreFile(Box<crate::db::FileContext>),
    NeverRestoreFile,
    AlwaysRestoreSession(Vec<String>),
    NeverRestoreSession,
    OpenMergeSelect,
    OpenMergedView {
        source_tab_indices: Vec<usize>,
    },
    ExportWithFooter {
        path: String,
        template_name: String,
        footer_fields: Vec<(String, String)>,
    },
    ApplyArchivePicker {
        source_path: String,
        tree: ArchiveTree,
    },
    /// The archive picker is still open (unlike `ApplyArchivePicker`, which
    /// replaces the mode) — the background fetch behind this lands back on
    /// the live `ArchivePickerMode` via `Mode::as_archive_picker_mut`.
    ExpandArchiveNode {
        node_id: NodeId,
    },
    /// Set (or, when `path` is `None`, clear) one format's default filter
    /// file mapping, emitted by the `:default-filters` popup. The popup
    /// itself stays open (see `DefaultFiltersMode`) — this only persists the
    /// change.
    SetDefaultFilterFile {
        format: String,
        path: Option<String>,
    },
    /// Emitted by the file switcher popup (`Ctrl+P`) when Enter is pressed
    /// on an entry — switches `App::active_tab` to the given `App::tabs`
    /// index.
    SwitchToTab(usize),
    /// Emitted by the `:theme` picker every time the highlighted entry
    /// changes, so the whole UI re-renders with the named theme applied
    /// immediately — before the user confirms with Enter.
    PreviewTheme(String),
    /// Emitted by the `:theme` picker on Enter — applies and persists the
    /// named theme.
    ConfirmTheme(String),
    /// Emitted by the `:theme` picker on Esc — restores the theme that was
    /// active before the picker opened, undoing any live preview. Boxed
    /// since `Theme` is large and every other `KeyResult` variant would
    /// otherwise pay for its size.
    RevertTheme(Box<crate::theme::Theme>),
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
    /// `highlight_mode` value at scan-start; part of the cache key so
    /// toggling it invalidates a cached scan computed under the other mode.
    pub scan_highlight_mode: bool,
    /// Group style definitions at scan-start; part of the cache key so a
    /// group-only style edit (e.g. `:group errors --fg Red`) — which leaves
    /// every `FilterDef` untouched — still invalidates the cache instead of
    /// replaying colors baked from the group's old style.
    pub scan_group_fingerprint: Vec<crate::filters::GroupDef>,
}

/// Cached result of a completed background filter scan.
/// Keyed by the set of enabled filters, file line count, and raw-mode flag at scan time.
/// Used by [`TabState::begin_filter_refresh`] to skip a redundant re-scan when the
/// filter state is toggled off and then back on without any changes in between.
pub struct CachedScanResult {
    pub filter_fingerprint: Vec<crate::filters::FilterDef>,
    pub line_count: usize,
    pub raw_mode: bool,
    pub highlight_mode: bool,
    /// Group style definitions at scan time — see
    /// `FilterHandle::scan_group_fingerprint` for why this must be part of
    /// the cache key alongside the filters themselves.
    pub group_fingerprint: Vec<crate::filters::GroupDef>,
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

/// Per-scan filter context passed to [`line_is_visible`].
///
/// Groups the parameters that are constant for an entire filter scan so they
/// do not need to be repeated at every call site.  Each parallel worker
/// constructs its own instance with its own `date_counts` accumulator.
pub struct FilterEvalContext<'a> {
    pub has_text_includes: bool,
    pub date_filters: &'a [crate::filters::DateFilter],
    /// Accumulates per-date-filter match counts across lines; indexed parallel to `date_filters`.
    pub date_counts: &'a mut [usize],
    pub inc_ff: &'a [crate::filters::FieldFilter],
    pub exc_ff: &'a [crate::filters::FieldFilter],
    pub year_override: Option<i32>,
}

impl<'a> FilterEvalContext<'a> {
    pub fn new(
        has_text_includes: bool,
        date_filters: &'a [crate::filters::DateFilter],
        date_counts: &'a mut [usize],
        inc_ff: &'a [crate::filters::FieldFilter],
        exc_ff: &'a [crate::filters::FieldFilter],
        year_override: Option<i32>,
    ) -> Self {
        Self {
            has_text_includes,
            date_filters,
            date_counts,
            inc_ff,
            exc_ff,
            year_override,
        }
    }
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
pub fn line_is_visible(
    text_dec: FilterDecision,
    ctx: &mut FilterEvalContext<'_>,
    parts: Option<&crate::parser::DisplayParts<'_>>,
    line: &[u8],
) -> bool {
    // Step 1: text filter result — fast path.
    if text_dec == FilterDecision::Exclude {
        return false;
    }

    // Step 2: date filter — AND constraint; count and check visibility in one pass.
    if !ctx.date_filters.is_empty()
        && let Some(ts) = parts.and_then(|p| p.timestamp)
    {
        let mut any_date_match = false;
        for (df, count) in ctx.date_filters.iter().zip(ctx.date_counts.iter_mut()) {
            if df.matches(ts, ctx.year_override) {
                *count += 1;
                any_date_match = true;
            }
        }
        if !any_date_match {
            return false;
        }
    }

    // Step 3: field exclude — hides the line if any matching exclude is found.
    if any_field_exclude_matches(ctx.exc_ff, parts, line) {
        return false;
    }

    // Step 4: include resolution — text include OR field include.
    if text_dec == FilterDecision::Include {
        return true;
    }

    // text_dec is Neutral; check field includes.
    if !ctx.inc_ff.is_empty() {
        return match field_include_vote(ctx.inc_ff, parts, line) {
            FieldVote::Match => true,
            FieldVote::Miss => false,
            // Pass-through: field filters don't apply to this line; fall back to
            // text-filter-only logic (visible iff there are no text include filters).
            FieldVote::PassThrough => !ctx.has_text_includes,
        };
    }

    // No field includes; visible iff no text include filters exist.
    !ctx.has_text_includes
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
        // Prefer the parser's own template reconstruction (same source of
        // truth as `log_panel.rs`/`render_line_text`) so match offsets land
        // on the same text that's actually highlighted on screen.
        if let Some(reconstructed) = super::field_layout::reconstructed_line_text(
            parser.as_ref(),
            &parts,
            field_layout,
            hidden_fields,
        ) {
            return reconstructed;
        }
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
                // A line matching the schema's declared `continuation.end_pattern`
                // is, by definition, the terminator of the block already open —
                // it must never start a new one, even if it also happens to
                // satisfy the schema's main `parse_line` pattern (an easy
                // authoring overlap: an end line's text often closely
                // resembles the header it's ending, e.g. "... ended at: ..."
                // vs "... started at: ...").
                if !line.is_empty()
                    && !parser.is_continuation_end(line)
                    && parser.parse_line(line).is_some()
                {
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

/// Walks forward from `line_idx + 1` while `cmap` marks each line as a
/// continuation of `line_idx` (`build_continuation_map`'s convention: a
/// continuation line's entry equals its parent's index), collecting lines up
/// to (and including) the first one `parser.is_continuation_end` matches, if
/// any — otherwise every continuation line `cmap` groups with `line_idx`.
/// The collected lines are handed to `parser.walk_continuation` in one
/// batch, merging its flat fields into `parts.extra_fields` and its
/// per-group items into `parts.field_groups`. Returns the index of the last
/// line still considered part of this record, or `line_idx` itself if there
/// are no continuation lines.
///
/// For a schema with no flat/`vec` continuation content declared,
/// `walk_continuation`'s default reduces to `extract_continuation_fields`
/// per line with no groups and `is_continuation_end` defaults to `false`,
/// so this returns exactly what the old `last_continuation_line` helper did
/// and leaves `parts.extra_fields`/`parts.field_groups` untouched.
fn apply_continuation_fields<'a>(
    parser: &dyn LogFormatParser,
    reader: &'a FileReader,
    cmap: &[usize],
    line_idx: usize,
    parts: &mut crate::parser::DisplayParts<'a>,
) -> usize {
    let mut block_end = line_idx;
    let mut lines: Vec<&'a [u8]> = Vec::new();
    let mut j = line_idx + 1;
    while j < cmap.len() && cmap[j] == line_idx {
        let line = reader.get_line(j);
        block_end = j;
        if parser.is_continuation_end(line) {
            break;
        }
        lines.push(line);
        j += 1;
    }
    let result = parser.walk_continuation(&lines);
    parts.extra_fields.extend(result.flat_fields);
    parts.field_groups = result.groups;
    block_end
}

/// Byte range (within `reader.data()`) covering `message` (if present, else
/// the start of the first continuation line) through the end of the last
/// continuation line — the zero-copy merged `message` span.
///
/// Both endpoints are guaranteed to fall within `reader.data()`: `message`
/// (when `Some`) always borrows from `reader.get_line(line_idx)`, itself a
/// subslice of `reader.data()`, and every line up to `last` is contiguous in
/// that same buffer (consecutive file lines, per `line_byte_range`).
fn merged_message_range(
    reader: &FileReader,
    message: Option<&str>,
    line_idx: usize,
    last: usize,
) -> std::ops::Range<usize> {
    let data = reader.data();
    let start = match message {
        Some(msg) => msg.as_ptr() as usize - data.as_ptr() as usize,
        None => reader.line_byte_range(line_idx + 1).start,
    };
    let end = reader.line_byte_range(last).end;
    start..end
}

/// Parses line `line_idx`, and — when `parser` wants a continuation walk
/// (`merges_continuation_into_message` and/or a `continuation` block) and
/// `line_idx` has continuation lines in `cmap` — merges structured fields
/// extracted from those lines into `parts.extra_fields`, and (when
/// `merges_continuation_into_message`) extends the `message` field's byte
/// range to cover them too (zero-copy, since consecutive file lines are
/// contiguous in `reader`'s backing buffer — see `merged_message_range`), so
/// field filters and the structured fields panel can see their content.
pub fn parse_line_with_continuation<'a>(
    parser: &dyn LogFormatParser,
    reader: &'a FileReader,
    cmap: Option<&[usize]>,
    line_idx: usize,
) -> Option<crate::parser::DisplayParts<'a>> {
    let mut parts = parser.parse_line(reader.get_line(line_idx))?;
    if !parser.wants_continuation_walk() {
        return Some(parts);
    }
    let Some(cmap) = cmap else {
        return Some(parts);
    };
    let block_end = apply_continuation_fields(parser, reader, cmap, line_idx, &mut parts);
    if parser.merges_continuation_into_message() && block_end > line_idx {
        let range = merged_message_range(reader, parts.message, line_idx, block_end);
        if let Ok(merged) = std::str::from_utf8(&reader.data()[range]) {
            parts.message = Some(merged);
        }
    }
    Some(parts)
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

    // Indices beyond the map belong to lines appended after the map was built
    // (e.g. file grew while the map was stale). Preserve them unchanged.
    let mut filter_visible = vec![0u8; n];
    for &idx in indices.iter() {
        if idx < n {
            filter_visible[idx] = 1;
        }
    }

    // When include filters are active, a line that individually matches pulls
    // its whole record (parent + every continuation) into view — the match
    // may live entirely in continuation content (e.g. a "Status: FAILED"
    // line deep in a multiline transaction record) that the header line
    // itself never mentions. `group_matched[p]` is set once for a parent `p`
    // if *any* member of its record — the header or a continuation —
    // individually matched.
    let mut group_matched = filter_visible.clone();
    if has_include_filters {
        for i in 0..n {
            if filter_visible[i] != 0 {
                group_matched[cmap[i]] = 1;
            }
        }
    }

    // Each line is visible iff its record matched: with include filters,
    // that's "some member of the record matched" (`group_matched`); with
    // only excludes, a line is visible iff both its parent and itself are
    // (an explicit exclude anywhere in the record must not be overridden by
    // an unrelated match elsewhere in it).
    let mut new_indices: Vec<usize> = (0..n)
        .into_par_iter()
        .filter(|&i| {
            if has_include_filters {
                group_matched[cmap[i]] != 0
            } else {
                filter_visible[cmap[i]] != 0 && filter_visible[i] != 0
            }
        })
        .collect();

    for &idx in indices.iter() {
        if idx >= n {
            new_indices.push(idx);
        }
    }
    new_indices.sort_unstable();

    *indices = new_indices;
}

/// Removes continuation lines whose parent is effectively collapsed from
/// `visible`. A parent `p`'s effective state is `default_collapsed XOR
/// overridden_groups.contains(p)`, so `overridden_groups` acts as a
/// per-parent override in either direction regardless of the global
/// default — this is what lets the normal-mode `<`/`>` keys collapse or
/// expand a single entry independent of whether `:collapse` has ever run.
/// Parent lines (`cmap[i] == i`) are always kept, as are indices beyond
/// `cmap`'s range (file grew after the map was built). Must run after
/// `apply_continuation_correction` — it further restricts an
/// already-correct filtered view, it doesn't re-derive filter matches.
///
/// Unlike `apply_continuation_correction`, this also applies to
/// `VisibleLines::All`: collapsing continuation lines on an otherwise
/// unfiltered file is the primary use case for `:collapse`.
pub fn apply_collapse_correction(
    visible: &mut VisibleLines,
    cmap: &[usize],
    default_collapsed: bool,
    overridden_groups: &HashSet<usize>,
) {
    visible.retain(|idx| {
        if idx >= cmap.len() || cmap[idx] == idx {
            return true;
        }
        let parent = cmap[idx];
        !(default_collapsed ^ overridden_groups.contains(&parent))
    });
}

pub struct TabState {
    pub file_reader: FileReader,
    pub log_manager: LogManager,
    pub mark_manager: MarkManager,
    pub comment_manager: CommentManager,
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
    /// For a picker-triggered merged tab (archive or directory), keeps each
    /// merge-marked source's own temp copy alive for the lifetime of this
    /// tab — a directory source is copied into temp same as an archive
    /// source is extracted into temp, so the merged tab is self-contained
    /// and never needs to re-open the original files. Empty for anything
    /// that isn't a picker-triggered merge (including a live `:merge` tab,
    /// which reads its still-growing sources directly).
    pub merge_source_temps: Vec<tempfile::NamedTempFile>,
    /// For a picker-triggered merged tab, the final sorted/interleaved
    /// content written to one temp file once the background merge build
    /// finishes — the literal "saved merged file". Not used for reading
    /// (the tab keeps reading through its `Storage::Merged` view); this
    /// exists so the merge result exists as a real file on disk and so
    /// [`TabState::is_temp_backed`] has something to point the `[TEMP]`
    /// marker at.
    pub merged_temp: Option<tempfile::NamedTempFile>,
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
            mark_manager: MarkManager::default(),
            comment_manager: CommentManager::default(),
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
            merge_source_temps: Vec::new(),
            merged_temp: None,
            extraction_progress: None,
            continuation_map,
            year_map,
            merged: None,
        };
        tab.refresh_visible();
        tab
    }

    /// Whether this tab's content lives only in a temp file rather than a
    /// location the user chose — an extracted archive file, or a
    /// picker-triggered merge (see `archive_temp`/`merged_temp`). Drives the
    /// `[TEMP]` title marker so it's clear the data disappears once the temp
    /// file is cleaned up, unlike a normally opened file.
    pub fn is_temp_backed(&self) -> bool {
        self.archive_temp.is_some() || self.merged_temp.is_some()
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

    /// Scan forward from `from` (exclusive) for the next visible marked line.
    pub fn next_marked_position(&self, from: usize) -> Option<usize> {
        let len = self.filter.visible_indices.len();
        (from.saturating_add(1)..len).find(|&pos| {
            self.mark_manager
                .is_marked(self.filter.visible_indices.get(pos))
        })
    }

    /// Scan backward from `from` (exclusive) for the previous visible marked line.
    pub fn prev_marked_position(&self, from: usize) -> Option<usize> {
        (0..from).rev().find(|&pos| {
            self.mark_manager
                .is_marked(self.filter.visible_indices.get(pos))
        })
    }

    fn scan_level_forward(&self, from: usize, errors: bool) -> Option<usize> {
        let len = self.filter.visible_indices.len();
        (from.saturating_add(1)..len).find(|&pos| self.pos_matches_level(pos, errors))
    }

    fn scan_level_backward(&self, from: usize, errors: bool) -> Option<usize> {
        (0..from)
            .rev()
            .find(|&pos| self.pos_matches_level(pos, errors))
    }

    /// The parser that applies to file line `line_idx`: for a merged tab,
    /// the specific source's own detected parser (via
    /// `MergedEntry::source_idx`) — a merged tab's lines can come from
    /// sources with different formats, so `display.format` (always `None`
    /// for a merged tab; see `App::build_merged_tab`) can't answer this.
    /// For a non-merged tab, just `display.format`. Mirrors the per-source
    /// resolution `log_panel.rs`'s render loop already uses for coloring —
    /// level classification here must agree with what's actually rendered.
    fn parser_for_line(&self, line_idx: usize) -> Option<&dyn LogFormatParser> {
        if self.display.raw_mode {
            return None;
        }
        if let Some(entries) = self.file_reader.merged_entries()
            && let Some(merged) = self.merged.as_ref()
        {
            return entries
                .get(line_idx)
                .and_then(|e| merged.source_parsers.get(e.source_idx))
                .and_then(|p| p.as_deref());
        }
        self.display.format.as_deref()
    }

    fn pos_matches_level(&self, pos: usize, errors: bool) -> bool {
        use crate::parser::LogLevel;
        let file_idx = self.filter.visible_indices.get(pos);
        let bytes = self.file_reader.get_line(file_idx);
        let level = self
            .parser_for_line(file_idx)
            .and_then(|p| {
                p.parse_line(bytes)
                    .and_then(|parts| parts.level)
                    .map(|raw| p.classify_level(raw))
            })
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
            self.sync_collapse_mask();
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
            let mut indices = self.mark_manager.get_indices();
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
            let parser = if self.display.raw_mode {
                None
            } else {
                self.display.format.as_deref()
            };
            let field_layout = &self.display.field_layout;
            let hidden_fields = &self.display.hidden_fields;
            let show_keys = self.display.show_keys;
            use rayon::prelude::*;
            let file_reader = &self.file_reader;
            let year_map = self.year_map.as_deref();
            let cmap = self.active_continuation_map().map(|c| c.as_slice());
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
                                    parser.and_then(|p| {
                                        parse_line_with_continuation(p, file_reader, cmap, idx)
                                    })
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
                                        line,
                                        &mut fc,
                                    );
                                }
                                let mut ctx = FilterEvalContext::new(
                                    has_text_includes,
                                    &date_filters,
                                    &mut dc,
                                    &inc_ff,
                                    &exc_ff,
                                    year_override,
                                );
                                line_is_visible(text_dec, &mut ctx, parts.as_ref(), line)
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
            if let Some(cmap) = self.active_continuation_map().cloned() {
                apply_continuation_correction(
                    &mut self.filter.visible_indices,
                    &cmap,
                    has_text_includes,
                );
            }
        }

        self.sync_collapse_mask();
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
            self.sync_collapse_mask();
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
            let mut indices = self.mark_manager.get_indices();
            indices.retain(|&i| i < self.file_reader.line_count());
            self.filter.visible_indices = VisibleLines::Filtered(indices);
            self.rebuild_filter_manager_cache();
            self.filter.match_counts = Vec::new();
            self.sync_collapse_mask();
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
            self.sync_collapse_mask();
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
            self.sync_collapse_mask();
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
            && cached.highlight_mode == self.filter.highlight_mode
            && cached.group_fingerprint == self.log_manager.get_group_styles()
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
        let highlight_mode = self.filter.highlight_mode;

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
                    // Highlight mode bypasses visibility but keeps counts,
                    // which evaluate_chunk_wholefile already computed accurately.
                    let vis = if highlight_mode {
                        (chunk_start..chunk_end).collect()
                    } else {
                        vis
                    };
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
                                // In highlight mode every line stays visible, so the
                                // skip-parsing optimization below (which assumes an
                                // excluded/neutral line's field & date data doesn't
                                // matter) must not apply — counts still need it.
                                let can_skip = !highlight_mode
                                    && (text_dec == FilterDecision::Exclude
                                        || (text_dec == FilterDecision::Neutral
                                            && has_text_includes
                                            && inc_ff.is_empty()
                                            && !synthetic_level));
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
                                            line,
                                            &mut fc,
                                        );
                                    }
                                    let mut ctx = FilterEvalContext::new(
                                        has_text_includes,
                                        &date_filters,
                                        &mut dc,
                                        &inc_ff,
                                        &exc_ff,
                                        year_override,
                                    );
                                    line_is_visible(text_dec, &mut ctx, parts.as_ref(), line)
                                };
                                if highlight_mode || visible {
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
            scan_highlight_mode: self.filter.highlight_mode,
            scan_group_fingerprint: self.log_manager.get_group_styles().to_vec(),
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

        // Captured against the current (possibly masked) `visible_indices`,
        // before it's replaced by the pristine baseline below — restored at
        // every exit point so the cursor doesn't drift onto an unrelated
        // line once the mask is recomputed (see `set_continuation_collapsed`
        // for the same concern with `<`/`>`).
        let current_line = self
            .filter
            .visible_indices
            .get_opt(self.scroll.scroll_offset);

        // If a collapse mask is currently applied, work from the pristine
        // (unmasked) baseline instead of the partially-hidden current view —
        // otherwise the new lines below would extend a mix of "old lines
        // already masked" and "new lines never masked", and snapshotting
        // that mix as the new baseline would corrupt future `:expand`/`<`/`>`
        // calls. `sync_collapse_mask` at every exit point re-derives the
        // mask (old + new lines alike) from this restored baseline.
        if let Some(baseline) = self.filter.pre_collapse_visible.take() {
            self.filter.visible_indices = baseline;
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
                // See `build_continuation_map`: an `end_pattern` match must
                // never start a new block, even if it also happens to
                // satisfy the schema's main `parse_line` pattern.
                if !line.is_empty()
                    && !parser.is_continuation_end(line)
                    && parser.parse_line(line).is_some()
                {
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
            self.sync_collapse_mask();
            self.restore_scroll_to_line(current_line);
            return;
        }

        if !self.filter.enabled {
            match &mut self.filter.visible_indices {
                VisibleLines::All(n) => *n = new_count,
                VisibleLines::Filtered(_) => {
                    self.filter.visible_indices = VisibleLines::All(new_count);
                }
            }
            self.sync_collapse_mask();
            self.restore_scroll_to_line(current_line);
            return;
        }

        if self.filter.show_marks_only {
            let mut indices = self.mark_manager.get_indices();
            indices.retain(|&i| i < new_count);
            self.filter.visible_indices = VisibleLines::Filtered(indices);
            self.sync_collapse_mask();
            self.restore_scroll_to_line(current_line);
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
                let mut ctx = FilterEvalContext::new(
                    has_text_includes,
                    &date_filters,
                    &mut dummy_date_counts,
                    &inc_ff,
                    &exc_ff,
                    year_override,
                );
                line_is_visible(text_dec, &mut ctx, parts.as_ref(), line)
            };
            if visible {
                new_visible.push(i);
            }
        }

        // Apply continuation semantics: continuation lines inherit their parent's
        // filter decision. A parent in this batch uses `new_visible`; a parent
        // from earlier uses `visible_indices`.
        if let Some(cmap) = self.active_continuation_map().cloned() {
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
        self.sync_collapse_mask();
        self.restore_scroll_to_line(current_line);
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
        if let Some(filter) = build_filter(pattern, decision, true, 0, false, false) {
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
        self.mark_manager.clear();
        self.comment_manager.clear();
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
            let detected =
                crate::ingestion::format_detect::detect_format_for_reader(&self.file_reader);
            // Apply default hidden fields only when the tab currently has none
            // (first detection for a streaming source, or non-streaming source
            // whose preview was too short for detection).
            if self.display.hidden_fields.is_empty()
                && let Some(f) = &detected.format
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
            self.display.format = detected.format;
            self.continuation_map = detected.continuation_map;
            self.year_map = detected.year_map;
        }
    }

    /// Sets the tab's format (or clears it to `None`) and rebuilds the
    /// derived continuation/year maps to match, then re-scans active
    /// filters against the new format.
    ///
    /// Without rebuilding these, manually switching schemas (`:schema
    /// <name>` / `:schema none`) left the continuation map from whatever
    /// format was previously active in place; filter visibility is gated
    /// on it every scan, so a line that individually matches a filter
    /// stayed hidden because its old structured "parent" line (under a
    /// format the tab no longer uses) didn't match.
    pub fn apply_format(&mut self, parser: Option<Arc<dyn LogFormatParser>>) {
        let (continuation_map, year_map) =
            crate::ingestion::format_detect::derive_format_structures(
                &self.file_reader,
                parser.as_deref(),
            );
        self.continuation_map = continuation_map;
        self.year_map = year_map;
        self.display.format = parser;
        self.invalidate_parse_cache();
        self.begin_filter_refresh();
    }

    /// The continuation map to use for multiline grouping, or `None` when
    /// raw mode bypasses format-aware parsing (matching the parser
    /// selection every scan already uses). Consulting the map while raw
    /// mode is on would hide lines that individually match a filter just
    /// because their old structured "parent" line doesn't.
    pub(crate) fn active_continuation_map(&self) -> Option<&Arc<Vec<usize>>> {
        if self.display.raw_mode || self.display.format.is_none() {
            None
        } else {
            self.continuation_map.as_ref()
        }
    }

    /// Snapshots the just-recomputed `visible_indices` into
    /// `pre_collapse_visible` and applies the collapse mask on top, when
    /// there's anything to collapse (`collapse_continuations` is on, or an
    /// individual `<` override is active despite the default being
    /// expanded). Otherwise clears `pre_collapse_visible` so later filter
    /// refreshes can skip collapse work entirely — cheap, since
    /// `visible_indices` is already correct in that case. Must run last in
    /// every place that (re)computes `visible_indices` fresh from the
    /// filter pipeline (both `refresh_visible_inner`'s exit points and the
    /// async `advance_filter_computation` path), so `:collapse`/`:expand`
    /// and `<`/`>` always have an accurate uncollapsed baseline to work
    /// from.
    pub(crate) fn sync_collapse_mask(&mut self) {
        if !self.display.collapse_continuations && self.filter.overridden_groups.is_empty() {
            self.filter.pre_collapse_visible = None;
            return;
        }
        self.filter.pre_collapse_visible = Some(self.filter.visible_indices.clone());
        if let Some(cmap) = self.active_continuation_map().cloned() {
            apply_collapse_correction(
                &mut self.filter.visible_indices,
                &cmap,
                self.display.collapse_continuations,
                &self.filter.overridden_groups,
            );
        }
    }

    /// Reapplies the collapse mask from `pre_collapse_visible` after
    /// `overridden_groups` changes (`>`/`<`), without re-running the filter
    /// pipeline. No-op if no baseline exists yet (`pre_collapse_visible` is
    /// `None`) — callers that might be establishing the first override
    /// must set it themselves first (see `set_continuation_collapsed`).
    pub(crate) fn recompute_collapse_mask(&mut self) {
        let Some(base) = self.filter.pre_collapse_visible.clone() else {
            return;
        };
        self.filter.visible_indices = base;
        if let Some(cmap) = self.active_continuation_map().cloned() {
            apply_collapse_correction(
                &mut self.filter.visible_indices,
                &cmap,
                self.display.collapse_continuations,
                &self.filter.overridden_groups,
            );
        }
    }

    /// Sets whether `parent` (a line with at least one continuation line)
    /// is collapsed, independent of the global `collapse_continuations`
    /// default — the entry point for the normal-mode `<`/`>` keys, which
    /// must work whether or not `:collapse` has ever run. Lazily
    /// establishes `pre_collapse_visible` the first time an override is
    /// introduced while nothing was collapsed before (at that point
    /// `visible_indices` is still the pristine baseline, per
    /// `sync_collapse_mask`'s early-return case).
    ///
    /// Re-pins the cursor to `parent` afterward (parent lines are always
    /// visible, per `apply_collapse_correction`) — without this, collapsing
    /// an entry the cursor was sitting inside (e.g. on one of its now-hidden
    /// continuation lines) would leave `scroll_offset` resolving to
    /// whatever line slides into that screen position once the file
    /// shrinks, silently retargeting the *next* `<`/`>` press at a
    /// different, unrelated entry — from the user's perspective, `>` then
    /// does nothing and the view looks permanently stuck collapsed.
    pub(crate) fn set_continuation_collapsed(&mut self, parent: usize, collapsed: bool) {
        if self.filter.pre_collapse_visible.is_none() {
            self.filter.pre_collapse_visible = Some(self.filter.visible_indices.clone());
        }
        if self.display.collapse_continuations != collapsed {
            self.filter.overridden_groups.insert(parent);
        } else {
            self.filter.overridden_groups.remove(&parent);
        }
        self.recompute_collapse_mask();
        if !self.display.collapse_continuations && self.filter.overridden_groups.is_empty() {
            // Back to "nothing collapsed" — drop the baseline to regain the
            // cheap no-op path on the next filter refresh.
            self.filter.pre_collapse_visible = None;
        }
        self.restore_scroll_to_line(Some(parent));
    }

    pub fn to_file_context(&self) -> Option<FileContext> {
        let source = self.log_manager.source_file()?;
        let marked_lines = self.mark_manager.get_indices();
        let comments = self.comment_manager.get().to_vec();
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
            self.mark_manager.set(ctx.marked_lines.clone());
        }
        if !ctx.comments.is_empty() {
            self.comment_manager.set(ctx.comments.clone());
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
    pub fn build_field_index(&self) -> crate::commands::auto_complete::FieldIndex {
        use std::collections::HashSet;

        let Some(parser) = &self.display.format else {
            return crate::commands::auto_complete::FieldIndex::default();
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

        crate::commands::auto_complete::FieldIndex {
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
    /// Per-file extraction progress updates for Space-ticked files.
    pub progress_rx: tokio::sync::watch::Receiver<crate::ingestion::ArchiveExtractionProgress>,
    /// Delivers both the Space-ticked and 'm'-marked outcomes when the
    /// background apply finishes.
    pub result_rx: tokio::sync::oneshot::Receiver<ArchivePickerApplyResult>,
    /// The "pending" merged tab created immediately (see
    /// `App::create_pending_merged_tab`) when the tree has any merge-marked
    /// file, and `merge_progress_rx`'s own, separate progress updates for
    /// it — kept apart from `progress_rx` since merge-marked and
    /// Space-ticked files are extracted one after another in the same
    /// background task and would otherwise overwrite each other's progress
    /// on a shared channel.
    pub merge_tab_idx: Option<usize>,
    pub merge_progress_rx:
        Option<tokio::sync::watch::Receiver<crate::ingestion::ArchiveExtractionProgress>>,
    /// Total merge-marked file count, for rendering `merge_progress_rx`'s
    /// `file_index` as "x/total".
    pub merge_total: usize,
}

/// Result of [`crate::ui::App::apply_archive_picker`]'s background task —
/// the Space-ticked (extract-to-separate-tabs) and 'm'-marked
/// (extract-and-merge-into-one-tab) outcomes, tracked independently so a
/// failure in one (e.g. an unrecognized format among merge-marked files)
/// never blocks the other from succeeding.
pub struct ArchivePickerApplyResult {
    pub selected_files: Result<Vec<crate::ingestion::ExtractedFile>, String>,
    /// `None` when nothing was merge-marked.
    pub merge_result: Option<Result<Vec<crate::ingestion::MergeMarkedSource>, String>>,
}

/// Tracks an in-progress background archive *listing* (the pre-extraction
/// file-tree scan shown in the archive picker popup).
pub struct ArchiveListingState {
    /// Path to the archive being listed, carried through to the picker mode
    /// once listing finishes (extraction later re-opens the same path).
    pub source_path: String,
    /// Delivers the listed tree (or error) when listing finishes.
    pub result_rx: tokio::sync::oneshot::Receiver<Result<ArchiveTree, String>>,
}

/// Tracks an in-progress background fetch of a single lazy archive node's
/// raw bytes, dispatched by `App::begin_archive_node_expand` and applied by
/// `App::poll_archive_expand`.
pub struct ArchiveExpandState {
    pub node_id: NodeId,
    /// Delivers the node's raw archive bytes (or a read failure) when the
    /// fetch finishes. Parsing those bytes into real children happens
    /// synchronously on the main thread, in `ArchiveTree::expand_lazy_node`.
    pub result_rx: tokio::sync::oneshot::Receiver<Result<Vec<u8>, String>>,
}

/// Tracks an in-progress background read+format-detect of files merge-marked
/// in a directory picker — the directory-opening counterpart of
/// `ArchiveExtractionState`'s merge path, minus decompression (a directory's
/// files are already real files on disk).
pub struct DirectoryMergeState {
    /// The "pending" merged tab created immediately (see
    /// `App::create_pending_merged_tab`), filled in once every source has
    /// been read or removed if reading fails.
    pub tab_idx: usize,
    pub total: usize,
    /// Count of sources read so far.
    pub progress_rx: tokio::sync::watch::Receiver<usize>,
    pub result_rx:
        tokio::sync::oneshot::Receiver<Result<Vec<crate::ingestion::MergeMarkedSource>, String>>,
}

/// Tracks an in-progress background build of a picker-triggered merged
/// tab's index (see `App::start_merge_build_streaming`). Sources are folded
/// in one at a time on a background thread; each update carries the merged
/// index recomputed so far, applied to the live tab by
/// `App::poll_merge_builds` as it arrives so the tab fills in progressively
/// instead of staying empty until every source has been read.
pub struct MergeBuildState {
    pub tab_idx: usize,
    /// The same source readers the tab's `FileReader::from_merged` was
    /// built with — kept here so each incremental update can rebuild the
    /// merged view without needing to look anything up on the tab itself.
    pub sources_arc: Arc<Vec<crate::ingestion::FileReader>>,
    pub update_rx: std::sync::mpsc::Receiver<crate::ui::tab_state::merged::MergeBuildUpdate>,
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
    use crate::db::Comment;
    use crate::db::LogManager;
    use crate::db::{AppSettingsStore, Database, FileContext};
    use crate::filters::{FilterOptions, FilterType};
    use crate::ingestion::FileReader;
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
    async fn test_is_temp_backed_false_for_a_regular_tab() {
        let tab = make_tab(&["line1"]).await;
        assert!(!tab.is_temp_backed());
    }

    #[tokio::test]
    async fn test_is_temp_backed_true_with_archive_temp() {
        let mut tab = make_tab(&["line1"]).await;
        tab.archive_temp = Some(tempfile::NamedTempFile::new().unwrap());
        assert!(tab.is_temp_backed());
    }

    #[tokio::test]
    async fn test_is_temp_backed_true_with_merged_temp() {
        let mut tab = make_tab(&["line1"]).await;
        tab.merged_temp = Some(tempfile::NamedTempFile::new().unwrap());
        assert!(tab.is_temp_backed());
    }

    #[tokio::test]
    async fn test_refresh_visible_all_lines() {
        let tab = make_tab(&["line1", "line2", "line3", "line4", "line5"]).await;
        assert_eq!(tab.filter.visible_indices.len(), 5);
    }

    #[tokio::test]
    async fn test_refresh_visible_marks_only() {
        let mut tab = make_tab(&["line1", "line2", "line3", "line4", "line5"]).await;
        tab.mark_manager.toggle(0);
        tab.mark_manager.toggle(2);
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
        tab.mark_manager.toggle(1);
        tab.mark_manager.toggle(3);
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
        tab.mark_manager.toggle(1);
        tab.mark_manager.toggle(3);
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
        tab.mark_manager.toggle(4);
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
        assert!(tab.mark_manager.is_marked(0));
        assert!(tab.mark_manager.is_marked(2));
        assert!(tab.comment_manager.has(0));
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
        assert!(!tab.mark_manager.is_marked(0));
        assert!(!tab.comment_manager.has(0));
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
            app.display.show_mode_bar,
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
        assert!(
            app.display.wrap,
            "config Some(true) should override DB false"
        );
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
    async fn test_begin_search_matches_custom_schema_template_reconstruction() {
        // Search offsets must be computed against the same reconstructed
        // text the log panel renders (schema template + collapsed
        // hidden-field separator), not the generic column layout —
        // otherwise a match's highlighted position drifts from what's on
        // screen, the same divergence class fixed for Visual Char Mode.
        let line = "INFO/Syscon/StartupMgr, hello there";
        let mut tab = make_tab(&[line]).await;
        tab.filter.visible_indices = VisibleLines::Filtered(vec![0]);
        let cfg = crate::config::CustomSchemaConfig {
            name: "acme".to_string(),
            description: None,
            template: Some(
                "{level}/{component}/{feature}, {message}"
                    .to_string()
                    .into(),
            ),
            pattern: None,
            fields: Default::default(),
            levels: Default::default(),
            multiline: false,
            ..Default::default()
        };
        tab.display.format = Some(std::sync::Arc::new(
            crate::parser::CustomParser::from_config(&cfg).unwrap(),
        ));
        tab.display.hidden_fields.insert("component".to_string());

        tab.begin_search("StartupMgr, hello", true, false);
        drain_search(&mut tab).await;

        let results = tab.search.query.get_results();
        assert_eq!(
            results.len(),
            1,
            "expected exactly one match against the reconstructed (hidden-field-collapsed) text"
        );
        let expected_start = "INFO/StartupMgr, hello there"
            .find("StartupMgr, hello")
            .unwrap();
        assert_eq!(results[0].matches[0].0, expected_start);
    }

    #[tokio::test]
    async fn test_begin_search_pattern_based_custom_schema_hides_field() {
        // A `pattern`-based (regex) custom schema has no `template_segments`
        // — it must fall back to the generic column layout (which already
        // omits hidden fields) rather than a nonexistent reconstruction.
        let line = "INFO shh needle-in-message";
        let mut tab = make_tab(&[line]).await;
        tab.filter.visible_indices = VisibleLines::Filtered(vec![0]);
        let cfg = crate::config::CustomSchemaConfig {
            name: "test".to_string(),
            description: None,
            template: None,
            pattern: Some("^(?P<level>\\w+) (?P<secret>\\w+) (?P<message>.*)$".to_string()),
            fields: [("secret".to_string(), "extra".to_string())]
                .into_iter()
                .collect(),
            levels: Default::default(),
            multiline: false,
            ..Default::default()
        };
        tab.display.format = Some(std::sync::Arc::new(
            crate::parser::CustomParser::from_config(&cfg).unwrap(),
        ));
        tab.display.hidden_fields.insert("secret".to_string());

        tab.begin_search("shh", true, false);
        drain_search(&mut tab).await;
        assert!(
            tab.search.query.get_results().is_empty(),
            "hidden field content must not be matched"
        );

        tab.begin_search("needle-in-message", true, false);
        drain_search(&mut tab).await;
        assert_eq!(tab.search.query.get_results().len(), 1);
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
        tab.mark_manager.toggle(0);
        tab.mark_manager.toggle(1);
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
        tab.mark_manager.toggle(0);
        tab.mark_manager.toggle(2);
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
            highlight_mode: false,
            group_fingerprint: vec![],
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
            highlight_mode: false,
            group_fingerprint: vec![],
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
    async fn test_begin_filter_refresh_cache_miss_on_group_style_change() {
        // Regression test: a filter with no color of its own falling back to
        // its group's style must re-scan (and thus re-derive its highlight
        // color) when the group's style changes, even though the filter
        // definitions themselves (the old cache key) are unchanged.
        let mut tab = make_tab(&["error line", "info line"]).await;
        tab.log_manager
            .add_filter_with_color(
                "error".to_string(),
                FilterType::Include,
                FilterOptions::default().group("errs"),
            )
            .await;
        tab.log_manager
            .set_group_style("errs", Some("Red"), None, true)
            .await;
        let fingerprint: Vec<crate::filters::FilterDef> = tab
            .log_manager
            .get_filters()
            .iter()
            .filter(|f| f.enabled)
            .cloned()
            .collect();
        // Cache records the group's style *before* it was changed to Red —
        // filters/line_count/etc. are otherwise identical to the current state.
        tab.filter.cached_scan = Some(CachedScanResult {
            filter_fingerprint: fingerprint,
            line_count: tab.file_reader.line_count(),
            raw_mode: false,
            highlight_mode: false,
            group_fingerprint: vec![],
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
        // Cache miss: background scan is spawned, so styles get rebuilt
        // against the group's current (Red) style.
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
            ignore_case: false,
            group: None,
        };
        tab.filter.cached_scan = Some(CachedScanResult {
            filter_fingerprint: vec![stale_filter],
            line_count: tab.file_reader.line_count(),
            raw_mode: false,
            highlight_mode: false,
            group_fingerprint: vec![],
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
            highlight_mode: false,
            group_fingerprint: vec![],
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
    async fn test_begin_filter_refresh_cache_miss_on_highlight_mode_change() {
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
        // Cache was built with highlight_mode=false, but tab now has it on.
        tab.filter.cached_scan = Some(CachedScanResult {
            filter_fingerprint: fingerprint,
            line_count: tab.file_reader.line_count(),
            raw_mode: false,
            highlight_mode: false,
            group_fingerprint: vec![],
            view: (
                VisibleLines::Filtered(vec![0]),
                tab.filter.manager.clone(),
                tab.filter.text_styles.clone(),
                tab.filter.date_styles.clone(),
                tab.filter.field_styles.clone(),
            ),
            match_counts: vec![1],
        });
        tab.filter.highlight_mode = true;
        tab.begin_filter_refresh();
        assert!(
            tab.filter.handle.is_some(),
            "toggling highlight_mode must invalidate the cached scan"
        );
    }

    #[tokio::test]
    async fn test_highlight_mode_bypasses_visibility_wholefile_path() {
        let mut tab = make_tab(&["ERROR a", "INFO b", "ERROR c"]).await;
        tab.log_manager
            .add_filter_with_color(
                "ERROR".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        tab.filter.highlight_mode = true;
        tab.begin_filter_refresh();
        let mut h = tab.filter.handle.take().unwrap();
        let mut all_visible = Vec::new();
        let mut final_counts = None;
        while let Some(chunk) = h.result_rx.recv().await {
            all_visible.extend(chunk.visible);
            if chunk.is_last {
                final_counts = chunk.filter_match_counts;
                break;
            }
        }
        assert_eq!(
            all_visible,
            vec![0, 1, 2],
            "highlight mode must show every line"
        );
        assert_eq!(
            final_counts.expect("counts must be Some"),
            vec![2],
            "match counts must stay accurate under highlight mode"
        );
    }

    #[tokio::test]
    async fn test_highlight_mode_off_filters_normally() {
        let mut tab = make_tab(&["ERROR a", "INFO b", "ERROR c"]).await;
        tab.log_manager
            .add_filter_with_color(
                "ERROR".to_string(),
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
        assert_eq!(all_visible, vec![0, 2]);
    }

    #[tokio::test]
    async fn test_highlight_mode_bypasses_visibility_per_line_path() {
        let mut tab = make_tab(&["ERROR a", "INFO b", "ERROR c"]).await;
        tab.log_manager
            .add_filter_with_color(
                "ERROR".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        // A field filter forces the per-line scan path (it never matches
        // anything here since these lines have no parser, but that's fine —
        // it's here purely to exercise the `!use_wholefile` branch).
        tab.log_manager
            .add_filter_with_color(
                "@field:level:error".to_string(),
                FilterType::Exclude,
                FilterOptions::default(),
            )
            .await;
        tab.filter.highlight_mode = true;
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
            vec![0, 1, 2],
            "highlight mode must show every line on the per-line path too"
        );
    }

    #[tokio::test]
    async fn test_highlight_mode_stacks_with_marks_only() {
        let mut tab = make_tab(&["line0", "line1", "line2", "line3"]).await;
        tab.mark_manager.toggle(1);
        tab.filter.show_marks_only = true;
        tab.filter.highlight_mode = true;
        tab.begin_filter_refresh();
        assert_eq!(
            tab.filter.visible_indices,
            VisibleLines::Filtered(vec![1]),
            "marks-only must still restrict to marked lines even in highlight mode"
        );
    }

    #[tokio::test]
    async fn test_highlight_mode_no_effect_when_filtering_disabled() {
        let mut tab = make_tab(&["ERROR a", "INFO b"]).await;
        tab.log_manager
            .add_filter_with_color(
                "ERROR".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        tab.filter.enabled = false;
        tab.filter.highlight_mode = true;
        tab.begin_filter_refresh();
        assert_eq!(tab.filter.visible_indices, VisibleLines::All(2));
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
    async fn test_next_error_position_uses_custom_schema_level_override() {
        let mut tab = make_tab(&["INFO line", "SEV1 oops", "SEV2 careful"]).await;
        let cfg = crate::config::CustomSchemaConfig {
            name: "sev".to_string(),
            description: None,
            template: Some("{level} {message}".to_string().into()),
            pattern: None,
            fields: Default::default(),
            levels: crate::config::CustomLevelValues {
                error: vec!["SEV1".to_string()],
                warning: vec!["SEV2".to_string()],
            },
            multiline: false,
            ..Default::default()
        };
        tab.display.format = Some(std::sync::Arc::new(
            crate::parser::CustomParser::from_config(&cfg).unwrap(),
        ));
        assert_eq!(tab.next_error_position(0), Some(1));
        assert_eq!(tab.next_warning_position(0), Some(2));
    }

    #[tokio::test]
    async fn test_next_error_position_ignores_unmapped_custom_level() {
        // Without a declared override, a non-keyword level value like "SEV1"
        // must not be (mis)matched as an error.
        let mut tab = make_tab(&["INFO line", "SEV1 oops"]).await;
        let cfg = crate::config::CustomSchemaConfig {
            name: "sev".to_string(),
            description: None,
            template: Some("{level} {message}".to_string().into()),
            pattern: None,
            fields: Default::default(),
            levels: Default::default(),
            multiline: false,
            ..Default::default()
        };
        tab.display.format = Some(std::sync::Arc::new(
            crate::parser::CustomParser::from_config(&cfg).unwrap(),
        ));
        assert_eq!(tab.next_error_position(0), None);
    }

    /// Regression test: `e`/`w` navigation on a merged tab must classify
    /// each line's level using *that line's own source's* parser, not
    /// `tab.display.format` — which `App::build_merged_tab` always sets to
    /// `None`, since a merged tab's lines can come from sources with
    /// different formats. Before this was fixed, every merged-tab line fell
    /// back to the generic keyword scan, which doesn't know a custom
    /// schema's non-standard level values (like "SEV1" here).
    #[tokio::test]
    async fn test_next_error_position_on_merged_tab_uses_per_source_parser() {
        use crate::ingestion::MergedEntry;

        let cfg = crate::config::CustomSchemaConfig {
            name: "sev".to_string(),
            description: None,
            template: Some("{level} {message}".to_string().into()),
            pattern: None,
            fields: Default::default(),
            levels: crate::config::CustomLevelValues {
                error: vec!["SEV1".to_string()],
                warning: vec![],
            },
            multiline: false,
            ..Default::default()
        };
        let source_a_parser = Arc::new(crate::parser::CustomParser::from_config(&cfg).unwrap());

        // source_a (the custom "sev" schema, at sources[1] below): line 0 is
        // a "SEV1" error that only source_a's own parser recognizes as such.
        let source_a = FileReader::from_bytes(b"SEV1 oops".to_vec());
        // source_b (at sources[0]): plain, no detected format at all.
        let source_b = FileReader::from_bytes(b"hello world".to_vec());

        let entries = vec![
            MergedEntry {
                sort_key: [0u8; 23],
                source_idx: 0, // source_b ("hello world")
                line_idx: 0,
            },
            MergedEntry {
                sort_key: [1u8; 23],
                source_idx: 1, // source_a ("SEV1 oops")
                line_idx: 0,
            },
        ];
        let file_reader =
            FileReader::from_merged(Arc::new(entries), Arc::new(vec![source_b, source_a]));
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        let mut tab = TabState::new(file_reader, log_manager, "merged".to_string());
        tab.display.format = None;
        tab.filter.visible_indices = VisibleLines::Filtered(vec![0, 1]);
        tab.merged = Some(MergedState {
            source_tab_indices: vec![0, 1],
            source_parsers: vec![None, Some(source_a_parser)],
            source_labels: vec!["b".to_string(), "a".to_string()],
            source_line_counts: vec![1, 1],
            label_col_width: 1,
            stopped: false,
            building: None,
        });

        assert_eq!(
            tab.next_error_position(0),
            Some(1),
            "the SEV1 line (merged position 1, from source_idx 1, the 'sev' \
             schema) must be found using that source's own parser"
        );
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
    async fn test_next_marked_position_finds_forward() {
        let mut tab = make_tab(&["line0", "line1", "line2", "line3"]).await;
        tab.mark_manager.toggle(1);
        tab.mark_manager.toggle(3);
        assert_eq!(tab.next_marked_position(0), Some(1));
        assert_eq!(tab.next_marked_position(1), Some(3));
        assert_eq!(tab.next_marked_position(3), None);
    }

    #[tokio::test]
    async fn test_prev_marked_position_finds_backward() {
        let mut tab = make_tab(&["line0", "line1", "line2", "line3"]).await;
        tab.mark_manager.toggle(1);
        tab.mark_manager.toggle(3);
        assert_eq!(tab.prev_marked_position(3), Some(1));
        assert_eq!(tab.prev_marked_position(1), None);
    }

    #[tokio::test]
    async fn test_marked_position_no_marks() {
        let tab = make_tab(&["line0", "line1"]).await;
        assert_eq!(tab.next_marked_position(0), None);
        assert_eq!(tab.prev_marked_position(1), None);
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

    fn multiline_schema_config(multiline: bool) -> crate::config::CustomSchemaConfig {
        crate::config::CustomSchemaConfig {
            name: "test".to_string(),
            description: None,
            template: Some("{level} {message}".to_string().into()),
            pattern: None,
            fields: Default::default(),
            levels: Default::default(),
            multiline,
            ..Default::default()
        }
    }

    /// A schema whose pattern matches only a bare header line (no `message`
    /// role captured at all) — models `journalctl --output=verbose`, whose
    /// header line carries no message text of its own.
    fn header_only_schema_config() -> crate::config::CustomSchemaConfig {
        crate::config::CustomSchemaConfig {
            name: "header-only".to_string(),
            description: None,
            template: None,
            pattern: Some(r"^HEADER (?P<id>\d+)$".to_string()),
            fields: [("id".to_string(), "extra".to_string())]
                .into_iter()
                .collect(),
            levels: Default::default(),
            multiline: true,
            ..Default::default()
        }
    }

    #[test]
    fn test_parse_line_with_continuation_non_multiline_schema_unchanged() {
        let reader = FileReader::from_bytes(
            b"INFO hello\n  stack trace line 1\n  stack trace line 2\n".to_vec(),
        );
        let parser =
            crate::parser::CustomParser::from_config(&multiline_schema_config(false)).unwrap();
        let cmap = build_continuation_map(&reader, &parser);
        assert_eq!(cmap, vec![0, 0, 0]);

        let parts = parse_line_with_continuation(&parser, &reader, Some(&cmap), 0).unwrap();
        assert_eq!(parts.message, Some("hello"));
    }

    #[test]
    fn test_parse_line_with_continuation_merges_own_message_and_continuations() {
        let reader = FileReader::from_bytes(
            b"INFO hello\n  stack trace line 1\n  stack trace line 2\n".to_vec(),
        );
        let parser =
            crate::parser::CustomParser::from_config(&multiline_schema_config(true)).unwrap();
        let cmap = build_continuation_map(&reader, &parser);
        assert_eq!(cmap, vec![0, 0, 0]);

        let parts = parse_line_with_continuation(&parser, &reader, Some(&cmap), 0).unwrap();
        assert_eq!(
            parts.message,
            Some("hello\n  stack trace line 1\n  stack trace line 2")
        );
        // Zero-copy: the merged message must be a literal subslice of the
        // reader's backing buffer, not an owned/allocated string.
        let data_range = reader.data().as_ptr_range();
        let msg = parts.message.unwrap();
        let msg_range = msg.as_bytes().as_ptr_range();
        assert!(data_range.start <= msg_range.start && msg_range.end <= data_range.end);
    }

    #[test]
    fn test_parse_line_with_continuation_synthesizes_message_when_none_captured() {
        // journalctl-verbose-style: header line has no message field at all;
        // the continuation (indented KEY=VALUE) lines become the message.
        let reader = FileReader::from_bytes(b"HEADER 1\n  KEY1=val1\n  KEY2=val2\n".to_vec());
        let parser =
            crate::parser::CustomParser::from_config(&header_only_schema_config()).unwrap();
        let cmap = build_continuation_map(&reader, &parser);
        assert_eq!(cmap, vec![0, 0, 0]);

        let parts = parse_line_with_continuation(&parser, &reader, Some(&cmap), 0).unwrap();
        assert_eq!(parts.message, Some("  KEY1=val1\n  KEY2=val2"));
    }

    #[test]
    fn test_parse_line_with_continuation_no_continuation_lines_unchanged() {
        let reader = FileReader::from_bytes(b"INFO hello\nINFO world\n".to_vec());
        let parser =
            crate::parser::CustomParser::from_config(&multiline_schema_config(true)).unwrap();
        let cmap = build_continuation_map(&reader, &parser);
        assert_eq!(cmap, vec![0, 1]); // both lines parse; neither is a continuation

        let parts = parse_line_with_continuation(&parser, &reader, Some(&cmap), 0).unwrap();
        assert_eq!(parts.message, Some("hello"));
    }

    #[test]
    fn test_parse_line_with_continuation_no_cmap_is_passthrough() {
        let reader = FileReader::from_bytes(b"INFO hello\n  stack trace line 1\n".to_vec());
        let parser =
            crate::parser::CustomParser::from_config(&multiline_schema_config(true)).unwrap();

        let parts = parse_line_with_continuation(&parser, &reader, None, 0).unwrap();
        assert_eq!(parts.message, Some("hello"));
    }

    /// Header `"### Start transaction {id}"`, continuation fields for
    /// `field1`/`field2`, terminated by `"### End transaction"` — the
    /// scenario from the multiline-schema feature request.
    fn transaction_schema_config() -> crate::config::CustomSchemaConfig {
        use crate::config::{ContinuationFieldSpec, TemplateLine, TemplateValue};
        let lines = vec![
            TemplateLine::Str("### Start transaction {id}".to_string()),
            TemplateLine::Plain(ContinuationFieldSpec {
                template: Some("field1: {field1}".to_string()),
                pattern: None,
                fields: Default::default(),
                json: false,
            }),
            TemplateLine::Plain(ContinuationFieldSpec {
                template: Some("field2: {field2}".to_string()),
                pattern: None,
                fields: Default::default(),
                json: false,
            }),
            TemplateLine::Str("### End transaction".to_string()),
        ];
        crate::config::CustomSchemaConfig {
            name: "transaction".to_string(),
            description: None,
            template: Some(TemplateValue::Lines(lines)),
            pattern: None,
            fields: [("id".to_string(), "extra".to_string())]
                .into_iter()
                .collect(),
            levels: Default::default(),
            multiline: false,
            ..Default::default()
        }
    }

    /// Header `"### Start transaction {id}"` with a repeating `operations`
    /// continuation group, terminated by `"### End transaction"`.
    fn transaction_schema_config_with_operations_group() -> crate::config::CustomSchemaConfig {
        use crate::config::{
            ContinuationFieldSpec, TemplateGroupConfig, TemplateLine, TemplateValue,
        };
        let lines = vec![
            TemplateLine::Str("### Start transaction {id}".to_string()),
            TemplateLine::Group(TemplateGroupConfig {
                vec: "operations".to_string(),
                template: ContinuationFieldSpec {
                    template: Some("operation_type: {operation_type}".to_string()),
                    pattern: None,
                    fields: Default::default(),
                    json: false,
                },
                fields: vec![ContinuationFieldSpec {
                    template: Some("object_name: {object_name}".to_string()),
                    pattern: None,
                    fields: Default::default(),
                    json: false,
                }],
                auto_fields: false,
            }),
            TemplateLine::Str("### End transaction".to_string()),
        ];
        crate::config::CustomSchemaConfig {
            name: "transaction".to_string(),
            description: None,
            template: Some(TemplateValue::Lines(lines)),
            pattern: None,
            fields: [("id".to_string(), "extra".to_string())]
                .into_iter()
                .collect(),
            levels: Default::default(),
            multiline: false,
            ..Default::default()
        }
    }

    #[test]
    fn test_apply_continuation_fields_merges_extracted_fields_into_parent() {
        let reader = FileReader::from_bytes(
            b"### Start transaction 42\nfield1: 10\nfield2: 3\n### End transaction\n".to_vec(),
        );
        let parser =
            crate::parser::CustomParser::from_config(&transaction_schema_config()).unwrap();
        let cmap = build_continuation_map(&reader, &parser);
        assert_eq!(cmap, vec![0, 0, 0, 0]);

        let parts = parse_line_with_continuation(&parser, &reader, Some(&cmap), 0).unwrap();
        let extra = |k: &str| {
            parts
                .extra_fields
                .iter()
                .find(|(_, key, _)| *key == k)
                .map(|(_, _, v)| *v)
        };
        assert_eq!(extra("id"), Some("42"));
        assert_eq!(extra("field1"), Some("10"));
        assert_eq!(extra("field2"), Some("3"));
    }

    #[test]
    fn test_apply_continuation_fields_populates_field_groups() {
        let reader = FileReader::from_bytes(
            b"### Start transaction 42\noperation_type: CREATE\nobject_name: txCarrier1\noperation_type: DELETE\nobject_name: txCarrier2\n### End transaction\n".to_vec(),
        );
        let parser = crate::parser::CustomParser::from_config(
            &transaction_schema_config_with_operations_group(),
        )
        .unwrap();
        let cmap = build_continuation_map(&reader, &parser);

        let parts = parse_line_with_continuation(&parser, &reader, Some(&cmap), 0).unwrap();
        assert_eq!(parts.field_groups.len(), 1);
        let (name, items) = &parts.field_groups[0];
        assert_eq!(*name, "operations");
        assert_eq!(items.len(), 2);
        assert!(items[0].fields.contains(&(
            crate::parser::FieldSemantic::Extra,
            "object_name",
            "txCarrier1"
        )));
        assert!(items[1].fields.contains(&(
            crate::parser::FieldSemantic::Extra,
            "object_name",
            "txCarrier2"
        )));
    }

    #[test]
    fn test_apply_continuation_fields_stops_at_end_pattern_even_if_cmap_groups_further_lines() {
        // Everything after "### End transaction" up to the next header still
        // gets grouped as a continuation by `build_continuation_map` (it only
        // looks at the header pattern) — `apply_continuation_fields` must
        // still stop extracting once it passes the end marker, so the stray
        // "field1: 999" line doesn't get folded into the first record.
        let reader = FileReader::from_bytes(
            b"### Start transaction 42\n\
              field1: 10\n\
              ### End transaction\n\
              field1: 999\n\
              ### Start transaction 43\n\
              field2: 3\n"
                .to_vec(),
        );
        let parser =
            crate::parser::CustomParser::from_config(&transaction_schema_config()).unwrap();
        let cmap = build_continuation_map(&reader, &parser);
        assert_eq!(cmap, vec![0, 0, 0, 0, 4, 4]);

        let mut parts = crate::parser::DisplayParts::default();
        let block_end = apply_continuation_fields(&parser, &reader, &cmap, 0, &mut parts);
        assert_eq!(
            block_end, 2,
            "should stop at the end_pattern line (index 2)"
        );
        assert_eq!(
            parts
                .extra_fields
                .iter()
                .find(|(_, k, _)| *k == "field1")
                .map(|(_, _, v)| *v),
            Some("10"),
            "field1 from before the end marker must be extracted"
        );
        assert!(
            !parts.extra_fields.iter().any(|(_, _, v)| *v == "999"),
            "field1 after the end marker must not leak into this record"
        );
    }

    #[test]
    fn test_wants_continuation_walk_gates_field_extraction_without_multiline() {
        // multiline: false, but a `continuation` block is declared — the
        // walk must still run so field1/field2 get extracted even though
        // `message` is never folded.
        let reader = FileReader::from_bytes(
            b"### Start transaction 42\nfield1: 10\n### End transaction\n".to_vec(),
        );
        let parser =
            crate::parser::CustomParser::from_config(&transaction_schema_config()).unwrap();
        assert!(!parser.merges_continuation_into_message());
        let cmap = build_continuation_map(&reader, &parser);

        let parts = parse_line_with_continuation(&parser, &reader, Some(&cmap), 0).unwrap();
        assert_eq!(parts.message, None, "no message field declared or merged");
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(_, k, v)| *k == "field1" && *v == "10")
        );
    }

    /// Sets `tab`'s format to a custom schema and rebuilds its continuation
    /// map to match — `make_tab` doesn't run format detection, so tests that
    /// override `display.format` directly must also (re)build the map
    /// `refresh_visible_inner` reads, mirroring what `TabState::new` does for
    /// an auto-detected format.
    fn set_custom_format(tab: &mut TabState, cfg: &crate::config::CustomSchemaConfig) {
        let parser = crate::parser::CustomParser::from_config(cfg).unwrap();
        tab.continuation_map = Some(std::sync::Arc::new(build_continuation_map(
            &tab.file_reader,
            &parser,
        )));
        tab.display.format = Some(std::sync::Arc::new(parser));
    }

    #[tokio::test]
    async fn test_multiline_field_filter_matches_message_merged_from_continuation() {
        // Only the continuation lines mention "NullPointerException" — the
        // header line's own message ("hello") does not.
        let mut tab = make_tab(&[
            "INFO hello",
            "  boom NullPointerException",
            "  at foo.bar(Baz.java:42)",
        ])
        .await;
        set_custom_format(&mut tab, &multiline_schema_config(true));

        tab.log_manager
            .add_filter_with_color(
                crate::filters::encode_field_filter(
                    &[("message".to_string(), "NullPointerException".to_string())],
                    None,
                ),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;

        tab.refresh_visible();

        let visible: Vec<usize> = tab.filter.visible_indices.iter().collect();
        assert!(
            visible.contains(&0),
            "header record must be visible: its merged message includes the \
             continuation lines' text; got {visible:?}"
        );
    }

    #[tokio::test]
    async fn test_non_multiline_field_filter_ignores_continuation_content() {
        // Same fixture and filter as above, but the schema does not opt into
        // `multiline` — the header's own message ("hello") never matches, so
        // the record must not be shown.
        let mut tab = make_tab(&[
            "INFO hello",
            "  boom NullPointerException",
            "  at foo.bar(Baz.java:42)",
        ])
        .await;
        set_custom_format(&mut tab, &multiline_schema_config(false));

        tab.log_manager
            .add_filter_with_color(
                crate::filters::encode_field_filter(
                    &[("message".to_string(), "NullPointerException".to_string())],
                    None,
                ),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;

        tab.refresh_visible();

        let visible: Vec<usize> = tab.filter.visible_indices.iter().collect();
        assert!(
            !visible.contains(&0),
            "header record's own message never contains the searched text \
             without merging; got {visible:?}"
        );
    }

    /// Regression test: toggling raw mode on a tab whose format was
    /// (correctly or incorrectly) detected as a multiline schema must not
    /// leave the old continuation map gating visibility. A line that
    /// individually matches an include filter has to show up even though,
    /// under the stale multiline grouping, its structured "parent" line
    /// doesn't match.
    #[tokio::test]
    async fn test_raw_mode_bypasses_stale_continuation_grouping() {
        // Only the continuation line mentions "NullPointerException" — the
        // header line's own text ("hello") does not.
        let mut tab = make_tab(&[
            "INFO hello",
            "  boom NullPointerException",
            "  at foo.bar(Baz.java:42)",
        ])
        .await;
        set_custom_format(&mut tab, &multiline_schema_config(true));

        tab.log_manager
            .add_filter_with_color(
                "NullPointerException".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;

        // Sanity check: under normal (non-raw) multiline grouping, the
        // matching continuation promotes its whole record — including the
        // header, whose own text never matched.
        tab.refresh_visible();
        let grouped: Vec<usize> = tab.filter.visible_indices.iter().collect();
        assert_eq!(
            grouped,
            vec![0, 1, 2],
            "sanity: the whole record should be promoted by the continuation's match"
        );

        tab.display.raw_mode = true;
        tab.refresh_visible();

        let visible: Vec<usize> = tab.filter.visible_indices.iter().collect();
        assert_eq!(
            visible,
            vec![1],
            "raw mode must evaluate lines independently of the stale multiline \
             continuation map — only the individually matching line, no group \
             promotion — got {visible:?}"
        );
    }

    /// Regression test: `TabState::apply_format` (used by `:schema none` /
    /// `:schema <name>`) must clear the continuation map along with the
    /// format, so a subsequent scan doesn't keep gating a matching line's
    /// visibility on a "parent" line from a format the tab no longer uses.
    #[tokio::test]
    async fn test_apply_format_none_clears_continuation_map_and_rescans() {
        let mut tab = make_tab(&[
            "INFO hello",
            "  boom NullPointerException",
            "  at foo.bar(Baz.java:42)",
        ])
        .await;
        set_custom_format(&mut tab, &multiline_schema_config(true));
        assert!(tab.continuation_map.is_some());

        tab.log_manager
            .add_filter_with_color(
                "NullPointerException".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;

        tab.apply_format(None);
        assert!(
            tab.continuation_map.is_none(),
            "clearing the schema must drop the now-stale continuation map"
        );
        assert!(
            tab.filter.handle.is_some(),
            "apply_format must trigger a rescan against the new format"
        );

        let mut h = tab.filter.handle.take().unwrap();
        let mut all_visible = Vec::new();
        while let Some(chunk) = h.result_rx.recv().await {
            all_visible.extend(chunk.visible);
            if chunk.is_last {
                break;
            }
        }
        assert!(
            all_visible.contains(&1),
            "with no format, the matching line must be independently visible, \
             got {all_visible:?}"
        );
    }

    /// Regression test: `apply_continuation_correction` must not panic when the
    /// visible-index list contains indices that lie beyond the continuation map
    /// (i.e. the file grew after the map was built).
    #[test]
    fn test_continuation_correction_indices_beyond_cmap() {
        // cmap covers 3 lines (indices 0-2); all are their own parent.
        let cmap = vec![0usize, 1, 2];
        // visible contains index 3, which is beyond cmap — simulates a file that
        // gained a new line after the map was built.
        let mut visible = VisibleLines::Filtered(vec![0, 1, 2, 3]);
        // Must not panic.
        apply_continuation_correction(&mut visible, &cmap, false);
        let result: Vec<usize> = visible.iter().collect();
        // All four lines should be visible and in order.
        assert_eq!(result, vec![0, 1, 2, 3]);
    }

    /// With `default_collapsed` on and no overrides, `apply_collapse_correction`
    /// hides every continuation line and keeps every parent line.
    #[test]
    fn test_collapse_correction_hides_all_continuations_by_default() {
        // Lines 0,3 are parents; 1,2 continue 0; 4 continues 3.
        let cmap = vec![0usize, 0, 0, 3, 3];
        let mut visible = VisibleLines::Filtered(vec![0, 1, 2, 3, 4]);
        apply_collapse_correction(&mut visible, &cmap, true, &HashSet::new());
        let result: Vec<usize> = visible.iter().collect();
        assert_eq!(result, vec![0, 3]);
    }

    /// A parent line present in `overridden_groups` keeps its continuation
    /// lines visible when `default_collapsed` is on; other groups stay
    /// collapsed.
    #[test]
    fn test_collapse_correction_keeps_overridden_group_visible_when_default_collapsed() {
        let cmap = vec![0usize, 0, 0, 3, 3];
        let mut visible = VisibleLines::Filtered(vec![0, 1, 2, 3, 4]);
        let overridden: HashSet<usize> = [0].into_iter().collect();
        apply_collapse_correction(&mut visible, &cmap, true, &overridden);
        let result: Vec<usize> = visible.iter().collect();
        assert_eq!(result, vec![0, 1, 2, 3]);
    }

    /// A parent line present in `overridden_groups` hides its continuation
    /// lines even when `default_collapsed` is off — this is what lets `<`
    /// collapse a single entry without needing `:collapse` first.
    #[test]
    fn test_collapse_correction_overridden_group_collapses_when_default_expanded() {
        let cmap = vec![0usize, 0, 0, 3, 3];
        let mut visible = VisibleLines::Filtered(vec![0, 1, 2, 3, 4]);
        let overridden: HashSet<usize> = [0].into_iter().collect();
        apply_collapse_correction(&mut visible, &cmap, false, &overridden);
        let result: Vec<usize> = visible.iter().collect();
        assert_eq!(result, vec![0, 3, 4]);
    }

    /// Indices beyond the continuation map (file grew after the map was
    /// built) are preserved unchanged, same convention as
    /// `apply_continuation_correction`.
    #[test]
    fn test_collapse_correction_indices_beyond_cmap() {
        let cmap = vec![0usize, 1, 2];
        let mut visible = VisibleLines::Filtered(vec![0, 1, 2, 3]);
        apply_collapse_correction(&mut visible, &cmap, true, &HashSet::new());
        let result: Vec<usize> = visible.iter().collect();
        assert_eq!(result, vec![0, 1, 2, 3]);
    }

    /// Unlike `apply_continuation_correction` (a no-op with no active
    /// filters), collapse must still hide continuation lines when every
    /// line is otherwise visible (`VisibleLines::All`) — that's the primary
    /// use case for `:collapse` on an unfiltered file.
    #[test]
    fn test_collapse_correction_applies_to_all_visible() {
        let cmap = vec![0usize, 0, 0];
        let mut visible = VisibleLines::All(3);
        apply_collapse_correction(&mut visible, &cmap, true, &HashSet::new());
        let result: Vec<usize> = visible.iter().collect();
        assert_eq!(result, vec![0]);
    }

    /// With `default_collapsed` off and no overrides, nothing is hidden —
    /// confirms the XOR formula's identity case.
    #[test]
    fn test_collapse_correction_noop_when_default_expanded_no_overrides() {
        let cmap = vec![0usize, 0, 0];
        let mut visible = VisibleLines::All(3);
        apply_collapse_correction(&mut visible, &cmap, false, &HashSet::new());
        let result: Vec<usize> = visible.iter().collect();
        assert_eq!(result, vec![0, 1, 2]);
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
            index.values.get("timestamp").is_none_or(|v| v.is_empty()),
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
            index.values.get("message").is_none_or(|v| v.is_empty()),
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

    fn make_fm_include(pattern: &str) -> FilterManager {
        let f =
            crate::filters::SubstringFilter::new(pattern, FilterDecision::Include, false, 0, false)
                .unwrap();
        FilterManager::new(vec![Box::new(f)], true)
    }

    fn make_fm_exclude(pattern: &str) -> FilterManager {
        let f =
            crate::filters::SubstringFilter::new(pattern, FilterDecision::Exclude, false, 0, false)
                .unwrap();
        FilterManager::new(vec![Box::new(f)], false)
    }

    #[test]
    fn test_line_is_visible_text_include_matches() {
        let fm = make_fm_include("ERROR");
        let dec = fm.evaluate_text(b"ERROR: bad");
        assert!(line_is_visible(
            dec,
            &mut FilterEvalContext::new(fm.has_include(), &[], &mut [], &[], &[], None),
            None,
            b""
        ));
    }

    #[test]
    fn test_line_is_visible_text_include_no_match_hidden() {
        let fm = make_fm_include("ERROR");
        let dec = fm.evaluate_text(b"INFO: fine");
        assert!(!line_is_visible(
            dec,
            &mut FilterEvalContext::new(fm.has_include(), &[], &mut [], &[], &[], None),
            None,
            b""
        ));
    }

    #[test]
    fn test_line_is_visible_text_exclude_hides() {
        let fm = make_fm_exclude("DEBUG");
        let dec = fm.evaluate_text(b"DEBUG: noisy");
        assert!(!line_is_visible(
            dec,
            &mut FilterEvalContext::new(fm.has_include(), &[], &mut [], &[], &[], None),
            None,
            b""
        ));
    }

    #[test]
    fn test_line_is_visible_text_exclude_non_matching_visible() {
        let fm = make_fm_exclude("DEBUG");
        let dec = fm.evaluate_text(b"INFO: keep");
        assert!(line_is_visible(
            dec,
            &mut FilterEvalContext::new(fm.has_include(), &[], &mut [], &[], &[], None),
            None,
            b""
        ));
    }

    #[test]
    fn test_line_is_visible_no_filters_always_visible() {
        assert!(line_is_visible(
            FilterDecision::Neutral,
            &mut FilterEvalContext::new(false, &[], &mut [], &[], &[], None),
            None,
            b""
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
        let dfs = [df];
        let mut ctx = FilterEvalContext::new(false, &dfs, &mut counts, &[], &[], None);
        assert!(line_is_visible(
            FilterDecision::Neutral,
            &mut ctx,
            Some(&parts),
            b""
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
        let dfs = [df];
        let mut ctx = FilterEvalContext::new(false, &dfs, &mut counts, &[], &[], None);
        assert!(!line_is_visible(
            FilterDecision::Neutral,
            &mut ctx,
            Some(&parts),
            b""
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
        let dfs = [df];
        let mut ctx = FilterEvalContext::new(false, &dfs, &mut counts, &[], &[], None);
        assert!(line_is_visible(
            FilterDecision::Neutral,
            &mut ctx,
            Some(&parts),
            b""
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
        let dfs = [df1, df2];
        let mut ctx = FilterEvalContext::new(false, &dfs, &mut counts, &[], &[], None);
        assert!(line_is_visible(
            FilterDecision::Neutral,
            &mut ctx,
            Some(&parts),
            b""
        ));
        assert_eq!(counts[0], 1);
        assert_eq!(counts[1], 1);
    }

    #[test]
    fn test_line_is_visible_field_exclude_hides() {
        use crate::filters::FieldFilter;
        use crate::parser::DisplayParts;
        let exc = FieldFilter {
            conditions: vec![("level".to_string(), "debug".to_string())],
            text: None,
            decision: FilterDecision::Exclude,
        };
        let parts = DisplayParts {
            level: Some("debug"),
            ..Default::default()
        };
        assert!(!line_is_visible(
            FilterDecision::Neutral,
            &mut FilterEvalContext::new(false, &[], &mut [], &[], &[exc], None),
            Some(&parts),
            b"",
        ));
    }

    #[test]
    fn test_line_is_visible_field_include_match_visible() {
        use crate::filters::FieldFilter;
        use crate::parser::DisplayParts;
        let inc = FieldFilter {
            conditions: vec![("level".to_string(), "error".to_string())],
            text: None,
            decision: FilterDecision::Include,
        };
        let parts = DisplayParts {
            level: Some("error"),
            ..Default::default()
        };
        assert!(line_is_visible(
            FilterDecision::Neutral,
            &mut FilterEvalContext::new(false, &[], &mut [], &[inc], &[], None),
            Some(&parts),
            b"",
        ));
    }

    #[test]
    fn test_line_is_visible_field_include_miss_hidden() {
        use crate::filters::FieldFilter;
        use crate::parser::DisplayParts;
        let inc = FieldFilter {
            conditions: vec![("level".to_string(), "error".to_string())],
            text: None,
            decision: FilterDecision::Include,
        };
        let parts = DisplayParts {
            level: Some("info"),
            ..Default::default()
        };
        assert!(!line_is_visible(
            FilterDecision::Neutral,
            &mut FilterEvalContext::new(false, &[], &mut [], &[inc], &[], None),
            Some(&parts),
            b"",
        ));
    }

    #[test]
    fn test_line_is_visible_text_include_beats_field_include_miss() {
        use crate::filters::FieldFilter;
        use crate::parser::DisplayParts;
        // Text include matched; field include miss doesn't override.
        let inc = FieldFilter {
            conditions: vec![("level".to_string(), "error".to_string())],
            text: None,
            decision: FilterDecision::Include,
        };
        let parts = DisplayParts {
            level: Some("info"),
            ..Default::default()
        };
        assert!(line_is_visible(
            FilterDecision::Include,
            &mut FilterEvalContext::new(true, &[], &mut [], &[inc], &[], None),
            Some(&parts),
            b"",
        ));
    }

    #[test]
    fn test_line_is_visible_field_passthrough_when_unparseable() {
        use crate::filters::FieldFilter;
        fn make_inc() -> FieldFilter {
            FieldFilter {
                conditions: vec![("level".to_string(), "error".to_string())],
                text: None,
                decision: FilterDecision::Include,
            }
        }
        // parts=None → field filters do not apply → falls back to text-only logic.
        // has_text_includes=false → visible (no include filter applies)
        assert!(line_is_visible(
            FilterDecision::Neutral,
            &mut FilterEvalContext::new(false, &[], &mut [], &[make_inc()], &[], None),
            None,
            b"",
        ));
        // has_text_includes=true → hidden (include filter present but nothing matched)
        assert!(!line_is_visible(
            FilterDecision::Neutral,
            &mut FilterEvalContext::new(true, &[], &mut [], &[make_inc()], &[], None),
            None,
            b"",
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

    /// Regression test: lines appended to a live/watched file while
    /// collapse mode is on must be masked too, not just whatever was in the
    /// file when `:collapse` ran — otherwise every newly streamed-in entry
    /// would appear permanently expanded regardless of the collapse state.
    #[tokio::test]
    async fn test_filter_new_lines_applies_collapse_mask_to_newly_appended_group() {
        let parsed0 = "2024-07-24T10:00:00Z INFO request processed";
        let access1 = "2019-01-26 20:29:10.000 5.120.204.67 200 GET / HTTP/1.1";
        let data = format!("{parsed0}\n{access1}\n").into_bytes();
        let file_reader = FileReader::from_bytes(data);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        let mut tab = TabState::new(file_reader, log_manager, "test".to_string());
        assert!(tab.continuation_map.is_some());

        tab.display.collapse_continuations = true;
        tab.begin_filter_refresh();
        assert_eq!(
            tab.filter.visible_indices.iter().collect::<Vec<_>>(),
            vec![0],
            "line 1 (a continuation of line 0) must start hidden"
        );

        let old = tab.file_reader.line_count();
        let parsed2 = "2024-07-24T10:01:00Z INFO another request";
        let access3 = "2019-01-26 20:30:00.000 5.120.204.68 200 GET /api HTTP/1.1";
        tab.file_reader
            .append_bytes(format!("{parsed2}\n{access3}\n").as_bytes());
        tab.filter_new_lines(old);

        assert_eq!(
            tab.filter.visible_indices.iter().collect::<Vec<_>>(),
            vec![0, 2],
            "the newly appended group's continuation line (3) must also be \
             hidden, not just the group that existed when :collapse ran"
        );
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
