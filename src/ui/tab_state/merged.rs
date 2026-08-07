use std::sync::Arc;

use crate::filters::{CanonicalTs, timestamp_to_canonical};
use crate::ingestion::{FileReader, MergedEntry};
use crate::parser::LogFormatParser;
use crate::ui::tab_state::year_map::YearMap;

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
    /// While a picker-triggered merge (archive/directory) is still building
    /// its index in the background, `Some((sources_done, sources_total))`.
    /// `None` once the build has finished (or for a `:merge`-built tab,
    /// which is never built in the background). Drives a progress
    /// notification so the tab doesn't look silently stuck while more
    /// sources are still being folded in.
    pub building: Option<(usize, usize)>,
}

impl MergedState {
    /// The single parser shared by every source with a detected format
    /// (sources with none are ignored), or `None` if none detected a
    /// format or two disagree. Used both to label a merged tab's format in
    /// the tab bar and to let a merged tab itself be used as a source in a
    /// further merge (see `App::merge_source_parser`).
    pub fn uniform_parser(&self) -> Option<Arc<dyn LogFormatParser>> {
        let mut detected = self.source_parsers.iter().flatten();
        let first = detected.next()?.clone();
        if detected.all(|p| p.name() == first.name()) {
            Some(first)
        } else {
            None
        }
    }
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
    // Stable: entries with equal sort_key (e.g. many lines within the same
    // second) must keep each source's original chronological order.
    entries.sort_by_key(|a| a.sort_key);
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
    // Stable, for the same reason as `build_merged_index`.
    entries.sort_by_key(|a| a.sort_key);
}

/// One incremental step of [`build_merged_index_streaming`]: the merged
/// index recomputed with one more source folded in.
pub struct MergeBuildUpdate {
    pub entries: Vec<MergedEntry>,
    pub sources_done: usize,
    pub sources_total: usize,
    /// The flattened, sorted merge content written out to one temp file —
    /// the literal "saved merged file". Only `Some` on the final update,
    /// once every source has been folded in and there's a complete result
    /// to write.
    pub merged_temp: Option<tempfile::NamedTempFile>,
}

/// Same result as [`build_merged_index`], but folds sources in one at a
/// time (reusing [`extend_merged_index`]) and reports the entries built so
/// far after each one via `update_tx`, instead of only returning once every
/// source has been read. Meant to run on a background thread — the caller
/// applies each update to a live tab as it arrives, so a merge with many or
/// large sources renders progressively instead of freezing the UI until
/// the very last source is folded in.
pub fn build_merged_index_streaming(
    sources: &[FileReader],
    parsers: &[Option<Arc<dyn LogFormatParser>>],
    year_maps: &[Option<Arc<YearMap>>],
    continuation_maps: &[Option<Arc<Vec<usize>>>],
    update_tx: &std::sync::mpsc::Sender<MergeBuildUpdate>,
) {
    let mut entries: Vec<MergedEntry> = Vec::new();
    let sources_total = sources.len();
    for (source_idx, source) in sources.iter().enumerate() {
        extend_merged_index(
            &mut entries,
            source,
            parsers[source_idx].as_deref(),
            year_maps[source_idx].as_deref(),
            continuation_maps[source_idx].as_deref(),
            source_idx,
            0,
        );
        let is_last = source_idx + 1 == sources_total;
        // Best-effort: a failure to write the snapshot must not block the
        // merge itself finishing — the tab still works, it just won't get
        // the `[TEMP]` marker or an on-disk copy of the merged result.
        let merged_temp = is_last
            .then(|| write_merged_temp_file(&entries, sources).ok())
            .flatten();
        let _ = update_tx.send(MergeBuildUpdate {
            entries: entries.clone(),
            sources_done: source_idx + 1,
            sources_total,
            merged_temp,
        });
    }
}

/// Writes every entry's line, in final sorted order, to one temp file —
/// exactly the bytes each source's line had, no source label prepended
/// (matching how the live merged view keeps labels purely visual).
fn write_merged_temp_file(
    entries: &[MergedEntry],
    sources: &[FileReader],
) -> std::io::Result<tempfile::NamedTempFile> {
    use std::io::Write;
    let temp = tempfile::NamedTempFile::new()?;
    {
        let mut writer = std::io::BufWriter::new(temp.as_file());
        for entry in entries {
            writer.write_all(sources[entry.source_idx].get_line(entry.line_idx))?;
            writer.write_all(b"\n")?;
        }
        writer.flush()?;
    }
    Ok(temp)
}

/// A batch starting at `from_line` can land in the middle of a continuation
/// group — the parent line was indexed by an earlier call (e.g. an earlier
/// live-poll batch), so this call has no `current_key` of its own to give
/// its leading continuation lines. Re-derive that parent's sort key by
/// walking back through `continuation_map` and re-parsing its timestamp, so
/// those continuation lines still get attached instead of being dropped.
fn seed_current_key(
    source: &FileReader,
    parser: Option<&dyn LogFormatParser>,
    year_map: Option<&YearMap>,
    continuation_map: Option<&Vec<usize>>,
    from_line: usize,
) -> Option<CanonicalTs> {
    let cmap = continuation_map?;
    let parent_idx = *cmap.get(from_line)?;
    if parent_idx >= from_line {
        return None;
    }
    let ts = parser?.parse_timestamp(source.get_line(parent_idx))?;
    let year_override = year_map.map(|ym| ym.year_for_line(parent_idx));
    timestamp_to_canonical(ts, year_override)
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
    let mut current_key = seed_current_key(source, parser, year_map, continuation_map, from_line);

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingestion::FileReader;
    use crate::parser::syslog::SyslogParser;

    /// A BSD-syslog source with `num_groups` distinct seconds, each repeated
    /// `group_size` times — i.e. many lines per source sharing one sort key,
    /// the common real-world case (log bursts within the same second).
    fn bsd_source(num_groups: usize, group_size: usize) -> FileReader {
        let mut data = String::new();
        for g in 0..num_groups {
            for i in 0..group_size {
                data.push_str(&format!("Jan  1 00:00:{:02} host tag: line {}\n", g, i));
            }
        }
        FileReader::from_bytes(data.into_bytes())
    }

    #[test]
    fn merge_preserves_intra_source_order_for_duplicate_timestamps() {
        let parser: Arc<dyn LogFormatParser> = Arc::new(SyslogParser::default());
        let sources = vec![bsd_source(5, 6), bsd_source(5, 6)];
        let parsers = vec![Some(parser.clone()), Some(parser)];
        let year_maps: Vec<Option<Arc<YearMap>>> = vec![None, None];
        let continuation_maps: Vec<Option<Arc<Vec<usize>>>> = vec![None, None];

        let entries = build_merged_index(&sources, &parsers, &year_maps, &continuation_maps);

        for source_idx in 0..sources.len() {
            let mut last_line: Option<usize> = None;
            for entry in entries.iter().filter(|e| e.source_idx == source_idx) {
                if let Some(prev) = last_line {
                    assert!(
                        entry.line_idx > prev,
                        "source {source_idx} line order violated: line {} came after line {prev}",
                        entry.line_idx
                    );
                }
                last_line = Some(entry.line_idx);
            }
        }
    }

    /// Simulates a live-tailed source whose continuation lines are read by
    /// a *later* poll than their parent (very plausible: the parent header
    /// and its multi-line body can land in different reads). `extend_merged_index`
    /// must still attach them to their parent's sort key instead of dropping
    /// them for lack of a `current_key` carried over from the earlier batch.
    #[test]
    fn merge_extend_recovers_continuation_lines_split_across_batches() {
        let parser: Arc<dyn LogFormatParser> = Arc::new(SyslogParser::default());

        let partial_source =
            FileReader::from_bytes(b"Jan  1 00:00:01 host tag: header one\n".to_vec());
        let sources = vec![partial_source];
        let parsers = vec![Some(parser.clone())];
        let year_maps: Vec<Option<Arc<YearMap>>> = vec![None];
        let continuation_maps: Vec<Option<Arc<Vec<usize>>>> = vec![None];
        let mut entries = build_merged_index(&sources, &parsers, &year_maps, &continuation_maps);
        assert_eq!(entries.len(), 1);

        // The continuation lines have now arrived; the source (and its
        // continuation map) reflect the full, grown content.
        let full_source = FileReader::from_bytes(
            b"Jan  1 00:00:01 host tag: header one\n  continuation 1a\n  continuation 1b\n"
                .to_vec(),
        );
        let cmap = crate::ui::tab_state::build_continuation_map(&full_source, parser.as_ref());
        assert_eq!(cmap, vec![0, 0, 0]);

        extend_merged_index(
            &mut entries,
            &full_source,
            Some(parser.as_ref()),
            None,
            Some(&cmap),
            0,
            1,
        );

        assert_eq!(
            entries.len(),
            3,
            "continuation lines split across batches must not be dropped"
        );
    }
}
