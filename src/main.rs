use clap::Parser;
use crossterm::{
    ExecutableCommand,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use logsmith_rs::analyzer::LogAnalyzer;
use logsmith_rs::db::Database;
use logsmith_rs::theme::Theme;
use logsmith_rs::ui::App;
use ratatui::prelude::*;
use std::io::{IsTerminal, stdin, stdout};
use std::sync::Arc;

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

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let file_path = args.file;

    let rt = Arc::new(tokio::runtime::Runtime::new()?);
    let db_path = get_db_path();
    let db = rt.block_on(Database::new(&db_path))?;
    let db = Arc::new(db);

    let analyzer = LogAnalyzer::new(db.clone(), rt.clone());

    // Always clear logs from previous sessions - only filters are persisted
    analyzer.clear_logs();

    if let Some(ref path) = file_path {
        analyzer.ingest_file(path)?;
    } else if !stdin().is_terminal() {
        analyzer.ingest_reader(stdin())?;
    }

    let res = {
        enable_raw_mode()?;
        stdout().execute(EnterAlternateScreen)?;
        let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
        terminal.clear()?;

        let mut app = App::new(analyzer, Theme::default());
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
