use std::io::{self, BufWriter, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::Result;

use crate::db::Database;
use crate::db::LogManager;
use crate::filters::FilterDecision;
use crate::filters::FilterType;
use crate::filters::extract_date_filters;
use crate::filters::extract_field_filters;
use crate::ingestion::{FileReader, VisibilityPredicate};
use crate::parser::detect_format;

/// Arguments required to run logana in headless mode.
pub struct HeadlessArgs {
    pub file: Option<String>,
    pub filters: Option<String>,
    pub include_filters: Vec<String>,
    pub exclude_filters: Vec<String>,
    pub timestamp_filters: Vec<String>,
    pub output: Option<std::path::PathBuf>,
}

pub fn same_file(input: &str, output: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let Ok(mi) = std::fs::metadata(input) else {
            return false;
        };
        let Ok(mo) = std::fs::metadata(output) else {
            return false;
        };
        mi.dev() == mo.dev() && mi.ino() == mo.ino()
    }
    #[cfg(not(unix))]
    {
        let Ok(a) = std::fs::canonicalize(input) else {
            return false;
        };
        let Ok(b) = std::fs::canonicalize(output) else {
            return false;
        };
        a == b
    }
}

fn finalize_output(tmp_path: Option<(PathBuf, PathBuf)>) -> Result<()> {
    if let Some((tmp, final_path)) = tmp_path
        && let Err(e) = std::fs::rename(&tmp, &final_path)
    {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

pub async fn run_headless(args: &HeadlessArgs) -> Result<()> {
    if let (Some(input), Some(output)) = (&args.file, &args.output)
        && same_file(input.as_str(), output)
    {
        anyhow::bail!(
            "Output path '{}' is the same as the input file — writing would destroy it",
            output.display()
        );
    }

    let db = Arc::new(Database::in_memory().await?);
    let mut log_manager = LogManager::new(db, None).await;

    if let Some(ref fpath) = args.filters {
        log_manager.load_filters(fpath).await?;
    }

    apply_inline_filters(
        &mut log_manager,
        &args.include_filters,
        &args.exclude_filters,
        &args.timestamp_filters,
    )
    .await?;

    // Write to a sibling temp file, then rename it over the final path.
    //
    // Writing directly to the output file with O_TRUNC forces ext4 (data=ordered)
    // to flush all dirty pages of the *previous* output to disk before it can
    // truncate — blocking for tens of seconds on slow disks.  Writing without
    // O_TRUNC and calling set_len() afterward has the same problem: ftruncate
    // also requires a full data flush before the journal can record the new size.
    //
    // With temp+rename: the old output file is unlinked by rename(), which lets
    // the kernel discard its dirty pages without a flush; the new file's dirty
    // pages are written by the kernel in the background after the process returns.
    let (mut writer, tmp_path): (Box<dyn Write>, Option<(PathBuf, PathBuf)>) = match &args.output {
        Some(path) => {
            let mut tmp = path.as_os_str().to_owned();
            tmp.push(".tmp");
            let tmp_path = PathBuf::from(tmp);
            let file = std::fs::File::create(&tmp_path)?;
            (
                Box::new(BufWriter::with_capacity(8 * 1024 * 1024, file)),
                Some((tmp_path, path.clone())),
            )
        }
        None => (Box::new(BufWriter::new(io::stdout())), None),
    };

    let Some(ref path) = args.file else {
        let mut bytes = Vec::new();
        io::stdin().read_to_end(&mut bytes)?;
        let reader = FileReader::from_bytes(bytes);
        run_headless_to_writer(reader, &log_manager, &mut *writer)?;
        writer.flush()?;
        finalize_output(tmp_path)?;
        return Ok(());
    };

    if crate::ingestion::detect_archive_type(path).is_some() {
        run_headless_archive(path, &log_manager, &mut *writer).await?;
        writer.flush()?;
        finalize_output(tmp_path)?;
        return Ok(());
    }

    let (fm, _, _, _) = log_manager.build_filter_manager();
    let needs_parse = {
        let filter_defs = log_manager.get_filters();
        let date_filters = extract_date_filters(filter_defs);
        let (inc_ff, exc_ff) = extract_field_filters(filter_defs);
        !date_filters.is_empty() || !inc_ff.is_empty() || !exc_ff.is_empty()
    };

    let cancel = Arc::new(AtomicBool::new(false));
    let predicate = (!needs_parse).then(|| VisibilityPredicate::new(fm));
    let keep_pages = predicate.is_some();
    let handle = FileReader::load(path.clone(), predicate, false, cancel, keep_pages).await?;
    let result = handle
        .result_rx
        .await
        .map_err(|_| io::Error::other("file load cancelled"))??;

    if let Some(visible) = result.precomputed_visible {
        write_visible_lines(&mut *writer, &result.reader, &visible)?;
    } else {
        run_headless_to_writer(result.reader, &log_manager, &mut *writer)?;
    }

    writer.flush()?;
    finalize_output(tmp_path)?;
    Ok(())
}

async fn run_headless_archive(
    path: &str,
    log_manager: &LogManager,
    writer: &mut dyn Write,
) -> Result<()> {
    let path_str = path.to_string();
    let files = tokio::task::spawn_blocking(move || crate::ingestion::extract(&path_str))
        .await
        .map_err(|e| anyhow::anyhow!("Archive extraction task failed: {e}"))?
        .map_err(|e| anyhow::anyhow!("Failed to extract '{path}': {e}"))?;

    let cancel = Arc::new(AtomicBool::new(false));
    for file in files {
        let tmp_path = file.temp_file.path().to_string_lossy().to_string();
        let (fm, _, _, _) = log_manager.build_filter_manager();
        let needs_parse = {
            let filter_defs = log_manager.get_filters();
            let date_filters = extract_date_filters(filter_defs);
            let (inc_ff, exc_ff) = extract_field_filters(filter_defs);
            !date_filters.is_empty() || !inc_ff.is_empty() || !exc_ff.is_empty()
        };
        let predicate = (!needs_parse).then(|| VisibilityPredicate::new(fm));
        let keep_pages = predicate.is_some();
        let handle =
            FileReader::load(tmp_path, predicate, false, Arc::clone(&cancel), keep_pages).await?;
        let result = handle
            .result_rx
            .await
            .map_err(|_| io::Error::other("file load cancelled"))??;

        if let Some(visible) = result.precomputed_visible {
            write_visible_lines(writer, &result.reader, &visible)?;
        } else {
            run_headless_to_writer(result.reader, log_manager, writer)?;
        }
    }
    Ok(())
}

async fn apply_inline_filters(
    log_manager: &mut LogManager,
    include_filters: &[String],
    exclude_filters: &[String],
    timestamp_filters: &[String],
) -> Result<()> {
    use clap::Parser;

    use crate::auto_complete::shell_split;
    use crate::commands::{CommandLine, Commands};

    for args_str in include_filters {
        let cmd = format!("filter {}", args_str);
        let parsed = CommandLine::try_parse_from(shell_split(&cmd))
            .map_err(|e| anyhow::anyhow!("Invalid include filter '{}': {}", args_str, e))?;
        if let Some(Commands::Filter { pattern, field, .. }) = parsed.command {
            let stored = if field {
                build_field_pattern(&pattern)?
            } else {
                pattern
            };
            log_manager
                .add_filter_with_color(stored, FilterType::Include, None, None, true)
                .await;
        }
    }

    for args_str in exclude_filters {
        let cmd = format!("exclude {}", args_str);
        let parsed = CommandLine::try_parse_from(shell_split(&cmd))
            .map_err(|e| anyhow::anyhow!("Invalid exclude filter '{}': {}", args_str, e))?;
        if let Some(Commands::Exclude { pattern, field }) = parsed.command {
            let stored = if field {
                build_field_pattern(&pattern)?
            } else {
                pattern
            };
            log_manager
                .add_filter_with_color(stored, FilterType::Exclude, None, None, true)
                .await;
        }
    }

    for args_str in timestamp_filters {
        let cmd = format!("date-filter {}", args_str);
        let parsed = CommandLine::try_parse_from(shell_split(&cmd))
            .map_err(|e| anyhow::anyhow!("Invalid timestamp filter '{}': {}", args_str, e))?;
        if let Some(Commands::DateFilter { expr, .. }) = parsed.command {
            let expression = expr.join(" ");
            let stored = format!("{}{}", crate::filters::DATE_PREFIX, expression);
            log_manager
                .add_filter_with_color(stored, FilterType::Include, None, None, true)
                .await;
        }
    }

    Ok(())
}

fn build_field_pattern(pattern: &str) -> Result<String> {
    let eq = pattern
        .find('=')
        .ok_or_else(|| anyhow::anyhow!("--field pattern must be 'key=value', got: {}", pattern))?;
    Ok(format!(
        "{}{}:{}",
        crate::filters::FIELD_PREFIX,
        &pattern[..eq],
        &pattern[eq + 1..]
    ))
}

fn write_visible_lines(
    writer: &mut dyn Write,
    reader: &FileReader,
    visible: &[usize],
) -> Result<()> {
    if visible.is_empty() {
        return Ok(());
    }

    let data = reader.data();
    let line_starts = reader.line_starts();

    let mut i = 0;
    while i < visible.len() {
        let mut j = i + 1;
        while j < visible.len() && visible[j] == visible[j - 1] + 1 {
            j += 1;
        }

        let first = visible[i];
        let last = visible[j - 1];
        let byte_start = line_starts[first];
        let byte_end = if last + 1 < line_starts.len() {
            line_starts[last + 1]
        } else {
            data.len()
        };

        let chunk = &data[byte_start..byte_end];
        writer.write_all(chunk)?;
        if !chunk.ends_with(b"\n") {
            writer.write_all(b"\n")?;
        }

        i = j;
    }

    Ok(())
}

pub fn run_headless_to_writer(
    reader: FileReader,
    log_manager: &LogManager,
    writer: &mut dyn Write,
) -> Result<()> {
    let sample_limit = reader.line_count().min(200);
    let sample: Vec<&[u8]> = (0..sample_limit).map(|i| reader.get_line(i)).collect();
    let parser = detect_format(&sample);
    let parser_ref = parser.as_deref();

    let (fm, _, _, _) = log_manager.build_filter_manager();
    let filter_defs = log_manager.get_filters();
    let date_filters = extract_date_filters(filter_defs);
    let (inc_ff, exc_ff) = extract_field_filters(filter_defs);
    let has_text_includes = fm.has_include();
    let synthetic_level =
        parser_ref.is_some_and(|p| p.has_synthetic_level()) && fm.filter_count() > 0;
    let needs_parse =
        !date_filters.is_empty() || !inc_ff.is_empty() || !exc_ff.is_empty() || synthetic_level;
    let date_only =
        !date_filters.is_empty() && inc_ff.is_empty() && exc_ff.is_empty() && !synthetic_level;
    let n_date = date_filters.len();
    let line_count = reader.line_count();
    let use_wholefile = !needs_parse && fm.has_combined_ac();

    use rayon::prelude::*;

    // Pipeline: filter each chunk in parallel, then write it before moving on.
    // Bounds peak memory to one chunk's worth of visible indices instead of
    // accumulating all of them before the first byte is written.
    const CHUNK_LINES: usize = 16_384;

    let mut chunk_start = 0;
    while chunk_start < line_count {
        let chunk_end = (chunk_start + CHUNK_LINES).min(line_count);

        #[cfg(unix)]
        reader.advise_for_scan(chunk_start..chunk_end);

        let visible: Vec<usize> = if use_wholefile {
            let (vis, _) = fm.evaluate_chunk_wholefile(
                reader.data(),
                reader.line_starts(),
                chunk_start..chunk_end,
            );
            vis
        } else {
            (chunk_start..chunk_end)
                .into_par_iter()
                .with_min_len(512)
                .fold(
                    || (Vec::new(), vec![0usize; n_date]),
                    |(mut vis, mut dc), idx| {
                        let line = reader.get_line(idx);
                        let mut text_dec = fm.evaluate_text(line);
                        let can_skip = text_dec == FilterDecision::Exclude
                            || (text_dec == FilterDecision::Neutral
                                && has_text_includes
                                && inc_ff.is_empty()
                                && !synthetic_level);
                        if date_only && !can_skip {
                            let visible = parser_ref
                                .and_then(|p| p.parse_timestamp(line))
                                .map(|ts| date_filters.iter().any(|df| df.matches(ts, None)))
                                .unwrap_or(true);
                            if visible {
                                vis.push(idx);
                            }
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
                            if crate::ui::line_is_visible(
                                text_dec,
                                has_text_includes,
                                &date_filters,
                                &mut dc,
                                &inc_ff,
                                &exc_ff,
                                parts.as_ref(),
                                None,
                            ) {
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
        };

        write_visible_lines(writer, &reader, &visible)?;

        chunk_start = chunk_end;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn make_log_manager() -> LogManager {
        let db = Arc::new(Database::in_memory().await.unwrap());
        LogManager::new(db, None).await
    }

    fn make_reader(lines: &[&str]) -> FileReader {
        let data = lines.join("\n").into_bytes();
        FileReader::from_bytes(data)
    }

    fn make_gz(content: &[u8]) -> tempfile::NamedTempFile {
        use std::io::Write as _;
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        let mut enc = flate2::write::GzEncoder::new(&mut tmp, flate2::Compression::default());
        enc.write_all(content).unwrap();
        enc.finish().unwrap();
        tmp
    }

    fn make_tar_gz(entries: &[(&str, &[u8])]) -> tempfile::NamedTempFile {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        {
            let enc = flate2::write::GzEncoder::new(&mut tmp, flate2::Compression::default());
            let mut builder = tar::Builder::new(enc);
            for (name, content) in entries {
                let mut header = tar::Header::new_gnu();
                header.set_size(content.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                builder.append_data(&mut header, name, *content).unwrap();
            }
            builder.into_inner().unwrap().finish().unwrap();
        }
        tmp
    }

    #[tokio::test]
    async fn test_headless_gz_filters_output() {
        let content = b"INFO line\nERROR boom\nDEBUG trace\n";
        let gz_tmp = make_gz(content);
        let path = gz_tmp.path().to_str().unwrap().to_string() + ".gz";
        std::fs::copy(gz_tmp.path(), &path).unwrap();

        let mut lm = make_log_manager().await;
        lm.add_filter_with_color("ERROR".to_string(), FilterType::Include, None, None, true)
            .await;
        let mut out = Vec::new();
        run_headless_archive(&path, &lm, &mut out).await.unwrap();
        std::fs::remove_file(&path).unwrap();

        let result = String::from_utf8(out).unwrap();
        assert_eq!(result, "ERROR boom\n");
    }

    #[tokio::test]
    async fn test_headless_tar_gz_multiple_files() {
        let tar_tmp = make_tar_gz(&[
            ("a.log", b"INFO alpha\nERROR beta\n"),
            ("b.log", b"INFO gamma\nERROR delta\n"),
        ]);
        let path = tar_tmp.path().to_str().unwrap().to_string() + ".tar.gz";
        std::fs::copy(tar_tmp.path(), &path).unwrap();

        let mut lm = make_log_manager().await;
        lm.add_filter_with_color("ERROR".to_string(), FilterType::Include, None, None, true)
            .await;
        let mut out = Vec::new();
        run_headless_archive(&path, &lm, &mut out).await.unwrap();
        std::fs::remove_file(&path).unwrap();

        let result = String::from_utf8(out).unwrap();
        assert!(
            result.contains("ERROR beta\n"),
            "expected ERROR beta in output"
        );
        assert!(
            result.contains("ERROR delta\n"),
            "expected ERROR delta in output"
        );
        assert!(
            !result.contains("INFO"),
            "INFO lines should be filtered out"
        );
    }

    #[test]
    fn test_write_visible_lines_coalesces_consecutive() {
        let reader = make_reader(&["aaa", "bbb", "ccc", "ddd"]);
        let mut out = Vec::new();
        write_visible_lines(&mut out, &reader, &[0, 1, 3]).unwrap();
        assert_eq!(out, b"aaa\nbbb\nddd\n");
    }

    #[test]
    fn test_write_visible_lines_all_consecutive() {
        let reader = make_reader(&["x", "y", "z"]);
        let mut out = Vec::new();
        write_visible_lines(&mut out, &reader, &[0, 1, 2]).unwrap();
        assert_eq!(out, b"x\ny\nz\n");
    }

    #[test]
    fn test_write_visible_lines_empty() {
        let reader = make_reader(&["a", "b"]);
        let mut out = Vec::new();
        write_visible_lines(&mut out, &reader, &[]).unwrap();
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn test_headless_no_filters() {
        let lm = make_log_manager().await;
        let reader = make_reader(&["INFO foo", "ERROR bar", "INFO baz"]);
        let mut out = Vec::new();
        run_headless_to_writer(reader, &lm, &mut out).unwrap();
        let result = String::from_utf8(out).unwrap();
        assert_eq!(result, "INFO foo\nERROR bar\nINFO baz\n");
    }

    #[tokio::test]
    async fn test_headless_include_filter() {
        let mut lm = make_log_manager().await;
        lm.add_filter_with_color("ERROR".to_string(), FilterType::Include, None, None, true)
            .await;
        let reader = make_reader(&["INFO foo", "ERROR bar", "INFO baz"]);
        let mut out = Vec::new();
        run_headless_to_writer(reader, &lm, &mut out).unwrap();
        let result = String::from_utf8(out).unwrap();
        assert_eq!(result, "ERROR bar\n");
    }

    #[tokio::test]
    async fn test_headless_exclude_filter() {
        let mut lm = make_log_manager().await;
        lm.add_filter_with_color("DEBUG".to_string(), FilterType::Exclude, None, None, true)
            .await;
        let reader = make_reader(&["INFO foo", "DEBUG bar", "ERROR baz"]);
        let mut out = Vec::new();
        run_headless_to_writer(reader, &lm, &mut out).unwrap();
        let result = String::from_utf8(out).unwrap();
        assert_eq!(result, "INFO foo\nERROR baz\n");
    }

    #[tokio::test]
    async fn test_headless_output_to_writer() {
        let lm = make_log_manager().await;
        let reader = make_reader(&["line1", "line2"]);
        let mut out = Vec::new();
        run_headless_to_writer(reader, &lm, &mut out).unwrap();
        assert_eq!(out, b"line1\nline2\n");
    }

    #[tokio::test]
    async fn test_headless_stdin() {
        let data = b"alpha\nbeta\ngamma".to_vec();
        let reader = FileReader::from_bytes(data);
        let mut lm = make_log_manager().await;
        lm.add_filter_with_color("beta".to_string(), FilterType::Include, None, None, true)
            .await;
        let mut out = Vec::new();
        run_headless_to_writer(reader, &lm, &mut out).unwrap();
        let result = String::from_utf8(out).unwrap();
        assert_eq!(result, "beta\n");
    }

    #[tokio::test]
    async fn test_headless_no_session_restore_outputs_all_lines() {
        use std::io::Write as _;

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "INFO foo").unwrap();
        writeln!(tmp, "ERROR bar").unwrap();
        writeln!(tmp, "DEBUG baz").unwrap();
        tmp.flush().unwrap();

        let out_tmp = tempfile::NamedTempFile::new().unwrap();

        run_headless(&HeadlessArgs {
            file: Some(tmp.path().to_str().unwrap().to_string()),
            filters: None,
            include_filters: vec![],
            exclude_filters: vec![],
            timestamp_filters: vec![],
            output: Some(out_tmp.path().to_path_buf()),
        })
        .await
        .unwrap();

        let result = std::fs::read_to_string(out_tmp.path()).unwrap();
        assert_eq!(result, "INFO foo\nERROR bar\nDEBUG baz\n");
    }

    #[tokio::test]
    async fn test_headless_session_filters_in_db_are_not_applied() {
        use crate::db::FilterStore as _;
        use std::io::Write as _;

        // Simulate what a previous TUI session would have saved: an include
        // filter that would restrict output to only ERROR lines.  Headless
        // must ignore it and output every line.
        let session_db = Arc::new(Database::in_memory().await.unwrap());
        session_db
            .insert_filter(
                "ERROR",
                &FilterType::Include,
                true,
                None,
                Some("some-source"),
            )
            .await
            .unwrap();
        // session_db is intentionally never passed to run_headless; headless
        // always starts with its own fresh in-memory database.
        drop(session_db);

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "INFO foo").unwrap();
        writeln!(tmp, "ERROR bar").unwrap();
        writeln!(tmp, "INFO baz").unwrap();
        tmp.flush().unwrap();

        let out_tmp = tempfile::NamedTempFile::new().unwrap();

        run_headless(&HeadlessArgs {
            file: Some(tmp.path().to_str().unwrap().to_string()),
            filters: None,
            include_filters: vec![],
            exclude_filters: vec![],
            timestamp_filters: vec![],
            output: Some(out_tmp.path().to_path_buf()),
        })
        .await
        .unwrap();

        let result = std::fs::read_to_string(out_tmp.path()).unwrap();
        // All three lines must appear — the saved DB filter was not loaded.
        assert_eq!(result, "INFO foo\nERROR bar\nINFO baz\n");
    }

    #[tokio::test]
    async fn test_headless_same_input_output_is_rejected() {
        use std::io::Write as _;

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "INFO foo").unwrap();
        tmp.flush().unwrap();

        let path = tmp.path().to_path_buf();
        let result = run_headless(&HeadlessArgs {
            file: Some(path.to_str().unwrap().to_string()),
            filters: None,
            include_filters: vec![],
            exclude_filters: vec![],
            timestamp_filters: vec![],
            output: Some(path.clone()),
        })
        .await;

        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("same as the input file"));
        // Original file must be untouched.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "INFO foo\n");
    }

    #[tokio::test]
    async fn test_headless_syslog_include_filter_matches_synthetic_level() {
        // Syslog lines encode the level as a numeric <PRI> prefix.
        // Priority 134 = facility 16 (local0), severity 6 → "INFO".
        // Priority 11  = facility 1  (user),   severity 3 → "ERROR".
        // The raw bytes contain no "INFO" or "ERROR" text, so a plain text
        // filter must match against the normalized level produced by the parser.
        let info_line = b"<134>Mar 23 10:00:00 host myapp: all systems go";
        let error_line = b"<11>Mar 23 10:00:01 host myapp: something failed";

        let mut lm = make_log_manager().await;
        lm.add_filter_with_color("INFO".to_string(), FilterType::Include, None, None, true)
            .await;

        let data = [info_line.as_ref(), b"\n", error_line.as_ref(), b"\n"].concat();
        let reader = FileReader::from_bytes(data);
        let mut out = Vec::new();
        run_headless_to_writer(reader, &lm, &mut out).unwrap();
        let result = String::from_utf8(out).unwrap();
        assert_eq!(
            result, "<134>Mar 23 10:00:00 host myapp: all systems go\n",
            "only the INFO line should be visible"
        );
    }

    #[tokio::test]
    async fn test_headless_syslog_regex_spanning_level_and_message() {
        // Regex filter that spans the synthetic level AND message text.
        // "ERROR.*failed" must match only the ERROR+failed line, not the INFO line.
        let info_line = b"<134>Mar 23 10:00:00 host myapp: all systems go";
        let error_line = b"<11>Mar 23 10:00:01 host myapp: something failed";

        let mut lm = make_log_manager().await;
        lm.add_filter_with_color(
            r"ERROR.*failed".to_string(),
            FilterType::Include,
            None,
            None,
            true,
        )
        .await;

        let data = [info_line.as_ref(), b"\n", error_line.as_ref(), b"\n"].concat();
        let reader = FileReader::from_bytes(data);
        let mut out = Vec::new();
        run_headless_to_writer(reader, &lm, &mut out).unwrap();
        let result = String::from_utf8(out).unwrap();
        assert_eq!(
            result, "<11>Mar 23 10:00:01 host myapp: something failed\n",
            "only the ERROR+failed line should match"
        );
    }

    #[tokio::test]
    async fn test_headless_syslog_exclude_filter_matches_synthetic_level() {
        let info_line = b"<134>Mar 23 10:00:00 host myapp: all systems go";
        let error_line = b"<11>Mar 23 10:00:01 host myapp: something failed";

        let mut lm = make_log_manager().await;
        lm.add_filter_with_color("INFO".to_string(), FilterType::Exclude, None, None, true)
            .await;

        let data = [info_line.as_ref(), b"\n", error_line.as_ref(), b"\n"].concat();
        let reader = FileReader::from_bytes(data);
        let mut out = Vec::new();
        run_headless_to_writer(reader, &lm, &mut out).unwrap();
        let result = String::from_utf8(out).unwrap();
        assert_eq!(
            result, "<11>Mar 23 10:00:01 host myapp: something failed\n",
            "only the non-INFO line should be visible"
        );
    }
}
