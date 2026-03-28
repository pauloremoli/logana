use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use logana::config::Config;
use logana::db::Database;
use logana::db::LogManager;
use logana::ingestion::{FileReader, VisibilityPredicate};
use logana::mode::app_mode::ConfirmOpenDirMode;
use logana::theme::Theme;
use logana::ui::{App, LoadContext, list_dir_files};
use ratatui::prelude::*;
use std::io::{IsTerminal, stdin, stdout};
use std::sync::Arc;

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

    /// Enable the embedded MCP server on startup. Port defaults to 9876.
    /// Use --mcp for the default port or --mcp <PORT> for a custom port.
    #[arg(long, num_args = 0..=1, default_missing_value = "9876", value_name = "PORT")]
    mcp: Option<u16>,

    /// Run without TUI, write matching lines to stdout or --output.
    #[arg(long)]
    headless: bool,

    /// Write output to PATH instead of stdout (requires --headless).
    #[arg(long, value_name = "PATH", requires = "headless")]
    output: Option<std::path::PathBuf>,
}

struct AlternateScreen {
    terminal: Terminal<CrosstermBackend<std::io::Stdout>>,
}

impl AlternateScreen {
    fn new() -> Result<Self> {
        enable_raw_mode()?;
        execute!(stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
        terminal.clear()?;
        Ok(Self { terminal })
    }
}

impl Drop for AlternateScreen {
    fn drop(&mut self) {
        while crossterm::event::poll(std::time::Duration::from_millis(0)).unwrap_or(false) {
            let _ = crossterm::event::read();
        }
        let _ = execute!(stdout(), DisableMouseCapture, LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

fn get_db_path() -> String {
    if let Some(data_dir) = dirs::data_dir() {
        let app_dir = data_dir.join("logana");
        app_dir.join("logana.db").to_string_lossy().to_string()
    } else {
        "logana.db".to_string()
    }
}

fn validate_file_arg(path: &str) -> std::result::Result<(), String> {
    let p = std::path::Path::new(path);
    if !p.exists() {
        return Err(format!("'{}' not found.", path));
    }
    Ok(())
}

fn validate_inline_filter(prefix: &str, args_str: &str) -> std::result::Result<(), String> {
    use clap::Parser as _;
    use logana::auto_complete::shell_split;
    use logana::commands::CommandLine;

    let cmd = format!("{} {}", prefix, args_str);
    CommandLine::try_parse_from(shell_split(&cmd))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

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

async fn init_database() -> Result<Arc<Database>> {
    let db_path = get_db_path();
    let db = match Database::new(&db_path).await {
        Ok(db) => db,
        Err(err) => {
            eprintln!(
                "Warning: could not open database at '{}': {}. Running without persistence.",
                db_path, err
            );
            Database::in_memory().await?
        }
    };
    Ok(Arc::new(db))
}

fn validate_startup_args(args: &Args) -> std::result::Result<(), String> {
    if let Some(ref path) = args.file
        && let Err(msg) = validate_file_arg(path)
    {
        return Err(format!("Error: {}", msg));
    }

    if let Some(ref fpath) = args.filters
        && let Err(msg) = validate_file_arg(fpath)
    {
        return Err(format!("Error (--filters): {}", msg));
    }
    for args_str in &args.include_filters {
        if let Err(msg) = validate_inline_filter("filter", args_str) {
            return Err(format!("Error (-i/--include): {}", msg));
        }
    }
    for args_str in &args.exclude_filters {
        if let Err(msg) = validate_inline_filter("exclude", args_str) {
            return Err(format!("Error (-o/--exclude): {}", msg));
        }
    }
    for args_str in &args.timestamp_filters {
        if let Err(msg) = validate_inline_filter("date-filter", args_str) {
            return Err(format!("Error (-t/--timestamp): {}", msg));
        }
    }

    Ok(())
}

async fn run_headless_mode(args: Args) -> Result<()> {
    if let Some(ref path) = args.file
        && std::path::Path::new(path).is_dir()
    {
        eprintln!(
            "Error: '{}' is a directory. --headless requires a file path or stdin.",
            path
        );
        std::process::exit(1);
    }
    logana::headless::run_headless(&logana::headless::HeadlessArgs {
        file: args.file,
        filters: args.filters,
        include_filters: args.include_filters,
        exclude_filters: args.exclude_filters,
        timestamp_filters: args.timestamp_filters,
        output: args.output,
    })
    .await
}

async fn build_app(log_manager: LogManager, config: Config) -> App {
    let theme = config
        .theme
        .as_deref()
        .and_then(|name| Theme::from_file(format!("{}.json", name)).ok())
        .unwrap_or_default();
    let keybinding_conflicts: Vec<String> = config.keybindings.validate();
    let keybindings = Arc::new(config.keybindings);

    let mut app = App::new(
        log_manager,
        FileReader::from_bytes(vec![]),
        theme,
        keybindings,
        config.restore_session,
        config.restore_file_context,
        config.show_mode_bar,
        config.show_borders,
        config.show_line_numbers,
        config.show_sidebar,
        config.wrap,
    )
    .await;
    app.preview_bytes = config.preview_bytes;
    app.dlt_devices = config.dlt_devices;
    app.mcp_port = config.mcp_port;
    app.startup_warnings = keybinding_conflicts;
    app
}

async fn apply_cli_args_to_app(app: &mut App, args: &Args) {
    if let Some(ref fpath) = args.filters
        && let Err(e) = app.tabs[0].log_manager.load_filters(fpath).await
    {
        eprintln!("Warning: could not load filters from '{}': {}", fpath, e);
    }

    app.startup_tail = args.tail;

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
    // Set before applying timestamp filters so cmd_date_filter can skip the
    // format check — format is not yet detected at startup.
    app.startup_filters = args.filters.is_some() || has_inline_filters;

    for args_str in &args.timestamp_filters {
        app.execute_command_str(format!("date-filter {}", args_str))
            .await;
    }

    if let Some(port) = args.mcp {
        let p = app.mcp_port.unwrap_or(port);
        if let Err(e) = app.start_mcp(p).await {
            app.startup_warnings
                .push(format!("Failed to start MCP server on port {p}: {e}"));
        }
    }
}

async fn begin_initial_load(
    app: &mut App,
    source_path: Option<String>,
    background_file_load: bool,
    stdin_is_piped: bool,
    args: &Args,
) {
    let has_inline_filters = !args.include_filters.is_empty()
        || !args.exclude_filters.is_empty()
        || !args.timestamp_filters.is_empty();

    let startup_predicate: Option<VisibilityPredicate> =
        if background_file_load && (args.filters.is_some() || has_inline_filters) {
            let (fm, _, _, _) = app.tabs[0].log_manager.build_filter_manager();
            Some(VisibilityPredicate::new(fm))
        } else {
            None
        };

    if background_file_load {
        if let Some(path) = source_path {
            if logana::ingestion::detect_archive_type(&path).is_some() {
                app.begin_archive_extraction(&path).await;
            } else {
                app.begin_file_load(
                    path,
                    LoadContext::ReplaceInitialTab,
                    startup_predicate,
                    args.tail,
                )
                .await;
            }
        }
    } else if stdin_is_piped {
        app.begin_stdin_load().await;
    }

    if let Some(ref path) = args.file
        && std::path::Path::new(path).is_dir()
    {
        let files = list_dir_files(path);
        app.tabs[0].interaction.mode = Box::new(ConfirmOpenDirMode {
            dir: path.clone(),
            files,
        });
    }
}

async fn run_tui(args: Args, db: Arc<Database>) -> Result<()> {
    let stdin_is_piped = args.file.is_none() && !stdin().is_terminal();
    let (source_path, background_file_load) = resolve_source(&args.file);

    let log_manager = LogManager::new(db, source_path.clone()).await;
    let (config, config_error) = match Config::load() {
        Ok(cfg) => (cfg, None),
        Err(e) => (Config::default(), Some(e)),
    };

    let mut screen = AlternateScreen::new()?;
    let mut app = build_app(log_manager, config).await;
    if let Some(err) = config_error {
        app.startup_warnings.push(err);
    }
    apply_cli_args_to_app(&mut app, &args).await;
    begin_initial_load(
        &mut app,
        source_path,
        background_file_load,
        stdin_is_piped,
        &args,
    )
    .await;

    let result = app.run(&mut screen.terminal).await;
    if let Err(ref err) = result {
        eprintln!("Application error: {:?}", err);
    }
    result
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let db = init_database().await?;
    if let Err(msg) = validate_startup_args(&args) {
        eprintln!("{}", msg);
        std::process::exit(1);
    }

    if args.headless {
        return run_headless_mode(args).await;
    }

    if let Some(ref path) = args.file
        && std::path::Path::new(path).is_dir()
        && logana::ui::list_dir_files(path).is_empty()
    {
        eprintln!("Error: '{}' contains no files.", path);
        std::process::exit(1);
    }
    run_tui(args, db).await
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn test_args_mcp_absent() {
        let args = Args::try_parse_from(["logana"]).unwrap();
        assert!(args.mcp.is_none());
    }

    #[test]
    fn test_args_mcp_flag_default_port() {
        let args = Args::try_parse_from(["logana", "--mcp"]).unwrap();
        assert_eq!(args.mcp, Some(9876));
    }

    #[test]
    fn test_args_mcp_flag_custom_port() {
        let args = Args::try_parse_from(["logana", "--mcp", "8080"]).unwrap();
        assert_eq!(args.mcp, Some(8080));
    }

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

    #[test]
    fn test_validate_file_arg_nonexistent() {
        let result = validate_file_arg("/nonexistent/path/file.log");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_validate_file_arg_directory_is_ok() {
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
        assert!(result.is_err());
    }

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
