use anyhow::Result;
use clap::Parser;
use crossterm::{
    ExecutableCommand,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use logana::config::Config;
use logana::db::Database;
use logana::file_reader::FileReader;
use logana::log_manager::LogManager;
use logana::theme::Theme;
use logana::ui::{App, LoadContext};
use ratatui::prelude::*;
use std::io::{IsTerminal, stdin, stdout};
use std::sync::Arc;
use tracing::error;
use tracing_appender::rolling;
use tracing_subscriber::{EnvFilter, fmt};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Optional file to process. If not provided, reads from stdin.
    file: Option<String>,
}

fn get_db_path() -> String {
    if let Some(data_dir) = dirs::data_dir() {
        let app_dir = data_dir.join("logana");
        app_dir.join("logana.db").to_string_lossy().to_string()
    } else {
        "logana.db".to_string()
    }
}

/// Validate that `path` exists and is a regular file.
/// Returns `Ok(())` on success, or an error message describing the problem.
fn validate_file_path(path: &str) -> std::result::Result<(), String> {
    let p = std::path::Path::new(path);
    if !p.exists() {
        return Err(format!("File '{}' not found.", path));
    }
    if p.is_dir() {
        return Err(format!("'{}' is a directory, not a file.", path));
    }
    Ok(())
}

/// Determine whether to start a background file load and what source path
/// to associate with the `LogManager`.
fn resolve_source(file_path: &Option<String>) -> (Option<String>, bool) {
    if let Some(path) = file_path {
        (Some(path.clone()), true)
    } else {
        (None, false)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let file_path = args.file;

    let file_appender = rolling::daily("logs", "logana.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    fmt()
        .with_env_filter(env_filter)
        .with_writer(non_blocking)
        .json()
        .init();

    let db_path = get_db_path();
    let db = Database::new(&db_path).await.inspect_err(|err| {
        error!("Failed to init database: {}", err);
    })?;
    let db = Arc::new(db);

    // Validate the file path before entering the TUI (gives a clean error message).
    if let Some(ref path) = file_path
        && let Err(msg) = validate_file_path(path)
    {
        eprintln!("Error: {}", msg);
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

    for conflict in config.keybindings.validate() {
        tracing::warn!("{}", conflict);
        eprintln!("Warning: {}", conflict);
    }

    let keybindings = Arc::new(config.keybindings);

    let res = {
        enable_raw_mode()?;
        stdout().execute(EnterAlternateScreen)?;
        let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
        terminal.clear()?;

        let mut app = App::new(
            log_manager,
            FileReader::from_bytes(vec![]),
            theme,
            keybindings,
        )
        .await;

        // Kick off the background file load now that the TUI is visible.
        if background_file_load {
            if let Some(path) = source_path {
                app.begin_file_load(path, LoadContext::ReplaceInitialTab)
                    .await;
            }
        } else if stdin_is_piped {
            app.begin_stdin_load().await;
        }

        let app_result = app.run(&mut terminal).await;

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
    }

    #[test]
    fn test_args_with_file() {
        let args = Args::try_parse_from(["logana", "/var/log/syslog"]).unwrap();
        assert_eq!(args.file, Some("/var/log/syslog".to_string()));
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

    // ── validate_file_path ────────────────────────────────────────────

    #[test]
    fn test_validate_file_path_nonexistent() {
        let result = validate_file_path("/nonexistent/path/file.log");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_validate_file_path_directory() {
        let result = validate_file_path("/tmp");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("directory"));
    }

    #[test]
    fn test_validate_file_path_valid_file() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap();
        assert!(validate_file_path(path).is_ok());
    }

    #[test]
    fn test_validate_file_path_empty_string() {
        let result = validate_file_path("");
        // Empty path doesn't exist.
        assert!(result.is_err());
    }

    // ── resolve_source ────────────────────────────────────────────────

    #[test]
    fn test_resolve_source_with_file() {
        let file_path = Some("/var/log/syslog".to_string());
        let (source, bg_load) = resolve_source(&file_path);
        assert_eq!(source, Some("/var/log/syslog".to_string()));
        assert!(bg_load);
    }

    #[test]
    fn test_resolve_source_without_file() {
        let file_path: Option<String> = None;
        let (source, bg_load) = resolve_source(&file_path);
        assert!(source.is_none());
        assert!(!bg_load);
    }
}
