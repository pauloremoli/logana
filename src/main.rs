//! Entry point for logana — a terminal log analysis tool.
//!
//! Parses CLI arguments, initialises the tokio runtime and SQLite database,
//! loads configuration, builds the [`FileReader`] and [`LogManager`], then
//! hands control to [`App::run`] for the interactive TUI event loop.

use anyhow::Result;
use clap::Parser;
use crossterm::{
    ExecutableCommand,
    event::{KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags},
    terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
        supports_keyboard_enhancement,
    },
};
use logana::config::Config;
use logana::db::Database;
use logana::file_reader::{FileReader, VisibilityPredicate};
use logana::log_manager::LogManager;
use logana::mode::app_mode::ConfirmOpenDirMode;
use logana::theme::Theme;
use logana::ui::{App, LoadContext, list_dir_files};
use ratatui::prelude::*;
use std::io::{IsTerminal, stdin, stdout};
use std::sync::Arc;
use tracing::error;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Optional file to process. If not provided, reads from stdin.
    file: Option<String>,

    /// Path to a JSON filter file to preload (e.g. saved with :save-filters).
    /// Filters are applied in a single pass during file indexing.
    #[arg(short = 'f', long)]
    filters: Option<String>,

    /// Start at the end of the file and enable tail mode.
    /// When combined with --filters, the predicate is evaluated from the last
    /// line backward so the tail is available immediately after loading.
    #[arg(short = 't', long)]
    tail: bool,
}

fn get_db_path() -> String {
    if let Some(data_dir) = dirs::data_dir() {
        let app_dir = data_dir.join("logana");
        app_dir.join("logana.db").to_string_lossy().to_string()
    } else {
        "logana.db".to_string()
    }
}

/// Validate that `path` exists (file or directory).
/// Returns `Ok(())` on success, or an error message describing the problem.
fn validate_file_arg(path: &str) -> std::result::Result<(), String> {
    let p = std::path::Path::new(path);
    if !p.exists() {
        return Err(format!("'{}' not found.", path));
    }
    Ok(())
}

/// Determine whether to start a background file load and what source path
/// to associate with the `LogManager`.
/// For directories, returns `(None, false)` — the dir mode is handled via
/// `ConfirmOpenDirMode` after the TUI is started.
fn resolve_source(file_path: &Option<String>) -> (Option<String>, bool) {
    if let Some(path) = file_path {
        let p = std::path::Path::new(path);
        if p.is_dir() {
            (None, false)
        } else {
            let abs = std::fs::canonicalize(p)
                .ok()
                .and_then(|c| c.to_str().map(|s| s.to_string()))
                .unwrap_or_else(|| path.clone());
            (Some(abs), true)
        }
    } else {
        (None, false)
    }
}

/// In debug builds, write logs to a fixed file in the system temp directory
/// (`$TMPDIR/logana.log`).  The returned guard must be kept alive for the
/// duration of the process.
#[cfg(debug_assertions)]
fn init_logging() -> tracing_appender::non_blocking::WorkerGuard {
    let file_appender = tracing_appender::rolling::never(std::env::temp_dir(), "logana.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::fmt()
        .with_writer(non_blocking)
        .json()
        .init();
    guard
}

/// In release builds logging is disabled entirely.
#[cfg(not(debug_assertions))]
fn init_logging() {}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let file_path = args.file;

    let _log_guard = init_logging();

    let db_path = get_db_path();
    let db = Database::new(&db_path).await.inspect_err(|err| {
        error!("Failed to init database: {}", err);
    })?;
    let db = Arc::new(db);

    // Validate the file path before entering the TUI (gives a clean error message).
    if let Some(ref path) = file_path
        && let Err(msg) = validate_file_arg(path)
    {
        eprintln!("Error: {}", msg);
        std::process::exit(1);
    }

    // Validate the filter file path before entering the TUI.
    if let Some(ref fpath) = args.filters
        && let Err(msg) = validate_file_arg(fpath)
    {
        eprintln!("Error (--filters): {}", msg);
        std::process::exit(1);
    }

    // For a directory argument, pre-check that it contains files so we can
    // give a clean error before entering the TUI.
    if let Some(ref path) = file_path
        && std::path::Path::new(path).is_dir()
        && logana::ui::list_dir_files(path).is_empty()
    {
        eprintln!("Error: '{}' contains no files.", path);
        std::process::exit(1);
    }

    // Detect piped stdin before entering the TUI.
    let stdin_is_piped = file_path.is_none() && !stdin().is_terminal();

    let (source_path, background_file_load) = resolve_source(&file_path);

    let log_manager = LogManager::new(db.clone(), source_path.clone()).await;
    let config = Config::load();
    let theme = config
        .theme
        .as_deref()
        .and_then(|name| Theme::from_file(format!("{}.json", name)).ok())
        .unwrap_or_default();
    let show_mode_bar = config.show_mode_bar;
    let show_borders = config.show_borders;

    for conflict in config.keybindings.validate() {
        tracing::warn!("{}", conflict);
        eprintln!("Warning: {}", conflict);
    }

    let keybindings = Arc::new(config.keybindings);

    let res = {
        enable_raw_mode()?;
        stdout().execute(EnterAlternateScreen)?;
        let keyboard_enhanced = supports_keyboard_enhancement().unwrap_or(false);
        if keyboard_enhanced {
            stdout().execute(PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES,
            ))?;
        }
        let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
        terminal.clear()?;

        let mut app = App::new(
            log_manager,
            FileReader::from_bytes(vec![]),
            theme,
            keybindings,
        )
        .await;

        // Apply display defaults from config.
        app.show_mode_bar_default = show_mode_bar;
        app.show_borders_default = show_borders;
        app.tabs[0].show_mode_bar = show_mode_bar;
        app.tabs[0].show_borders = show_borders;

        // If a filter file was provided, load it into the initial tab's log manager
        // so filters are active both for the single-pass optimisation and for
        // interactive use (add/remove/edit in filter management mode).
        if let Some(ref fpath) = args.filters
            && let Err(e) = app.tabs[0].log_manager.load_filters(fpath).await
        {
            eprintln!("Warning: could not load filters from '{}': {}", fpath, e);
        }

        // Mark the initial tab for tail mode so on_load_success can apply it.
        app.startup_tail = args.tail;
        // Suppress the previous-session restore prompt when --filters was provided.
        app.startup_filters = args.filters.is_some();

        // Build a visibility predicate for the single-pass optimisation when both
        // a filter file and a background file load are in play.
        let startup_predicate: Option<VisibilityPredicate> =
            if background_file_load && args.filters.is_some() {
                let (fm, _, _) = app.tabs[0].log_manager.build_filter_manager();
                Some(Box::new(move |line: &[u8]| fm.is_visible(line)))
            } else {
                None
            };

        // Kick off the background file load now that the TUI is visible.
        if background_file_load {
            if let Some(path) = source_path {
                app.begin_file_load(
                    path,
                    LoadContext::ReplaceInitialTab,
                    startup_predicate,
                    args.tail,
                )
                .await;
            }
        } else if stdin_is_piped {
            app.begin_stdin_load().await;
        }

        // Directory argument: show confirmation popup to open each file in its own tab.
        if let Some(ref path) = file_path
            && std::path::Path::new(path).is_dir()
        {
            let files = list_dir_files(path);
            app.tabs[0].mode = Box::new(ConfirmOpenDirMode {
                dir: path.clone(),
                files,
            });
        }

        let app_result = app.run(&mut terminal).await;

        if keyboard_enhanced {
            stdout().execute(PopKeyboardEnhancementFlags)?;
        }
        disable_raw_mode()?;
        stdout().execute(LeaveAlternateScreen)?;
        app_result
    };

    if let Err(err) = res {
        eprintln!("Application error: {:?}", err);
        return Err(err);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Args (clap) ───────────────────────────────────────────────────

    #[test]
    fn test_args_no_file() {
        let args = Args::try_parse_from(["logana"]).unwrap();
        assert!(args.file.is_none());
        assert!(args.filters.is_none());
        assert!(!args.tail);
    }

    #[test]
    fn test_args_with_file() {
        let args = Args::try_parse_from(["logana", "/var/log/syslog"]).unwrap();
        assert_eq!(args.file, Some("/var/log/syslog".to_string()));
    }

    #[test]
    fn test_args_filters_short() {
        let args = Args::try_parse_from(["logana", "file.log", "-f", "my.json"]).unwrap();
        assert_eq!(args.filters, Some("my.json".to_string()));
    }

    #[test]
    fn test_args_filters_long() {
        let args = Args::try_parse_from(["logana", "file.log", "--filters", "my.json"]).unwrap();
        assert_eq!(args.filters, Some("my.json".to_string()));
    }

    #[test]
    fn test_args_tail_short() {
        let args = Args::try_parse_from(["logana", "file.log", "-t"]).unwrap();
        assert!(args.tail);
    }

    #[test]
    fn test_args_tail_long() {
        let args = Args::try_parse_from(["logana", "file.log", "--tail"]).unwrap();
        assert!(args.tail);
    }

    #[test]
    fn test_args_tail_default_false() {
        let args = Args::try_parse_from(["logana", "file.log"]).unwrap();
        assert!(!args.tail);
    }

    #[test]
    fn test_args_filters_and_tail_combined() {
        let args =
            Args::try_parse_from(["logana", "file.log", "-f", "filters.json", "-t"]).unwrap();
        assert_eq!(args.filters, Some("filters.json".to_string()));
        assert!(args.tail);
    }

    #[test]
    fn test_args_rejects_unknown_flags() {
        let result = Args::try_parse_from(["logana", "--unknown"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_args_rejects_multiple_positional() {
        let result = Args::try_parse_from(["logana", "file1.log", "file2.log"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_args_version_flag() {
        let result = Args::try_parse_from(["logana", "--version"]);
        // --version causes clap to print and exit with an error variant.
        assert!(result.is_err());
    }

    #[test]
    fn test_args_help_flag() {
        let result = Args::try_parse_from(["logana", "--help"]);
        assert!(result.is_err());
    }

    // ── get_db_path ───────────────────────────────────────────────────

    #[test]
    fn test_get_db_path_contains_logana() {
        let path = get_db_path();
        assert!(
            path.contains("logana"),
            "DB path should contain 'logana': {}",
            path
        );
        assert!(
            path.ends_with("logana.db"),
            "DB path should end with 'logana.db': {}",
            path
        );
    }

    #[test]
    fn test_get_db_path_uses_data_dir_when_available() {
        let path = get_db_path();
        // On most systems dirs::data_dir() returns Some, so the path
        // should include the app subdirectory.
        if dirs::data_dir().is_some() {
            assert!(
                path.contains("logana"),
                "DB path should include app directory: {}",
                path
            );
        } else {
            assert_eq!(path, "logana.db");
        }
    }

    // ── validate_file_arg ─────────────────────────────────────────────

    #[test]
    fn test_validate_file_arg_nonexistent() {
        let result = validate_file_arg("/nonexistent/path/file.log");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_validate_file_arg_directory_is_ok() {
        // Directories are now accepted (handled via ConfirmOpenDirMode).
        let result = validate_file_arg("/tmp");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_file_arg_valid_file() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap();
        assert!(validate_file_arg(path).is_ok());
    }

    #[test]
    fn test_validate_file_arg_empty_string() {
        let result = validate_file_arg("");
        // Empty path doesn't exist.
        assert!(result.is_err());
    }

    // ── resolve_source ────────────────────────────────────────────────

    #[test]
    fn test_resolve_source_with_file() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        let file_path = Some(path.clone());
        let (source, bg_load) = resolve_source(&file_path);
        assert_eq!(source, Some(path));
        assert!(bg_load);
    }

    #[test]
    fn test_resolve_source_without_file() {
        let file_path: Option<String> = None;
        let (source, bg_load) = resolve_source(&file_path);
        assert!(source.is_none());
        assert!(!bg_load);
    }

    #[test]
    fn test_resolve_source_with_dir_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_str().unwrap().to_string();
        let file_path = Some(dir);
        let (source, bg_load) = resolve_source(&file_path);
        assert!(source.is_none());
        assert!(!bg_load);
    }
}
