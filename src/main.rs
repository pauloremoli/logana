use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, KeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
        supports_keyboard_enhancement,
    },
};
use logana::db::Database;
use logana::db::LogManager;
use logana::db::{AppSettingsStore, SettingsKey};
use logana::ingestion::{FileReader, VisibilityPredicate};
use logana::theme::Theme;
use logana::ui::{App, LoadContext};
use logana::{
    config::{Config, DEFAULT_PREVIEW_BYTES, init_schemas},
    utils::filesystem::list_dir_files,
};
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

    /// Execute a command and stream its output to a tab.
    /// The value is a quoted command string passed to the program directly
    /// (no shell); arguments are separated by whitespace.
    /// Example: logana --run "docker logs -f mycontainer"
    /// Stderr lines are prefixed with "ERROR " for visibility.
    #[arg(long, value_name = "COMMAND", conflicts_with = "file")]
    run: Option<String>,
}

struct AlternateScreen {
    terminal: Terminal<CrosstermBackend<std::io::Stdout>>,
    /// `true` only when `PushKeyboardEnhancementFlags` was actually sent, so
    /// `Drop` pops exactly what it pushed. On a terminal that supports the
    /// protocol (Kitty, WezTerm, foot, Ghostty, …), this lets keybindings
    /// like `Ctrl+m` be reported distinctly from a plain `Enter` — outside
    /// it, both otherwise arrive as the identical carriage-return byte, and
    /// the terminal reports it as `Enter` (see
    /// `ArchivePickerKeybindings::search_merge_toggle`'s doc comment).
    keyboard_enhancement_enabled: bool,
}

impl AlternateScreen {
    fn new() -> Result<Self> {
        enable_raw_mode()?;
        let keyboard_enhancement_enabled = supports_keyboard_enhancement().unwrap_or(false)
            && execute!(
                stdout(),
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            )
            .is_ok();
        execute!(stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
        terminal.clear()?;
        Ok(Self {
            terminal,
            keyboard_enhancement_enabled,
        })
    }
}

impl Drop for AlternateScreen {
    fn drop(&mut self) {
        while crossterm::event::poll(std::time::Duration::from_millis(0)).unwrap_or(false) {
            let _ = crossterm::event::read();
        }
        if self.keyboard_enhancement_enabled {
            let _ = execute!(stdout(), PopKeyboardEnhancementFlags);
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
    use logana::commands::CommandLine;
    use logana::commands::auto_complete::shell_split;

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

    let mut schema_warnings = init_schemas();
    schema_warnings.extend(logana::parser::validate_custom_schemas(
        logana::config::custom_schemas(),
    ));
    for warning in &schema_warnings {
        eprintln!("Warning: {warning}");
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
    let theme_name = if config.theme.is_some() {
        config.theme.clone()
    } else {
        log_manager
            .db
            .load_app_setting(SettingsKey::Theme)
            .await
            .ok()
            .flatten()
    };
    let theme = theme_name
        .as_deref()
        .and_then(|name| Theme::from_file(format!("{}.json", name)).ok())
        .unwrap_or_default();
    let default_filter_files =
        logana::config::resolve_default_filter_files(&config, log_manager.db.as_ref()).await;
    let mut schema_warnings = init_schemas();
    schema_warnings.extend(logana::parser::validate_custom_schemas(
        logana::config::custom_schemas(),
    ));
    let keybinding_conflicts: Vec<String> = config.keybindings.validate();
    let keybindings = Arc::new(config.keybindings);

    let mut app = App::builder(
        log_manager,
        FileReader::from_bytes(vec![]),
        theme,
        keybindings,
    )
    .restore_policy(config.restore_session)
    .restore_file_policy(config.restore_file_context)
    .show_mode_bar(config.show_mode_bar)
    .show_borders(config.show_borders)
    .show_line_numbers(config.show_line_numbers)
    .show_sidebar(config.show_sidebar)
    .wrap(config.wrap)
    .sidebar_side(config.sidebar_side)
    .build()
    .await;
    app.preview_bytes = config.preview_bytes.unwrap_or(DEFAULT_PREVIEW_BYTES);
    app.dlt_devices = config.dlt_devices;
    app.default_filter_files = default_filter_files;
    app.mcp.port = config.mcp_port;
    app.session.startup_warnings = keybinding_conflicts;
    app.session.startup_warnings.extend(schema_warnings);
    app
}

async fn apply_cli_args_to_app(app: &mut App, args: &Args) {
    if let Some(ref fpath) = args.filters
        && let Err(e) = app.tabs[0].log_manager.load_filters(fpath).await
    {
        app.session
            .startup_warnings
            .push(format!("could not load filters from '{}': {}", fpath, e));
    }

    app.session.startup_tail = args.tail;

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
    app.session.startup_filters = args.filters.is_some() || has_inline_filters;

    for args_str in &args.timestamp_filters {
        app.execute_command_str(format!("date-filter {}", args_str))
            .await;
    }

    if let Some(port) = args.mcp {
        let p = app.mcp.port.unwrap_or(port);
        if let Err(e) = app.start_mcp(p).await {
            app.session
                .startup_warnings
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
                app.begin_archive_listing(&path).await;
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
        && let Ok(tree) = logana::ingestion::list_directory_tree(path)
    {
        // A directory was explicitly given, so this isn't a bare "resume
        // where I left off" launch — cancel any queued session restore
        // (`AppBuilder::build` can't tell a directory apart from "no
        // argument at all", since it never sees `args.file` directly) so it
        // doesn't overwrite this mode the moment `app.run()` starts.
        app.session.pending_session_restore = None;
        app.tabs[0].interaction.mode = Box::new(
            logana::mode::archive_picker_mode::ArchivePickerMode::new(tree, path.clone()),
        );
    }
}

fn install_panic_hook() {
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(stdout(), DisableMouseCapture, LeaveAlternateScreen);
        original_hook(panic_info);
    }));
}

async fn run_tui(args: Args, db: Arc<Database>) -> Result<()> {
    install_panic_hook();
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
        app.session.startup_warnings.push(err);
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

    if let Some(cmd) = args.run {
        let tokens: Vec<String> = cmd.split_whitespace().map(str::to_string).collect();
        app.open_run_command(tokens).await;
    }

    // Not eprintln!'d here: `screen` (the alternate-screen guard) is still
    // alive at this point, so a direct stderr write would land in the
    // alternate screen buffer and be discarded when it's restored below —
    // never actually seen by the user. `main` already reports this same
    // error once `run_tui` returns and the terminal has been restored.
    app.run(&mut screen.terminal).await
}

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("Fatal error: {:?}", err);
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
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
        && list_dir_files(path).is_empty()
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

    async fn make_test_app() -> App {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let fr = FileReader::from_bytes(b"line\n".to_vec());
        let lm = LogManager::new(db, None).await;
        App::builder(
            lm,
            fr,
            Theme::default(),
            Arc::new(logana::config::Keybindings::default()),
        )
        .build()
        .await
    }

    /// A bad `--filters` path must be recorded as a startup warning, not
    /// printed to stderr — by the time this runs the terminal is already in
    /// raw/alt-screen mode (see `run_tui`), so a direct stderr write would
    /// corrupt the TUI's display instead of being visible to the user.
    #[tokio::test]
    async fn test_apply_cli_args_records_bad_filters_path_as_startup_warning() {
        let mut app = make_test_app().await;
        let args = Args::try_parse_from([
            "logana",
            "file.log",
            "--filters",
            "/nonexistent/bad-filters.json",
        ])
        .unwrap();
        apply_cli_args_to_app(&mut app, &args).await;
        assert!(
            app.session
                .startup_warnings
                .iter()
                .any(|w| w.contains("bad-filters.json")),
            "expected a startup warning naming the failed filters path, got: {:?}",
            app.session.startup_warnings
        );
    }
}
