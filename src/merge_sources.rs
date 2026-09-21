use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::Result;

use crate::ingestion::{FileReader, MergeMarkedSource};

/// Expands `--merge`'s CLI file arguments into one [`MergeMarkedSource`]
/// per mergeable file, in argument order — a plain file maps to itself; an
/// archive is expanded (recursing through any nesting, decompressing lone
/// compressed entries) via the same `ArchiveTree` machinery the TUI's
/// archive-picker merge uses. Shared by headless `--merge` (which further
/// filters each source before merging) and the TUI's CLI-triggered merge
/// tab (which uses every source as-is).
///
/// Requires at least 2 input files. A source with no detected timestamp
/// format is still returned — it simply contributes no lines once merged —
/// but adds a warning naming it, rather than silently dropping it.
pub(crate) async fn resolve_merge_sources(
    files: &[String],
    out_dir: &Path,
) -> Result<(Vec<MergeMarkedSource>, Vec<String>)> {
    if files.len() < 2 {
        anyhow::bail!(
            "--merge requires at least 2 input files, got {}",
            files.len()
        );
    }

    let mut warnings = Vec::new();
    let mut sources = Vec::new();
    for path in files {
        if crate::ingestion::detect_archive_type(path).is_some() {
            let (mut archive_sources, mut errors) = resolve_archive_sources(path, out_dir).await?;
            warnings.append(&mut errors);
            sources.append(&mut archive_sources);
        } else {
            sources.push(resolve_plain_source(path).await?);
        }
    }

    for source in &sources {
        if source.detected.format.is_none() {
            warnings.push(format!(
                "'{}' has no detected timestamp format; its lines will not appear in the merged output.",
                source.label
            ));
        }
    }

    Ok((sources, warnings))
}

/// Loads a plain (non-archive) merge source: its full content and detected
/// format. Never installs a `VisibilityPredicate` — a merge needs every
/// line's bytes to compute sort keys, unlike the concat headless path's
/// optional predicate-driven shortcut.
async fn resolve_plain_source(path: &str) -> Result<MergeMarkedSource> {
    let cancel = Arc::new(AtomicBool::new(false));
    let handle = FileReader::load(path.to_string(), None, false, cancel, true).await?;
    let result = handle
        .result_rx
        .await
        .map_err(|_| std::io::Error::other("file load cancelled"))??;
    let reader = result.reader;
    let detected = crate::ingestion::format_detect::detect_format_for_reader(&reader);
    Ok(MergeMarkedSource {
        label: path.to_string(),
        reader,
        detected,
        path: std::path::PathBuf::from(path),
    })
}

/// Expands one archive path into a merge source per file it contains,
/// recursing through any nesting (an archive inside an archive) and
/// decompressing lone compressed entries (e.g. a `.tar` containing
/// `app.log.gz`) — the same `ArchiveTree` machinery the TUI's own archive
/// merge-picker uses (`list_archive_tree`, `extract_and_detect_merge_marked`),
/// rather than a flat single-level extraction that would leave nested
/// compression undecoded. A file that fails to extract is reported as a
/// warning and excluded, rather than failing the whole merge.
///
/// Extracts into a dedicated `.logana-merge` subdirectory of `out_dir`
/// rather than `out_dir` directly: `extract_and_detect_merge_marked` names
/// each entry's destination after the archive's own basename (e.g.
/// `bundle.tar.gz/inner.log`), which would collide with the archive file
/// itself when `out_dir` is its own directory — the common case, since
/// `--out` defaults to the current directory.
async fn resolve_archive_sources(
    path: &str,
    out_dir: &Path,
) -> Result<(Vec<MergeMarkedSource>, Vec<String>)> {
    let path_owned = path.to_string();
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

    let errors: Vec<String> = outcome
        .errors
        .into_iter()
        .map(|e| format!("'{path}': {e}"))
        .collect();
    Ok((outcome.files, errors))
}
