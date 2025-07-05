use clap::Parser;
use crossterm::{
    ExecutableCommand,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use logsmith_rs::analyzer::LogAnalyzer;
use logsmith_rs::theme::Theme;
use logsmith_rs::ui::App;
use ratatui::prelude::*;
use std::io::{IsTerminal, stdin, stdout};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Optional file to process. If not provided, reads from stdin.
    file: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let file_path = args.file;
    let mut analyzer = LogAnalyzer::new();

    let file_path_ref: Option<&str> = file_path.as_deref();
    if let Some(path) = file_path_ref {
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
