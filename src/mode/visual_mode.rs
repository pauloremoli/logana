use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyModifiers};

use crate::{
    mode::{app_mode::Mode, comment_mode::CommentMode, normal_mode::NormalMode},
    ui::{KeyResult, TabState},
};

/// Visual line selection mode, entered by pressing `V` in NormalMode.
///
/// The line under the cursor at the time `V` is pressed becomes the anchor.
/// Moving up/down with j/k (or arrow keys) extends/shrinks the selection.
/// Pressing `c` opens `CommentMode` for the selected range.
/// `Esc` cancels and returns to NormalMode.
#[derive(Debug)]
pub struct VisualLineMode {
    /// Index into `visible_indices` that was the cursor when `V` was pressed.
    pub anchor: usize,
}

#[async_trait]
impl Mode for VisualLineMode {
    async fn handle_key(
        self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        _modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        match key {
            KeyCode::Char('j') | KeyCode::Down => {
                tab.scroll_offset = tab.scroll_offset.saturating_add(1);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                tab.scroll_offset = tab.scroll_offset.saturating_sub(1);
            }
            // Comment the selected range
            KeyCode::Char('c') => {
                if tab.visible_indices.is_empty() {
                    return (Box::new(NormalMode), KeyResult::Handled);
                }
                let max_idx = tab.visible_indices.len() - 1;
                let lo = self.anchor.min(tab.scroll_offset).min(max_idx);
                let hi = self.anchor.max(tab.scroll_offset).min(max_idx);
                let line_indices: Vec<usize> = tab.visible_indices[lo..=hi].to_vec();
                if !line_indices.is_empty() {
                    return (Box::new(CommentMode::new(line_indices)), KeyResult::Handled);
                }
                return (Box::new(NormalMode), KeyResult::Handled);
            }
            KeyCode::Esc => {
                return (Box::new(NormalMode), KeyResult::Handled);
            }
            _ => {}
        }
        (self, KeyResult::Handled)
    }

    fn status_line(&self) -> &str {
        "[VISUAL] j/k to extend selection | [c] comment selection | [Esc] cancel"
    }

    fn visual_selection_anchor(&self) -> Option<usize> {
        Some(self.anchor)
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

    async fn make_tab(lines: &[&str]) -> TabState {
        let data = lines.join("\n").into_bytes();
        let file_reader = FileReader::from_bytes(data);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        TabState::new(file_reader, log_manager, "test".to_string())
    }

    async fn press(
        mode: VisualLineMode,
        tab: &mut TabState,
        key: KeyCode,
    ) -> (Box<dyn Mode>, KeyResult) {
        Box::new(mode)
            .handle_key(tab, key, KeyModifiers::NONE)
            .await
    }

    #[tokio::test]
    async fn test_j_moves_cursor_down() {
        let mut tab = make_tab(&["a", "b", "c"]).await;
        tab.scroll_offset = 0;
        let mode = VisualLineMode { anchor: 0 };
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('j')).await;
        assert_eq!(tab.scroll_offset, 1);
        assert!(mode2.visual_selection_anchor().is_some());
    }

    #[tokio::test]
    async fn test_k_moves_cursor_up() {
        let mut tab = make_tab(&["a", "b", "c"]).await;
        tab.scroll_offset = 2;
        let mode = VisualLineMode { anchor: 2 };
        let (_, _) = press(mode, &mut tab, KeyCode::Char('k')).await;
        assert_eq!(tab.scroll_offset, 1);
    }

    #[tokio::test]
    async fn test_k_saturates_at_zero() {
        let mut tab = make_tab(&["a"]).await;
        tab.scroll_offset = 0;
        let mode = VisualLineMode { anchor: 0 };
        let _ = press(mode, &mut tab, KeyCode::Char('k')).await;
        assert_eq!(tab.scroll_offset, 0);
    }

    #[tokio::test]
    async fn test_esc_returns_normal_mode() {
        let mut tab = make_tab(&["a"]).await;
        let mode = VisualLineMode { anchor: 0 };
        let (mode2, result) = press(mode, &mut tab, KeyCode::Esc).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(mode2.visual_selection_anchor().is_none());
        assert!(mode2.status_line().contains("[NORMAL]"));
    }

    #[tokio::test]
    async fn test_c_opens_comment_mode_with_selected_lines() {
        let mut tab = make_tab(&["a", "b", "c", "d"]).await;
        tab.scroll_offset = 3; // cursor at line index 3
        let mode = VisualLineMode { anchor: 1 }; // anchor at visible index 1
        let (mode2, result) = press(mode, &mut tab, KeyCode::Char('c')).await;
        assert!(matches!(result, KeyResult::Handled));
        // Should be in comment mode
        let popup = mode2.comment_popup();
        assert!(popup.is_some());
        let (_, _, _, count) = popup.unwrap();
        assert_eq!(count, 3); // visible indices 1,2,3 → 3 lines
    }

    #[tokio::test]
    async fn test_c_with_anchor_above_cursor() {
        let mut tab = make_tab(&["a", "b", "c"]).await;
        tab.scroll_offset = 0;
        let mode = VisualLineMode { anchor: 2 }; // anchor below cursor
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('c')).await;
        let popup = mode2.comment_popup();
        assert!(popup.is_some());
        let (_, _, _, count) = popup.unwrap();
        assert_eq!(count, 3); // lines 0,1,2
    }

    #[tokio::test]
    async fn test_visual_selection_anchor_returns_anchor() {
        let mode = VisualLineMode { anchor: 5 };
        assert_eq!(mode.visual_selection_anchor(), Some(5));
    }

    #[test]
    fn test_status_line_contains_visual() {
        let mode = VisualLineMode { anchor: 0 };
        assert!(mode.status_line().contains("[VISUAL]"));
    }
}
