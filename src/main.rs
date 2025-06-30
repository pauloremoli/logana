use clap::Parser;
use crossterm::{
    ExecutableCommand,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use logsmith_rs::analyzer::LogAnalyzer;
use logsmith_rs::ui::App;
use ratatui::prelude::*;
use std::io::{stdin, stdout, IsTerminal};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Optional file to process. If not provided, reads from stdin.
    file: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let mut analyzer = LogAnalyzer::new();

    if let Some(file_path) = args.file {
        analyzer.ingest_file(&file_path)?;
    } else if !stdin().is_terminal() {
        analyzer.ingest_reader(stdin())?;
    }

    let res = {
        enable_raw_mode()?;
        stdout().execute(EnterAlternateScreen)?;
        let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
        terminal.clear()?;

        let mut app = App::new(analyzer);
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
