use ratatui::{
    prelude::*,
    widgets::{Paragraph, Wrap},
};

use crate::auto_complete::{
    FieldCompletion, complete_color, complete_field_name, complete_field_value, complete_file_path,
    complete_flags, extract_color_partial, extract_field_partial, extract_flag_partial,
    find_command_completions, fuzzy_match, shell_split,
};
use crate::commands::{FILE_PATH_COMMANDS, find_matching_command};
use crate::theme::{Theme, complete_theme};
use crate::types::parse_color;
use crate::ui::TabState;

pub enum CompletionSource {
    Error(String),
    Items(Vec<String>),
    ColorItems(Vec<String>),
    FileItems(Vec<String>),
    CommandHelp(String),
}

pub fn resolve_completions(
    tab: &mut TabState,
    query_text: &str,
    completion_index: Option<usize>,
) -> CompletionSource {
    if let Some(err) = &tab.interaction.command_error {
        return CompletionSource::Error(err.clone());
    }
    if let Some(fc) = extract_field_partial(query_text.trim_start()) {
        let field_index = tab.build_field_index();
        let completions = match &fc {
            FieldCompletion::Name(partial) => complete_field_name(partial, &field_index),
            FieldCompletion::Value { field, partial } => {
                complete_field_value(field, partial, &field_index)
            }
        };
        return CompletionSource::Items(completions);
    }
    if let Some((_, partial)) = extract_flag_partial(query_text) {
        let cmd = shell_split(query_text)
            .into_iter()
            .next()
            .unwrap_or_default();
        return CompletionSource::Items(
            complete_flags(&cmd, &partial)
                .into_iter()
                .map(|s| s.to_string())
                .collect(),
        );
    }
    if let Some(partial) = extract_color_partial(query_text.trim_start()) {
        return CompletionSource::ColorItems(
            complete_color(partial)
                .into_iter()
                .map(|s| s.to_string())
                .collect(),
        );
    }
    let trimmed = query_text.trim();
    let file_cmd = FILE_PATH_COMMANDS
        .iter()
        .find(|cmd| trimmed.starts_with(&format!("{} ", cmd)));
    if let Some(&cmd) = file_cmd {
        let partial = trimmed[cmd.len()..].trim_start();
        return CompletionSource::FileItems(complete_file_path(partial));
    }
    if let Some(partial_raw) = query_text.trim_start().strip_prefix("set-theme ") {
        return CompletionSource::Items(complete_theme(partial_raw.trim_start()));
    }
    if let Some(partial_raw) = query_text.trim_start().strip_prefix("hide-field ") {
        let partial = partial_raw.trim_start();
        let index = tab.build_field_index();
        return CompletionSource::Items(complete_field_name(partial, &index));
    }
    if let Some(partial_raw) = query_text.trim_start().strip_prefix("show-field ") {
        let partial = partial_raw.trim_start();
        let candidates: Vec<String> = if tab.display.hidden_fields.is_empty() {
            tab.build_field_index().names
        } else {
            let mut v: Vec<String> = tab.display.hidden_fields.iter().cloned().collect();
            v.sort();
            v
        };
        let completions = candidates
            .iter()
            .filter(|n| fuzzy_match(partial, n))
            .cloned()
            .collect();
        return CompletionSource::Items(completions);
    }
    if completion_index.is_none()
        && let Some(cmd) = find_matching_command(query_text)
    {
        return CompletionSource::CommandHelp(format!("  {} - {}", cmd.usage, cmd.description));
    }
    CompletionSource::Items(
        find_command_completions(trimmed)
            .into_iter()
            .map(|s| s.to_string())
            .collect(),
    )
}

pub fn file_display_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| {
            if path.ends_with('/') {
                format!("{}/", n.trim_end_matches('/'))
            } else {
                n.to_string()
            }
        })
        .unwrap_or_else(|| path.to_string())
}

pub struct CommandBar<'a> {
    pub input_text: &'a str,
    pub cursor_pos: usize,
    pub completion: CompletionSource,
    pub theme: &'a Theme,
}

impl<'a> CommandBar<'a> {
    pub fn cursor_position(&self, input_area: Rect) -> Option<(u16, u16)> {
        let cursor_x = input_area.x + 1 + self.cursor_pos as u16;
        if cursor_x < input_area.x + input_area.width {
            Some((cursor_x, input_area.y))
        } else {
            None
        }
    }

    fn render_hints(&self, area: Rect, buf: &mut Buffer) {
        let normal_style = Style::default().fg(self.theme.text).bg(self.theme.root_bg);
        let highlight_style = Style::default()
            .fg(self.theme.cursor_fg)
            .bg(self.theme.cursor_bg);
        let root_bg = self.theme.root_bg;
        let cursor_bg = self.theme.cursor_bg;

        match &self.completion {
            CompletionSource::Error(err) => {
                Paragraph::new(err.as_str())
                    .style(Style::default().fg(Color::Red).bg(root_bg))
                    .wrap(Wrap { trim: false })
                    .render(area, buf);
            }
            CompletionSource::Items(items) => {
                render_completion_hints_buf(items, None, root_bg, area, buf, |_i, name| {
                    (name.to_string(), normal_style, highlight_style)
                });
            }
            CompletionSource::ColorItems(items) => {
                render_completion_hints_buf(items, None, root_bg, area, buf, |_i, name| {
                    let color = parse_color(name).unwrap_or(Color::White);
                    (
                        name.to_string(),
                        Style::default().fg(color).bg(root_bg),
                        Style::default().fg(color).bg(cursor_bg),
                    )
                });
            }
            CompletionSource::FileItems(items) => {
                render_completion_hints_buf(items, None, root_bg, area, buf, |_i, path| {
                    (file_display_name(path), normal_style, highlight_style)
                });
            }
            CompletionSource::CommandHelp(help) => {
                Paragraph::new(help.as_str())
                    .style(normal_style)
                    .wrap(Wrap { trim: false })
                    .render(area, buf);
            }
        }
    }
}

fn render_completion_hints_buf(
    completions: &[String],
    completion_index: Option<usize>,
    bg: Color,
    area: Rect,
    buf: &mut Buffer,
    span_fn: impl Fn(usize, &str) -> (String, Style, Style),
) {
    if completions.is_empty() {
        return;
    }
    let hint_spans: Vec<Span> = completions
        .iter()
        .enumerate()
        .flat_map(|(i, name)| {
            let (display, item_normal, item_highlight) = span_fn(i, name);
            let style = if completion_index == Some(i) {
                item_highlight
            } else {
                item_normal
            };
            vec![
                Span::styled(format!(" {} ", display), style),
                Span::raw(" "),
            ]
        })
        .collect();
    Paragraph::new(Line::from(hint_spans))
        .style(Style::default().bg(bg))
        .wrap(Wrap { trim: false })
        .render(area, buf);
}

impl<'a> Widget for CommandBar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(area);

        let command_line = Paragraph::new(format!(":{}", self.input_text))
            .style(
                Style::default()
                    .fg(self.theme.cursor_fg)
                    .bg(self.theme.cursor_bg),
            )
            .wrap(Wrap { trim: false });
        command_line.render(chunks[0], buf);

        self.render_hints(chunks[1], buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn test_command_bar_renders_items() {
        let theme = Theme::default();
        let bar = CommandBar {
            input_text: "filter",
            cursor_pos: 6,
            completion: CompletionSource::Items(vec!["filter".to_string(), "grep".to_string()]),
            theme: &theme,
        };
        let mut terminal = Terminal::new(TestBackend::new(80, 2)).unwrap();
        terminal.draw(|f| f.render_widget(bar, f.area())).unwrap();
    }

    #[test]
    fn test_command_bar_renders_error() {
        let theme = Theme::default();
        let bar = CommandBar {
            input_text: "bad-cmd",
            cursor_pos: 7,
            completion: CompletionSource::Error("unknown command".to_string()),
            theme: &theme,
        };
        let mut terminal = Terminal::new(TestBackend::new(80, 2)).unwrap();
        terminal.draw(|f| f.render_widget(bar, f.area())).unwrap();
    }

    #[test]
    fn test_command_bar_renders_help() {
        let theme = Theme::default();
        let bar = CommandBar {
            input_text: "filter",
            cursor_pos: 6,
            completion: CompletionSource::CommandHelp("  filter <pat> - add filter".to_string()),
            theme: &theme,
        };
        let mut terminal = Terminal::new(TestBackend::new(80, 2)).unwrap();
        terminal.draw(|f| f.render_widget(bar, f.area())).unwrap();
    }

    #[test]
    fn test_cursor_position_within_bounds() {
        let theme = Theme::default();
        let bar = CommandBar {
            input_text: "abc",
            cursor_pos: 3,
            completion: CompletionSource::Items(vec![]),
            theme: &theme,
        };
        let area = Rect::new(0, 10, 80, 1);
        let pos = bar.cursor_position(area);
        assert_eq!(pos, Some((4, 10)));
    }

    #[test]
    fn test_cursor_position_out_of_bounds() {
        let theme = Theme::default();
        let bar = CommandBar {
            input_text: "",
            cursor_pos: 100,
            completion: CompletionSource::Items(vec![]),
            theme: &theme,
        };
        let area = Rect::new(0, 0, 10, 1);
        assert_eq!(bar.cursor_position(area), None);
    }

    #[test]
    fn test_file_display_name_simple() {
        assert_eq!(file_display_name("/home/user/file.log"), "file.log");
    }

    #[test]
    fn test_file_display_name_directory() {
        assert_eq!(file_display_name("/home/user/logs/"), "logs/");
    }

    #[test]
    fn test_file_display_name_just_filename() {
        assert_eq!(file_display_name("file.txt"), "file.txt");
    }

    #[test]
    fn test_file_display_name_no_separator() {
        assert_eq!(file_display_name("nodirfile"), "nodirfile");
    }

    #[test]
    fn test_command_bar_renders_color_items() {
        let theme = Theme::default();
        let bar = CommandBar {
            input_text: "filter --fg ",
            cursor_pos: 12,
            completion: CompletionSource::ColorItems(vec!["red".to_string(), "blue".to_string()]),
            theme: &theme,
        };
        let mut terminal = Terminal::new(TestBackend::new(80, 4)).unwrap();
        terminal.draw(|f| f.render_widget(bar, f.area())).unwrap();
    }

    #[test]
    fn test_command_bar_renders_file_items() {
        let theme = Theme::default();
        let bar = CommandBar {
            input_text: "open /tmp/",
            cursor_pos: 10,
            completion: CompletionSource::FileItems(vec![
                "/tmp/file.log".to_string(),
                "/tmp/logs/".to_string(),
            ]),
            theme: &theme,
        };
        let mut terminal = Terminal::new(TestBackend::new(80, 4)).unwrap();
        terminal.draw(|f| f.render_widget(bar, f.area())).unwrap();
    }

    #[test]
    fn test_command_bar_renders_empty_items() {
        let theme = Theme::default();
        let bar = CommandBar {
            input_text: "unknowncmd",
            cursor_pos: 10,
            completion: CompletionSource::Items(vec![]),
            theme: &theme,
        };
        let mut terminal = Terminal::new(TestBackend::new(80, 4)).unwrap();
        terminal.draw(|f| f.render_widget(bar, f.area())).unwrap();
    }

    #[tokio::test]
    async fn test_resolve_completions_error() {
        use crate::db::Database;
        use crate::db::LogManager;
        use crate::ingestion::FileReader;
        use crate::ui::TabState;
        use std::sync::Arc;
        let db = Arc::new(Database::in_memory().await.unwrap());
        let fr = FileReader::from_bytes(b"line".to_vec());
        let lm = LogManager::new(db, None).await;
        let mut tab = TabState::new(fr, lm, "test".to_string());
        tab.interaction.command_error = Some("err".to_string());
        let result = resolve_completions(&mut tab, "filter", None);
        assert!(matches!(result, CompletionSource::Error(_)));
    }

    #[tokio::test]
    async fn test_resolve_completions_field_name() {
        use crate::db::Database;
        use crate::db::LogManager;
        use crate::ingestion::FileReader;
        use crate::ui::TabState;
        use std::sync::Arc;
        let db = Arc::new(Database::in_memory().await.unwrap());
        let data = br#"{"level":"INFO","msg":"hello"}"#.to_vec();
        let fr = FileReader::from_bytes(data);
        let lm = LogManager::new(db, None).await;
        let mut tab = TabState::new(fr, lm, "test".to_string());
        let result = resolve_completions(&mut tab, "@field:le", None);
        assert!(matches!(result, CompletionSource::Items(_)));
    }

    #[tokio::test]
    async fn test_resolve_completions_flag() {
        use crate::db::Database;
        use crate::db::LogManager;
        use crate::ingestion::FileReader;
        use crate::ui::TabState;
        use std::sync::Arc;
        let db = Arc::new(Database::in_memory().await.unwrap());
        let fr = FileReader::from_bytes(b"line".to_vec());
        let lm = LogManager::new(db, None).await;
        let mut tab = TabState::new(fr, lm, "test".to_string());
        let result = resolve_completions(&mut tab, "filter pattern --f", None);
        assert!(matches!(result, CompletionSource::Items(_)));
    }

    #[tokio::test]
    async fn test_resolve_completions_color() {
        use crate::db::Database;
        use crate::db::LogManager;
        use crate::ingestion::FileReader;
        use crate::ui::TabState;
        use std::sync::Arc;
        let db = Arc::new(Database::in_memory().await.unwrap());
        let fr = FileReader::from_bytes(b"line".to_vec());
        let lm = LogManager::new(db, None).await;
        let mut tab = TabState::new(fr, lm, "test".to_string());
        let result = resolve_completions(&mut tab, "filter pattern --fg red", None);
        assert!(matches!(result, CompletionSource::ColorItems(_)));
    }

    #[tokio::test]
    async fn test_resolve_completions_file_path() {
        use crate::db::Database;
        use crate::db::LogManager;
        use crate::ingestion::FileReader;
        use crate::ui::TabState;
        use std::sync::Arc;
        let db = Arc::new(Database::in_memory().await.unwrap());
        let fr = FileReader::from_bytes(b"line".to_vec());
        let lm = LogManager::new(db, None).await;
        let mut tab = TabState::new(fr, lm, "test".to_string());
        let result = resolve_completions(&mut tab, "open /tmp/", None);
        assert!(matches!(result, CompletionSource::FileItems(_)));
    }

    #[tokio::test]
    async fn test_resolve_completions_set_theme() {
        use crate::db::Database;
        use crate::db::LogManager;
        use crate::ingestion::FileReader;
        use crate::ui::TabState;
        use std::sync::Arc;
        let db = Arc::new(Database::in_memory().await.unwrap());
        let fr = FileReader::from_bytes(b"line".to_vec());
        let lm = LogManager::new(db, None).await;
        let mut tab = TabState::new(fr, lm, "test".to_string());
        let result = resolve_completions(&mut tab, "set-theme dar", None);
        assert!(matches!(result, CompletionSource::Items(_)));
    }

    #[tokio::test]
    async fn test_resolve_completions_hide_field() {
        use crate::db::Database;
        use crate::db::LogManager;
        use crate::ingestion::FileReader;
        use crate::ui::TabState;
        use std::sync::Arc;
        let db = Arc::new(Database::in_memory().await.unwrap());
        let data = br#"{"level":"INFO","msg":"hello"}"#.to_vec();
        let fr = FileReader::from_bytes(data);
        let lm = LogManager::new(db, None).await;
        let mut tab = TabState::new(fr, lm, "test".to_string());
        let result = resolve_completions(&mut tab, "hide-field le", None);
        assert!(matches!(result, CompletionSource::Items(_)));
    }

    #[tokio::test]
    async fn test_resolve_completions_show_field_with_hidden() {
        use crate::db::Database;
        use crate::db::LogManager;
        use crate::ingestion::FileReader;
        use crate::ui::TabState;
        use std::sync::Arc;
        let db = Arc::new(Database::in_memory().await.unwrap());
        let data = br#"{"level":"INFO","msg":"hello"}"#.to_vec();
        let fr = FileReader::from_bytes(data);
        let lm = LogManager::new(db, None).await;
        let mut tab = TabState::new(fr, lm, "test".to_string());
        tab.display.hidden_fields.insert("level".to_string());
        let result = resolve_completions(&mut tab, "show-field le", None);
        assert!(matches!(result, CompletionSource::Items(_)));
    }

    #[tokio::test]
    async fn test_resolve_completions_show_field_no_hidden() {
        use crate::db::Database;
        use crate::db::LogManager;
        use crate::ingestion::FileReader;
        use crate::ui::TabState;
        use std::sync::Arc;
        let db = Arc::new(Database::in_memory().await.unwrap());
        let data = br#"{"level":"INFO","msg":"hello"}"#.to_vec();
        let fr = FileReader::from_bytes(data);
        let lm = LogManager::new(db, None).await;
        let mut tab = TabState::new(fr, lm, "test".to_string());
        let result = resolve_completions(&mut tab, "show-field le", None);
        assert!(matches!(result, CompletionSource::Items(_)));
    }

    #[tokio::test]
    async fn test_resolve_completions_command_help() {
        use crate::db::Database;
        use crate::db::LogManager;
        use crate::ingestion::FileReader;
        use crate::ui::TabState;
        use std::sync::Arc;
        let db = Arc::new(Database::in_memory().await.unwrap());
        let fr = FileReader::from_bytes(b"line".to_vec());
        let lm = LogManager::new(db, None).await;
        let mut tab = TabState::new(fr, lm, "test".to_string());
        let result = resolve_completions(&mut tab, "filter", None);
        assert!(matches!(result, CompletionSource::CommandHelp(_)));
    }

    #[tokio::test]
    async fn test_resolve_completions_fallback_items() {
        use crate::db::Database;
        use crate::db::LogManager;
        use crate::ingestion::FileReader;
        use crate::ui::TabState;
        use std::sync::Arc;
        let db = Arc::new(Database::in_memory().await.unwrap());
        let fr = FileReader::from_bytes(b"line".to_vec());
        let lm = LogManager::new(db, None).await;
        let mut tab = TabState::new(fr, lm, "test".to_string());
        let result = resolve_completions(&mut tab, "fi", None);
        assert!(matches!(result, CompletionSource::Items(_)));
    }
}
