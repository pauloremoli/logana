use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::{
    config::Keybindings,
    mode::app_mode::{Mode, ModeRenderState, status_entry},
    mode::command_mode::CommandMode,
    mode::comment_mode::CommentMode,
    mode::normal_mode::NormalMode,
    mode::search_mode::SearchMode,
    theme::Theme,
    ui::{KeyResult, TabState, field_layout::apply_field_layout},
};

/// Visual line selection mode, entered by pressing `V` in NormalMode.
///
/// The line under the cursor at the time `V` is pressed becomes the anchor.
/// Moving up/down with j/k (or arrow keys) extends/shrinks the selection.
/// Pressing `c` opens `CommentMode` for the selected range.
/// Pressing `y` yanks (copies) selected lines to the system clipboard.
/// `Esc` cancels and returns to NormalMode.
#[derive(Debug)]
pub struct VisualLineMode {
    /// Index into `visible_indices` that was the cursor when `V` was pressed.
    pub anchor: usize,
    /// Vim-style count prefix for multi-line motions.
    pub count: Option<usize>,
}

#[async_trait]
impl Mode for VisualLineMode {
    async fn handle_key(
        mut self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        let kb = &tab.keybindings;

        // ── Digit accumulation for count prefix ─────────────────────────
        if let KeyCode::Char(c @ '1'..='9') = key
            && (modifiers.is_empty() || modifiers == KeyModifiers::SHIFT)
        {
            let digit = (c as u32 - '0' as u32) as usize;
            let n = self
                .count
                .unwrap_or(0)
                .saturating_mul(10)
                .saturating_add(digit);
            self.count = Some(n.min(999_999));
            return (self, KeyResult::Handled);
        }
        if let KeyCode::Char('0') = key
            && self.count.is_some()
            && (modifiers.is_empty() || modifiers == KeyModifiers::SHIFT)
        {
            self.count = Some(self.count.unwrap().saturating_mul(10).min(999_999));
            return (self, KeyResult::Handled);
        }

        if kb.navigation.scroll_down.matches(key, modifiers) {
            let count = self.count.take().unwrap_or(1);
            tab.scroll_offset = tab.scroll_offset.saturating_add(count);
            tab.g_key_pressed = false;
        } else if kb.navigation.scroll_up.matches(key, modifiers) {
            let count = self.count.take().unwrap_or(1);
            tab.scroll_offset = tab.scroll_offset.saturating_sub(count);
            tab.g_key_pressed = false;
        } else if kb.navigation.half_page_down.matches(key, modifiers) {
            let half = (tab.visible_height / 2).max(1);
            let count = self.count.take().unwrap_or(1);
            tab.scroll_offset = tab.scroll_offset.saturating_add(half.saturating_mul(count));
            tab.g_key_pressed = false;
        } else if kb.navigation.half_page_up.matches(key, modifiers) {
            let half = (tab.visible_height / 2).max(1);
            let count = self.count.take().unwrap_or(1);
            tab.scroll_offset = tab.scroll_offset.saturating_sub(half.saturating_mul(count));
            tab.g_key_pressed = false;
        } else if kb.navigation.page_down.matches(key, modifiers) {
            let page = tab.visible_height.max(1);
            let count = self.count.take().unwrap_or(1);
            tab.scroll_offset = tab.scroll_offset.saturating_add(page.saturating_mul(count));
            tab.g_key_pressed = false;
        } else if kb.navigation.page_up.matches(key, modifiers) {
            let page = tab.visible_height.max(1);
            let count = self.count.take().unwrap_or(1);
            tab.scroll_offset = tab.scroll_offset.saturating_sub(page.saturating_mul(count));
            tab.g_key_pressed = false;
        } else if kb.normal.go_to_bottom.matches(key, modifiers) {
            if let Some(count) = self.count.take() {
                let _ = tab.goto_line(count);
            } else {
                let n = tab.visible_indices.len();
                if n > 0 {
                    tab.scroll_offset = n - 1;
                }
            }
            tab.g_key_pressed = false;
        } else if kb.normal.go_to_top_chord.matches(key, modifiers) {
            if tab.g_key_pressed {
                if let Some(count) = self.count.take() {
                    let _ = tab.goto_line(count);
                } else {
                    tab.scroll_offset = 0;
                }
                tab.g_key_pressed = false;
            } else {
                tab.g_key_pressed = true;
            }
        } else if kb.visual_line.comment.matches(key, modifiers) {
            tab.g_key_pressed = false;
            // Comment the selected range
            if tab.visible_indices.is_empty() {
                return (Box::new(NormalMode::default()), KeyResult::Handled);
            }
            let max_idx = tab.visible_indices.len() - 1;
            let lo = self.anchor.min(tab.scroll_offset).min(max_idx);
            let hi = self.anchor.max(tab.scroll_offset).min(max_idx);
            let line_indices = tab.visible_indices.slice_to_vec(lo, hi);
            if !line_indices.is_empty() {
                return (Box::new(CommentMode::new(line_indices)), KeyResult::Handled);
            }
            return (Box::new(NormalMode::default()), KeyResult::Handled);
        } else if kb.visual_line.mark.matches(key, modifiers) {
            // Mark/unmark all selected lines as a group.
            // If every selected line is already marked → unmark all; otherwise → mark all.
            if !tab.visible_indices.is_empty() {
                let max_idx = tab.visible_indices.len() - 1;
                let lo = self.anchor.min(tab.scroll_offset).min(max_idx);
                let hi = self.anchor.max(tab.scroll_offset).min(max_idx);
                let line_indices = tab.visible_indices.slice_to_vec(lo, hi);
                let all_marked = line_indices.iter().all(|&i| tab.log_manager.is_marked(i));
                if all_marked {
                    for idx in &line_indices {
                        tab.log_manager.toggle_mark(*idx);
                    }
                } else {
                    for idx in &line_indices {
                        if !tab.log_manager.is_marked(*idx) {
                            tab.log_manager.toggle_mark(*idx);
                        }
                    }
                }
            }
            return (Box::new(NormalMode::default()), KeyResult::Handled);
        } else if kb.visual_line.yank.matches(key, modifiers) {
            // Yank (copy) selected lines to clipboard
            if tab.visible_indices.is_empty() {
                tab.command_error = Some("No lines to copy".to_string());
                return (Box::new(NormalMode::default()), KeyResult::Handled);
            }
            let max_idx = tab.visible_indices.len() - 1;
            let lo = self.anchor.min(tab.scroll_offset).min(max_idx);
            let hi = self.anchor.max(tab.scroll_offset).min(max_idx);
            let line_indices = tab.visible_indices.slice_to_vec(lo, hi);
            let text: String = line_indices
                .iter()
                .map(|&idx| {
                    let bytes = tab.file_reader.get_line(idx);
                    if tab.raw_mode {
                        None
                    } else {
                        tab.detected_format.as_ref()
                    }
                    .and_then(|parser| parser.parse_line(bytes))
                    .map(|parts| {
                        apply_field_layout(
                            &parts,
                            &tab.field_layout,
                            &tab.hidden_fields,
                            tab.show_keys,
                        )
                        .join(" ")
                    })
                    .unwrap_or_else(|| String::from_utf8_lossy(bytes).into_owned())
                })
                .collect::<Vec<_>>()
                .join("\n");
            return (
                Box::new(NormalMode::default()),
                KeyResult::CopyToClipboard(text),
            );
        } else if kb.visual_line.filter_include.matches(key, modifiers) {
            let text = regex::escape(&lo_line_text(tab, self.anchor, tab.scroll_offset));
            let input = format!("filter {}", text);
            let cursor = input.len();
            let history = tab.command_history.clone();
            return (
                Box::new(CommandMode::with_history(input, cursor, history)),
                KeyResult::Handled,
            );
        } else if kb.visual_line.filter_exclude.matches(key, modifiers) {
            let text = regex::escape(&lo_line_text(tab, self.anchor, tab.scroll_offset));
            let input = format!("exclude {}", text);
            let cursor = input.len();
            let history = tab.command_history.clone();
            return (
                Box::new(CommandMode::with_history(input, cursor, history)),
                KeyResult::Handled,
            );
        } else if kb.visual_line.search.matches(key, modifiers) {
            let text = regex::escape(&lo_line_text(tab, self.anchor, tab.scroll_offset));
            return (
                Box::new(SearchMode {
                    input: text,
                    forward: true,
                }),
                KeyResult::Handled,
            );
        } else if kb.visual_line.exit.matches(key, modifiers) {
            return (Box::new(NormalMode::default()), KeyResult::Handled);
        }

        (self, KeyResult::Handled)
    }

    fn mode_bar_content(&self, kb: &Keybindings, theme: &Theme) -> Line<'static> {
        let label = match self.count {
            Some(n) => format!("[VISUAL] {}  ", n),
            None => "[VISUAL]  ".to_string(),
        };
        let mut spans: Vec<Span<'static>> = vec![Span::styled(
            label,
            Style::default()
                .fg(theme.text_highlight_fg)
                .add_modifier(Modifier::BOLD),
        )];
        // Extend up/down
        spans.push(Span::styled("<", Style::default().fg(theme.text)));
        spans.push(Span::styled(
            kb.navigation.scroll_up.display(),
            Style::default()
                .fg(theme.text_highlight_fg)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled("/", Style::default().fg(theme.text)));
        spans.push(Span::styled(
            kb.navigation.scroll_down.display(),
            Style::default()
                .fg(theme.text_highlight_fg)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled("> extend  ", Style::default().fg(theme.text)));
        status_entry(
            &mut spans,
            kb.visual_line.comment.display(),
            "comment",
            theme,
        );
        status_entry(&mut spans, kb.visual_line.yank.display(), "yank", theme);
        status_entry(&mut spans, kb.visual_line.mark.display(), "mark", theme);
        status_entry(
            &mut spans,
            kb.visual_line.filter_include.display(),
            "filter",
            theme,
        );
        status_entry(
            &mut spans,
            kb.visual_line.filter_exclude.display(),
            "exclude",
            theme,
        );
        status_entry(&mut spans, kb.visual_line.search.display(), "search", theme);
        status_entry(&mut spans, kb.visual_line.exit.display(), "cancel", theme);
        Line::from(spans)
    }

    fn render_state(&self) -> ModeRenderState {
        ModeRenderState::VisualLine {
            anchor: self.anchor,
        }
    }
}

/// Returns the displayed text of the lo line of the visual selection.
fn lo_line_text(tab: &TabState, anchor: usize, scroll_offset: usize) -> String {
    if tab.visible_indices.is_empty() {
        return String::new();
    }
    let max_idx = tab.visible_indices.len() - 1;
    let lo = anchor.min(scroll_offset).min(max_idx);
    let idx = tab.visible_indices.get(lo);
    let bytes = tab.file_reader.get_line(idx);
    if !tab.raw_mode
        && let Some(parser) = tab.detected_format.as_ref()
        && let Some(parts) = parser.parse_line(bytes)
    {
        return apply_field_layout(&parts, &tab.field_layout, &tab.hidden_fields, tab.show_keys)
            .join(" ");
    }
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::file_reader::FileReader;
    use crate::log_manager::LogManager;
    use crate::mode::app_mode::ModeRenderState;
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
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('j')).await;
        assert_eq!(tab.scroll_offset, 1);
        assert!(matches!(
            mode2.render_state(),
            ModeRenderState::VisualLine { .. }
        ));
    }

    #[tokio::test]
    async fn test_k_moves_cursor_up() {
        let mut tab = make_tab(&["a", "b", "c"]).await;
        tab.scroll_offset = 2;
        let mode = VisualLineMode {
            anchor: 2,
            count: None,
        };
        let (_, _) = press(mode, &mut tab, KeyCode::Char('k')).await;
        assert_eq!(tab.scroll_offset, 1);
    }

    #[tokio::test]
    async fn test_k_saturates_at_zero() {
        let mut tab = make_tab(&["a"]).await;
        tab.scroll_offset = 0;
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        let _ = press(mode, &mut tab, KeyCode::Char('k')).await;
        assert_eq!(tab.scroll_offset, 0);
    }

    #[tokio::test]
    async fn test_esc_returns_normal_mode() {
        let mut tab = make_tab(&["a"]).await;
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        let (mode2, result) = press(mode, &mut tab, KeyCode::Esc).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(!matches!(
            mode2.render_state(),
            ModeRenderState::VisualLine { .. }
        ));
        assert!(matches!(mode2.render_state(), ModeRenderState::Normal));
    }

    #[tokio::test]
    async fn test_c_opens_comment_mode_with_selected_lines() {
        let mut tab = make_tab(&["a", "b", "c", "d"]).await;
        tab.scroll_offset = 3; // cursor at line index 3
        let mode = VisualLineMode {
            anchor: 1,
            count: None,
        }; // anchor at visible index 1
        let (mode2, result) = press(mode, &mut tab, KeyCode::Char('c')).await;
        assert!(matches!(result, KeyResult::Handled));
        // Should be in comment mode
        match mode2.render_state() {
            ModeRenderState::Comment { line_count, .. } => {
                assert_eq!(line_count, 3); // visible indices 1,2,3 → 3 lines
            }
            other => panic!("expected Comment, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_c_with_anchor_above_cursor() {
        let mut tab = make_tab(&["a", "b", "c"]).await;
        tab.scroll_offset = 0;
        let mode = VisualLineMode {
            anchor: 2,
            count: None,
        }; // anchor below cursor
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('c')).await;
        match mode2.render_state() {
            ModeRenderState::Comment { line_count, .. } => {
                assert_eq!(line_count, 3); // lines 0,1,2
            }
            other => panic!("expected Comment, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_visual_selection_anchor_returns_anchor() {
        let mode = VisualLineMode {
            anchor: 5,
            count: None,
        };
        match mode.render_state() {
            ModeRenderState::VisualLine { anchor } => assert_eq!(anchor, 5),
            other => panic!("expected VisualLine, got {:?}", other),
        }
    }

    #[test]
    fn test_mode_bar_content_contains_visual() {
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        assert!(matches!(
            mode.render_state(),
            ModeRenderState::VisualLine { .. }
        ));
    }

    #[test]
    fn test_mode_bar_content_contains_yank() {
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        let content = mode.mode_bar_content(&Keybindings::default(), &Theme::default());
        let text: String = content.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("yank"));
    }

    #[tokio::test]
    async fn test_y_yanks_and_returns_normal() {
        let mut tab = make_tab(&["line one", "line two", "line three"]).await;
        tab.scroll_offset = 0;
        let mode = VisualLineMode {
            anchor: 2,
            count: None,
        };
        let (mode2, result) = press(mode, &mut tab, KeyCode::Char('y')).await;
        // Should return to normal mode
        assert!(matches!(mode2.render_state(), ModeRenderState::Normal));
        // Should return the selected text for clipboard via App
        match result {
            KeyResult::CopyToClipboard(text) => {
                assert_eq!(text, "line one\nline two\nline three");
            }
            other => panic!("expected CopyToClipboard, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_y_yanks_structured_lines_as_displayed() {
        // JSON log lines — the parser should detect them and format columns.
        let mut tab = make_tab(&[
            r#"{"timestamp":"2024-01-01T00:00:00Z","level":"INFO","message":"hello"}"#,
            r#"{"timestamp":"2024-01-01T00:00:01Z","level":"WARN","message":"world"}"#,
        ])
        .await;
        // Ensure the parser was detected.
        assert!(
            tab.detected_format.is_some(),
            "expected a format parser to be detected"
        );
        tab.scroll_offset = 0;
        let mode = VisualLineMode {
            anchor: 1,
            count: None,
        };
        let (_, result) = press(mode, &mut tab, KeyCode::Char('y')).await;
        match result {
            KeyResult::CopyToClipboard(text) => {
                // Should contain the formatted columns, not raw JSON.
                assert!(!text.contains('{'), "expected formatted text, got raw JSON");
                assert!(text.contains("hello"));
                assert!(text.contains("world"));
            }
            other => panic!("expected CopyToClipboard, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_m_marks_all_selected_lines() {
        let mut tab = make_tab(&["a", "b", "c", "d"]).await;
        tab.scroll_offset = 3;
        let mode = VisualLineMode {
            anchor: 1,
            count: None,
        };
        let (mode2, result) = press(mode, &mut tab, KeyCode::Char('m')).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(matches!(mode2.render_state(), ModeRenderState::Normal));
        // Lines at visible indices 1, 2, 3 should be marked.
        assert!(tab.log_manager.is_marked(1));
        assert!(tab.log_manager.is_marked(2));
        assert!(tab.log_manager.is_marked(3));
        assert!(!tab.log_manager.is_marked(0));
    }

    #[tokio::test]
    async fn test_m_unmarks_when_all_already_marked() {
        let mut tab = make_tab(&["a", "b", "c"]).await;
        tab.log_manager.toggle_mark(0);
        tab.log_manager.toggle_mark(1);
        tab.log_manager.toggle_mark(2);
        tab.scroll_offset = 2;
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        let (_, _) = press(mode, &mut tab, KeyCode::Char('m')).await;
        // All were marked → all should now be unmarked.
        assert!(!tab.log_manager.is_marked(0));
        assert!(!tab.log_manager.is_marked(1));
        assert!(!tab.log_manager.is_marked(2));
    }

    #[tokio::test]
    async fn test_m_marks_all_when_partially_marked() {
        let mut tab = make_tab(&["a", "b", "c"]).await;
        tab.log_manager.toggle_mark(0); // only first is marked
        tab.scroll_offset = 2;
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        let (_, _) = press(mode, &mut tab, KeyCode::Char('m')).await;
        // Partial marks → mark all.
        assert!(tab.log_manager.is_marked(0));
        assert!(tab.log_manager.is_marked(1));
        assert!(tab.log_manager.is_marked(2));
    }

    #[tokio::test]
    async fn test_y_raw_mode_yanks_raw_bytes() {
        let mut tab =
            make_tab(&[r#"{"timestamp":"2024-01-01T00:00:00Z","level":"INFO","message":"hello"}"#])
                .await;
        assert!(tab.detected_format.is_some());
        tab.raw_mode = true;
        tab.scroll_offset = 0;
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        let (_, result) = press(mode, &mut tab, KeyCode::Char('y')).await;
        match result {
            KeyResult::CopyToClipboard(text) => {
                // Raw mode: should preserve the original JSON, not format columns.
                assert!(text.contains('{'), "expected raw JSON, got formatted text");
                assert!(text.contains("hello"));
            }
            other => panic!("expected CopyToClipboard, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_y_empty_visible_indices_returns_normal() {
        let mut tab = make_tab(&[]).await;
        tab.scroll_offset = 0;
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('y')).await;
        assert!(matches!(mode2.render_state(), ModeRenderState::Normal));
        assert_eq!(tab.command_error.as_deref(), Some("No lines to copy"));
    }

    // ── Count prefix tests ───────────────────────────────────────────────

    #[tokio::test]
    async fn test_visual_count_5j_moves_down_5() {
        let lines: Vec<&str> = (0..20).map(|_| "line").collect();
        let mut tab = make_tab(&lines).await;
        tab.scroll_offset = 0;
        let mode = VisualLineMode {
            anchor: 0,
            count: Some(5),
        };
        let (_, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Char('j'), KeyModifiers::NONE)
            .await;
        assert_eq!(tab.scroll_offset, 5);
    }

    #[tokio::test]
    async fn test_visual_count_3k_moves_up_3() {
        let lines: Vec<&str> = (0..20).map(|_| "line").collect();
        let mut tab = make_tab(&lines).await;
        tab.scroll_offset = 10;
        let mode = VisualLineMode {
            anchor: 10,
            count: Some(3),
        };
        let (_, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Char('k'), KeyModifiers::NONE)
            .await;
        assert_eq!(tab.scroll_offset, 7);
    }

    #[tokio::test]
    async fn test_visual_digit_accumulation() {
        let lines: Vec<&str> = (0..200).map(|_| "line").collect();
        let mut tab = make_tab(&lines).await;
        tab.scroll_offset = 0;
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        // Type "15j"
        let (mode, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Char('1'), KeyModifiers::NONE)
            .await;
        let (mode, _) = mode
            .handle_key(&mut tab, KeyCode::Char('5'), KeyModifiers::NONE)
            .await;
        let _ = mode
            .handle_key(&mut tab, KeyCode::Char('j'), KeyModifiers::NONE)
            .await;
        assert_eq!(tab.scroll_offset, 15);
    }

    #[tokio::test]
    async fn test_i_prefills_filter_with_lo_line_text() {
        let mut tab = make_tab(&["foo", "bar", "baz"]).await;
        tab.scroll_offset = 2;
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('i')).await;
        match mode2.render_state() {
            ModeRenderState::Command { input, .. } => {
                assert!(input.starts_with("filter "), "got: {input}");
                assert!(
                    input.contains("foo"),
                    "expected lo-line text 'foo', got: {input}"
                );
            }
            other => panic!("expected Command, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_i_escapes_regex_metacharacters_in_filter() {
        let mut tab = make_tab(&["192.168.1.1 GET /path"]).await;
        tab.scroll_offset = 0;
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('i')).await;
        match mode2.render_state() {
            ModeRenderState::Command { input, .. } => {
                assert!(
                    input.starts_with("filter "),
                    "expected filter prefix, got: {input}"
                );
                assert!(
                    input.contains(r"192\.168\.1\.1"),
                    "dots must be escaped, got: {input}"
                );
            }
            other => panic!("expected Command, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_o_prefills_exclude_with_lo_line_text() {
        let mut tab = make_tab(&["foo", "bar"]).await;
        tab.scroll_offset = 1;
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('o')).await;
        match mode2.render_state() {
            ModeRenderState::Command { input, .. } => {
                assert!(input.starts_with("exclude "), "got: {input}");
                assert!(
                    input.contains("foo"),
                    "expected lo-line text 'foo', got: {input}"
                );
            }
            other => panic!("expected Command, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_o_escapes_regex_metacharacters_in_exclude() {
        let mut tab = make_tab(&["panic: goroutine (exit)"]).await;
        tab.scroll_offset = 0;
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('o')).await;
        match mode2.render_state() {
            ModeRenderState::Command { input, .. } => {
                assert!(
                    input.starts_with("exclude "),
                    "expected exclude prefix, got: {input}"
                );
                assert!(
                    input.contains(r"panic: goroutine \(exit\)"),
                    "parens must be escaped, got: {input}"
                );
            }
            other => panic!("expected Command, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_slash_enters_search_with_lo_line_text() {
        let mut tab = make_tab(&["foo", "bar"]).await;
        tab.scroll_offset = 1;
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('/')).await;
        match mode2.render_state() {
            ModeRenderState::Search { query, forward } => {
                assert!(query.contains("foo"), "expected lo-line text, got: {query}");
                assert!(forward);
            }
            other => panic!("expected Search, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_slash_escapes_regex_metacharacters_in_search() {
        let mut tab = make_tab(&["GET /api/v2?limit=10"]).await;
        tab.scroll_offset = 0;
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('/')).await;
        match mode2.render_state() {
            ModeRenderState::Search { query, forward } => {
                assert!(
                    query.contains(r"GET /api/v2\?limit=10"),
                    "? must be escaped, got: {query}"
                );
                assert!(forward);
            }
            other => panic!("expected Search, got {:?}", other),
        }
    }

    // ── New motion tests ─────────────────────────────────────────────────

    async fn press_ctrl(
        mode: VisualLineMode,
        tab: &mut TabState,
        c: char,
    ) -> (Box<dyn Mode>, KeyResult) {
        Box::new(mode)
            .handle_key(tab, KeyCode::Char(c), KeyModifiers::CONTROL)
            .await
    }

    #[tokio::test]
    async fn test_ctrl_d_moves_half_page_down() {
        let lines: Vec<&str> = (0..40).map(|_| "line").collect();
        let mut tab = make_tab(&lines).await;
        tab.scroll_offset = 0;
        tab.visible_height = 20;
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        let _ = press_ctrl(mode, &mut tab, 'd').await;
        assert_eq!(tab.scroll_offset, 10); // half of 20
    }

    #[tokio::test]
    async fn test_ctrl_u_moves_half_page_up() {
        let lines: Vec<&str> = (0..40).map(|_| "line").collect();
        let mut tab = make_tab(&lines).await;
        tab.scroll_offset = 20;
        tab.visible_height = 20;
        let mode = VisualLineMode {
            anchor: 20,
            count: None,
        };
        let _ = press_ctrl(mode, &mut tab, 'u').await;
        assert_eq!(tab.scroll_offset, 10);
    }

    #[tokio::test]
    async fn test_page_down_moves_full_page() {
        let lines: Vec<&str> = (0..60).map(|_| "line").collect();
        let mut tab = make_tab(&lines).await;
        tab.scroll_offset = 0;
        tab.visible_height = 20;
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        let _ = Box::new(mode)
            .handle_key(&mut tab, KeyCode::PageDown, KeyModifiers::NONE)
            .await;
        assert_eq!(tab.scroll_offset, 20);
    }

    #[tokio::test]
    async fn test_page_up_moves_full_page() {
        let lines: Vec<&str> = (0..60).map(|_| "line").collect();
        let mut tab = make_tab(&lines).await;
        tab.scroll_offset = 20;
        tab.visible_height = 20;
        let mode = VisualLineMode {
            anchor: 20,
            count: None,
        };
        let _ = Box::new(mode)
            .handle_key(&mut tab, KeyCode::PageUp, KeyModifiers::NONE)
            .await;
        assert_eq!(tab.scroll_offset, 0);
    }

    #[tokio::test]
    async fn test_G_goes_to_last_line() {
        let lines: Vec<&str> = (0..10).map(|_| "line").collect();
        let mut tab = make_tab(&lines).await;
        tab.scroll_offset = 0;
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        let _ = press(mode, &mut tab, KeyCode::Char('G')).await;
        assert_eq!(tab.scroll_offset, 9);
    }

    #[tokio::test]
    async fn test_gg_goes_to_first_line() {
        let lines: Vec<&str> = (0..10).map(|_| "line").collect();
        let mut tab = make_tab(&lines).await;
        tab.scroll_offset = 9;
        let mode = VisualLineMode {
            anchor: 9,
            count: None,
        };
        // First 'g' sets the flag
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('g')).await;
        assert!(tab.g_key_pressed);
        assert_eq!(tab.scroll_offset, 9); // not moved yet
        // Second 'g' jumps to top
        let _ = mode2
            .handle_key(&mut tab, KeyCode::Char('g'), KeyModifiers::NONE)
            .await;
        assert_eq!(tab.scroll_offset, 0);
        assert!(!tab.g_key_pressed);
    }

    #[tokio::test]
    async fn test_j_resets_g_key_pressed() {
        let lines: Vec<&str> = (0..10).map(|_| "line").collect();
        let mut tab = make_tab(&lines).await;
        tab.g_key_pressed = true;
        tab.scroll_offset = 0;
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        let _ = press(mode, &mut tab, KeyCode::Char('j')).await;
        assert!(!tab.g_key_pressed);
    }
}
