#[cfg(feature = "tui")]
use crate::config::Keybindings;
use crate::input::{KeyCode, KeyModifiers};
#[cfg(feature = "tui")]
use crate::mode::app_mode::status_entry;
#[cfg(feature = "tui")]
use crate::style::{Modifier, Style};
#[cfg(feature = "tui")]
use crate::theme::Theme;
use crate::{
    mode::app_mode::{Mode, ModeRenderState},
    mode::comment_mode::CommentMode,
    mode::normal_mode::NormalMode,
    mode::search_mode::SearchMode,
    shared::{KeyResult, TabState},
};
use async_trait::async_trait;
#[cfg(feature = "tui")]
use ratatui::text::{Line, Span};

#[derive(Debug)]
pub struct VisualLineMode {
    /// Index into `visible_indices` that was the cursor when `V` was pressed.
    pub anchor: usize,
    /// Vim-style count prefix for multi-line motions.
    pub count: Option<usize>,
}

impl VisualLineMode {
    pub(super) fn extend_selection(
        &mut self,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> bool {
        let kb = &tab.interaction.keybindings.gui;
        let count = self.count.unwrap_or(1);
        let last = tab.filter.visible_indices.len().saturating_sub(1);
        let target = if kb.extend_down.matches(key, modifiers) {
            tab.scroll.scroll_offset.saturating_add(count).min(last)
        } else if kb.extend_up.matches(key, modifiers) {
            tab.scroll.scroll_offset.saturating_sub(count).min(last)
        } else if kb.extend_first.matches(key, modifiers) {
            0
        } else if kb.extend_last.matches(key, modifiers) {
            last
        } else {
            return false;
        };
        self.count = None;
        tab.scroll.scroll_offset = target;
        tab.scroll.wrapped_row = 0;
        tab.stream.tail_mode = false;
        tab.interaction.g_key_pressed = false;
        true
    }
}

#[async_trait]
impl Mode for VisualLineMode {
    async fn handle_key(
        mut self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        if self.extend_selection(tab, key, modifiers) {
            return (self, KeyResult::Handled);
        }
        let kb = &tab.interaction.keybindings;

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
            tab.scroll.scroll_offset = tab.scroll.scroll_offset.saturating_add(count);
            tab.interaction.g_key_pressed = false;
        } else if kb.navigation.scroll_up.matches(key, modifiers) {
            let count = self.count.take().unwrap_or(1);
            tab.scroll.scroll_offset = tab.scroll.scroll_offset.saturating_sub(count);
            tab.interaction.g_key_pressed = false;
        } else if kb.navigation.half_page_down.matches(key, modifiers) {
            let half = (tab.scroll.visible_height / 2).max(1);
            let count = self.count.take().unwrap_or(1);
            tab.scroll.scroll_offset = tab
                .scroll
                .scroll_offset
                .saturating_add(half.saturating_mul(count));
            tab.interaction.g_key_pressed = false;
        } else if kb.navigation.half_page_up.matches(key, modifiers) {
            let half = (tab.scroll.visible_height / 2).max(1);
            let count = self.count.take().unwrap_or(1);
            tab.scroll.scroll_offset = tab
                .scroll
                .scroll_offset
                .saturating_sub(half.saturating_mul(count));
            tab.interaction.g_key_pressed = false;
        } else if kb.navigation.page_down.matches(key, modifiers) {
            let page = tab.scroll.visible_height.max(1);
            let count = self.count.take().unwrap_or(1);
            tab.scroll.scroll_offset = tab
                .scroll
                .scroll_offset
                .saturating_add(page.saturating_mul(count));
            tab.interaction.g_key_pressed = false;
        } else if kb.navigation.page_up.matches(key, modifiers) {
            let page = tab.scroll.visible_height.max(1);
            let count = self.count.take().unwrap_or(1);
            tab.scroll.scroll_offset = tab
                .scroll
                .scroll_offset
                .saturating_sub(page.saturating_mul(count));
            tab.interaction.g_key_pressed = false;
        } else if kb.normal.go_to_bottom.matches(key, modifiers) {
            if let Some(count) = self.count.take() {
                let _ = tab.goto_line(count);
            } else {
                let n = tab.filter.visible_indices.len();
                if n > 0 {
                    tab.scroll.scroll_offset = n - 1;
                }
            }
            tab.interaction.g_key_pressed = false;
        } else if kb.normal.go_to_top_chord.matches(key, modifiers) {
            if tab.interaction.g_key_pressed {
                if let Some(count) = self.count.take() {
                    let _ = tab.goto_line(count);
                } else {
                    tab.scroll.scroll_offset = 0;
                }
                tab.interaction.g_key_pressed = false;
            } else {
                tab.interaction.g_key_pressed = true;
            }
        } else if kb.visual_line.comment.matches(key, modifiers) {
            tab.interaction.g_key_pressed = false;
            // Comment the selected range
            if tab.filter.visible_indices.is_empty() {
                return (Box::new(NormalMode::default()), KeyResult::Handled);
            }
            let max_idx = tab.filter.visible_indices.len() - 1;
            let lo = self.anchor.min(tab.scroll.scroll_offset).min(max_idx);
            let hi = self.anchor.max(tab.scroll.scroll_offset).min(max_idx);
            let line_indices = tab.filter.visible_indices.slice_to_vec(lo, hi);
            if !line_indices.is_empty() {
                return (Box::new(CommentMode::new(line_indices)), KeyResult::Handled);
            }
            return (Box::new(NormalMode::default()), KeyResult::Handled);
        } else if kb.visual_line.mark.matches(key, modifiers) {
            // Mark/unmark all selected lines as a group.
            // If every selected line is already marked → unmark all; otherwise → mark all.
            if !tab.filter.visible_indices.is_empty() {
                let max_idx = tab.filter.visible_indices.len() - 1;
                let lo = self.anchor.min(tab.scroll.scroll_offset).min(max_idx);
                let hi = self.anchor.max(tab.scroll.scroll_offset).min(max_idx);
                let line_indices = tab.filter.visible_indices.slice_to_vec(lo, hi);
                let all_marked = line_indices.iter().all(|&i| tab.mark_manager.is_marked(i));
                if all_marked {
                    for idx in &line_indices {
                        tab.toggle_mark(*idx);
                    }
                } else {
                    for idx in &line_indices {
                        if !tab.mark_manager.is_marked(*idx) {
                            tab.toggle_mark(*idx);
                        }
                    }
                }
            }
            return (Box::new(NormalMode::default()), KeyResult::Handled);
        } else if kb.visual_line.yank.matches(key, modifiers) {
            // Yank (copy) selected lines to clipboard
            if tab.filter.visible_indices.is_empty() {
                tab.interaction.command_error = Some("No lines to copy".to_string());
                return (Box::new(NormalMode::default()), KeyResult::Handled);
            }
            let max_idx = tab.filter.visible_indices.len() - 1;
            let lo = self.anchor.min(tab.scroll.scroll_offset).min(max_idx);
            let hi = self.anchor.max(tab.scroll.scroll_offset).min(max_idx);
            let line_indices = tab.filter.visible_indices.slice_to_vec(lo, hi);
            let text: String = line_indices
                .iter()
                .map(|&idx| tab.get_display_text(idx))
                .collect::<Vec<_>>()
                .join("\n");
            return (
                Box::new(NormalMode::default()),
                KeyResult::CopyToClipboard(text),
            );
        } else if kb.visual_line.search.matches(key, modifiers) {
            let text = regex::escape(&lo_line_text(tab, self.anchor, tab.scroll.scroll_offset));
            return (
                Box::new(SearchMode {
                    cursor: text.len(),
                    input: text,
                    forward: true,
                }),
                KeyResult::Handled,
            );
        } else if kb.visual_line.exit.matches(key, modifiers) {
            return (Box::new(NormalMode::default()), KeyResult::Handled);
        }

        (self, KeyResult::Ignored)
    }

    #[cfg(feature = "tui")]
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
    if tab.filter.visible_indices.is_empty() {
        return String::new();
    }
    let max_idx = tab.filter.visible_indices.len() - 1;
    let lo = anchor.min(scroll_offset).min(max_idx);
    let idx = tab.filter.visible_indices.get(lo);
    tab.get_display_text(idx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::db::LogManager;
    use crate::ingestion::FileReader;
    use crate::mode::app_mode::ModeRenderState;
    use crate::shared::TabState;
    use std::sync::Arc;

    async fn make_tab(lines: &[&str]) -> TabState {
        let data = lines.join("\n").into_bytes();
        let file_reader = FileReader::from_bytes(data);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        TabState::new(file_reader, log_manager, "test".to_string())
    }

    #[tokio::test]
    async fn extension_keys_start_and_extend_a_filtered_line_selection() {
        let mut tab = make_tab(&["zero", "hidden", "two", "hidden", "four"]).await;
        tab.filter.visible_indices = crate::shared::VisibleLines::Filtered(vec![0, 2, 4]);
        tab.scroll.scroll_offset = 1;
        tab.scroll.wrapped_row = 3;
        tab.stream.tail_mode = true;
        let mut mode: Box<dyn Mode> = Box::new(NormalMode::default());
        for (key, expected) in [
            (KeyCode::Down, 2),
            (KeyCode::Down, 2),
            (KeyCode::Home, 0),
            (KeyCode::End, 2),
            (KeyCode::Up, 1),
            (KeyCode::Up, 0),
        ] {
            let (next, result) = mode.handle_key(&mut tab, key, KeyModifiers::SHIFT).await;
            assert!(matches!(result, KeyResult::Handled));
            assert!(matches!(
                next.render_state(),
                ModeRenderState::VisualLine { anchor: 1 }
            ));
            assert_eq!(tab.scroll.scroll_offset, expected);
            mode = next;
        }
        assert_eq!(tab.scroll.wrapped_row, 0);
        assert!(!tab.stream.tail_mode);
        mode.handle_key(&mut tab, KeyCode::Char('m'), KeyModifiers::NONE)
            .await;
        assert_eq!(tab.mark_manager.get_indices(), vec![0, 2]);
    }

    #[tokio::test]
    async fn extension_keys_reuse_custom_bindings_counts_and_disabled_bindings() {
        let mut tab = make_tab(&["zero", "one", "two", "three", "four"]).await;
        let kb = Arc::make_mut(&mut tab.interaction.keybindings);
        kb.gui.extend_up = serde_json::from_str("\"F5\"").unwrap();
        kb.gui.extend_down = serde_json::from_str("\"F6\"").unwrap();
        kb.gui.extend_first = serde_json::from_str("\"F7\"").unwrap();
        kb.gui.extend_last = serde_json::from_str("\"F8\"").unwrap();
        let (mode, _) = Box::new(NormalMode::default())
            .handle_key(&mut tab, KeyCode::Down, KeyModifiers::SHIFT)
            .await;
        assert!(matches!(mode.render_state(), ModeRenderState::Normal));
        let (mut mode, _) = mode
            .handle_key(&mut tab, KeyCode::Char('2'), KeyModifiers::NONE)
            .await;
        for (key, expected) in [(6, 2), (5, 1), (8, 4), (7, 0)] {
            let (next, result) = mode
                .handle_key(&mut tab, KeyCode::F(key), KeyModifiers::NONE)
                .await;
            assert!(matches!(result, KeyResult::Handled));
            assert!(matches!(
                next.render_state(),
                ModeRenderState::VisualLine { anchor: 0 }
            ));
            assert_eq!(tab.scroll.scroll_offset, expected);
            mode = next;
        }
        Arc::make_mut(&mut tab.interaction.keybindings)
            .gui
            .extend_down
            .0
            .clear();
        let (mode, _) = mode
            .handle_key(&mut tab, KeyCode::F(6), KeyModifiers::NONE)
            .await;
        assert_eq!(tab.scroll.scroll_offset, 0);
        let (mode, _) = mode
            .handle_key(&mut tab, KeyCode::Esc, KeyModifiers::NONE)
            .await;
        assert!(matches!(mode.render_state(), ModeRenderState::Normal));
    }

    #[tokio::test]
    async fn extension_keys_leave_an_empty_log_in_normal_mode() {
        let mut tab = make_tab(&[]).await;
        let (mode, _) = Box::new(NormalMode::default())
            .handle_key(&mut tab, KeyCode::End, KeyModifiers::SHIFT)
            .await;
        assert!(matches!(mode.render_state(), ModeRenderState::Normal));
        assert_eq!(tab.scroll.scroll_offset, 0);
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
        tab.scroll.scroll_offset = 0;
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('j')).await;
        assert_eq!(tab.scroll.scroll_offset, 1);
        assert!(matches!(
            mode2.render_state(),
            ModeRenderState::VisualLine { .. }
        ));
    }

    #[tokio::test]
    async fn test_k_moves_cursor_up() {
        let mut tab = make_tab(&["a", "b", "c"]).await;
        tab.scroll.scroll_offset = 2;
        let mode = VisualLineMode {
            anchor: 2,
            count: None,
        };
        let (_, _) = press(mode, &mut tab, KeyCode::Char('k')).await;
        assert_eq!(tab.scroll.scroll_offset, 1);
    }

    #[tokio::test]
    async fn test_k_saturates_at_zero() {
        let mut tab = make_tab(&["a"]).await;
        tab.scroll.scroll_offset = 0;
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        let _ = press(mode, &mut tab, KeyCode::Char('k')).await;
        assert_eq!(tab.scroll.scroll_offset, 0);
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
        tab.scroll.scroll_offset = 3; // cursor at line index 3
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
        tab.scroll.scroll_offset = 0;
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
    #[cfg(feature = "tui")]
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
        tab.scroll.scroll_offset = 0;
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
            tab.display.format.is_some(),
            "expected a format parser to be detected"
        );
        tab.scroll.scroll_offset = 0;
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
    async fn test_y_yanks_custom_schema_template_reconstruction_with_hidden_field() {
        // Yanked text must match what's actually rendered (schema template,
        // hidden field's separator collapsed), not the generic column
        // layout.
        let line = "INFO/Nexora/LaunchNode, hello there";
        let mut tab = make_tab(&[line]).await;
        let cfg = crate::config::CustomSchemaConfig {
            name: "acme".to_string(),
            description: None,
            template: Some(
                "{level}/{component}/{feature}, {message}"
                    .to_string()
                    .into(),
            ),
            pattern: None,
            fields: Default::default(),
            levels: Default::default(),
            multiline: false,
            ..Default::default()
        };
        tab.display.format = Some(std::sync::Arc::new(
            crate::parser::CustomParser::from_config(&cfg).unwrap(),
        ));
        tab.display.hidden_fields.insert("component".to_string());
        tab.scroll.scroll_offset = 0;
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        let (_, result) = press(mode, &mut tab, KeyCode::Char('y')).await;
        match result {
            KeyResult::CopyToClipboard(text) => {
                assert_eq!(text, "INFO/LaunchNode, hello there");
            }
            other => panic!("expected CopyToClipboard, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_y_yanks_pattern_based_custom_schema_with_hidden_field() {
        // A `pattern`-based (regex) custom schema has no `template_segments`
        // — must also fall back to the generic column layout.
        let line = "INFO shh hello world";
        let mut tab = make_tab(&[line]).await;
        let cfg = crate::config::CustomSchemaConfig {
            name: "test".to_string(),
            description: None,
            template: None,
            pattern: Some("^(?P<level>\\w+) (?P<secret>\\w+) (?P<message>.*)$".to_string()),
            fields: [("secret".to_string(), "extra".to_string())]
                .into_iter()
                .collect(),
            levels: Default::default(),
            multiline: false,
            ..Default::default()
        };
        tab.display.format = Some(std::sync::Arc::new(
            crate::parser::CustomParser::from_config(&cfg).unwrap(),
        ));
        tab.display.hidden_fields.insert("secret".to_string());
        tab.scroll.scroll_offset = 0;
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        let (_, result) = press(mode, &mut tab, KeyCode::Char('y')).await;
        match result {
            KeyResult::CopyToClipboard(text) => {
                assert!(
                    !text.contains("shh"),
                    "hidden field must not appear: {text}"
                );
                assert!(text.contains("INFO"));
                assert!(text.contains("hello world"));
            }
            other => panic!("expected CopyToClipboard, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_m_marks_all_selected_lines() {
        let mut tab = make_tab(&["a", "b", "c", "d"]).await;
        tab.scroll.scroll_offset = 3;
        let mode = VisualLineMode {
            anchor: 1,
            count: None,
        };
        let (mode2, result) = press(mode, &mut tab, KeyCode::Char('m')).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(matches!(mode2.render_state(), ModeRenderState::Normal));
        // Lines at visible indices 1, 2, 3 should be marked.
        assert!(tab.mark_manager.is_marked(1));
        assert!(tab.mark_manager.is_marked(2));
        assert!(tab.mark_manager.is_marked(3));
        assert!(!tab.mark_manager.is_marked(0));
    }

    #[tokio::test]
    async fn test_m_unmarks_when_all_already_marked() {
        let mut tab = make_tab(&["a", "b", "c"]).await;
        tab.mark_manager.toggle(0);
        tab.mark_manager.toggle(1);
        tab.mark_manager.toggle(2);
        tab.scroll.scroll_offset = 2;
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        let (_, _) = press(mode, &mut tab, KeyCode::Char('m')).await;
        // All were marked → all should now be unmarked.
        assert!(!tab.mark_manager.is_marked(0));
        assert!(!tab.mark_manager.is_marked(1));
        assert!(!tab.mark_manager.is_marked(2));
    }

    #[tokio::test]
    async fn test_m_marks_all_when_partially_marked() {
        let mut tab = make_tab(&["a", "b", "c"]).await;
        tab.mark_manager.toggle(0); // only first is marked
        tab.scroll.scroll_offset = 2;
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        let (_, _) = press(mode, &mut tab, KeyCode::Char('m')).await;
        // Partial marks → mark all.
        assert!(tab.mark_manager.is_marked(0));
        assert!(tab.mark_manager.is_marked(1));
        assert!(tab.mark_manager.is_marked(2));
    }

    #[tokio::test]
    async fn test_y_raw_mode_yanks_raw_bytes() {
        let mut tab =
            make_tab(&[r#"{"timestamp":"2024-01-01T00:00:00Z","level":"INFO","message":"hello"}"#])
                .await;
        assert!(tab.display.format.is_some());
        tab.display.raw_mode = true;
        tab.scroll.scroll_offset = 0;
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
        tab.scroll.scroll_offset = 0;
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('y')).await;
        assert!(matches!(mode2.render_state(), ModeRenderState::Normal));
        assert_eq!(
            tab.interaction.command_error.as_deref(),
            Some("No lines to copy")
        );
    }

    #[tokio::test]
    async fn test_visual_count_5j_moves_down_5() {
        let lines: Vec<&str> = (0..20).map(|_| "line").collect();
        let mut tab = make_tab(&lines).await;
        tab.scroll.scroll_offset = 0;
        let mode = VisualLineMode {
            anchor: 0,
            count: Some(5),
        };
        let (_, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Char('j'), KeyModifiers::NONE)
            .await;
        assert_eq!(tab.scroll.scroll_offset, 5);
    }

    #[tokio::test]
    async fn test_visual_count_3k_moves_up_3() {
        let lines: Vec<&str> = (0..20).map(|_| "line").collect();
        let mut tab = make_tab(&lines).await;
        tab.scroll.scroll_offset = 10;
        let mode = VisualLineMode {
            anchor: 10,
            count: Some(3),
        };
        let (_, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Char('k'), KeyModifiers::NONE)
            .await;
        assert_eq!(tab.scroll.scroll_offset, 7);
    }

    #[tokio::test]
    async fn test_visual_digit_accumulation() {
        let lines: Vec<&str> = (0..200).map(|_| "line").collect();
        let mut tab = make_tab(&lines).await;
        tab.scroll.scroll_offset = 0;
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
        assert_eq!(tab.scroll.scroll_offset, 15);
    }

    #[tokio::test]
    async fn test_slash_enters_search_with_lo_line_text() {
        let mut tab = make_tab(&["foo", "bar"]).await;
        tab.scroll.scroll_offset = 1;
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('/')).await;
        match mode2.render_state() {
            ModeRenderState::Search { query, forward, .. } => {
                assert!(query.contains("foo"), "expected lo-line text, got: {query}");
                assert!(forward);
            }
            other => panic!("expected Search, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_slash_seeds_search_with_custom_schema_template_reconstruction() {
        // The seeded search text must match what's rendered (schema
        // template, hidden field collapsed), not the generic column layout.
        let line = "INFO/Nexora/LaunchNode, hello there";
        let mut tab = make_tab(&[line]).await;
        let cfg = crate::config::CustomSchemaConfig {
            name: "acme".to_string(),
            description: None,
            template: Some(
                "{level}/{component}/{feature}, {message}"
                    .to_string()
                    .into(),
            ),
            pattern: None,
            fields: Default::default(),
            levels: Default::default(),
            multiline: false,
            ..Default::default()
        };
        tab.display.format = Some(std::sync::Arc::new(
            crate::parser::CustomParser::from_config(&cfg).unwrap(),
        ));
        tab.display.hidden_fields.insert("component".to_string());
        tab.scroll.scroll_offset = 0;
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('/')).await;
        match mode2.render_state() {
            ModeRenderState::Search { query, .. } => {
                assert_eq!(query, regex::escape("INFO/LaunchNode, hello there"));
            }
            other => panic!("expected Search, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_slash_seeds_search_with_pattern_based_custom_schema_hidden_field() {
        // A `pattern`-based (regex) custom schema has no `template_segments`
        // — must also fall back to the generic column layout.
        let line = "INFO shh hello world";
        let mut tab = make_tab(&[line]).await;
        let cfg = crate::config::CustomSchemaConfig {
            name: "test".to_string(),
            description: None,
            template: None,
            pattern: Some("^(?P<level>\\w+) (?P<secret>\\w+) (?P<message>.*)$".to_string()),
            fields: [("secret".to_string(), "extra".to_string())]
                .into_iter()
                .collect(),
            levels: Default::default(),
            multiline: false,
            ..Default::default()
        };
        tab.display.format = Some(std::sync::Arc::new(
            crate::parser::CustomParser::from_config(&cfg).unwrap(),
        ));
        tab.display.hidden_fields.insert("secret".to_string());
        tab.scroll.scroll_offset = 0;
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('/')).await;
        match mode2.render_state() {
            ModeRenderState::Search { query, .. } => {
                assert!(
                    !query.contains("shh"),
                    "hidden field must not appear: {query}"
                );
                assert!(query.contains("INFO"));
                assert!(query.contains("hello world"));
            }
            other => panic!("expected Search, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_slash_escapes_regex_metacharacters_in_search() {
        let mut tab = make_tab(&["GET /api/v2?limit=10"]).await;
        tab.scroll.scroll_offset = 0;
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('/')).await;
        match mode2.render_state() {
            ModeRenderState::Search { query, forward, .. } => {
                assert!(
                    query.contains(r"GET /api/v2\?limit=10"),
                    "? must be escaped, got: {query}"
                );
                assert!(forward);
            }
            other => panic!("expected Search, got {:?}", other),
        }
    }

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
        tab.scroll.scroll_offset = 0;
        tab.scroll.visible_height = 20;
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        let _ = press_ctrl(mode, &mut tab, 'd').await;
        assert_eq!(tab.scroll.scroll_offset, 10); // half of 20
    }

    #[tokio::test]
    async fn test_ctrl_u_moves_half_page_up() {
        let lines: Vec<&str> = (0..40).map(|_| "line").collect();
        let mut tab = make_tab(&lines).await;
        tab.scroll.scroll_offset = 20;
        tab.scroll.visible_height = 20;
        let mode = VisualLineMode {
            anchor: 20,
            count: None,
        };
        let _ = press_ctrl(mode, &mut tab, 'u').await;
        assert_eq!(tab.scroll.scroll_offset, 10);
    }

    #[tokio::test]
    async fn test_page_down_moves_full_page() {
        let lines: Vec<&str> = (0..60).map(|_| "line").collect();
        let mut tab = make_tab(&lines).await;
        tab.scroll.scroll_offset = 0;
        tab.scroll.visible_height = 20;
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        let _ = Box::new(mode)
            .handle_key(&mut tab, KeyCode::PageDown, KeyModifiers::NONE)
            .await;
        assert_eq!(tab.scroll.scroll_offset, 20);
    }

    #[tokio::test]
    async fn test_page_up_moves_full_page() {
        let lines: Vec<&str> = (0..60).map(|_| "line").collect();
        let mut tab = make_tab(&lines).await;
        tab.scroll.scroll_offset = 20;
        tab.scroll.visible_height = 20;
        let mode = VisualLineMode {
            anchor: 20,
            count: None,
        };
        let _ = Box::new(mode)
            .handle_key(&mut tab, KeyCode::PageUp, KeyModifiers::NONE)
            .await;
        assert_eq!(tab.scroll.scroll_offset, 0);
    }

    #[tokio::test]
    async fn test_g_goes_to_last_line() {
        let lines: Vec<&str> = (0..10).map(|_| "line").collect();
        let mut tab = make_tab(&lines).await;
        tab.scroll.scroll_offset = 0;
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        let _ = press(mode, &mut tab, KeyCode::Char('G')).await;
        assert_eq!(tab.scroll.scroll_offset, 9);
    }

    #[tokio::test]
    async fn test_gg_goes_to_first_line() {
        let lines: Vec<&str> = (0..10).map(|_| "line").collect();
        let mut tab = make_tab(&lines).await;
        tab.scroll.scroll_offset = 9;
        let mode = VisualLineMode {
            anchor: 9,
            count: None,
        };
        // First 'g' sets the flag
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('g')).await;
        assert!(tab.interaction.g_key_pressed);
        assert_eq!(tab.scroll.scroll_offset, 9); // not moved yet
        // Second 'g' jumps to top
        let _ = mode2
            .handle_key(&mut tab, KeyCode::Char('g'), KeyModifiers::NONE)
            .await;
        assert_eq!(tab.scroll.scroll_offset, 0);
        assert!(!tab.interaction.g_key_pressed);
    }

    #[tokio::test]
    async fn test_j_resets_g_key_pressed() {
        let lines: Vec<&str> = (0..10).map(|_| "line").collect();
        let mut tab = make_tab(&lines).await;
        tab.interaction.g_key_pressed = true;
        tab.scroll.scroll_offset = 0;
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        let _ = press(mode, &mut tab, KeyCode::Char('j')).await;
        assert!(!tab.interaction.g_key_pressed);
    }

    #[tokio::test]
    async fn test_unrecognized_key_returns_ignored() {
        let mut tab = make_tab(&["a"]).await;
        let mode = VisualLineMode {
            anchor: 0,
            count: None,
        };
        let (_, result) = press(mode, &mut tab, KeyCode::F(2)).await;
        assert!(matches!(result, KeyResult::Ignored));
    }
}
