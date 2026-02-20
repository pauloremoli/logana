use anyhow::Result;
use clap::Parser;
use crossterm::{
    ExecutableCommand,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use logsmith_rs::db::Database;
use logsmith_rs::file_reader::FileReader;
use logsmith_rs::log_manager::LogManager;
use logsmith_rs::theme::Theme;
use logsmith_rs::ui::{App, LoadContext};
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
        let app_dir = data_dir.join("logsmith-rs");
        app_dir.join("logsmith.db").to_string_lossy().to_string()
    } else {
        "logsmith.db".to_string()
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    let file_path = args.file;

    let file_appender = rolling::daily("logs", "logsmith.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    fmt()
        .with_env_filter(env_filter)
        .with_writer(non_blocking)
        .json()
        .init();

    let rt = Arc::new(tokio::runtime::Runtime::new()?);
    let db_path = get_db_path();
    let db = rt.block_on(Database::new(&db_path)).inspect_err(|err| {
        error!("Failed to init database: {}", err);
    })?;
    let db = Arc::new(db);

    // Validate the file path before entering the TUI (gives a clean error message).
    if let Some(ref path) = file_path {
        let p = std::path::Path::new(path);
        if !p.exists() {
            eprintln!("Error: File '{}' not found.", path);
            std::process::exit(1);
        }
        if p.is_dir() {
            eprintln!("Error: '{}' is a directory, not a file.", path);
            std::process::exit(1);
        }
    }

    // Detect piped stdin before entering the TUI.
    let stdin_is_piped = file_path.is_none() && !stdin().is_terminal();

    // For a file argument use an empty placeholder — the real FileReader is
    // loaded in the background after the TUI starts so the progress bar is shown.
    // For stdin, also use an empty placeholder; reading happens in the background.
    let (source_path, background_file_load) = if let Some(ref path) = file_path {
        (Some(path.clone()), true)
    } else {
        (None, false)
    };

    let log_manager = LogManager::new(db.clone(), rt.clone(), source_path.clone());

    let res = {
        enable_raw_mode()?;
        stdout().execute(EnterAlternateScreen)?;
        let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
        terminal.clear()?;

        let mut app = App::new(log_manager, FileReader::from_bytes(vec![]), Theme::default());

        // Kick off the background file load now that the TUI is visible.
        if background_file_load {
            if let Some(path) = source_path {
                app.begin_file_load(path, LoadContext::ReplaceInitialTab);
            }
        } else if stdin_is_piped {
            app.begin_stdin_load();
        }

        let app_result = app.run(&mut terminal);

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
