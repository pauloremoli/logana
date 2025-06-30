use crate::analyzer::{LogAnalyzer, LogEntry};
use crate::ui::App;
use crossterm::event::KeyCode;

fn setup_test_app_for_vim_motions() -> App {
    let mut analyzer = LogAnalyzer::new();
    for i in 0..100 {
        analyzer.entries.push(LogEntry {
            id: i,
            content: format!("line {}", i),
            marked: false,
        });
    }
    App::new(analyzer)
}

#[test]
fn test_vim_j_key() {
    let mut app = setup_test_app_for_vim_motions();
    app.handle_key_event(KeyCode::Char('j'));
    assert_eq!(app.scroll_offset, 1);
}

#[test]
fn test_vim_k_key() {
    let mut app = setup_test_app_for_vim_motions();
    app.scroll_offset = 5;
    app.handle_key_event(KeyCode::Char('k'));
    assert_eq!(app.scroll_offset, 4);
}

#[test]
fn test_vim_gg_key() {
    let mut app = setup_test_app_for_vim_motions();
    app.scroll_offset = 50;
    app.handle_key_event(KeyCode::Char('g'));
    app.handle_key_event(KeyCode::Char('g'));
    assert_eq!(app.scroll_offset, 0);
}

#[test]
fn test_vim_g_key() {
    let mut app = setup_test_app_for_vim_motions();
    app.scroll_offset = 50;
    app.handle_key_event(KeyCode::Char('G'));
    assert_eq!(app.scroll_offset, 99);
}