#[cfg(test)]
mod tests {
    use crate::analyzer::{LogAnalyzer, LogEntry};
    use crate::ui::{App, AppMode};
    use crossterm::event::KeyCode;

    fn setup_test_app() -> App {
        let mut analyzer = LogAnalyzer::new();
        analyzer.entries.push(LogEntry {
            id: 0,
            content: "line 1".to_string(),
            marked: false,
        });
        analyzer.entries.push(LogEntry {
            id: 1,
            content: "line 2".to_string(),
            marked: false,
        });
        analyzer.entries.push(LogEntry {
            id: 2,
            content: "line 3".to_string(),
            marked: false,
        });
        App::new(analyzer)
    }

    #[test]
    fn test_app_initialization() {
        let app = setup_test_app();
        assert!(matches!(app.mode, AppMode::Normal));
        assert_eq!(app.scroll_offset, 0);
        assert_eq!(app.analyzer.entries.len(), 3);
    }

    #[test]
    fn test_mode_transitions() {
        let mut app = setup_test_app();

        // Normal -> Command
        app.handle_key_event(KeyCode::Char(':'));
        assert!(matches!(app.mode, AppMode::Command));

        // Command -> Normal (on Esc)
        app.handle_key_event(KeyCode::Esc);
        assert!(matches!(app.mode, AppMode::Normal));

        // Normal -> Command
        app.handle_key_event(KeyCode::Char(':'));
        assert!(matches!(app.mode, AppMode::Command));

        // Command -> Normal (on Enter)
        app.handle_key_event(KeyCode::Enter);
        assert!(matches!(app.mode, AppMode::Normal));

        // Normal -> FilterManagement
        app.handle_key_event(KeyCode::Char('f'));
        assert!(matches!(app.mode, AppMode::FilterManagement));

        // FilterManagement -> Normal (on Esc)
        app.handle_key_event(KeyCode::Esc);
        assert!(matches!(app.mode, AppMode::Normal));

        // Normal -> Search (forward)
        app.handle_key_event(KeyCode::Char('/'));
        assert!(matches!(app.mode, AppMode::Search));
        assert!(app.search_forward);

        // Search -> Normal (on Esc)
        app.handle_key_event(KeyCode::Esc);
        assert!(matches!(app.mode, AppMode::Normal));

        // Normal -> Search (backward)
        app.handle_key_event(KeyCode::Char('?'));
        assert!(matches!(app.mode, AppMode::Search));
        assert!(!app.search_forward);
    }

    #[test]
    fn test_command_input_and_execution() {
        let mut app = setup_test_app();

        // Enter command mode
        app.handle_key_event(KeyCode::Char(':'));
        assert!(matches!(app.mode, AppMode::Command));

        // Type command
        app.handle_key_event(KeyCode::Char('f'));
        app.handle_key_event(KeyCode::Char('i'));
        app.handle_key_event(KeyCode::Char('l'));
        app.handle_key_event(KeyCode::Char('t'));
        app.handle_key_event(KeyCode::Char('e'));
        app.handle_key_event(KeyCode::Char('r'));
        app.handle_key_event(KeyCode::Char(' '));
        app.handle_key_event(KeyCode::Char('e'));
        app.handle_key_event(KeyCode::Char('r'));
        app.handle_key_event(KeyCode::Char('r'));
        app.handle_key_event(KeyCode::Char('o'));
        app.handle_key_event(KeyCode::Char('r'));

        assert_eq!(app.command_input, "filter error");

        // Execute command
        app.handle_key_event(KeyCode::Enter);
        assert!(matches!(app.mode, AppMode::Normal));
        assert_eq!(app.analyzer.filters.len(), 1);
        assert_eq!(app.analyzer.filters[0].pattern, "error");
    }

    #[test]
    fn test_filter_management() {
        let mut app = setup_test_app();
        app.analyzer
            .add_filter("filter1".to_string(), crate::analyzer::FilterType::Include);
        app.analyzer
            .add_filter("filter2".to_string(), crate::analyzer::FilterType::Include);

        // Enter filter management mode
        app.handle_key_event(KeyCode::Char('f'));
        assert!(matches!(app.mode, AppMode::FilterManagement));
        assert_eq!(app.selected_filter_index, 0);

        // Move down
        app.handle_key_event(KeyCode::Down);
        assert_eq!(app.selected_filter_index, 1);

        // Move up
        app.handle_key_event(KeyCode::Up);
        assert_eq!(app.selected_filter_index, 0);

        // Toggle filter
        app.handle_key_event(KeyCode::Char(' '));
        assert!(!app.analyzer.filters[0].enabled);

        // Edit filter
        app.handle_key_event(KeyCode::Char('e'));
        assert!(matches!(app.mode, AppMode::FilterEdit));
        app.handle_key_event(KeyCode::Char('a'));
        app.handle_key_event(KeyCode::Enter);
        assert!(matches!(app.mode, AppMode::FilterManagement));
        assert_eq!(app.analyzer.filters[0].pattern, "filter1a");

        // Delete filter
        app.handle_key_event(KeyCode::Char('d'));
        assert_eq!(app.analyzer.filters.len(), 1);
        assert_eq!(app.analyzer.filters[0].pattern, "filter2");
    }

    #[test]
    fn test_searching() {
        let mut app = setup_test_app();
        app.analyzer.entries.push(LogEntry {
            id: 3,
            content: "search me".to_string(),
            marked: false,
        });
        app.analyzer.entries.push(LogEntry {
            id: 4,
            content: "another search".to_string(),
            marked: false,
        });

        // Enter search mode
        app.handle_key_event(KeyCode::Char('/'));
        assert!(matches!(app.mode, AppMode::Search));

        // Type search query
        app.handle_key_event(KeyCode::Char('s'));
        app.handle_key_event(KeyCode::Char('e'));
        app.handle_key_event(KeyCode::Char('a'));
        app.handle_key_event(KeyCode::Char('r'));
        app.handle_key_event(KeyCode::Char('c'));
        app.handle_key_event(KeyCode::Char('h'));

        assert_eq!(app.search_input, "search");

        // Execute search
        app.handle_key_event(KeyCode::Enter);
        assert!(matches!(app.mode, AppMode::Normal));
        assert_eq!(app.search.get_results().len(), 2);
        assert_eq!(app.scroll_offset, 4);

        // Next match
        app.handle_key_event(KeyCode::Char('n'));
        assert_eq!(app.scroll_offset, 3);

        // Next match (wraps around)
        app.handle_key_event(KeyCode::Char('n'));
        assert_eq!(app.scroll_offset, 4);

        // Previous match
        app.handle_key_event(KeyCode::Char('N'));
        assert_eq!(app.scroll_offset, 3);

        // Previous match (wraps around)
        app.handle_key_event(KeyCode::Char('N'));
        assert_eq!(app.scroll_offset, 4);
    }

    #[test]
    fn test_scrolling_and_marking() {
        let mut app = setup_test_app();

        // Scroll down
        app.handle_key_event(KeyCode::Down);
        assert_eq!(app.scroll_offset, 1);

        // Mark line
        app.handle_key_event(KeyCode::Char('m'));
        assert!(app.analyzer.entries[1].marked);

        // Scroll up
        app.handle_key_event(KeyCode::Up);
        assert_eq!(app.scroll_offset, 0);

        // Unmark line
        app.handle_key_event(KeyCode::Down);
        app.handle_key_event(KeyCode::Char('m'));
        assert!(!app.analyzer.entries[1].marked);
    }
}

mod color_config;
mod filter_persistence;
mod line_wrapping;
mod stdin;
mod vim_motions;
