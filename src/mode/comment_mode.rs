use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::{
    config::Keybindings,
    mode::{
        app_mode::{Mode, status_entry},
        normal_mode::NormalMode,
    },
    theme::Theme,
    ui::{KeyResult, TabState},
};

/// Multi-line comment editor mode.
///
/// Opened from `VisualLineMode` when the user presses `c` after selecting
/// a range of lines.  The comment text is stored line-by-line so the
/// cursor can be positioned precisely.
///
/// Keys:
///   Char          → insert at cursor
///   Enter         → split line (insert newline)
///   Backspace     → delete char / merge with previous line
///   Left/Right    → move cursor within / across lines
///   Up/Down       → move cursor between rows
///   Ctrl+S        → save comment and return to NormalMode (configurable)
///   Esc           → cancel, discard text, return to NormalMode
#[derive(Debug)]
pub struct CommentMode {
    pub lines: Vec<String>,
    pub cursor_row: usize,
    pub cursor_col: usize,
    /// The actual file-line indices this comment will be attached to.
    pub line_indices: Vec<usize>,
}

impl CommentMode {
    pub fn new(line_indices: Vec<usize>) -> Self {
        CommentMode {
            lines: vec![String::new()],
            cursor_row: 0,
            cursor_col: 0,
            line_indices,
        }
    }
}

#[async_trait]
impl Mode for CommentMode {
    async fn handle_key(
        mut self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        let ctrl = modifiers.contains(KeyModifiers::CONTROL);
        let save_kb = tab.keybindings.comment.save.clone();

        // Save (configurable, default Ctrl+S)
        if save_kb.matches(key, modifiers) {
            let text = self.lines.join("\n");
            tab.log_manager.add_comment(text, self.line_indices);
            return (Box::new(NormalMode), KeyResult::Handled);
        }

        match key {
            // Cancel
            KeyCode::Esc => {
                return (Box::new(NormalMode), KeyResult::Handled);
            }
            // Insert newline: split current line at cursor
            KeyCode::Enter => {
                let rest = self.lines[self.cursor_row][self.cursor_col..].to_string();
                self.lines[self.cursor_row].truncate(self.cursor_col);
                self.cursor_row += 1;
                self.lines.insert(self.cursor_row, rest);
                self.cursor_col = 0;
            }
            // Delete / merge with previous line
            KeyCode::Backspace => {
                if self.cursor_col > 0 {
                    self.cursor_col -= 1;
                    self.lines[self.cursor_row].remove(self.cursor_col);
                } else if self.cursor_row > 0 {
                    let current = self.lines.remove(self.cursor_row);
                    self.cursor_row -= 1;
                    self.cursor_col = self.lines[self.cursor_row].len();
                    self.lines[self.cursor_row].push_str(&current);
                }
            }
            // Insert character (ignore Ctrl combos)
            KeyCode::Char(c) if !ctrl => {
                self.lines[self.cursor_row].insert(self.cursor_col, c);
                self.cursor_col += 1;
            }
            // Cursor left — wrap to end of previous line
            KeyCode::Left => {
                if self.cursor_col > 0 {
                    self.cursor_col -= 1;
                } else if self.cursor_row > 0 {
                    self.cursor_row -= 1;
                    self.cursor_col = self.lines[self.cursor_row].len();
                }
            }
            // Cursor right — wrap to start of next line
            KeyCode::Right => {
                let line_len = self.lines[self.cursor_row].len();
                if self.cursor_col < line_len {
                    self.cursor_col += 1;
                } else if self.cursor_row + 1 < self.lines.len() {
                    self.cursor_row += 1;
                    self.cursor_col = 0;
                }
            }
            // Cursor up — clamp col to new line length
            KeyCode::Up => {
                if self.cursor_row > 0 {
                    self.cursor_row -= 1;
                    self.cursor_col = self.cursor_col.min(self.lines[self.cursor_row].len());
                }
            }
            // Cursor down — clamp col to new line length
            KeyCode::Down => {
                if self.cursor_row + 1 < self.lines.len() {
                    self.cursor_row += 1;
                    self.cursor_col = self.cursor_col.min(self.lines[self.cursor_row].len());
                }
            }
            _ => {}
        }

        (self, KeyResult::Handled)
    }

    fn status_line(&self) -> &str {
        "[COMMENT] Type comment text | [Ctrl+S] Save | [Esc] Cancel"
    }

    fn dynamic_status_line(&self, kb: &Keybindings, theme: &Theme) -> Line<'static> {
        let mut spans: Vec<Span<'static>> = vec![
            Span::styled(
                "[COMMENT]  ",
                Style::default()
                    .fg(theme.text_highlight)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("type text  ", Style::default().fg(theme.text)),
        ];
        status_entry(&mut spans, kb.comment.save.display(), "save", theme);
        status_entry(&mut spans, "Esc".to_string(), "cancel", theme);
        Line::from(spans)
    }

    fn comment_popup(&self) -> Option<(Vec<String>, usize, usize, usize)> {
        Some((
            self.lines.clone(),
            self.cursor_row,
            self.cursor_col,
            self.line_indices.len(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::file_reader::FileReader;
    use crate::log_manager::LogManager;
    use crate::ui::TabState;
    use std::sync::Arc;

    async fn make_tab() -> TabState {
        let file_reader = FileReader::from_bytes(b"a\nb\nc\n".to_vec());
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        TabState::new(file_reader, log_manager, "test".to_string())
    }

    async fn press(
        mode: CommentMode,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        Box::new(mode).handle_key(tab, key, modifiers).await
    }

    #[tokio::test]
    async fn test_char_inserts_at_cursor() {
        let mut tab = make_tab().await;
        let mode = CommentMode::new(vec![0, 1]);
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('h'), KeyModifiers::NONE).await;
        let (lines, _, col, _) = mode2.comment_popup().unwrap();
        assert_eq!(lines[0], "h");
        assert_eq!(col, 1);
    }

    #[tokio::test]
    async fn test_enter_splits_line() {
        let mut tab = make_tab().await;
        let mut mode = CommentMode::new(vec![0]);
        mode.lines[0] = "hello world".to_string();
        mode.cursor_col = 5;
        let (mode2, _) = press(mode, &mut tab, KeyCode::Enter, KeyModifiers::NONE).await;
        let (lines, row, col, _) = mode2.comment_popup().unwrap();
        assert_eq!(lines[0], "hello");
        assert_eq!(lines[1], " world");
        assert_eq!(row, 1);
        assert_eq!(col, 0);
    }

    #[tokio::test]
    async fn test_backspace_removes_char() {
        let mut tab = make_tab().await;
        let mut mode = CommentMode::new(vec![0]);
        mode.lines[0] = "hi".to_string();
        mode.cursor_col = 2;
        let (mode2, _) = press(mode, &mut tab, KeyCode::Backspace, KeyModifiers::NONE).await;
        let (lines, _, col, _) = mode2.comment_popup().unwrap();
        assert_eq!(lines[0], "h");
        assert_eq!(col, 1);
    }

    #[tokio::test]
    async fn test_backspace_merges_lines() {
        let mut tab = make_tab().await;
        let mut mode = CommentMode::new(vec![0]);
        mode.lines = vec!["first".to_string(), "second".to_string()];
        mode.cursor_row = 1;
        mode.cursor_col = 0;
        let (mode2, _) = press(mode, &mut tab, KeyCode::Backspace, KeyModifiers::NONE).await;
        let (lines, row, col, _) = mode2.comment_popup().unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], "firstsecond");
        assert_eq!(row, 0);
        assert_eq!(col, 5);
    }

    #[tokio::test]
    async fn test_ctrl_s_saves_comment() {
        let mut tab = make_tab().await;
        let mut mode = CommentMode::new(vec![0, 1, 2]);
        mode.lines = vec!["line one".to_string(), "line two".to_string()];
        let (mode2, result) =
            press(mode, &mut tab, KeyCode::Char('s'), KeyModifiers::CONTROL).await;
        assert!(matches!(result, KeyResult::Handled));
        // returned to NormalMode
        assert!(mode2.comment_popup().is_none());
        // comment stored
        let comments = tab.log_manager.get_comments();
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].text, "line one\nline two");
        assert_eq!(comments[0].line_indices, vec![0, 1, 2]);
    }

    #[tokio::test]
    async fn test_esc_cancels_without_saving() {
        let mut tab = make_tab().await;
        let mut mode = CommentMode::new(vec![0]);
        mode.lines[0] = "some text".to_string();
        let (mode2, result) = press(mode, &mut tab, KeyCode::Esc, KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(mode2.comment_popup().is_none());
        assert!(tab.log_manager.get_comments().is_empty());
    }

    #[tokio::test]
    async fn test_left_wraps_to_previous_line() {
        let mut tab = make_tab().await;
        let mut mode = CommentMode::new(vec![0]);
        mode.lines = vec!["ab".to_string(), "cd".to_string()];
        mode.cursor_row = 1;
        mode.cursor_col = 0;
        let (mode2, _) = press(mode, &mut tab, KeyCode::Left, KeyModifiers::NONE).await;
        let (_, row, col, _) = mode2.comment_popup().unwrap();
        assert_eq!(row, 0);
        assert_eq!(col, 2); // end of "ab"
    }

    #[tokio::test]
    async fn test_right_wraps_to_next_line() {
        let mut tab = make_tab().await;
        let mut mode = CommentMode::new(vec![0]);
        mode.lines = vec!["ab".to_string(), "cd".to_string()];
        mode.cursor_row = 0;
        mode.cursor_col = 2; // end of "ab"
        let (mode2, _) = press(mode, &mut tab, KeyCode::Right, KeyModifiers::NONE).await;
        let (_, row, col, _) = mode2.comment_popup().unwrap();
        assert_eq!(row, 1);
        assert_eq!(col, 0);
    }

    #[tokio::test]
    async fn test_up_down_navigation() {
        let mut tab = make_tab().await;
        let mut mode = CommentMode::new(vec![0]);
        mode.lines = vec!["hello".to_string(), "hi".to_string()];
        mode.cursor_row = 0;
        mode.cursor_col = 5;
        let (mode2, _) = press(mode, &mut tab, KeyCode::Down, KeyModifiers::NONE).await;
        let (_, row, col, _) = mode2.comment_popup().unwrap();
        assert_eq!(row, 1);
        assert_eq!(col, 2); // clamped to len("hi")=2

        // Re-create for Up test
        let mut mode3 = CommentMode::new(vec![0]);
        mode3.lines = vec!["hello".to_string(), "hi".to_string()];
        mode3.cursor_row = 1;
        mode3.cursor_col = 2;
        let (mode4, _) = press(mode3, &mut tab, KeyCode::Up, KeyModifiers::NONE).await;
        let (_, row2, col2, _) = mode4.comment_popup().unwrap();
        assert_eq!(row2, 0);
        assert_eq!(col2, 2);
    }

    #[test]
    fn test_comment_popup_returns_line_count() {
        let mode = CommentMode::new(vec![5, 6, 7, 8]);
        let (_, _, _, line_count) = mode.comment_popup().unwrap();
        assert_eq!(line_count, 4);
    }

    #[test]
    fn test_status_line_contains_comment() {
        let mode = CommentMode::new(vec![0]);
        assert!(mode.status_line().contains("[COMMENT]"));
    }
}
