use ratatui::{
    prelude::*,
    style::Modifier,
    widgets::{Block, Borders, Padding, Paragraph},
};

use crate::config::Keybindings;
use crate::theme::Theme;

use super::popup_entry;

pub struct ConfirmRestoreModal<'a> {
    pub theme: &'a Theme,
    pub keybindings: &'a Keybindings,
}

impl<'a> Widget for ConfirmRestoreModal<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let modal_width = 44_u16;
        let modal_height = 5_u16;
        let x = area.x + (area.width.saturating_sub(modal_width)) / 2;
        let y = area.y + (area.height.saturating_sub(modal_height)) / 2;
        let modal_area = Rect::new(x, y, modal_width, modal_height);

        let kb = &self.keybindings;
        let key_style = Style::default()
            .fg(self.theme.text_highlight_fg)
            .add_modifier(Modifier::BOLD);
        let txt_style = Style::default().fg(self.theme.text);
        let br_style = Style::default().fg(self.theme.text);

        let mut spans: Vec<Span<'static>> = vec![Span::styled(" ", txt_style)];
        popup_entry(
            &mut spans,
            kb.confirm.yes.display(),
            "yes",
            key_style,
            txt_style,
            br_style,
        );
        popup_entry(
            &mut spans,
            kb.confirm.no.display(),
            "no",
            key_style,
            txt_style,
            br_style,
        );

        ratatui::widgets::Clear.render(modal_area, buf);
        Paragraph::new(Line::from(spans))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(self.theme.border_title))
                    .title(" Restore previous session? ")
                    .title_style(
                        Style::default()
                            .fg(self.theme.text_highlight_fg)
                            .add_modifier(Modifier::BOLD),
                    )
                    .title_alignment(Alignment::Center)
                    .padding(Padding::new(0, 0, 1, 0)),
            )
            .style(Style::default().bg(self.theme.root_bg))
            .render(modal_area, buf);
    }
}

pub struct ConfirmRestoreSessionModal<'a> {
    pub theme: &'a Theme,
    pub keybindings: &'a Keybindings,
    pub files: &'a [String],
}

impl<'a> Widget for ConfirmRestoreSessionModal<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let file_names: Vec<&str> = self
            .files
            .iter()
            .map(|f| {
                std::path::Path::new(f)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(f.as_str())
            })
            .collect();

        let modal_width = 50_u16;
        let modal_height = (file_names.len() as u16 + 6).min(area.height);
        let x = area.x + (area.width.saturating_sub(modal_width)) / 2;
        let y = area.y + (area.height.saturating_sub(modal_height)) / 2;
        let modal_area = Rect::new(x, y, modal_width, modal_height);

        ratatui::widgets::Clear.render(modal_area, buf);

        let mut lines: Vec<Line> = vec![Line::from(Span::styled(
            " Files:",
            Style::default().fg(self.theme.text),
        ))];
        for name in &file_names {
            lines.push(Line::from(Span::styled(
                format!("  \u{2022} {}", name),
                Style::default().fg(self.theme.text),
            )));
        }
        lines.push(Line::from(""));

        let kb = &self.keybindings.confirm;
        let key_style = Style::default()
            .fg(self.theme.text_highlight_fg)
            .add_modifier(Modifier::BOLD);
        let txt_style = Style::default().fg(self.theme.text);
        let br_style = Style::default().fg(self.theme.text);
        let mut yn_spans: Vec<Span<'static>> = vec![Span::styled(" ", txt_style)];
        popup_entry(
            &mut yn_spans,
            kb.yes.display(),
            "yes",
            key_style,
            txt_style,
            br_style,
        );
        popup_entry(
            &mut yn_spans,
            kb.no.display(),
            "no",
            key_style,
            txt_style,
            br_style,
        );
        lines.push(Line::from(yn_spans));

        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(self.theme.border_title))
                    .title(" Restore last session? ")
                    .title_style(
                        Style::default()
                            .fg(self.theme.text_highlight_fg)
                            .add_modifier(Modifier::BOLD),
                    )
                    .title_alignment(Alignment::Center)
                    .padding(Padding::new(0, 0, 1, 0)),
            )
            .style(Style::default().bg(self.theme.root_bg))
            .render(modal_area, buf);
    }
}

pub struct ConfirmOpenDirModal<'a> {
    pub theme: &'a Theme,
    pub keybindings: &'a Keybindings,
    pub files: &'a [String],
}

impl<'a> Widget for ConfirmOpenDirModal<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        const MAX_DISPLAY: usize = 10;
        let display_count = self.files.len().min(MAX_DISPLAY);
        let extra = self.files.len().saturating_sub(MAX_DISPLAY);
        let extra_line: u16 = if extra > 0 { 1 } else { 0 };
        let modal_width = 60_u16;
        let modal_height = (display_count as u16 + 4 + extra_line).min(area.height);
        let x = area.x + (area.width.saturating_sub(modal_width)) / 2;
        let y = area.y + (area.height.saturating_sub(modal_height)) / 2;
        let modal_area = Rect::new(x, y, modal_width, modal_height);

        ratatui::widgets::Clear.render(modal_area, buf);

        let txt_style = Style::default().fg(self.theme.text);
        let br_style = Style::default().fg(self.theme.text);
        let key_style = Style::default()
            .fg(self.theme.text_highlight_fg)
            .add_modifier(Modifier::BOLD);

        let mut lines_out: Vec<Line> = Vec::new();
        for path in self.files.iter().take(MAX_DISPLAY) {
            let name = std::path::Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(path.as_str());
            lines_out.push(Line::from(Span::styled(
                format!("  \u{2022} {}", name),
                txt_style,
            )));
        }
        if extra > 0 {
            lines_out.push(Line::from(Span::styled(
                format!("  \u{2026} and {} more", extra),
                br_style,
            )));
        }
        lines_out.push(Line::from(""));

        let kb = &self.keybindings.confirm;
        let mut yn_spans: Vec<Span<'static>> = vec![Span::styled(" ", txt_style)];
        popup_entry(
            &mut yn_spans,
            kb.yes.display(),
            "yes",
            key_style,
            txt_style,
            br_style,
        );
        popup_entry(
            &mut yn_spans,
            kb.no.display(),
            "no",
            key_style,
            txt_style,
            br_style,
        );
        lines_out.push(Line::from(yn_spans));

        let title = format!(
            " Open directory? ({} file{}) ",
            self.files.len(),
            if self.files.len() == 1 { "" } else { "s" }
        );
        Paragraph::new(lines_out)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(self.theme.border_title))
                    .title(title)
                    .title_style(
                        Style::default()
                            .fg(self.theme.text_highlight_fg)
                            .add_modifier(Modifier::BOLD),
                    )
                    .title_alignment(Alignment::Center)
                    .padding(Padding::new(0, 0, 1, 0)),
            )
            .style(Style::default().bg(self.theme.root_bg))
            .render(modal_area, buf);
    }
}

#[cfg(test)]
mod tests {
    use crate::config::Keybindings;
    use crate::db::LogManager;
    use crate::db::{Database, FileContext};
    use crate::ingestion::FileReader;
    use crate::mode::app_mode::{ConfirmRestoreMode, ConfirmRestoreSessionMode};
    use crate::theme::Theme;
    use crate::ui::App;
    use ratatui::{Terminal, backend::TestBackend};
    use std::sync::Arc;

    async fn make_app(lines: &[&str]) -> App {
        let data: Vec<u8> = lines.join("\n").into_bytes();
        let file_reader = FileReader::from_bytes(data);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        App::builder(
            log_manager,
            file_reader,
            Theme::default(),
            Arc::new(Keybindings::default()),
        )
        .build()
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
    async fn test_confirm_restore_session() {
        let mut app = make_app(&["line one", "line two"]).await;
        app.tabs[0].interaction.mode = Box::new(ConfirmRestoreSessionMode {
            files: std::sync::Arc::new(vec!["file1.log".to_string(), "file2.log".to_string()]),
        });
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[test]
    fn test_confirm_restore_modal_renders() {
        use super::{ConfirmRestoreModal, Theme};
        use crate::config::Keybindings;
        use ratatui::{Terminal, backend::TestBackend};
        let theme = Theme::default();
        let kb = Keybindings::default();
        let modal = ConfirmRestoreModal {
            theme: &theme,
            keybindings: &kb,
        };
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|f| f.render_widget(modal, f.area())).unwrap();
    }

    #[test]
    fn test_confirm_restore_session_modal_no_files() {
        use super::ConfirmRestoreSessionModal;
        use crate::config::Keybindings;
        use crate::theme::Theme;
        use ratatui::{Terminal, backend::TestBackend};
        let theme = Theme::default();
        let kb = Keybindings::default();
        let modal = ConfirmRestoreSessionModal {
            theme: &theme,
            keybindings: &kb,
            files: &[],
        };
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|f| f.render_widget(modal, f.area())).unwrap();
    }

    #[test]
    fn test_confirm_restore_session_modal_many_files() {
        use super::ConfirmRestoreSessionModal;
        use crate::config::Keybindings;
        use crate::theme::Theme;
        use ratatui::{Terminal, backend::TestBackend};
        let theme = Theme::default();
        let kb = Keybindings::default();
        let files: Vec<String> = (0..10)
            .map(|i| format!("/home/user/logs/file{i}.log"))
            .collect();
        let modal = ConfirmRestoreSessionModal {
            theme: &theme,
            keybindings: &kb,
            files: &files,
        };
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|f| f.render_widget(modal, f.area())).unwrap();
    }

    #[test]
    fn test_confirm_open_dir_modal_one_file() {
        use super::ConfirmOpenDirModal;
        use crate::config::Keybindings;
        use crate::theme::Theme;
        use ratatui::{Terminal, backend::TestBackend};
        let theme = Theme::default();
        let kb = Keybindings::default();
        let files = vec!["/tmp/app.log".to_string()];
        let modal = ConfirmOpenDirModal {
            theme: &theme,
            keybindings: &kb,
            files: &files,
        };
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|f| f.render_widget(modal, f.area())).unwrap();
    }

    #[test]
    fn test_confirm_open_dir_modal_many_files_truncated() {
        use super::ConfirmOpenDirModal;
        use crate::config::Keybindings;
        use crate::theme::Theme;
        use ratatui::{Terminal, backend::TestBackend};
        let theme = Theme::default();
        let kb = Keybindings::default();
        let files: Vec<String> = (0..15).map(|i| format!("/tmp/file{i}.log")).collect();
        let modal = ConfirmOpenDirModal {
            theme: &theme,
            keybindings: &kb,
            files: &files,
        };
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|f| f.render_widget(modal, f.area())).unwrap();
    }

    #[test]
    fn test_confirm_open_dir_modal_plural_title() {
        use super::ConfirmOpenDirModal;
        use crate::config::Keybindings;
        use crate::theme::Theme;
        use ratatui::{buffer::Buffer, prelude::Widget};
        let theme = Theme::default();
        let kb = Keybindings::default();
        let files = vec!["/tmp/a.log".to_string(), "/tmp/b.log".to_string()];
        let modal = ConfirmOpenDirModal {
            theme: &theme,
            keybindings: &kb,
            files: &files,
        };
        let area = ratatui::prelude::Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        modal.render(area, &mut buf);
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("2 files"));
    }

    #[test]
    fn test_confirm_open_dir_modal_singular_title() {
        use super::ConfirmOpenDirModal;
        use crate::config::Keybindings;
        use crate::theme::Theme;
        use ratatui::{buffer::Buffer, prelude::Widget};
        let theme = Theme::default();
        let kb = Keybindings::default();
        let files = vec!["/tmp/a.log".to_string()];
        let modal = ConfirmOpenDirModal {
            theme: &theme,
            keybindings: &kb,
            files: &files,
        };
        let area = ratatui::prelude::Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        modal.render(area, &mut buf);
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("1 file"));
    }

    #[test]
    fn test_confirm_restore_modal_renders_to_buffer() {
        use super::ConfirmRestoreModal;
        use ratatui::{buffer::Buffer, prelude::Widget};
        let theme = crate::theme::Theme::default();
        let kb = crate::config::Keybindings::default();
        let modal = ConfirmRestoreModal {
            theme: &theme,
            keybindings: &kb,
        };
        let area = ratatui::prelude::Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        modal.render(area, &mut buf);
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("Restore"));
    }

    #[test]
    fn test_confirm_restore_session_modal_renders_with_files() {
        use super::ConfirmRestoreSessionModal;
        use ratatui::{buffer::Buffer, prelude::Widget};
        let theme = crate::theme::Theme::default();
        let kb = crate::config::Keybindings::default();
        let files = vec![
            "/home/user/app.log".to_string(),
            "/home/user/server.log".to_string(),
        ];
        let modal = ConfirmRestoreSessionModal {
            theme: &theme,
            keybindings: &kb,
            files: &files,
        };
        let area = ratatui::prelude::Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        modal.render(area, &mut buf);
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("app.log"));
    }

    #[test]
    fn test_confirm_open_dir_modal_exactly_ten_files() {
        use super::ConfirmOpenDirModal;
        use ratatui::{buffer::Buffer, prelude::Widget};
        let theme = crate::theme::Theme::default();
        let kb = crate::config::Keybindings::default();
        let files: Vec<String> = (0..10).map(|i| format!("/tmp/file{i}.log")).collect();
        let modal = ConfirmOpenDirModal {
            theme: &theme,
            keybindings: &kb,
            files: &files,
        };
        let area = ratatui::prelude::Rect::new(0, 0, 80, 30);
        let mut buf = Buffer::empty(area);
        modal.render(area, &mut buf);
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("10 files"));
    }

    #[tokio::test]
    async fn test_confirm_open_dir_mode_render() {
        use crate::mode::app_mode::ConfirmOpenDirMode;
        let mut app = make_app(&["line"]).await;
        app.tabs[0].interaction.mode = Box::new(ConfirmOpenDirMode {
            dir: "/tmp/logs".to_string(),
            files: std::sync::Arc::new(vec!["a.log".to_string(), "b.log".to_string()]),
        });
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }
}
