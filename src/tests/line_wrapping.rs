use crate::ui::App;
use crossterm::event::KeyCode;

#[test]
fn test_toggle_line_wrapping() {
    let mut app = App::new(Default::default());
    assert!(app.wrap);
    app.handle_key_event(KeyCode::Char('w'));
    assert!(!app.wrap);
    app.handle_key_event(KeyCode::Char('w'));
    assert!(app.wrap);
}

#[test]
fn test_horizontal_scroll() {
    let mut app = App::new(Default::default());
    app.wrap = false;
    assert_eq!(app.horizontal_scroll, 0);
    app.handle_key_event(KeyCode::Char('l'));
    assert_eq!(app.horizontal_scroll, 1);
    app.handle_key_event(KeyCode::Char('h'));
    assert_eq!(app.horizontal_scroll, 0);
}