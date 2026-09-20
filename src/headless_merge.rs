use std::io::{self, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::Result;

use crate::db::LogManager;
use crate::filters::{FilterDecision, extract_date_filters, extract_field_filters};
use crate::ingestion::FileReader;
use crate::parser::LogFormatParser;
use crate::ui::YearMap;

/// One fully-loaded `--merge` input: its content plus everything
/// `build_merged_index` and filtering need to fold it into the merged,
/// timestamp-sorted output.
struct MergeSource {
    reader: FileReader,
    parser: Option<Arc<dyn LogFormatParser>>,
    year_map: Option<Arc<YearMap>>,
    continuation_map: Option<Arc<Vec<usize>>>,
    /// Ascending line indices this source's active filters keep.
    visible: Vec<usize>,
}

/// Interleaves every input file (and, for archive inputs, every file inside
/// the archive) into `writer` in timestamp order, applying `log_manager`'s
/// filters. Returns one warning string per source with no detected
/// timestamp format — those sources contribute no lines to the output, but
/// don't fail the run.
pub(crate) async fn run_headless_merge(
    files: &[String],
    out_dir: &Path,
    log_manager: &LogManager,
    writer: &mut dyn Write,
) -> Result<Vec<String>> {
    if files.len() < 2 {
        anyhow::bail!(
            "--merge requires at least 2 input files, got {}",
            files.len()
        );
    }

    let mut warnings = Vec::new();
    let mut merge_sources = Vec::new();
    for path in files {
        if crate::ingestion::detect_archive_type(path).is_some() {
            let (sources, mut errors) =
                load_archive_merge_sources(path, out_dir, log_manager).await?;
            warnings.append(&mut errors);
            merge_sources.extend(sources);
        } else {
            let (source, warning) = load_plain_merge_source(path, log_manager).await?;
            warnings.extend(warning);
            merge_sources.push(source);
        }
    }

    let parsers: Vec<_> = merge_sources.iter().map(|s| s.parser.clone()).collect();
    let year_maps: Vec<_> = merge_sources.iter().map(|s| s.year_map.clone()).collect();
    let continuation_maps: Vec<_> = merge_sources
        .iter()
        .map(|s| s.continuation_map.clone())
        .collect();
    let (sources, visible): (Vec<FileReader>, Vec<Vec<usize>>) = merge_sources
        .into_iter()
        .map(|s| (s.reader, s.visible))
        .unzip();

    let entries = crate::ui::build_merged_index(&sources, &parsers, &year_maps, &continuation_maps);

    for entry in &entries {
        if visible[entry.source_idx]
            .binary_search(&entry.line_idx)
            .is_err()
        {
            continue;
        }
        writer.write_all(&sources[entry.source_idx].get_line(entry.line_idx))?;
        writer.write_all(b"\n")?;
    }

    Ok(warnings)
}

/// Loads a plain (non-archive) merge source: its full content, detected
/// format, and the line indices the active filters keep. Returns a warning
/// when no format was detected — that source will contribute no lines.
async fn load_plain_merge_source(
    path: &str,
    log_manager: &LogManager,
) -> Result<(MergeSource, Option<String>)> {
    let reader = load_merge_source(path).await?;
    let detected = crate::ingestion::format_detect::detect_format_for_reader(&reader);
    let warning = detected.format.is_none().then(|| {
        format!(
            "'{path}' has no detected timestamp format; its lines will not appear in the merged output."
        )
    });
    let visible = collect_visible_lines(&reader, log_manager, detected.format.as_deref());
    Ok((
        MergeSource {
            reader,
            parser: detected.format,
            year_map: detected.year_map,
            continuation_map: detected.continuation_map,
            visible,
        },
        warning,
    ))
}

/// Loads a merge source's full content. Unlike the plain-concatenation
/// headless path, merge always needs every line's bytes to compute sort
/// keys, so it never installs a `VisibilityPredicate`.
async fn load_merge_source(load_path: &str) -> Result<FileReader> {
    let cancel = Arc::new(AtomicBool::new(false));
    let handle = FileReader::load(load_path.to_string(), None, false, cancel, true).await?;
    let result = handle
        .result_rx
        .await
        .map_err(|_| io::Error::other("file load cancelled"))??;
    Ok(result.reader)
}

/// Expands one archive path into a merge source per file it contains,
/// recursing through any nesting (an archive inside an archive) and
/// decompressing lone compressed entries (e.g. a `.tar` containing
/// `app.log.gz`) — the same `ArchiveTree` machinery the TUI's own archive
/// merge-picker uses (`crate::ingestion::list_archive_tree`,
/// `extract_and_detect_merge_marked`), rather than a flat single-level
/// extraction that would leave nested compression undecoded. A file that
/// fails to extract is reported as a warning and excluded, rather than
/// failing the whole merge.
async fn load_archive_merge_sources(
    path: &str,
    out_dir: &Path,
    log_manager: &LogManager,
) -> Result<(Vec<MergeSource>, Vec<String>)> {
    let path_owned = path.to_string();
    // `extract_and_detect_merge_marked` names each entry's destination
    // after the archive's own basename (e.g. `bundle.tar.gz/inner.log`) to
    // keep entries from different archives apart — but that collides with
    // the archive file itself when `out_dir` is its own directory (the
    // common case: `--out` defaults to the current directory, and that's
    // usually also where the archive being merged lives). Extracting into
    // a dedicated subdirectory keeps every entry's path away from any
    // input file, while still landing under the caller-chosen `out_dir` as
    // real, permanent files.
    let out_dir_owned = out_dir.join(".logana-merge");
    let outcome = tokio::task::spawn_blocking(move || {
        let mut tree = crate::ingestion::list_archive_tree(&path_owned)?;
        for root in tree.roots.clone() {
            tree.merge_select_subtree(root);
        }
        let (progress_tx, _progress_rx) =
            tokio::sync::watch::channel(crate::ingestion::ArchiveExtractionProgress {
                file_index: 0,
                fraction: 0.0,
            });
        Ok::<_, String>(crate::ingestion::extract_and_detect_merge_marked(
            &path_owned,
            &out_dir_owned,
            &tree,
            progress_tx,
        ))
    })
    .await
    .map_err(|e| anyhow::anyhow!("Archive extraction task failed: {e}"))?
    .map_err(|e| anyhow::anyhow!("Failed to extract '{path}': {e}"))?;

    let mut warnings: Vec<String> = outcome
        .errors
        .into_iter()
        .map(|e| format!("'{path}': {e}"))
        .collect();

    let mut sources = Vec::with_capacity(outcome.files.len());
    for file in outcome.files {
        if file.detected.format.is_none() {
            warnings.push(format!(
                "'{}' has no detected timestamp format; its lines will not appear in the merged output.",
                file.label
            ));
        }
        let visible =
            collect_visible_lines(&file.reader, log_manager, file.detected.format.as_deref());
        sources.push(MergeSource {
            reader: file.reader,
            parser: file.detected.format,
            year_map: file.detected.year_map,
            continuation_map: file.detected.continuation_map,
            visible,
        });
    }
    Ok((sources, warnings))
}

/// Returns the ascending line indices `log_manager`'s active filters keep,
/// evaluated the same way `run_headless_to_writer` does — minus that
/// function's whole-file Aho-Corasick and paged fast paths, which write
/// directly instead of collecting indices and aren't worth threading a
/// collect-mode through for merge's typically small source counts.
fn collect_visible_lines(
    reader: &FileReader,
    log_manager: &LogManager,
    parser: Option<&dyn LogFormatParser>,
) -> Vec<usize> {
    let (fm, _, _, _) = log_manager.build_filter_manager();
    let filter_defs = log_manager.get_filters();
    let date_filters = extract_date_filters(filter_defs);
    let (inc_ff, exc_ff) = extract_field_filters(filter_defs);
    let has_text_includes = fm.has_include();
    let synthetic_level = parser.is_some_and(|p| p.has_synthetic_level()) && fm.filter_count() > 0;
    let needs_parse =
        !date_filters.is_empty() || !inc_ff.is_empty() || !exc_ff.is_empty() || synthetic_level;
    let date_only =
        !date_filters.is_empty() && inc_ff.is_empty() && exc_ff.is_empty() && !synthetic_level;
    let line_count = reader.line_count();

    if fm.filter_count() == 0 && date_filters.is_empty() && inc_ff.is_empty() && exc_ff.is_empty() {
        return (0..line_count).collect();
    }

    let n_date = date_filters.len();

    use rayon::prelude::*;
    (0..line_count)
        .into_par_iter()
        .with_min_len(512)
        .fold(
            || (Vec::new(), vec![0usize; n_date]),
            |(mut vis, mut dc), idx| {
                let line_bytes = reader.get_line(idx);
                let line: &[u8] = &line_bytes;
                let mut text_dec = fm.evaluate_text(line);
                let can_skip = text_dec == FilterDecision::Exclude
                    || (text_dec == FilterDecision::Neutral
                        && has_text_includes
                        && inc_ff.is_empty()
                        && !synthetic_level);
                if date_only && !can_skip {
                    let visible = parser
                        .and_then(|p| p.parse_timestamp(line))
                        .map(|ts| date_filters.iter().any(|df| df.matches(ts, None)))
                        .unwrap_or(true);
                    if visible {
                        vis.push(idx);
                    }
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
                            &crate::ui::FieldLayout::default(),
                            &std::collections::HashSet::new(),
                            false,
                            None,
                        )
                        .join(" ");
                        let dec = fm.evaluate_text(display.as_bytes());
                        if dec != FilterDecision::Neutral {
                            text_dec = dec;
                        }
                    }
                    let mut ctx = crate::ui::FilterEvalContext::new(
                        has_text_includes,
                        &date_filters,
                        &mut dc,
                        &inc_ff,
                        &exc_ff,
                        None,
                    );
                    if crate::ui::line_is_visible(text_dec, &mut ctx, parts.as_ref(), line) {
                        vis.push(idx);
                    }
                }
                (vis, dc)
            },
        )
        .reduce(
            || (Vec::new(), vec![0usize; n_date]),
            |(mut va, mut da), (vb, db)| {
                va.extend(vb);
                for (a, b) in da.iter_mut().zip(db) {
                    *a += b;
                }
                (va, da)
            },
        )
        .0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    async fn log_manager() -> LogManager {
        let db = Arc::new(Database::in_memory().await.unwrap());
        LogManager::new(db, None).await
    }

    fn syslog_file(dir: &Path, name: &str, lines: &[&str]) -> String {
        let path = dir.join(name);
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
        path.to_string_lossy().to_string()
    }

    fn make_tar_gz(dir: &Path, name: &str, entries: &[(&str, &[u8])]) -> String {
        let path = dir.join(name);
        {
            let file = std::fs::File::create(&path).unwrap();
            let enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
            let mut builder = tar::Builder::new(enc);
            for (entry_name, content) in entries {
                let mut header = tar::Header::new_gnu();
                header.set_size(content.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                builder
                    .append_data(&mut header, entry_name, *content)
                    .unwrap();
            }
            builder.into_inner().unwrap().finish().unwrap();
        }
        path.to_string_lossy().to_string()
    }

    #[tokio::test]
    async fn merge_requires_at_least_two_files() {
        let lm = log_manager().await;
        let mut out = Vec::new();
        let err = run_headless_merge(&["only-one.log".to_string()], Path::new("."), &lm, &mut out)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("at least 2"));
    }

    #[tokio::test]
    async fn merge_interleaves_two_sources_by_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let a = syslog_file(
            dir.path(),
            "a.log",
            &[
                "Jan  1 00:00:01 host tag: a1",
                "Jan  1 00:00:03 host tag: a2",
            ],
        );
        let b = syslog_file(
            dir.path(),
            "b.log",
            &[
                "Jan  1 00:00:02 host tag: b1",
                "Jan  1 00:00:04 host tag: b2",
            ],
        );

        let lm = log_manager().await;
        let mut out = Vec::new();
        let warnings = run_headless_merge(&[a, b], dir.path(), &lm, &mut out)
            .await
            .unwrap();
        assert!(warnings.is_empty());

        let output = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(
            lines,
            vec![
                "Jan  1 00:00:01 host tag: a1",
                "Jan  1 00:00:02 host tag: b1",
                "Jan  1 00:00:03 host tag: a2",
                "Jan  1 00:00:04 host tag: b2",
            ]
        );
    }

    #[tokio::test]
    async fn merge_preserves_order_for_duplicate_timestamps_within_a_source() {
        let dir = tempfile::tempdir().unwrap();
        let a = syslog_file(
            dir.path(),
            "a.log",
            &[
                "Jan  1 00:00:01 host tag: a1",
                "Jan  1 00:00:01 host tag: a2",
                "Jan  1 00:00:01 host tag: a3",
            ],
        );
        let b = syslog_file(dir.path(), "b.log", &["Jan  1 00:00:05 host tag: b1"]);

        let lm = log_manager().await;
        let mut out = Vec::new();
        run_headless_merge(&[a, b], dir.path(), &lm, &mut out)
            .await
            .unwrap();

        let output = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(
            lines,
            vec![
                "Jan  1 00:00:01 host tag: a1",
                "Jan  1 00:00:01 host tag: a2",
                "Jan  1 00:00:01 host tag: a3",
                "Jan  1 00:00:05 host tag: b1",
            ]
        );
    }

    #[tokio::test]
    async fn merge_warns_and_skips_a_source_with_no_detected_format() {
        let dir = tempfile::tempdir().unwrap();
        let a = syslog_file(dir.path(), "a.log", &["Jan  1 00:00:01 host tag: a1"]);
        let plain = syslog_file(dir.path(), "plain.log", &["just some text", "more text"]);

        let lm = log_manager().await;
        let mut out = Vec::new();
        let warnings = run_headless_merge(&[a, plain.clone()], dir.path(), &lm, &mut out)
            .await
            .unwrap();

        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains(&plain));

        let output = String::from_utf8(out).unwrap();
        assert_eq!(output, "Jan  1 00:00:01 host tag: a1\n");
    }

    #[tokio::test]
    async fn merge_applies_include_filter_across_sources() {
        let dir = tempfile::tempdir().unwrap();
        let a = syslog_file(
            dir.path(),
            "a.log",
            &[
                "Jan  1 00:00:01 host tag: keep me",
                "Jan  1 00:00:03 host tag: drop me",
            ],
        );
        let b = syslog_file(
            dir.path(),
            "b.log",
            &["Jan  1 00:00:02 host tag: keep me too"],
        );

        let mut lm = log_manager().await;
        lm.add_filter_with_color(
            "keep".to_string(),
            crate::filters::FilterType::Include,
            crate::filters::FilterOptions::default(),
        )
        .await;

        let mut out = Vec::new();
        run_headless_merge(&[a, b], dir.path(), &lm, &mut out)
            .await
            .unwrap();

        let output = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(
            lines,
            vec![
                "Jan  1 00:00:01 host tag: keep me",
                "Jan  1 00:00:02 host tag: keep me too",
            ]
        );
    }

    #[tokio::test]
    async fn merge_expands_archive_entries_into_separate_sources() {
        let dir = tempfile::tempdir().unwrap();
        let archive = make_tar_gz(
            dir.path(),
            "bundle.tar.gz",
            &[
                ("inner_a.log", b"Jan  1 00:00:01 host tag: from archive a\n"),
                ("inner_b.log", b"Jan  1 00:00:03 host tag: from archive b\n"),
            ],
        );
        let plain = syslog_file(
            dir.path(),
            "plain.log",
            &["Jan  1 00:00:02 host tag: plain source"],
        );

        let lm = log_manager().await;
        let mut out = Vec::new();
        let warnings = run_headless_merge(&[archive, plain], dir.path(), &lm, &mut out)
            .await
            .unwrap();
        assert!(warnings.is_empty(), "warnings: {warnings:?}");

        let output = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(
            lines,
            vec![
                "Jan  1 00:00:01 host tag: from archive a",
                "Jan  1 00:00:02 host tag: plain source",
                "Jan  1 00:00:03 host tag: from archive b",
            ]
        );
    }

    #[tokio::test]
    async fn merge_decompresses_a_lone_compressed_entry_nested_inside_an_archive() {
        let dir = tempfile::tempdir().unwrap();

        let mut inner_gz = Vec::new();
        {
            let mut enc =
                flate2::write::GzEncoder::new(&mut inner_gz, flate2::Compression::default());
            enc.write_all(b"Jan  1 00:00:01 host tag: nested gz entry\n")
                .unwrap();
            enc.finish().unwrap();
        }

        let archive = make_tar_gz(
            dir.path(),
            "bundle.tar.gz",
            &[
                ("inner.log.gz", &inner_gz),
                (
                    "inner_plain.log",
                    b"Jan  1 00:00:03 host tag: plain archive entry\n",
                ),
            ],
        );
        let plain = syslog_file(
            dir.path(),
            "plain.log",
            &["Jan  1 00:00:02 host tag: plain source"],
        );

        let lm = log_manager().await;
        let mut out = Vec::new();
        let warnings = run_headless_merge(&[archive, plain], dir.path(), &lm, &mut out)
            .await
            .unwrap();
        assert!(warnings.is_empty(), "warnings: {warnings:?}");

        let output = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(
            lines,
            vec![
                "Jan  1 00:00:01 host tag: nested gz entry",
                "Jan  1 00:00:02 host tag: plain source",
                "Jan  1 00:00:03 host tag: plain archive entry",
            ]
        );
    }

    #[tokio::test]
    async fn merge_archive_extraction_does_not_collide_when_out_dir_holds_the_archive_itself() {
        // Regression test: extracting a directly-opened archive names each
        // entry's destination after the archive's own basename (e.g.
        // "bundle.tar.gz/inner.log"), which used to collide with the real
        // archive file when `out_dir` was the same directory it lives in —
        // the common case for `--out`'s cwd default.
        let dir = tempfile::tempdir().unwrap();
        let archive = make_tar_gz(
            dir.path(),
            "bundle.tar.gz",
            &[("inner.log", b"Jan  1 00:00:01 host tag: inner\n")],
        );
        let plain = syslog_file(
            dir.path(),
            "plain.log",
            &["Jan  1 00:00:02 host tag: plain"],
        );

        let lm = log_manager().await;
        let mut out = Vec::new();
        let warnings = run_headless_merge(&[archive, plain], dir.path(), &lm, &mut out)
            .await
            .unwrap();

        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        let output = String::from_utf8(out).unwrap();
        assert!(output.contains("inner"), "output: {output:?}");
    }
}
