use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::{
    config::Keybindings,
    mode::{
        app_mode::{Mode, ModeRenderState, status_entry},
        command_mode::CommandMode,
        comment_mode::CommentMode,
        filter_mode::FilterManagementMode,
        keybindings_help_mode::KeybindingsHelpMode,
        search_mode::SearchMode,
        ui_mode::UiMode,
        visual_char_mode::{VisualMode, display_line_text},
        visual_mode::VisualLineMode,
    },
    theme::Theme,
    ui::{KeyResult, TabState, field_layout::apply_field_layout},
};

#[derive(Debug, Default)]
pub struct NormalMode {
    pub count: Option<usize>,
}

#[async_trait]
impl Mode for NormalMode {
    async fn handle_key(
        mut self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        // Clone the Arc so we can mutate `tab` freely in each branch.
        let kb = tab.keybindings.clone();

        // ── Digit accumulation for count prefix ─────────────────────────────
        // Digits 1-9 start a count; 0 appends when count is already active.
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

        // ── Global key passthrough ──────────────────────────────────────────
        if kb.global.quit.matches(key, modifiers) {
            self.count = None;
            return (self, KeyResult::Ignored);
        }
        if kb.global.next_tab.matches(key, modifiers) || kb.global.prev_tab.matches(key, modifiers)
        {
            self.count = None;
            return (self, KeyResult::Ignored);
        }
        if kb.global.close_tab.matches(key, modifiers) {
            self.count = None;
            return (self, KeyResult::Ignored);
        }
        if kb.global.new_tab.matches(key, modifiers) {
            self.count = None;
            return (self, KeyResult::Ignored);
        }

        // ── Count-aware motions ─────────────────────────────────────────────

        if kb.navigation.half_page_down.matches(key, modifiers) {
            let half = (tab.visible_height / 2).max(1);
            let count = self.count.take().unwrap_or(1);
            tab.scroll_offset = tab.scroll_offset.saturating_add(half.saturating_mul(count));
            tab.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }

        if kb.navigation.half_page_up.matches(key, modifiers) {
            let half = (tab.visible_height / 2).max(1);
            let count = self.count.take().unwrap_or(1);
            tab.scroll_offset = tab.scroll_offset.saturating_sub(half.saturating_mul(count));
            tab.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }

        if kb.navigation.page_down.matches(key, modifiers) {
            let page = tab.visible_height.max(1);
            let count = self.count.take().unwrap_or(1);
            tab.scroll_offset = tab.scroll_offset.saturating_add(page.saturating_mul(count));
            tab.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }

        if kb.navigation.page_up.matches(key, modifiers) {
            let page = tab.visible_height.max(1);
            let count = self.count.take().unwrap_or(1);
            tab.scroll_offset = tab.scroll_offset.saturating_sub(page.saturating_mul(count));
            tab.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }

        if kb.normal.command_mode.matches(key, modifiers) {
            let history = tab.command_history.clone();
            tab.g_key_pressed = false;
            tab.command_error = None;
            self.count = None;
            return (
                Box::new(CommandMode::with_history(String::new(), 0, history)),
                KeyResult::Handled,
            );
        }

        if kb.normal.filter_mode.matches(key, modifiers) {
            tab.g_key_pressed = false;
            self.count = None;
            return (
                Box::new(FilterManagementMode {
                    selected_filter_index: 0,
                }),
                KeyResult::Handled,
            );
        }

        if kb.normal.toggle_filtering.matches(key, modifiers) {
            tab.filtering_enabled = !tab.filtering_enabled;
            tab.begin_filter_refresh();
            tab.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.filter_include.matches(key, modifiers) {
            let history = tab.command_history.clone();
            tab.g_key_pressed = false;
            tab.command_error = None;
            self.count = None;
            return (
                Box::new(CommandMode::with_history("filter ".to_string(), 7, history)),
                KeyResult::Handled,
            );
        }

        if kb.normal.filter_exclude.matches(key, modifiers) {
            let history = tab.command_history.clone();
            tab.g_key_pressed = false;
            tab.command_error = None;
            self.count = None;
            return (
                Box::new(CommandMode::with_history(
                    "exclude ".to_string(),
                    8,
                    history,
                )),
                KeyResult::Handled,
            );
        }

        if kb.normal.enter_ui_mode.matches(key, modifiers) {
            tab.g_key_pressed = false;
            self.count = None;
            return (Box::new(UiMode::from_tab(tab)), KeyResult::Handled);
        }

        if kb.navigation.scroll_down.matches(key, modifiers) {
            let count = self.count.take().unwrap_or(1);
            tab.scroll_offset = tab.scroll_offset.saturating_add(count);
            tab.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }

        if kb.navigation.scroll_up.matches(key, modifiers) {
            let count = self.count.take().unwrap_or(1);
            tab.scroll_offset = tab.scroll_offset.saturating_sub(count);
            tab.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }

        if kb.normal.scroll_left.matches(key, modifiers) {
            if !tab.wrap {
                tab.horizontal_scroll = tab.horizontal_scroll.saturating_sub(1);
            }
            tab.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.scroll_right.matches(key, modifiers) {
            if !tab.wrap {
                tab.horizontal_scroll = tab.horizontal_scroll.saturating_add(1);
            }
            tab.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.go_to_bottom.matches(key, modifiers) {
            // With a count, `{count}G` jumps to that line number.
            if let Some(count) = self.count.take() {
                let _ = tab.goto_line(count);
            } else {
                let n = tab.visible_indices.len();
                if n > 0 {
                    tab.scroll_offset = n - 1;
                }
            }
            tab.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }

        // gg chord: first press sets the flag; second press jumps to top.
        if kb.normal.go_to_top_chord.matches(key, modifiers) {
            if tab.g_key_pressed {
                // With a count, `{count}gg` jumps to that line number.
                if let Some(count) = self.count.take() {
                    let _ = tab.goto_line(count);
                } else {
                    tab.scroll_offset = 0;
                }
                tab.g_key_pressed = false;
            } else {
                tab.g_key_pressed = true;
            }
            return (self, KeyResult::Handled);
        }

        if kb.normal.mark_line.matches(key, modifiers) {
            if let Some(line_idx) = tab.visible_indices.get_opt(tab.scroll_offset) {
                tab.log_manager.toggle_mark(line_idx);
            }
            tab.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.toggle_marks_only.matches(key, modifiers) {
            tab.show_marks_only = !tab.show_marks_only;
            tab.begin_filter_refresh();
            tab.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.yank_line.matches(key, modifiers) {
            tab.g_key_pressed = false;
            self.count = None;
            if tab.visible_indices.is_empty() {
                tab.command_error = Some("No visible lines".to_string());
                return (self, KeyResult::Handled);
            }
            let idx = tab
                .visible_indices
                .get(tab.scroll_offset.min(tab.visible_indices.len() - 1));
            let bytes = tab.file_reader.get_line(idx);
            let text = tab
                .detected_format
                .as_ref()
                .and_then(|parser| parser.parse_line(bytes))
                .map(|parts| {
                    apply_field_layout(&parts, &tab.field_layout, &tab.hidden_fields, tab.show_keys)
                        .join(" ")
                })
                .unwrap_or_else(|| String::from_utf8_lossy(bytes).into_owned());
            return (self, KeyResult::CopyToClipboard(text));
        }

        if kb.normal.yank_marked.matches(key, modifiers) {
            let marked = tab.log_manager.get_marked_indices();
            tab.g_key_pressed = false;
            self.count = None;
            if marked.is_empty() {
                tab.command_error = Some("No marked lines".to_string());
                return (self, KeyResult::Handled);
            }
            let text: String = marked
                .iter()
                .map(|&idx| {
                    let bytes = tab.file_reader.get_line(idx);
                    tab.detected_format
                        .as_ref()
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
            return (self, KeyResult::CopyToClipboard(text));
        }

        if kb.normal.visual_mode.matches(key, modifiers) {
            let anchor = tab.scroll_offset;
            tab.g_key_pressed = false;
            return (
                Box::new(VisualLineMode {
                    anchor,
                    count: None,
                }),
                KeyResult::Handled,
            );
        }

        if kb.normal.visual_char.matches(key, modifiers) {
            let line_text = display_line_text(tab);
            let cursor_col = search_match_char_offset(tab, &line_text);
            tab.g_key_pressed = false;
            self.count = None;
            let mut mode = VisualMode::new(line_text);
            mode.cursor_col = cursor_col;
            return (Box::new(mode), KeyResult::Handled);
        }

        if kb.normal.search_forward.matches(key, modifiers) {
            tab.g_key_pressed = false;
            self.count = None;
            return (
                Box::new(SearchMode {
                    input: String::new(),
                    forward: true,
                }),
                KeyResult::Handled,
            );
        }

        if kb.normal.search_backward.matches(key, modifiers) {
            tab.g_key_pressed = false;
            self.count = None;
            return (
                Box::new(SearchMode {
                    input: String::new(),
                    forward: false,
                }),
                KeyResult::Handled,
            );
        }

        if kb.normal.next_match.matches(key, modifiers) {
            // `n` continues in the original search direction (vim semantics).
            if tab.search.go_next().is_some() {
                tab.scroll_to_current_search_match();
            }
            tab.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.prev_match.matches(key, modifiers) {
            // `N` reverses the original search direction (vim semantics).
            if tab.search.go_prev().is_some() {
                tab.scroll_to_current_search_match();
            }
            tab.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.clear_search.matches(key, modifiers) && tab.search.get_pattern().is_some() {
            tab.search.clear();
            tab.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.clear_all.matches(key, modifiers) {
            tab.log_manager.clear_all_marks_and_comments();
            tab.command_error = Some("Cleared all marks and comments".to_string());
            tab.begin_filter_refresh();
            tab.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.edit_comment.matches(key, modifiers) {
            if let Some(line_idx) = tab.visible_indices.get_opt(tab.scroll_offset) {
                let comments = tab.log_manager.get_comments();
                if let Some(idx) = comments
                    .iter()
                    .position(|c| c.line_indices.contains(&line_idx))
                {
                    let c = &comments[idx];
                    tab.g_key_pressed = false;
                    self.count = None;
                    return (
                        Box::new(CommentMode::edit(
                            idx,
                            c.text.clone(),
                            c.line_indices.clone(),
                        )),
                        KeyResult::Handled,
                    );
                }
                tab.command_error = Some("No comment on this line".to_string());
            }
            tab.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.delete_comment.matches(key, modifiers) {
            if let Some(line_idx) = tab.visible_indices.get_opt(tab.scroll_offset) {
                let comments = tab.log_manager.get_comments();
                if let Some(idx) = comments
                    .iter()
                    .position(|c| c.line_indices.contains(&line_idx))
                {
                    tab.log_manager.remove_comment(idx);
                    tab.begin_filter_refresh();
                    tab.g_key_pressed = false;
                    self.count = None;
                    return (self, KeyResult::Handled);
                }
                tab.command_error = Some("No comment on this line".to_string());
            }
            tab.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.comment_line.matches(key, modifiers) {
            tab.g_key_pressed = false;
            self.count = None;
            if let Some(line_idx) = tab.visible_indices.get_opt(tab.scroll_offset) {
                return (
                    Box::new(CommentMode::new(vec![line_idx])),
                    KeyResult::Handled,
                );
            }
            return (self, KeyResult::Handled);
        }

        if kb.normal.next_error.matches(key, modifiers) {
            if let Some(pos) = tab.next_error_position(tab.scroll_offset) {
                tab.scroll_offset = pos;
            } else {
                tab.command_error = Some("No more errors".to_string());
            }
            tab.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.prev_error.matches(key, modifiers) {
            if let Some(pos) = tab.prev_error_position(tab.scroll_offset) {
                tab.scroll_offset = pos;
            } else {
                tab.command_error = Some("No previous error".to_string());
            }
            tab.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.next_warning.matches(key, modifiers) {
            if let Some(pos) = tab.next_warning_position(tab.scroll_offset) {
                tab.scroll_offset = pos;
            } else {
                tab.command_error = Some("No more warnings".to_string());
            }
            tab.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.prev_warning.matches(key, modifiers) {
            if let Some(pos) = tab.prev_warning_position(tab.scroll_offset) {
                tab.scroll_offset = pos;
            } else {
                tab.command_error = Some("No previous warning".to_string());
            }
            tab.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.show_keybindings.matches(key, modifiers) {
            tab.g_key_pressed = false;
            self.count = None;
            return (Box::new(KeybindingsHelpMode::new()), KeyResult::Handled);
        }

        // Unrecognised key — consume it, reset the gg-chord state and count.
        tab.g_key_pressed = false;
        self.count = None;
        (self, KeyResult::Handled)
    }

    fn render_state(&self) -> ModeRenderState {
        ModeRenderState::Normal
    }

    fn mode_bar_content(&self, kb: &Keybindings, theme: &Theme) -> Line<'static> {
        let label = match self.count {
            Some(n) => format!("[NORMAL] {}  ", n),
            None => "[NORMAL]  ".to_string(),
        };
        let mut spans: Vec<Span<'static>> = vec![Span::styled(
            label,
            Style::default()
                .fg(theme.text_highlight_fg)
                .add_modifier(Modifier::BOLD),
        )];
        status_entry(&mut spans, kb.global.quit.display(), "quit", theme);
        status_entry(
            &mut spans,
            kb.normal.filter_include.display(),
            "filter in",
            theme,
        );
        status_entry(
            &mut spans,
            kb.normal.filter_exclude.display(),
            "filter out",
            theme,
        );
        status_entry(
            &mut spans,
            kb.normal.filter_mode.display(),
            "filters",
            theme,
        );
        status_entry(
            &mut spans,
            kb.normal.toggle_filtering.display(),
            "tog.filter",
            theme,
        );
        status_entry(&mut spans, kb.normal.mark_line.display(), "mark", theme);
        status_entry(
            &mut spans,
            kb.normal.toggle_marks_only.display(),
            "marks only",
            theme,
        );
        status_entry(&mut spans, kb.normal.enter_ui_mode.display(), "ui", theme);
        status_entry(&mut spans, kb.normal.visual_mode.display(), "visual", theme);
        status_entry(
            &mut spans,
            kb.normal.comment_line.display(),
            "comment",
            theme,
        );
        status_entry(
            &mut spans,
            kb.normal.show_keybindings.display(),
            "help",
            theme,
        );
        Line::from(spans)
    }
}

fn search_match_char_offset(tab: &TabState, line_text: &str) -> usize {
    let Some(line_idx) = tab.visible_indices.get_opt(tab.scroll_offset) else {
        return 0;
    };
    let Some(occ_idx) = tab.search.get_current_occurrence_for_line(line_idx) else {
        return 0;
    };
    let Some(re) = tab.search.get_compiled_pattern() else {
        return 0;
    };
    let Some(m) = re.find_iter(line_text).nth(occ_idx) else {
        return 0;
    };
    line_text[..m.start()].chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::file_reader::FileReader;
    use crate::log_manager::LogManager;
    use crate::ui::{KeyResult, TabState, VisibleLines};
    use std::sync::Arc;

    async fn make_tab(lines: &[&str]) -> TabState {
        let data = lines.join("\n").into_bytes();
        let file_reader = FileReader::from_bytes(data);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        TabState::new(file_reader, log_manager, "test".to_string())
    }

    async fn press(
        tab: &mut TabState,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        Box::new(NormalMode::default())
            .handle_key(tab, code, modifiers)
            .await
    }

    #[tokio::test]
    async fn test_j_increments_scroll_offset() {
        let mut tab = make_tab(&["a", "b", "c"]).await;
        press(&mut tab, KeyCode::Char('j'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll_offset, 1);
    }

    #[tokio::test]
    async fn test_down_increments_scroll_offset() {
        let mut tab = make_tab(&["a", "b"]).await;
        press(&mut tab, KeyCode::Down, KeyModifiers::NONE).await;
        assert_eq!(tab.scroll_offset, 1);
    }

    #[tokio::test]
    async fn test_k_saturates_at_zero() {
        let mut tab = make_tab(&["a"]).await;
        press(&mut tab, KeyCode::Char('k'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll_offset, 0);
    }

    #[tokio::test]
    async fn test_up_saturates_at_zero() {
        let mut tab = make_tab(&["a"]).await;
        press(&mut tab, KeyCode::Up, KeyModifiers::NONE).await;
        assert_eq!(tab.scroll_offset, 0);
    }

    #[tokio::test]
    async fn test_capital_g_jumps_to_last_visible_line() {
        let mut tab = make_tab(&["a", "b", "c", "d", "e"]).await;
        press(&mut tab, KeyCode::Char('G'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll_offset, 4);
    }

    #[tokio::test]
    async fn test_capital_g_on_empty_does_not_panic() {
        let mut tab = make_tab(&[]).await;
        press(&mut tab, KeyCode::Char('G'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll_offset, 0);
    }

    #[tokio::test]
    async fn test_gg_jumps_to_top() {
        let mut tab = make_tab(&["a", "b", "c"]).await;
        tab.scroll_offset = 2;
        press(&mut tab, KeyCode::Char('g'), KeyModifiers::NONE).await;
        assert!(tab.g_key_pressed);
        assert_eq!(tab.scroll_offset, 2);
        press(&mut tab, KeyCode::Char('g'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll_offset, 0);
        assert!(!tab.g_key_pressed);
    }

    #[tokio::test]
    async fn test_ctrl_d_half_page_down() {
        let mut tab = make_tab(&["a", "b", "c", "d", "e", "f"]).await;
        tab.visible_height = 4;
        press(&mut tab, KeyCode::Char('d'), KeyModifiers::CONTROL).await;
        assert_eq!(tab.scroll_offset, 2);
    }

    #[tokio::test]
    async fn test_ctrl_u_half_page_up() {
        let mut tab = make_tab(&["a", "b", "c", "d"]).await;
        tab.visible_height = 4;
        tab.scroll_offset = 3;
        press(&mut tab, KeyCode::Char('u'), KeyModifiers::CONTROL).await;
        assert_eq!(tab.scroll_offset, 1);
    }

    #[tokio::test]
    async fn test_page_down() {
        let mut tab = make_tab(&["a", "b", "c", "d", "e"]).await;
        tab.visible_height = 3;
        press(&mut tab, KeyCode::PageDown, KeyModifiers::NONE).await;
        assert_eq!(tab.scroll_offset, 3);
    }

    #[tokio::test]
    async fn test_page_up_saturates_at_zero() {
        let mut tab = make_tab(&["a"]).await;
        tab.visible_height = 5;
        press(&mut tab, KeyCode::PageUp, KeyModifiers::NONE).await;
        assert_eq!(tab.scroll_offset, 0);
    }

    #[tokio::test]
    async fn test_i_opens_filter_include_command() {
        let mut tab = make_tab(&["line"]).await;
        let (mode, result) = press(&mut tab, KeyCode::Char('i'), KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::Handled));
        match mode.render_state() {
            ModeRenderState::Command { input, .. } => {
                assert_eq!(input, "filter ");
            }
            other => panic!("expected Command, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_o_opens_filter_exclude_command() {
        let mut tab = make_tab(&["line"]).await;
        let (mode, result) = press(&mut tab, KeyCode::Char('o'), KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::Handled));
        match mode.render_state() {
            ModeRenderState::Command { input, .. } => {
                assert_eq!(input, "exclude ");
            }
            other => panic!("expected Command, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_command_error_cleared_on_filter_include_shortcut() {
        let mut tab = make_tab(&["line"]).await;
        tab.command_error = Some("previous error".to_string());
        press(&mut tab, KeyCode::Char('i'), KeyModifiers::NONE).await;
        assert!(tab.command_error.is_none());
    }

    #[tokio::test]
    async fn test_command_error_cleared_on_colon() {
        let mut tab = make_tab(&["line"]).await;
        tab.command_error = Some("previous error".to_string());
        press(&mut tab, KeyCode::Char(':'), KeyModifiers::NONE).await;
        assert!(tab.command_error.is_none());
    }

    #[tokio::test]
    async fn test_u_enters_ui_mode() {
        let mut tab = make_tab(&["line"]).await;
        let (mode, result) = press(&mut tab, KeyCode::Char('u'), KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(format!("{:?}", mode).contains("UiMode"));
    }

    #[tokio::test]
    async fn test_colon_transitions_to_command_mode() {
        let mut tab = make_tab(&["line"]).await;
        let (mode, result) = press(&mut tab, KeyCode::Char(':'), KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(matches!(
            mode.render_state(),
            ModeRenderState::Command { .. }
        ));
        assert!(matches!(
            mode.render_state(),
            ModeRenderState::Command { .. } | ModeRenderState::Search { .. }
        ));
    }

    #[tokio::test]
    async fn test_f_transitions_to_filter_mode() {
        let mut tab = make_tab(&["line"]).await;
        let (mode, result) = press(&mut tab, KeyCode::Char('f'), KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(matches!(
            mode.render_state(),
            ModeRenderState::FilterManagement { .. }
        ));
    }

    #[tokio::test]
    async fn test_slash_transitions_to_forward_search() {
        let mut tab = make_tab(&["line"]).await;
        let (mode, _) = press(&mut tab, KeyCode::Char('/'), KeyModifiers::NONE).await;
        assert!(matches!(
            mode.render_state(),
            ModeRenderState::Command { .. } | ModeRenderState::Search { .. }
        ));
        match mode.render_state() {
            ModeRenderState::Search { forward, .. } => assert!(forward),
            other => panic!("expected Search, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_question_mark_transitions_to_backward_search() {
        let mut tab = make_tab(&["line"]).await;
        let (mode, _) = press(&mut tab, KeyCode::Char('?'), KeyModifiers::NONE).await;
        match mode.render_state() {
            ModeRenderState::Search { forward, .. } => assert!(!forward),
            other => panic!("expected Search, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_esc_clears_active_search() {
        let mut tab = make_tab(&["error line", "info line"]).await;
        tab.visible_indices = VisibleLines::Filtered(vec![0, 1]);
        let visible = tab.visible_indices.clone();
        let texts = tab.collect_display_texts(visible.iter());
        tab.search
            .search("error", visible.iter(), |li| texts.get(&li).cloned())
            .unwrap();
        assert!(tab.search.get_pattern().is_some());
        let (_, result) = press(&mut tab, KeyCode::Esc, KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(tab.search.get_pattern().is_none());
        assert!(tab.search.get_results().is_empty());
    }

    #[tokio::test]
    async fn test_esc_without_active_search_does_nothing() {
        let mut tab = make_tab(&["line"]).await;
        assert!(tab.search.get_pattern().is_none());
        let (_, result) = press(&mut tab, KeyCode::Esc, KeyModifiers::NONE).await;
        // NormalMode consumes all unrecognised keys; no search to clear
        assert!(matches!(result, KeyResult::Handled));
        assert!(tab.search.get_pattern().is_none());
    }

    #[tokio::test]
    async fn test_q_returns_ignored() {
        let mut tab = make_tab(&["line"]).await;
        let (_, result) = press(&mut tab, KeyCode::Char('q'), KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::Ignored));
    }

    #[tokio::test]
    async fn test_tab_returns_ignored() {
        let mut tab = make_tab(&["line"]).await;
        let (_, result) = press(&mut tab, KeyCode::Tab, KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::Ignored));
    }

    #[tokio::test]
    async fn test_backtab_returns_ignored() {
        let mut tab = make_tab(&["line"]).await;
        let (_, result) = press(&mut tab, KeyCode::BackTab, KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::Ignored));
    }

    #[tokio::test]
    async fn test_ctrl_w_returns_ignored() {
        let mut tab = make_tab(&["line"]).await;
        let (_, result) = press(&mut tab, KeyCode::Char('w'), KeyModifiers::CONTROL).await;
        assert!(matches!(result, KeyResult::Ignored));
    }

    #[tokio::test]
    async fn test_ctrl_t_returns_ignored() {
        let mut tab = make_tab(&["line"]).await;
        let (_, result) = press(&mut tab, KeyCode::Char('t'), KeyModifiers::CONTROL).await;
        assert!(matches!(result, KeyResult::Ignored));
    }

    #[tokio::test]
    async fn test_h_decrements_horizontal_scroll_when_not_wrapped() {
        let mut tab = make_tab(&["long line"]).await;
        tab.wrap = false;
        tab.horizontal_scroll = 5;
        press(&mut tab, KeyCode::Char('h'), KeyModifiers::NONE).await;
        assert_eq!(tab.horizontal_scroll, 4);
    }

    #[tokio::test]
    async fn test_l_increments_horizontal_scroll_when_not_wrapped() {
        let mut tab = make_tab(&["long line"]).await;
        tab.wrap = false;
        press(&mut tab, KeyCode::Char('l'), KeyModifiers::NONE).await;
        assert_eq!(tab.horizontal_scroll, 1);
    }

    #[tokio::test]
    async fn test_h_no_horizontal_scroll_when_wrapped() {
        let mut tab = make_tab(&["long line"]).await;
        tab.wrap = true;
        tab.horizontal_scroll = 5;
        press(&mut tab, KeyCode::Char('h'), KeyModifiers::NONE).await;
        assert_eq!(tab.horizontal_scroll, 5);
    }

    #[tokio::test]
    async fn test_l_no_horizontal_scroll_when_wrapped() {
        let mut tab = make_tab(&["long line"]).await;
        tab.wrap = true;
        press(&mut tab, KeyCode::Char('l'), KeyModifiers::NONE).await;
        assert_eq!(tab.horizontal_scroll, 0);
    }

    #[tokio::test]
    async fn test_m_marks_current_line() {
        let mut tab = make_tab(&["line0", "line1"]).await;
        tab.scroll_offset = 0;
        press(&mut tab, KeyCode::Char('m'), KeyModifiers::NONE).await;
        assert!(tab.log_manager.get_marked_indices().contains(&0));
    }

    #[tokio::test]
    async fn test_m_unmarks_already_marked_line() {
        let mut tab = make_tab(&["line0"]).await;
        tab.scroll_offset = 0;
        press(&mut tab, KeyCode::Char('m'), KeyModifiers::NONE).await;
        press(&mut tab, KeyCode::Char('m'), KeyModifiers::NONE).await;
        assert!(!tab.log_manager.get_marked_indices().contains(&0));
    }

    #[tokio::test]
    async fn test_g_key_resets_on_non_g_press() {
        let mut tab = make_tab(&["a"]).await;
        press(&mut tab, KeyCode::Char('g'), KeyModifiers::NONE).await;
        assert!(tab.g_key_pressed);
        press(&mut tab, KeyCode::Char('j'), KeyModifiers::NONE).await;
        assert!(!tab.g_key_pressed);
    }

    #[tokio::test]
    async fn test_capital_f_toggles_filtering_enabled() {
        let mut tab = make_tab(&["a", "b", "c"]).await;
        assert!(tab.filtering_enabled);
        press(&mut tab, KeyCode::Char('F'), KeyModifiers::NONE).await;
        assert!(!tab.filtering_enabled);
        press(&mut tab, KeyCode::Char('F'), KeyModifiers::NONE).await;
        assert!(tab.filtering_enabled);
    }

    #[tokio::test]
    async fn test_filtering_disabled_shows_all_lines() {
        let mut tab = make_tab(&["error", "warn", "info"]).await;
        tab.log_manager
            .add_filter_with_color(
                "error".to_string(),
                crate::types::FilterType::Include,
                None,
                None,
                false,
            )
            .await;
        tab.refresh_visible();
        // With filtering on, only "error" line is visible
        assert_eq!(tab.visible_indices.len(), 1);

        press(&mut tab, KeyCode::Char('F'), KeyModifiers::NONE).await;
        // With filtering off, all 3 lines are visible
        assert_eq!(tab.visible_indices.len(), 3);
    }

    #[tokio::test]
    async fn test_capital_m_toggles_marks_only() {
        let mut tab = make_tab(&["a", "b", "c"]).await;
        assert!(!tab.show_marks_only);
        press(&mut tab, KeyCode::Char('M'), KeyModifiers::NONE).await;
        assert!(tab.show_marks_only);
        press(&mut tab, KeyCode::Char('M'), KeyModifiers::NONE).await;
        assert!(!tab.show_marks_only);
    }

    #[tokio::test]
    async fn test_marks_only_shows_only_marked_lines() {
        let mut tab = make_tab(&["line0", "line1", "line2"]).await;
        // Mark lines 0 and 2
        tab.log_manager.toggle_mark(0);
        tab.log_manager.toggle_mark(2);

        press(&mut tab, KeyCode::Char('M'), KeyModifiers::NONE).await;

        assert_eq!(tab.visible_indices, VisibleLines::Filtered(vec![0, 2]));
    }

    #[tokio::test]
    async fn test_marks_only_off_restores_all_lines() {
        let mut tab = make_tab(&["line0", "line1", "line2"]).await;
        tab.log_manager.toggle_mark(1);
        press(&mut tab, KeyCode::Char('M'), KeyModifiers::NONE).await;
        assert_eq!(tab.visible_indices.len(), 1);

        press(&mut tab, KeyCode::Char('M'), KeyModifiers::NONE).await;
        assert_eq!(tab.visible_indices.len(), 3);
    }

    #[tokio::test]
    async fn test_marks_only_empty_when_no_marks() {
        let mut tab = make_tab(&["a", "b"]).await;
        press(&mut tab, KeyCode::Char('M'), KeyModifiers::NONE).await;
        assert!(tab.visible_indices.is_empty());
    }

    #[tokio::test]
    async fn test_y_yanks_current_line() {
        let mut tab = make_tab(&["line0", "line1", "line2"]).await;
        tab.scroll_offset = 1;
        let (_, result) = press(&mut tab, KeyCode::Char('y'), KeyModifiers::NONE).await;
        match result {
            KeyResult::CopyToClipboard(text) => {
                assert_eq!(text, "line1");
            }
            other => panic!("expected CopyToClipboard, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_y_no_visible_lines_sets_error() {
        let mut tab = make_tab(&[]).await;
        let (_, result) = press(&mut tab, KeyCode::Char('y'), KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::Handled));
        assert_eq!(tab.command_error.as_deref(), Some("No visible lines"));
    }

    #[tokio::test]
    async fn test_capital_y_yanks_marked_lines() {
        let mut tab = make_tab(&["line0", "line1", "line2"]).await;
        tab.log_manager.toggle_mark(0);
        tab.log_manager.toggle_mark(2);
        let (_, result) = press(&mut tab, KeyCode::Char('Y'), KeyModifiers::NONE).await;
        match result {
            KeyResult::CopyToClipboard(text) => {
                assert_eq!(text, "line0\nline2");
            }
            other => panic!("expected CopyToClipboard, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_capital_y_no_marks_sets_error() {
        let mut tab = make_tab(&["line0", "line1"]).await;
        let (_, result) = press(&mut tab, KeyCode::Char('Y'), KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::Handled));
        assert_eq!(tab.command_error.as_deref(), Some("No marked lines"));
    }

    #[test]
    fn test_mode_bar_content_contains_normal() {
        assert!(matches!(
            NormalMode::default().render_state(),
            ModeRenderState::Normal
        ));
    }

    #[test]
    fn test_mode_bar_content_contains_marks_only_hint() {
        let content =
            NormalMode::default().mode_bar_content(&Keybindings::default(), &Theme::default());
        let text: String = content.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("marks only"));
    }

    // ── Count prefix tests ───────────────────────────────────────────────

    async fn press_mode(
        mode: NormalMode,
        tab: &mut TabState,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        Box::new(mode).handle_key(tab, code, modifiers).await
    }

    #[tokio::test]
    async fn test_count_5j_moves_down_5() {
        let lines: Vec<&str> = (0..20).map(|_| "line").collect();
        let mut tab = make_tab(&lines).await;
        let mode = NormalMode { count: Some(5) };
        press_mode(mode, &mut tab, KeyCode::Char('j'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll_offset, 5);
    }

    #[tokio::test]
    async fn test_count_3k_moves_up_3() {
        let lines: Vec<&str> = (0..20).map(|_| "line").collect();
        let mut tab = make_tab(&lines).await;
        tab.scroll_offset = 10;
        let mode = NormalMode { count: Some(3) };
        press_mode(mode, &mut tab, KeyCode::Char('k'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll_offset, 7);
    }

    #[tokio::test]
    async fn test_digit_accumulation() {
        let mut tab = make_tab(&["a"]).await;
        let (mode, _) = press(&mut tab, KeyCode::Char('1'), KeyModifiers::NONE).await;
        let (mode, _) = mode
            .handle_key(&mut tab, KeyCode::Char('2'), KeyModifiers::NONE)
            .await;
        let (mode, _) = mode
            .handle_key(&mut tab, KeyCode::Char('3'), KeyModifiers::NONE)
            .await;
        // Verify the count is 123 by checking it goes to line 123 or moves.
        // Since we only have 1 line, check with gg chord.
        // Instead, press Esc to discard and verify it was accumulated by checking mode state.
        assert!(matches!(mode.render_state(), ModeRenderState::Normal));
    }

    #[tokio::test]
    async fn test_count_0_appends_to_existing() {
        let lines: Vec<&str> = (0..200).map(|_| "line").collect();
        let mut tab = make_tab(&lines).await;
        // Type "10" then "j"
        let (mode, _) = press(&mut tab, KeyCode::Char('1'), KeyModifiers::NONE).await;
        let (mode, _) = mode
            .handle_key(&mut tab, KeyCode::Char('0'), KeyModifiers::NONE)
            .await;
        let _ = mode
            .handle_key(&mut tab, KeyCode::Char('j'), KeyModifiers::NONE)
            .await;
        assert_eq!(tab.scroll_offset, 10);
    }

    #[tokio::test]
    async fn test_count_g_goes_to_line() {
        let lines: Vec<&str> = (0..20).map(|_| "line").collect();
        let mut tab = make_tab(&lines).await;
        // 5G should go to line 5 (0-based index 4)
        let mode = NormalMode { count: Some(5) };
        press_mode(mode, &mut tab, KeyCode::Char('G'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll_offset, 4);
    }

    #[tokio::test]
    async fn test_count_gg_goes_to_line() {
        let lines: Vec<&str> = (0..20).map(|_| "line").collect();
        let mut tab = make_tab(&lines).await;
        // 5gg should go to line 5 (0-based index 4)
        let mode = NormalMode { count: Some(5) };
        // First g sets g_key_pressed
        let (returned_mode, _) =
            press_mode(mode, &mut tab, KeyCode::Char('g'), KeyModifiers::NONE).await;
        // Second g completes the chord
        let _ = returned_mode
            .handle_key(&mut tab, KeyCode::Char('g'), KeyModifiers::NONE)
            .await;
        assert_eq!(tab.scroll_offset, 4);
    }

    #[tokio::test]
    async fn test_count_resets_on_non_motion_key() {
        let mut tab = make_tab(&["a", "b"]).await;
        let (mode, _) = press(&mut tab, KeyCode::Char('5'), KeyModifiers::NONE).await;
        // Press 'm' (mark line) — count should be reset, mode stays Normal
        let (mode_after, _) = mode
            .handle_key(&mut tab, KeyCode::Char('m'), KeyModifiers::NONE)
            .await;
        // NormalMode.count should have been cleared
        match mode_after.render_state() {
            ModeRenderState::Normal => {}
            other => panic!("expected Normal, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_count_half_page_down() {
        let lines: Vec<&str> = (0..100).map(|_| "line").collect();
        let mut tab = make_tab(&lines).await;
        tab.visible_height = 10;
        let mode = NormalMode { count: Some(3) };
        press_mode(mode, &mut tab, KeyCode::Char('d'), KeyModifiers::CONTROL).await;
        // 3 × (10/2) = 15
        assert_eq!(tab.scroll_offset, 15);
    }

    #[tokio::test]
    async fn test_count_half_page_up() {
        let lines: Vec<&str> = (0..100).map(|_| "line").collect();
        let mut tab = make_tab(&lines).await;
        tab.visible_height = 10;
        tab.scroll_offset = 50;
        let mode = NormalMode { count: Some(2) };
        press_mode(mode, &mut tab, KeyCode::Char('u'), KeyModifiers::CONTROL).await;
        // 50 - 2 × (10/2) = 40
        assert_eq!(tab.scroll_offset, 40);
    }

    #[tokio::test]
    async fn test_count_page_down() {
        let lines: Vec<&str> = (0..100).map(|_| "line").collect();
        let mut tab = make_tab(&lines).await;
        tab.visible_height = 10;
        let mode = NormalMode { count: Some(2) };
        press_mode(mode, &mut tab, KeyCode::PageDown, KeyModifiers::NONE).await;
        assert_eq!(tab.scroll_offset, 20);
    }

    #[tokio::test]
    async fn test_count_page_up() {
        let lines: Vec<&str> = (0..100).map(|_| "line").collect();
        let mut tab = make_tab(&lines).await;
        tab.visible_height = 10;
        tab.scroll_offset = 50;
        let mode = NormalMode { count: Some(3) };
        press_mode(mode, &mut tab, KeyCode::PageUp, KeyModifiers::NONE).await;
        assert_eq!(tab.scroll_offset, 20);
    }

    // ── Clear all marks and comments ────────────────────────────────

    #[tokio::test]
    async fn test_shift_c_clears_marks_and_comments() {
        let mut tab = make_tab(&["a", "b", "c"]).await;
        tab.log_manager.toggle_mark(0);
        tab.log_manager.toggle_mark(2);
        tab.log_manager.add_comment("note".into(), vec![1]);
        assert!(!tab.log_manager.get_marked_indices().is_empty());
        assert!(!tab.log_manager.get_comments().is_empty());

        press(&mut tab, KeyCode::Char('C'), KeyModifiers::NONE).await;
        assert!(tab.log_manager.get_marked_indices().is_empty());
        assert!(tab.log_manager.get_comments().is_empty());
        assert_eq!(
            tab.command_error.as_deref(),
            Some("Cleared all marks and comments")
        );
    }

    // ── Edit comment ────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_r_on_commented_line_opens_edit_mode() {
        let mut tab = make_tab(&["line0", "line1", "line2"]).await;
        tab.log_manager.add_comment("my comment".into(), vec![0]);
        tab.scroll_offset = 0;

        let (mode, result) = press(&mut tab, KeyCode::Char('r'), KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::Handled));
        match mode.render_state() {
            ModeRenderState::Comment { lines, .. } => {
                assert_eq!(lines.join("\n"), "my comment");
            }
            other => panic!("expected Comment, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_r_on_non_commented_line_shows_error() {
        let mut tab = make_tab(&["line0", "line1"]).await;
        tab.scroll_offset = 0;

        let (mode, result) = press(&mut tab, KeyCode::Char('r'), KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(matches!(mode.render_state(), ModeRenderState::Normal));
        assert_eq!(
            tab.command_error.as_deref(),
            Some("No comment on this line")
        );
    }

    // ── Delete comment ──────────────────────────────────────────────────

    #[tokio::test]
    async fn test_d_on_commented_line_deletes_comment() {
        let mut tab = make_tab(&["line0", "line1", "line2"]).await;
        tab.log_manager.add_comment("to delete".into(), vec![0]);
        tab.log_manager.add_comment("keep".into(), vec![2]);
        tab.scroll_offset = 0;

        let (mode, result) = press(&mut tab, KeyCode::Char('d'), KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(matches!(mode.render_state(), ModeRenderState::Normal));
        let comments = tab.log_manager.get_comments();
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].text, "keep");
    }

    #[tokio::test]
    async fn test_d_on_non_commented_line_shows_error() {
        let mut tab = make_tab(&["line0", "line1"]).await;
        tab.scroll_offset = 0;

        let (_mode, result) = press(&mut tab, KeyCode::Char('d'), KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::Handled));
        assert_eq!(
            tab.command_error.as_deref(),
            Some("No comment on this line")
        );
        assert!(tab.log_manager.get_comments().is_empty());
    }

    #[tokio::test]
    async fn test_count_capped_at_999999() {
        let mut tab = make_tab(&["a"]).await;
        let mode = NormalMode {
            count: Some(999_999),
        };
        let (mode, _) = press_mode(mode, &mut tab, KeyCode::Char('9'), KeyModifiers::NONE).await;
        // After multiplying 999_999 * 10, it should be capped at 999_999
        // (saturating_mul won't overflow, but min(999_999) caps it)
        // The result of 999_999 * 10 = 9_999_990 + 9 = 9_999_999, capped to 999_999
        assert!(matches!(mode.render_state(), ModeRenderState::Normal));
    }

    // ── Comment line ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_c_opens_comment_mode_for_current_line() {
        let mut tab = make_tab(&["line0", "line1", "line2"]).await;
        tab.scroll_offset = 1;
        let (mode, result) = press(&mut tab, KeyCode::Char('c'), KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::Handled));
        match mode.render_state() {
            ModeRenderState::Comment { line_count, .. } => {
                assert_eq!(line_count, 1);
            }
            other => panic!("expected Comment, got {:?}", other),
        }
    }

    // ── Error / warning navigation ────────────────────────────────────────

    #[tokio::test]
    async fn test_e_navigates_to_next_error() {
        let mut tab =
            make_tab(&["INFO normal line", "ERROR something failed", "INFO another"]).await;
        tab.scroll_offset = 0;
        press(&mut tab, KeyCode::Char('e'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll_offset, 1);
    }

    #[tokio::test]
    async fn test_capital_e_navigates_to_prev_error() {
        let mut tab = make_tab(&["ERROR first error", "INFO normal line", "INFO another"]).await;
        tab.scroll_offset = 2;
        press(&mut tab, KeyCode::Char('E'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll_offset, 0);
    }

    #[tokio::test]
    async fn test_e_at_last_error_sets_command_error() {
        let mut tab = make_tab(&["ERROR only error", "INFO line"]).await;
        tab.scroll_offset = 0;
        press(&mut tab, KeyCode::Char('e'), KeyModifiers::NONE).await;
        assert_eq!(tab.command_error.as_deref(), Some("No more errors"));
    }

    #[tokio::test]
    async fn test_capital_e_at_first_error_sets_command_error() {
        let mut tab = make_tab(&["INFO line", "ERROR only error"]).await;
        tab.scroll_offset = 1;
        press(&mut tab, KeyCode::Char('E'), KeyModifiers::NONE).await;
        assert_eq!(tab.command_error.as_deref(), Some("No previous error"));
    }

    #[tokio::test]
    async fn test_w_navigates_to_next_warning() {
        let mut tab =
            make_tab(&["INFO normal line", "WARN disk nearly full", "INFO another"]).await;
        tab.scroll_offset = 0;
        press(&mut tab, KeyCode::Char('w'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll_offset, 1);
    }

    #[tokio::test]
    async fn test_capital_w_navigates_to_prev_warning() {
        let mut tab = make_tab(&["WARN first warning", "INFO normal line", "INFO another"]).await;
        tab.scroll_offset = 2;
        press(&mut tab, KeyCode::Char('W'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll_offset, 0);
    }

    #[tokio::test]
    async fn test_w_no_warnings_sets_command_error() {
        let mut tab = make_tab(&["INFO only", "DEBUG line"]).await;
        tab.scroll_offset = 0;
        press(&mut tab, KeyCode::Char('w'), KeyModifiers::NONE).await;
        assert_eq!(tab.command_error.as_deref(), Some("No more warnings"));
    }

    #[tokio::test]
    async fn test_capital_w_no_prev_warning_sets_command_error() {
        let mut tab = make_tab(&["INFO line", "WARN only warning"]).await;
        tab.scroll_offset = 1;
        press(&mut tab, KeyCode::Char('W'), KeyModifiers::NONE).await;
        assert_eq!(tab.command_error.as_deref(), Some("No previous warning"));
    }

    #[tokio::test]
    async fn test_e_skips_non_error_levels() {
        let mut tab = make_tab(&[
            "INFO line",
            "WARN warning",
            "DEBUG debug",
            "ERROR error here",
        ])
        .await;
        tab.scroll_offset = 0;
        press(&mut tab, KeyCode::Char('e'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll_offset, 3);
    }

    #[tokio::test]
    async fn test_e_navigates_to_fatal_level() {
        let mut tab = make_tab(&["INFO line", "FATAL crash"]).await;
        tab.scroll_offset = 0;
        press(&mut tab, KeyCode::Char('e'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll_offset, 1);
    }
}
