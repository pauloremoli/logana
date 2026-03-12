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
    #[arg(long)]
    tail: bool,

    /// Add an include filter. Accepts the same arguments as the :filter command.
    /// May be repeated. Examples: -i "error"  or  -i "--bg Red --field level=ERROR"
    #[arg(
        short = 'i',
        long = "include",
        value_name = "ARGS",
        allow_hyphen_values = true
    )]
    include_filters: Vec<String>,

    /// Add an exclude filter. Accepts the same arguments as the :exclude command.
    /// May be repeated. Examples: -o "debug"  or  -o "--field level=debug"
    #[arg(
        short = 'o',
        long = "exclude",
        value_name = "ARGS",
        allow_hyphen_values = true
    )]
    exclude_filters: Vec<String>,

    /// Add a date/time range filter. Accepts the same arguments as :date-filter.
    /// May be repeated. Examples: -t "> 2024-02-21"  or  -t "01:00 .. 02:00"
    #[arg(
        short = 't',
        long = "timestamp",
        value_name = "ARGS",
        allow_hyphen_values = true
    )]
    timestamp_filters: Vec<String>,

    /// Run without TUI, write matching lines to stdout or --output.
    #[arg(long)]
    headless: bool,

    /// Write output to PATH instead of stdout (requires --headless).
    #[arg(long, value_name = "PATH", requires = "headless")]
    output: Option<std::path::PathBuf>,
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

/// Validate an inline filter argument string by pre-parsing it with the same
/// clap parser used by the TUI command mode. Returns `Err` with a message if
/// the string is not a valid command argument list.
fn validate_inline_filter(prefix: &str, args_str: &str) -> std::result::Result<(), String> {
    use clap::Parser as _;
    use logana::auto_complete::shell_split;
    use logana::mode::command_mode::CommandLine;

    let cmd = format!("{} {}", prefix, args_str);
    CommandLine::try_parse_from(shell_split(&cmd))
        .map(|_| ())
        .map_err(|e| e.to_string())
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
    let db = match Database::new(&db_path).await {
        Ok(db) => db,
        Err(err) => {
            error!("Failed to open database at {}: {}", db_path, err);
            eprintln!(
                "Warning: could not open database at '{}': {}. Running without persistence.",
                db_path, err
            );
            Database::in_memory().await.inspect_err(|e| {
                error!("Failed to create in-memory database: {}", e);
            })?
        }
    };
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

    // Validate inline filter argument strings before entering the TUI.
    for args_str in &args.include_filters {
        if let Err(msg) = validate_inline_filter("filter", args_str) {
            eprintln!("Error (-i/--include): {}", msg);
            std::process::exit(1);
        }
    }
    for args_str in &args.exclude_filters {
        if let Err(msg) = validate_inline_filter("exclude", args_str) {
            eprintln!("Error (-o/--exclude): {}", msg);
            std::process::exit(1);
        }
    }
    for args_str in &args.timestamp_filters {
        if let Err(msg) = validate_inline_filter("date-filter", args_str) {
            eprintln!("Error (-t/--timestamp): {}", msg);
            std::process::exit(1);
        }
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
    let show_line_numbers = config.show_line_numbers;
    let show_sidebar = config.show_sidebar;
    let wrap = config.wrap;
    let preview_bytes = config.preview_bytes;
    let restore_policy = config.restore_session;
    let restore_file_policy = config.restore_file_context;

    for conflict in config.keybindings.validate() {
        tracing::warn!("{}", conflict);
        eprintln!("Warning: {}", conflict);
    }

    let keybindings = Arc::new(config.keybindings);

    if args.headless {
        logana::headless::run_headless(&logana::headless::HeadlessArgs {
            file: file_path,
            filters: args.filters,
            include_filters: args.include_filters,
            exclude_filters: args.exclude_filters,
            timestamp_filters: args.timestamp_filters,
            output: args.output,
        })
        .await?;
        return Ok(());
    }

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
            restore_policy,
            restore_file_policy,
            show_mode_bar,
            show_borders,
            show_line_numbers,
            show_sidebar,
            wrap,
        )
        .await;

        app.preview_bytes = preview_bytes;

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

        // Apply inline CLI filters (already validated before entering the TUI).
        let has_inline_filters = !args.include_filters.is_empty()
            || !args.exclude_filters.is_empty()
            || !args.timestamp_filters.is_empty();
        for args_str in &args.include_filters {
            app.execute_command_str(format!("filter {}", args_str))
                .await;
        }
        for args_str in &args.exclude_filters {
            app.execute_command_str(format!("exclude {}", args_str))
                .await;
        }
        for args_str in &args.timestamp_filters {
            app.execute_command_str(format!("date-filter {}", args_str))
                .await;
        }

        // Suppress the previous-session restore prompt when any filters were provided.
        app.startup_filters = args.filters.is_some() || has_inline_filters;

        // Build a visibility predicate for the single-pass optimisation when both
        // filters and a background file load are in play.
        let startup_predicate: Option<VisibilityPredicate> =
            if background_file_load && (args.filters.is_some() || has_inline_filters) {
                let (fm, _, _, _) = app.tabs[0].log_manager.build_filter_manager();
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
            Args::try_parse_from(["logana", "file.log", "-f", "filters.json", "--tail"]).unwrap();
        assert_eq!(args.filters, Some("filters.json".to_string()));
        assert!(args.tail);
    }

    #[test]
    fn test_args_include_short() {
        let args = Args::try_parse_from(["logana", "file.log", "-i", "error"]).unwrap();
        assert_eq!(args.include_filters, vec!["error"]);
    }

    #[test]
    fn test_args_include_long() {
        let args = Args::try_parse_from(["logana", "--include", "error"]).unwrap();
        assert_eq!(args.include_filters, vec!["error"]);
    }

    #[test]
    fn test_args_include_repeated() {
        let args =
            Args::try_parse_from(["logana", "-i", "error", "-i", "--field level=ERROR"]).unwrap();
        assert_eq!(args.include_filters, vec!["error", "--field level=ERROR"]);
    }

    #[test]
    fn test_args_exclude_short() {
        let args = Args::try_parse_from(["logana", "file.log", "-o", "debug"]).unwrap();
        assert_eq!(args.exclude_filters, vec!["debug"]);
    }

    #[test]
    fn test_args_exclude_long() {
        let args = Args::try_parse_from(["logana", "--exclude", "debug"]).unwrap();
        assert_eq!(args.exclude_filters, vec!["debug"]);
    }

    #[test]
    fn test_args_timestamp_short() {
        let args = Args::try_parse_from(["logana", "-t", "> 2024-02-21"]).unwrap();
        assert_eq!(args.timestamp_filters, vec!["> 2024-02-21"]);
    }

    #[test]
    fn test_args_timestamp_long() {
        let args = Args::try_parse_from(["logana", "--timestamp", "01:00 .. 02:00"]).unwrap();
        assert_eq!(args.timestamp_filters, vec!["01:00 .. 02:00"]);
    }

    #[test]
    fn test_args_timestamp_repeated() {
        let args = Args::try_parse_from(["logana", "-t", "> 10:00", "-t", "< 11:00"]).unwrap();
        assert_eq!(args.timestamp_filters, vec!["> 10:00", "< 11:00"]);
    }

    #[test]
    fn test_args_inline_filters_default_empty() {
        let args = Args::try_parse_from(["logana", "file.log"]).unwrap();
        assert!(args.include_filters.is_empty());
        assert!(args.exclude_filters.is_empty());
        assert!(args.timestamp_filters.is_empty());
    }

    #[test]
    fn test_args_inline_filters_combined() {
        let args = Args::try_parse_from([
            "logana",
            "file.log",
            "-i",
            "--bg Red error",
            "-o",
            "debug",
            "-t",
            "> 10:00",
        ])
        .unwrap();
        assert_eq!(args.include_filters, vec!["--bg Red error"]);
        assert_eq!(args.exclude_filters, vec!["debug"]);
        assert_eq!(args.timestamp_filters, vec!["> 10:00"]);
    }

    #[test]
    fn test_args_include_with_flags() {
        let args = Args::try_parse_from(["logana", "-i", "--field level=ERROR"]).unwrap();
        assert_eq!(args.include_filters, vec!["--field level=ERROR"]);
    }

    // ── validate_inline_filter ────────────────────────────────────────

    #[test]
    fn test_validate_inline_filter_valid_pattern() {
        assert!(validate_inline_filter("filter", "error").is_ok());
    }

    #[test]
    fn test_validate_inline_filter_with_field_flag() {
        assert!(validate_inline_filter("filter", "--field level=ERROR").is_ok());
    }

    #[test]
    fn test_validate_inline_filter_with_color_flags() {
        assert!(validate_inline_filter("filter", "--bg Red --fg White error").is_ok());
    }

    #[test]
    fn test_validate_inline_filter_exclude_valid() {
        assert!(validate_inline_filter("exclude", "debug").is_ok());
    }

    #[test]
    fn test_validate_inline_filter_date_filter_valid() {
        assert!(validate_inline_filter("date-filter", "> 2024-02-21").is_ok());
    }

    #[test]
    fn test_validate_inline_filter_unknown_flag_rejected() {
        assert!(validate_inline_filter("filter", "--unknown-flag value").is_err());
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
