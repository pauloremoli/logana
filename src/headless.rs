//! Headless (non-TUI) execution mode.
//!
//! [`run_headless`] reads a file (or stdin), applies the same filter pipeline
//! as the interactive TUI, and writes matching lines to stdout or a file.
//! [`run_headless_to_writer`] is the testable inner function that accepts an
//! arbitrary [`std::io::Write`] sink.

use std::io::{self, Read, Write};
use std::sync::Arc;

use anyhow::Result;

use crate::date_filter::extract_date_filters;
use crate::db::Database;
use crate::field_filter::extract_field_filters;
use crate::file_reader::FileReader;
use crate::filters::FilterDecision;
use crate::log_manager::LogManager;
use crate::parser::detect_format;
use crate::types::FilterType;

/// Arguments required to run logana in headless mode.
pub struct HeadlessArgs {
    pub file: Option<String>,
    pub filters: Option<String>,
    pub include_filters: Vec<String>,
    pub exclude_filters: Vec<String>,
    pub timestamp_filters: Vec<String>,
    pub output: Option<std::path::PathBuf>,
}

/// Run logana without a TUI: apply filters and write matching lines.
///
/// Always uses an in-memory database so no saved session state (filters,
/// marks, scroll position) from previous TUI runs is ever applied.
/// Output is determined solely by the parameters in `args`.
pub async fn run_headless(args: &HeadlessArgs) -> Result<()> {
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

    let reader = load_reader(&args.file)?;

    let mut writer: Box<dyn Write> = match &args.output {
        Some(path) => Box::new(std::fs::File::create(path)?),
        None => Box::new(io::stdout()),
    };

    run_headless_to_writer(reader, log_manager, &mut *writer)
}

async fn apply_inline_filters(
    log_manager: &mut LogManager,
    include_filters: &[String],
    exclude_filters: &[String],
    timestamp_filters: &[String],
) -> Result<()> {
    use clap::Parser;

    use crate::auto_complete::shell_split;
    use crate::mode::command_mode::{CommandLine, Commands};

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
            let stored = format!("{}{}", crate::date_filter::DATE_PREFIX, expression);
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
        crate::field_filter::FIELD_PREFIX,
        &pattern[..eq],
        &pattern[eq + 1..]
    ))
}

fn load_reader(file: &Option<String>) -> io::Result<FileReader> {
    match file {
        Some(path) => FileReader::new(path),
        None => {
            let mut bytes = Vec::new();
            io::stdin().read_to_end(&mut bytes)?;
            Ok(FileReader::from_bytes(bytes))
        }
    }
}

pub fn run_headless_to_writer(
    reader: FileReader,
    log_manager: LogManager,
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
    let needs_parse = !date_filters.is_empty() || !inc_ff.is_empty() || !exc_ff.is_empty();
    let has_text_includes = fm.has_include();
    let n_date = date_filters.len();
    let line_count = reader.line_count();

    // Parallel filter pass: determine visible indices using all available cores.
    let visible: Vec<usize> = {
        use rayon::prelude::*;
        (0..line_count)
            .into_par_iter()
            .with_min_len(1024)
            .fold(
                || (Vec::new(), vec![0usize; n_date]),
                |(mut vis, mut dc), idx| {
                    let line = reader.get_line(idx);
                    let text_dec = fm.evaluate_text(line);
                    let can_skip = text_dec == FilterDecision::Exclude
                        || (text_dec == FilterDecision::Neutral
                            && has_text_includes
                            && inc_ff.is_empty());
                    let parts = if needs_parse && !can_skip {
                        parser_ref.and_then(|p| p.parse_line(line))
                    } else {
                        None
                    };
                    if crate::ui::line_is_visible(
                        text_dec,
                        has_text_includes,
                        &date_filters,
                        &mut dc,
                        &inc_ff,
                        &exc_ff,
                        parts.as_ref(),
                    ) {
                        vis.push(idx);
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

    // Sequential write pass: output matching lines in original order.
    for idx in visible {
        let line = reader.get_line(idx);
        writer.write_all(line)?;
        writer.write_all(b"\n")?;
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

    #[tokio::test]
    async fn test_headless_no_filters() {
        let lm = make_log_manager().await;
        let reader = make_reader(&["INFO foo", "ERROR bar", "INFO baz"]);
        let mut out = Vec::new();
        run_headless_to_writer(reader, lm, &mut out).unwrap();
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
        run_headless_to_writer(reader, lm, &mut out).unwrap();
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
        run_headless_to_writer(reader, lm, &mut out).unwrap();
        let result = String::from_utf8(out).unwrap();
        assert_eq!(result, "INFO foo\nERROR baz\n");
    }

    #[tokio::test]
    async fn test_headless_output_to_writer() {
        let lm = make_log_manager().await;
        let reader = make_reader(&["line1", "line2"]);
        let mut out = Vec::new();
        run_headless_to_writer(reader, lm, &mut out).unwrap();
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
        run_headless_to_writer(reader, lm, &mut out).unwrap();
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
}
