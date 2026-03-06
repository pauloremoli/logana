use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::{
    config::Keybindings,
    mode::{
        app_mode::{Mode, ModeRenderState, status_entry},
        normal_mode::NormalMode,
    },
    theme::Theme,
    types::Comment,
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
///   Enter         → split line (insert newline, configurable)
///   Backspace     → delete char / merge with previous line
///   Left/Right    → move cursor within / across lines
///   Up/Down       → move cursor between rows
///   Ctrl+Enter    → save comment and return to NormalMode (configurable)
///   Esc           → cancel, discard text, return to NormalMode
#[derive(Debug)]
pub struct CommentMode {
    pub lines: Vec<String>,
    pub cursor_row: usize,
    pub cursor_col: usize,
    /// The actual file-line indices this comment will be attached to.
    pub line_indices: Vec<usize>,
    /// When editing an existing comment, holds its index in LogManager::comments.
    pub editing_index: Option<usize>,
}

impl CommentMode {
    pub fn new(line_indices: Vec<usize>) -> Self {
        CommentMode {
            lines: vec![String::new()],
            cursor_row: 0,
            cursor_col: 0,
            line_indices,
            editing_index: None,
        }
    }

    /// Open the editor pre-filled with an existing comment's text.
    pub fn edit(index: usize, text: String, line_indices: Vec<usize>) -> Self {
        let lines: Vec<String> = if text.is_empty() {
            vec![String::new()]
        } else {
            text.split('\n').map(String::from).collect()
        };
        let cursor_row = lines.len() - 1;
        let cursor_col = lines.last().map_or(0, |l| l.len());
        CommentMode {
            lines,
            cursor_row,
            cursor_col,
            line_indices,
            editing_index: Some(index),
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
        let comment_kb = tab.keybindings.comment.clone();

        // Insert newline: split current line at cursor (configurable, default Enter)
        // Checked first so it runs before save — Enter = newline is the common case.
        if comment_kb.newline.matches(key, modifiers) {
            let rest = self.lines[self.cursor_row][self.cursor_col..].to_string();
            self.lines[self.cursor_row].truncate(self.cursor_col);
            self.cursor_row += 1;
            self.lines.insert(self.cursor_row, rest);
            self.cursor_col = 0;
            return (self, KeyResult::Handled);
        }

        // Save (configurable, default Ctrl+S)
        if comment_kb.save.matches(key, modifiers) {
            let text = self.lines.join("\n");
            if let Some(idx) = self.editing_index {
                let mut comments = tab.log_manager.get_comments().to_vec();
                if idx < comments.len() {
                    comments[idx] = Comment {
                        text,
                        line_indices: self.line_indices,
                    };
                    tab.log_manager.set_comments(comments);
                }
            } else {
                tab.log_manager.add_comment(text, self.line_indices);
            }
            return (Box::new(NormalMode::default()), KeyResult::Handled);
        }

        // Delete comment (only when editing, default Ctrl+D)
        if comment_kb.delete.matches(key, modifiers) {
            if let Some(idx) = self.editing_index {
                tab.log_manager.remove_comment(idx);
            }
            return (Box::new(NormalMode::default()), KeyResult::Handled);
        }

        // Cancel (configurable, default Esc)
        if comment_kb.cancel.matches(key, modifiers) {
            return (Box::new(NormalMode::default()), KeyResult::Handled);
        }

        match key {
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

    fn mode_bar_content(&self, kb: &Keybindings, theme: &Theme) -> Line<'static> {
        let label = if self.editing_index.is_some() {
            "[COMMENT EDIT]  "
        } else {
            "[COMMENT]  "
        };
        let mut spans: Vec<Span<'static>> = vec![
            Span::styled(
                label,
                Style::default()
                    .fg(theme.text_highlight_fg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("type text  ", Style::default().fg(theme.text)),
        ];
        status_entry(&mut spans, kb.comment.newline.display(), "newline", theme);
        status_entry(&mut spans, kb.comment.save.display(), "save", theme);
        if self.editing_index.is_some() {
            status_entry(&mut spans, kb.comment.delete.display(), "delete", theme);
        }
        status_entry(&mut spans, kb.comment.cancel.display(), "cancel", theme);
        Line::from(spans)
    }

    fn render_state(&self) -> ModeRenderState {
        ModeRenderState::Comment {
            lines: self.lines.clone(),
            cursor_row: self.cursor_row,
            cursor_col: self.cursor_col,
            line_count: self.line_indices.len(),
        }
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
        match mode2.render_state() {
            ModeRenderState::Comment {
                lines, cursor_col, ..
            } => {
                assert_eq!(lines[0], "h");
                assert_eq!(cursor_col, 1);
            }
            other => panic!("expected Comment, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_enter_splits_line() {
        let mut tab = make_tab().await;
        let mut mode = CommentMode::new(vec![0]);
        mode.lines[0] = "hello world".to_string();
        mode.cursor_col = 5;
        let (mode2, _) = press(mode, &mut tab, KeyCode::Enter, KeyModifiers::NONE).await;
        match mode2.render_state() {
            ModeRenderState::Comment {
                lines,
                cursor_row,
                cursor_col,
                ..
            } => {
                assert_eq!(lines[0], "hello");
                assert_eq!(lines[1], " world");
                assert_eq!(cursor_row, 1);
                assert_eq!(cursor_col, 0);
            }
            other => panic!("expected Comment, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_backspace_removes_char() {
        let mut tab = make_tab().await;
        let mut mode = CommentMode::new(vec![0]);
        mode.lines[0] = "hi".to_string();
        mode.cursor_col = 2;
        let (mode2, _) = press(mode, &mut tab, KeyCode::Backspace, KeyModifiers::NONE).await;
        match mode2.render_state() {
            ModeRenderState::Comment {
                lines, cursor_col, ..
            } => {
                assert_eq!(lines[0], "h");
                assert_eq!(cursor_col, 1);
            }
            other => panic!("expected Comment, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_backspace_merges_lines() {
        let mut tab = make_tab().await;
        let mut mode = CommentMode::new(vec![0]);
        mode.lines = vec!["first".to_string(), "second".to_string()];
        mode.cursor_row = 1;
        mode.cursor_col = 0;
        let (mode2, _) = press(mode, &mut tab, KeyCode::Backspace, KeyModifiers::NONE).await;
        match mode2.render_state() {
            ModeRenderState::Comment {
                lines,
                cursor_row,
                cursor_col,
                ..
            } => {
                assert_eq!(lines.len(), 1);
                assert_eq!(lines[0], "firstsecond");
                assert_eq!(cursor_row, 0);
                assert_eq!(cursor_col, 5);
            }
            other => panic!("expected Comment, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_ctrl_s_saves_comment() {
        let mut tab = make_tab().await;
        let mut mode = CommentMode::new(vec![0, 1, 2]);
        mode.lines = vec!["line one".to_string(), "line two".to_string()];
        let (mode2, result) = press(mode, &mut tab, KeyCode::Enter, KeyModifiers::CONTROL).await;
        assert!(matches!(result, KeyResult::Handled));
        // returned to NormalMode
        assert!(!matches!(
            mode2.render_state(),
            ModeRenderState::Comment { .. }
        ));
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
        assert!(!matches!(
            mode2.render_state(),
            ModeRenderState::Comment { .. }
        ));
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
        match mode2.render_state() {
            ModeRenderState::Comment {
                cursor_row,
                cursor_col,
                ..
            } => {
                assert_eq!(cursor_row, 0);
                assert_eq!(cursor_col, 2); // end of "ab"
            }
            other => panic!("expected Comment, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_right_wraps_to_next_line() {
        let mut tab = make_tab().await;
        let mut mode = CommentMode::new(vec![0]);
        mode.lines = vec!["ab".to_string(), "cd".to_string()];
        mode.cursor_row = 0;
        mode.cursor_col = 2; // end of "ab"
        let (mode2, _) = press(mode, &mut tab, KeyCode::Right, KeyModifiers::NONE).await;
        match mode2.render_state() {
            ModeRenderState::Comment {
                cursor_row,
                cursor_col,
                ..
            } => {
                assert_eq!(cursor_row, 1);
                assert_eq!(cursor_col, 0);
            }
            other => panic!("expected Comment, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_up_down_navigation() {
        let mut tab = make_tab().await;
        let mut mode = CommentMode::new(vec![0]);
        mode.lines = vec!["hello".to_string(), "hi".to_string()];
        mode.cursor_row = 0;
        mode.cursor_col = 5;
        let (mode2, _) = press(mode, &mut tab, KeyCode::Down, KeyModifiers::NONE).await;
        match mode2.render_state() {
            ModeRenderState::Comment {
                cursor_row,
                cursor_col,
                ..
            } => {
                assert_eq!(cursor_row, 1);
                assert_eq!(cursor_col, 2); // clamped to len("hi")=2
            }
            other => panic!("expected Comment, got {:?}", other),
        }

        // Re-create for Up test
        let mut mode3 = CommentMode::new(vec![0]);
        mode3.lines = vec!["hello".to_string(), "hi".to_string()];
        mode3.cursor_row = 1;
        mode3.cursor_col = 2;
        let (mode4, _) = press(mode3, &mut tab, KeyCode::Up, KeyModifiers::NONE).await;
        match mode4.render_state() {
            ModeRenderState::Comment {
                cursor_row,
                cursor_col,
                ..
            } => {
                assert_eq!(cursor_row, 0);
                assert_eq!(cursor_col, 2);
            }
            other => panic!("expected Comment, got {:?}", other),
        }
    }

    #[test]
    fn test_render_state_returns_line_count() {
        let mode = CommentMode::new(vec![5, 6, 7, 8]);
        match mode.render_state() {
            ModeRenderState::Comment { line_count, .. } => {
                assert_eq!(line_count, 4);
            }
            other => panic!("expected Comment, got {:?}", other),
        }
    }

    #[test]
    fn test_mode_bar_content_contains_comment() {
        let mode = CommentMode::new(vec![0]);
        assert!(matches!(
            mode.render_state(),
            ModeRenderState::Comment { .. }
        ));
    }

    // ── Edit mode tests ─────────────────────────────────────────────────

    #[test]
    fn test_edit_constructor_prefills_text() {
        let mode = CommentMode::edit(0, "hello\nworld".to_string(), vec![1, 2]);
        assert_eq!(mode.lines, vec!["hello", "world"]);
        assert_eq!(mode.cursor_row, 1);
        assert_eq!(mode.cursor_col, 5);
        assert_eq!(mode.editing_index, Some(0));
        assert_eq!(mode.line_indices, vec![1, 2]);
    }

    #[test]
    fn test_edit_constructor_empty_text() {
        let mode = CommentMode::edit(3, String::new(), vec![0]);
        assert_eq!(mode.lines, vec![""]);
        assert_eq!(mode.cursor_row, 0);
        assert_eq!(mode.cursor_col, 0);
        assert_eq!(mode.editing_index, Some(3));
    }

    #[tokio::test]
    async fn test_save_in_edit_mode_replaces_comment() {
        let mut tab = make_tab().await;
        tab.log_manager.add_comment("original".into(), vec![0, 1]);
        tab.log_manager.add_comment("other".into(), vec![2]);

        let mut mode = CommentMode::edit(0, "original".to_string(), vec![0, 1]);
        mode.lines = vec!["updated text".to_string()];
        let (mode2, result) = press(mode, &mut tab, KeyCode::Enter, KeyModifiers::CONTROL).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(!matches!(
            mode2.render_state(),
            ModeRenderState::Comment { .. }
        ));

        let comments = tab.log_manager.get_comments();
        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0].text, "updated text");
        assert_eq!(comments[1].text, "other");
    }

    #[tokio::test]
    async fn test_ctrl_d_deletes_comment_in_edit_mode() {
        let mut tab = make_tab().await;
        tab.log_manager.add_comment("to delete".into(), vec![0]);
        tab.log_manager.add_comment("keep".into(), vec![1]);

        let mode = CommentMode::edit(0, "to delete".to_string(), vec![0]);
        let (mode2, result) =
            press(mode, &mut tab, KeyCode::Char('d'), KeyModifiers::CONTROL).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(!matches!(
            mode2.render_state(),
            ModeRenderState::Comment { .. }
        ));

        let comments = tab.log_manager.get_comments();
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].text, "keep");
    }

    #[tokio::test]
    async fn test_ctrl_d_in_new_mode_returns_to_normal() {
        let mut tab = make_tab().await;
        let mode = CommentMode::new(vec![0]);
        let (mode2, result) =
            press(mode, &mut tab, KeyCode::Char('d'), KeyModifiers::CONTROL).await;
        assert!(matches!(result, KeyResult::Handled));
        // Returns to normal mode even when not editing
        assert!(!matches!(
            mode2.render_state(),
            ModeRenderState::Comment { .. }
        ));
        // No comments were deleted (none existed)
        assert!(tab.log_manager.get_comments().is_empty());
    }

    #[test]
    fn test_mode_bar_content_edit_mode_contains_delete() {
        let mode = CommentMode::edit(0, "text".to_string(), vec![0]);
        let content = mode.mode_bar_content(&Keybindings::default(), &Theme::default());
        let text: String = content.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("delete"));
    }

    #[test]
    fn test_mode_bar_content_new_mode_no_delete() {
        let mode = CommentMode::new(vec![0]);
        let content = mode.mode_bar_content(&Keybindings::default(), &Theme::default());
        let text: String = content.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(!text.contains("delete"));
    }
}
