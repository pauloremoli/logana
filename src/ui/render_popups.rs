#[cfg(test)]
mod tests {
    use crate::config::Keybindings;
    use crate::db::{Database, FileContext};
    use crate::file_reader::FileReader;
    use crate::log_manager::LogManager;
    use crate::mode::app_mode::{ConfirmRestoreMode, ConfirmRestoreSessionMode};
    use crate::mode::comment_mode::CommentMode;
    use crate::mode::docker_select_mode::DockerSelectMode;
    use crate::mode::keybindings_help_mode::KeybindingsHelpMode;
    use crate::mode::select_fields_mode::SelectFieldsMode;
    use crate::mode::value_colors_mode::{ValueColorEntry, ValueColorGroup, ValueColorsMode};
    use crate::theme::Theme;
    use crate::types::{DockerContainer, FieldLayout};
    use crate::ui::App;
    use ratatui::{Terminal, backend::TestBackend};
    use std::collections::HashSet;
    use std::sync::Arc;

    async fn make_app(lines: &[&str]) -> App {
        let data: Vec<u8> = lines.join("\n").into_bytes();
        let file_reader = FileReader::from_bytes(data);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        App::new(
            log_manager,
            file_reader,
            Theme::default(),
            Arc::new(Keybindings::default()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
    }

    fn make_terminal() -> Terminal<TestBackend> {
        Terminal::new(TestBackend::new(80, 24)).unwrap()
    }

    #[tokio::test]
    async fn test_confirm_restore_modal() {
        let mut app = make_app(&["line one", "line two"]).await;
        let context = FileContext {
            source_file: "/tmp/test.log".to_string(),
            scroll_offset: 0,
            search_query: String::new(),
            level_colors_disabled: std::collections::HashSet::new(),
            horizontal_scroll: 0,
            marked_lines: vec![],
            file_hash: None,
            comments: vec![],
            show_keys: false,
            raw_mode: false,
            sidebar_width: 30,
            hidden_fields: std::collections::HashSet::new(),
            field_layout_columns: None,
            filtering_enabled: true,
        };
        app.tabs[0].interaction.mode = Box::new(ConfirmRestoreMode { context });
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_select_fields_basic() {
        let mut app = make_app(&["line one", "line two"]).await;
        let fields = vec![
            ("timestamp".to_string(), true),
            ("level".to_string(), true),
            ("message".to_string(), false),
        ];
        app.tabs[0].interaction.mode = Box::new(SelectFieldsMode::new(
            fields,
            FieldLayout::default(),
            std::collections::HashSet::new(),
        ));
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_select_fields_with_scroll() {
        let mut app = make_app(&["line one", "line two"]).await;
        let fields: Vec<(String, bool)> = (0..35)
            .map(|i| (format!("field_{}", i), i % 2 == 0))
            .collect();
        app.tabs[0].interaction.mode = Box::new(SelectFieldsMode::new(
            fields,
            FieldLayout::default(),
            std::collections::HashSet::new(),
        ));
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_value_colors_basic() {
        let mut app = make_app(&["line one", "line two"]).await;
        let groups = vec![ValueColorGroup {
            label: "HTTP Methods".to_string(),
            children: vec![
                ValueColorEntry {
                    key: "http_get".to_string(),
                    label: "GET".to_string(),
                    color: ratatui::style::Color::Green,
                    enabled: true,
                },
                ValueColorEntry {
                    key: "http_post".to_string(),
                    label: "POST".to_string(),
                    color: ratatui::style::Color::Yellow,
                    enabled: true,
                },
            ],
        }];
        let mode = ValueColorsMode::new(groups, HashSet::new());
        app.tabs[0].interaction.mode = Box::new(mode);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_value_colors_with_search() {
        let mut app = make_app(&["line one", "line two"]).await;
        let groups = vec![ValueColorGroup {
            label: "HTTP Methods".to_string(),
            children: vec![
                ValueColorEntry {
                    key: "http_get".to_string(),
                    label: "GET".to_string(),
                    color: ratatui::style::Color::Green,
                    enabled: true,
                },
                ValueColorEntry {
                    key: "http_post".to_string(),
                    label: "POST".to_string(),
                    color: ratatui::style::Color::Yellow,
                    enabled: true,
                },
            ],
        }];
        let mut mode = ValueColorsMode::new(groups, HashSet::new());
        mode.search = "http".to_string();
        app.tabs[0].interaction.mode = Box::new(mode);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_value_colors_partial_enabled() {
        let mut app = make_app(&["line one", "line two"]).await;
        let groups = vec![ValueColorGroup {
            label: "Status Codes".to_string(),
            children: vec![
                ValueColorEntry {
                    key: "status_2xx".to_string(),
                    label: "2xx".to_string(),
                    color: ratatui::style::Color::Green,
                    enabled: true,
                },
                ValueColorEntry {
                    key: "status_4xx".to_string(),
                    label: "4xx".to_string(),
                    color: ratatui::style::Color::Red,
                    enabled: false,
                },
                ValueColorEntry {
                    key: "status_5xx".to_string(),
                    label: "5xx".to_string(),
                    color: ratatui::style::Color::Magenta,
                    enabled: true,
                },
            ],
        }];
        let mut disabled = HashSet::new();
        disabled.insert("status_4xx".to_string());
        let mode = ValueColorsMode::new(groups, disabled);
        app.tabs[0].interaction.mode = Box::new(mode);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_value_colors_scrollbar() {
        let mut app = make_app(&["line one", "line two"]).await;
        let children: Vec<ValueColorEntry> = (0..30)
            .map(|i| ValueColorEntry {
                key: format!("key_{}", i),
                label: format!("Entry {}", i),
                color: ratatui::style::Color::Cyan,
                enabled: true,
            })
            .collect();
        let groups = vec![ValueColorGroup {
            label: "Many Entries".to_string(),
            children,
        }];
        let mode = ValueColorsMode::new(groups, HashSet::new());
        app.tabs[0].interaction.mode = Box::new(mode);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_docker_select_basic() {
        let mut app = make_app(&["line one", "line two"]).await;
        let containers = vec![
            DockerContainer {
                id: "abc123".to_string(),
                name: "web-app".to_string(),
                image: "nginx:latest".to_string(),
                status: "Up 2 hours".to_string(),
            },
            DockerContainer {
                id: "def456".to_string(),
                name: "db".to_string(),
                image: "postgres:15".to_string(),
                status: "Up 3 hours".to_string(),
            },
            DockerContainer {
                id: "ghi789".to_string(),
                name: "cache".to_string(),
                image: "redis:7".to_string(),
                status: "Up 1 hour".to_string(),
            },
        ];
        app.tabs[0].interaction.mode = Box::new(DockerSelectMode::new(containers));
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_docker_select_error() {
        let mut app = make_app(&["line one", "line two"]).await;
        app.tabs[0].interaction.mode =
            Box::new(DockerSelectMode::with_error("Docker not found".to_string()));
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_docker_select_scrollbar() {
        let mut app = make_app(&["line one", "line two"]).await;
        let containers: Vec<DockerContainer> = (0..25)
            .map(|i| DockerContainer {
                id: format!("id_{}", i),
                name: format!("container_{}", i),
                image: format!("image_{}:latest", i),
                status: format!("Up {} hours", i),
            })
            .collect();
        app.tabs[0].interaction.mode = Box::new(DockerSelectMode::new(containers));
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_keybindings_help_basic() {
        let mut app = make_app(&["line one", "line two"]).await;
        app.tabs[0].interaction.mode = Box::new(KeybindingsHelpMode::new());
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_keybindings_help_with_search() {
        let mut app = make_app(&["line one", "line two"]).await;
        let mut mode = KeybindingsHelpMode::new();
        mode.search = "scroll".to_string();
        app.tabs[0].interaction.mode = Box::new(mode);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_keybindings_help_scroll() {
        let mut app = make_app(&["line one", "line two"]).await;
        let mut mode = KeybindingsHelpMode::new();
        mode.scroll = 5;
        app.tabs[0].interaction.mode = Box::new(mode);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_confirm_restore_session() {
        let mut app = make_app(&["line one", "line two"]).await;
        app.tabs[0].interaction.mode = Box::new(ConfirmRestoreSessionMode {
            files: vec!["file1.log".to_string(), "file2.log".to_string()],
        });
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_comment_popup_basic() {
        let mut app = make_app(&["line one", "line two"]).await;
        app.tabs[0].interaction.mode = Box::new(CommentMode::new(vec![0, 1]));
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_comment_popup_multiline() {
        let mut app = make_app(&["line one", "line two", "line three"]).await;
        let mut mode = CommentMode::new(vec![0, 1]);
        mode.lines = vec!["line 1".to_string(), "line 2".to_string()];
        mode.cursor_row = 1;
        app.tabs[0].interaction.mode = Box::new(mode);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_comment_popup_cursor_boundary() {
        let mut app = make_app(&["line one", "line two"]).await;
        let mut mode = CommentMode::new(vec![0]);
        mode.lines = vec!["short".to_string()];
        mode.cursor_col = 100;
        app.tabs[0].interaction.mode = Box::new(mode);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }
}
