//! Headless (non-TUI) execution mode.
//!
//! [`run_headless`] reads a file (or stdin), applies the same filter pipeline
//! as the interactive TUI, and writes matching lines to stdout or a file.
//! [`run_headless_to_writer`] is the testable inner function that accepts an
//! arbitrary [`std::io::Write`] sink.

use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

use anyhow::Result;

use crate::date_filter::extract_date_filters;
use crate::db::Database;
use crate::field_filter::extract_field_filters;
use crate::file_reader::FileReader;
use crate::log_manager::LogManager;
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

pub(crate) fn run_headless_to_writer(
    reader: FileReader,
    log_manager: LogManager,
    writer: &mut dyn Write,
) -> Result<()> {
    let (fm, _, _, _) = log_manager.build_filter_manager();
    let filter_defs = log_manager.get_filters();
    let date_filters = extract_date_filters(filter_defs);
    let (inc_ff, exc_ff) = extract_field_filters(filter_defs);
    let df_counts: Vec<AtomicUsize> = (0..date_filters.len())
        .map(|_| AtomicUsize::new(0))
        .collect();

    for idx in 0..reader.line_count() {
        let line = reader.get_line(idx);
        if crate::ui::line_is_visible(&fm, line, &date_filters, &df_counts, &inc_ff, &exc_ff, None)
        {
            writer.write_all(line)?;
            writer.write_all(b"\n")?;
        }
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
}
