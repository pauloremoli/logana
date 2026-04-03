use std::sync::Arc;

use crate::filters::{CanonicalTs, timestamp_to_canonical};
use crate::ingestion::{FileReader, MergedEntry};
use crate::parser::LogFormatParser;
use crate::ui::tab_state::year_map::YearMap;

/// Upper bits of compound key encode the source index; lower bits encode line index.
pub const SOURCE_IDX_SHIFT: usize = 24;
const LINE_IDX_MASK: usize = (1 << SOURCE_IDX_SHIFT) - 1;

pub fn encode_key(source_idx: usize, line_idx: usize) -> usize {
    (source_idx << SOURCE_IDX_SHIFT) | (line_idx & LINE_IDX_MASK)
}

pub fn decode_key(key: usize) -> (usize, usize) {
    (key >> SOURCE_IDX_SHIFT, key & LINE_IDX_MASK)
}

pub struct MergedState {
    /// Which tab index each source maps to (for live-update polling).
    pub source_tab_indices: Vec<usize>,
    /// Per-source parser (mirrors what was used for format detection in each source tab).
    pub source_parsers: Vec<Option<Arc<dyn LogFormatParser>>>,
    /// Human-readable label for each source (e.g. filename).
    pub source_labels: Vec<String>,
    /// Line count at the time the merged index was last built, per source.
    /// Used to detect when a source has received new lines.
    pub source_line_counts: Vec<usize>,
    /// Width of the widest source label; used to pad all labels to a fixed column.
    pub label_col_width: usize,
    /// When `true`, live updates from source tabs are no longer applied.
    pub stopped: bool,
}

/// Build a sorted merged index from multiple sources.
///
/// For each source, lines with parseable timestamps are indexed.
/// Continuation lines (those whose `continuation_map[i] != i`) inherit
/// the sort key of their entry-start parent.  Lines before the first
/// parseable timestamp in a source are skipped.
pub fn build_merged_index(
    sources: &[FileReader],
    parsers: &[Option<Arc<dyn LogFormatParser>>],
    year_maps: &[Option<Arc<YearMap>>],
    continuation_maps: &[Option<Arc<Vec<usize>>>],
) -> Vec<MergedEntry> {
    let mut entries = Vec::new();
    for (source_idx, source) in sources.iter().enumerate() {
        let parser = parsers[source_idx].as_deref();
        let year_map = year_maps[source_idx].as_deref();
        let cmap = continuation_maps[source_idx].as_deref();
        append_source_entries(&mut entries, source, parser, year_map, cmap, source_idx, 0);
    }
    entries.sort_unstable_by(|a, b| a.sort_key.cmp(&b.sort_key));
    entries
}

/// Append new entries from a grown source (lines `from_line..`) into an
/// existing sorted entries vec, then re-sort.
pub fn extend_merged_index(
    entries: &mut Vec<MergedEntry>,
    source: &FileReader,
    parser: Option<&dyn LogFormatParser>,
    year_map: Option<&YearMap>,
    continuation_map: Option<&Vec<usize>>,
    source_idx: usize,
    from_line: usize,
) {
    append_source_entries(
        entries,
        source,
        parser,
        year_map,
        continuation_map,
        source_idx,
        from_line,
    );
    entries.sort_unstable_by(|a, b| a.sort_key.cmp(&b.sort_key));
}

fn append_source_entries(
    entries: &mut Vec<MergedEntry>,
    source: &FileReader,
    parser: Option<&dyn LogFormatParser>,
    year_map: Option<&YearMap>,
    continuation_map: Option<&Vec<usize>>,
    source_idx: usize,
    from_line: usize,
) {
    let count = source.line_count();
    let mut current_key: Option<CanonicalTs> = None;

    for line_idx in from_line..count {
        let line = source.get_line(line_idx);
        let ts = parser.and_then(|p| p.parse_timestamp(line));

        if let Some(ts_str) = ts {
            let year_override = year_map.map(|ym| ym.year_for_line(line_idx));
            if let Some(key) = timestamp_to_canonical(ts_str, year_override) {
                current_key = Some(key);
                entries.push(MergedEntry {
                    sort_key: key,
                    source_idx,
                    line_idx,
                });
                continue;
            }
        }

        let is_continuation = continuation_map
            .map(|cmap| cmap.get(line_idx).copied().unwrap_or(line_idx) != line_idx)
            .unwrap_or(false);

        if is_continuation && let Some(key) = current_key {
            entries.push(MergedEntry {
                sort_key: key,
                source_idx,
                line_idx,
            });
        }
    }
}
