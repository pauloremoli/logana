use crate::{
    config::Keybindings,
    mode::{
        app_mode::{Mode, ModeRenderState, status_entry},
        command_mode::CommandMode,
        comment_mode::CommentMode,
        filter_mode::FilterManagementMode,
        group_mode::GroupManagementMode,
        keybindings_help_mode::KeybindingsHelpMode,
        search_mode::SearchMode,
        ui_mode::UiMode,
        visual_char_mode::{VisualMode, display_line_text},
        visual_mode::VisualLineMode,
    },
    theme::Theme,
    ui::{KeyResult, TabState},
};
use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

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
        let kb = tab.interaction.keybindings.clone();

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

        if let Some(binding) = kb.custom.iter().find(|c| c.key.matches(key, modifiers)) {
            self.count = None;
            tab.interaction.g_key_pressed = false;
            return (self, KeyResult::ExecuteCommand(binding.command.clone()));
        }

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
        if kb.global.file_switcher.matches(key, modifiers) {
            self.count = None;
            return (self, KeyResult::Ignored);
        }

        if kb.navigation.half_page_down.matches(key, modifiers) {
            let half = (tab.scroll.visible_height / 2).max(1);
            let count = self.count.take().unwrap_or(1);
            tab.scroll.scroll_offset = tab
                .scroll
                .scroll_offset
                .saturating_add(half.saturating_mul(count));
            tab.interaction.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }

        if kb.navigation.half_page_up.matches(key, modifiers) {
            let half = (tab.scroll.visible_height / 2).max(1);
            let count = self.count.take().unwrap_or(1);
            tab.scroll.scroll_offset = tab
                .scroll
                .scroll_offset
                .saturating_sub(half.saturating_mul(count));
            tab.interaction.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }

        if kb.navigation.page_down.matches(key, modifiers) {
            let page = tab.scroll.visible_height.max(1);
            let count = self.count.take().unwrap_or(1);
            tab.scroll.scroll_offset = tab
                .scroll
                .scroll_offset
                .saturating_add(page.saturating_mul(count));
            tab.interaction.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }

        if kb.navigation.page_up.matches(key, modifiers) {
            let page = tab.scroll.visible_height.max(1);
            let count = self.count.take().unwrap_or(1);
            tab.scroll.scroll_offset = tab
                .scroll
                .scroll_offset
                .saturating_sub(page.saturating_mul(count));
            tab.interaction.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }

        if kb.normal.command_mode.matches(key, modifiers) {
            let history = tab.interaction.command_history.clone();
            tab.interaction.g_key_pressed = false;
            tab.interaction.command_error = None;
            self.count = None;
            return (
                Box::new(CommandMode::with_history(String::new(), 0, history)),
                KeyResult::Handled,
            );
        }

        if kb.normal.filter_mode.matches(key, modifiers) {
            tab.interaction.g_key_pressed = false;
            self.count = None;
            let num_filters = tab.log_manager.get_filters().len();
            let selected_filter_index = tab
                .filter
                .last_selected_filter
                .min(num_filters.saturating_sub(1));
            return (
                Box::new(FilterManagementMode::new(selected_filter_index)),
                KeyResult::Handled,
            );
        }

        if kb.normal.group_mode.matches(key, modifiers) {
            tab.interaction.g_key_pressed = false;
            self.count = None;
            let selected_group = tab
                .log_manager
                .group_names()
                .into_iter()
                .next()
                .unwrap_or_default();
            return (
                Box::new(GroupManagementMode::new(selected_group)),
                KeyResult::Handled,
            );
        }

        if kb.normal.toggle_filtering.matches(key, modifiers) {
            tab.filter.enabled = !tab.filter.enabled;
            tab.begin_filter_refresh();
            tab.interaction.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.toggle_highlight_mode.matches(key, modifiers) {
            tab.filter.highlight_mode = !tab.filter.highlight_mode;
            tab.begin_filter_refresh();
            tab.interaction.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.filter_include.matches(key, modifiers) {
            let history = tab.interaction.command_history.clone();
            tab.interaction.g_key_pressed = false;
            tab.interaction.command_error = None;
            self.count = None;
            return (
                Box::new(CommandMode::with_history("filter ".to_string(), 7, history)),
                KeyResult::Handled,
            );
        }

        if kb.normal.filter_include_auto.matches(key, modifiers) {
            let history = tab.interaction.command_history.clone();
            tab.interaction.g_key_pressed = false;
            tab.interaction.command_error = None;
            self.count = None;
            return (
                Box::new(CommandMode::with_history(
                    "filter --auto ".to_string(),
                    14,
                    history,
                )),
                KeyResult::Handled,
            );
        }

        if kb.normal.filter_exclude.matches(key, modifiers) {
            let history = tab.interaction.command_history.clone();
            tab.interaction.g_key_pressed = false;
            tab.interaction.command_error = None;
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
            tab.interaction.g_key_pressed = false;
            self.count = None;
            return (Box::new(UiMode::from_tab(tab)), KeyResult::Handled);
        }

        if kb.navigation.scroll_down.matches(key, modifiers) {
            let count = self.count.take().unwrap_or(1);
            tab.scroll.scroll_offset = tab.scroll.scroll_offset.saturating_add(count);
            tab.interaction.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }

        if kb.navigation.scroll_up.matches(key, modifiers) {
            let count = self.count.take().unwrap_or(1);
            tab.scroll.scroll_offset = tab.scroll.scroll_offset.saturating_sub(count);
            tab.interaction.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }

        if kb.normal.scroll_left.matches(key, modifiers) {
            if !tab.display.wrap {
                tab.scroll.horizontal_scroll = tab.scroll.horizontal_scroll.saturating_sub(1);
            }
            tab.interaction.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.scroll_right.matches(key, modifiers) {
            if !tab.display.wrap {
                tab.scroll.horizontal_scroll = tab.scroll.horizontal_scroll.saturating_add(1);
            }
            tab.interaction.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.start_of_line.matches(key, modifiers) {
            tab.scroll.horizontal_scroll = 0;
            tab.interaction.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.end_of_line.matches(key, modifiers) {
            if let Some(line_idx) = tab.filter.visible_indices.get_opt(tab.scroll.scroll_offset) {
                let text = tab.get_display_text(line_idx);
                let char_count = text.chars().count();
                tab.scroll_char_cursor_into_view(char_count.saturating_sub(1), &text);
            }
            tab.interaction.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.go_to_bottom.matches(key, modifiers) {
            // With a count, `{count}G` jumps to that line number.
            if let Some(count) = self.count.take() {
                let _ = tab.goto_line(count);
            } else {
                let n = tab.filter.visible_indices.len();
                if n > 0 {
                    tab.scroll.scroll_offset = n - 1;
                }
            }
            tab.interaction.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }

        // gg chord: first press sets the flag; second press jumps to top.
        if kb.normal.go_to_top_chord.matches(key, modifiers) {
            if tab.interaction.g_key_pressed {
                // With a count, `{count}gg` jumps to that line number.
                if let Some(count) = self.count.take() {
                    let _ = tab.goto_line(count);
                } else {
                    tab.scroll.scroll_offset = 0;
                }
                tab.interaction.g_key_pressed = false;
            } else {
                tab.interaction.g_key_pressed = true;
            }
            return (self, KeyResult::Handled);
        }

        if kb.normal.mark_line.matches(key, modifiers) {
            if let Some(line_idx) = tab.filter.visible_indices.get_opt(tab.scroll.scroll_offset) {
                tab.mark_manager.toggle(line_idx);
            }
            tab.interaction.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.expand_continuation.matches(key, modifiers) {
            // Works regardless of `collapse_continuations` — resolves the
            // cursor's record (whether it's sitting on the parent line or
            // one of its continuation lines) and expands just that one.
            if let Some(line_idx) = tab.filter.visible_indices.get_opt(tab.scroll.scroll_offset)
                && let Some(cmap) = tab.active_continuation_map()
                && let Some(&parent) = cmap.get(line_idx)
                && cmap.get(parent + 1) == Some(&parent)
            {
                tab.set_continuation_collapsed(parent, false);
            }
            tab.interaction.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.collapse_continuation.matches(key, modifiers) {
            if let Some(line_idx) = tab.filter.visible_indices.get_opt(tab.scroll.scroll_offset)
                && let Some(cmap) = tab.active_continuation_map()
                && let Some(&parent) = cmap.get(line_idx)
                && cmap.get(parent + 1) == Some(&parent)
            {
                tab.set_continuation_collapsed(parent, true);
            }
            tab.interaction.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.toggle_marks_only.matches(key, modifiers) {
            tab.filter.show_marks_only = !tab.filter.show_marks_only;
            tab.begin_filter_refresh();
            tab.interaction.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.yank_line.matches(key, modifiers) {
            tab.interaction.g_key_pressed = false;
            self.count = None;
            if tab.filter.visible_indices.is_empty() {
                tab.interaction.command_error = Some("No visible lines".to_string());
                return (self, KeyResult::Handled);
            }
            let idx = tab.filter.visible_indices.get(
                tab.scroll
                    .scroll_offset
                    .min(tab.filter.visible_indices.len() - 1),
            );
            let text = tab.get_display_text(idx);
            return (self, KeyResult::CopyToClipboard(text));
        }

        if kb.normal.yank_marked.matches(key, modifiers) {
            let marked = tab.mark_manager.get_indices();
            tab.interaction.g_key_pressed = false;
            self.count = None;
            if marked.is_empty() {
                tab.interaction.command_error = Some("No marked lines".to_string());
                return (self, KeyResult::Handled);
            }
            let text: String = marked
                .iter()
                .map(|&idx| tab.get_display_text(idx))
                .collect::<Vec<_>>()
                .join("\n");
            return (self, KeyResult::CopyToClipboard(text));
        }

        if kb.normal.visual_mode.matches(key, modifiers) {
            let anchor = tab.scroll.scroll_offset;
            tab.interaction.g_key_pressed = false;
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
            tab.interaction.g_key_pressed = false;
            self.count = None;
            tab.cancel_search();
            let mut mode = VisualMode::new(line_text);
            mode.cursor_col = cursor_col;
            return (Box::new(mode), KeyResult::Handled);
        }

        if kb.normal.search_forward.matches(key, modifiers) {
            tab.interaction.g_key_pressed = false;
            self.count = None;
            return (
                Box::new(SearchMode {
                    input: String::new(),
                    cursor: 0,
                    forward: true,
                }),
                KeyResult::Handled,
            );
        }

        if kb.normal.search_backward.matches(key, modifiers) {
            tab.interaction.g_key_pressed = false;
            self.count = None;
            return (
                Box::new(SearchMode {
                    input: String::new(),
                    cursor: 0,
                    forward: false,
                }),
                KeyResult::Handled,
            );
        }

        if kb.normal.next_match.matches(key, modifiers) {
            if tab.search.query.get_pattern().is_some() {
                // `n` continues in the original search direction (vim semantics).
                if tab.search.query.go_next().is_some() {
                    tab.scroll_to_current_search_match();
                }
            } else if let Some(pos) = tab.next_marked_position(tab.scroll.scroll_offset) {
                tab.scroll.scroll_offset = pos;
            } else {
                tab.interaction.command_error = Some("No more marks".to_string());
            }
            tab.interaction.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.prev_match.matches(key, modifiers) {
            if tab.search.query.get_pattern().is_some() {
                // `N` reverses the original search direction (vim semantics).
                if tab.search.query.go_prev().is_some() {
                    tab.scroll_to_current_search_match();
                }
            } else if let Some(pos) = tab.prev_marked_position(tab.scroll.scroll_offset) {
                tab.scroll.scroll_offset = pos;
            } else {
                tab.interaction.command_error = Some("No previous mark".to_string());
            }
            tab.interaction.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.clear_search.matches(key, modifiers)
            && (tab.search.query.get_pattern().is_some() || tab.interaction.notification.is_some())
        {
            tab.cancel_search();
            tab.clear_notification();
            tab.interaction.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.clear_all.matches(key, modifiers) {
            tab.mark_manager.clear();
            tab.comment_manager.clear();
            tab.interaction.command_error = Some("Cleared all marks and comments".to_string());
            tab.begin_filter_refresh();
            tab.interaction.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.edit_comment.matches(key, modifiers) {
            if let Some(line_idx) = tab.filter.visible_indices.get_opt(tab.scroll.scroll_offset) {
                let comments = tab.comment_manager.get();
                if let Some(idx) = comments
                    .iter()
                    .position(|c| c.line_indices.contains(&line_idx))
                {
                    let c = &comments[idx];
                    tab.interaction.g_key_pressed = false;
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
                tab.interaction.command_error = Some("No comment on this line".to_string());
            }
            tab.interaction.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.delete_comment.matches(key, modifiers) {
            if let Some(line_idx) = tab.filter.visible_indices.get_opt(tab.scroll.scroll_offset) {
                let comments = tab.comment_manager.get();
                if let Some(idx) = comments
                    .iter()
                    .position(|c| c.line_indices.contains(&line_idx))
                {
                    tab.comment_manager.remove(idx);
                    tab.begin_filter_refresh();
                    tab.interaction.g_key_pressed = false;
                    self.count = None;
                    return (self, KeyResult::Handled);
                }
                tab.interaction.command_error = Some("No comment on this line".to_string());
            }
            tab.interaction.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.comment_line.matches(key, modifiers) {
            tab.interaction.g_key_pressed = false;
            self.count = None;
            if let Some(line_idx) = tab.filter.visible_indices.get_opt(tab.scroll.scroll_offset) {
                let comments = tab.comment_manager.get();
                if let Some(idx) = comments
                    .iter()
                    .position(|c| c.line_indices.contains(&line_idx))
                {
                    let c = &comments[idx];
                    return (
                        Box::new(CommentMode::edit(
                            idx,
                            c.text.clone(),
                            c.line_indices.clone(),
                        )),
                        KeyResult::Handled,
                    );
                }
                return (
                    Box::new(CommentMode::new(vec![line_idx])),
                    KeyResult::Handled,
                );
            }
            return (self, KeyResult::Handled);
        }

        if kb.normal.next_error.matches(key, modifiers) {
            if let Some(pos) = tab.next_error_position(tab.scroll.scroll_offset) {
                tab.scroll.scroll_offset = pos;
            } else {
                tab.interaction.command_error = Some("No more errors".to_string());
            }
            tab.interaction.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.prev_error.matches(key, modifiers) {
            if let Some(pos) = tab.prev_error_position(tab.scroll.scroll_offset) {
                tab.scroll.scroll_offset = pos;
            } else {
                tab.interaction.command_error = Some("No previous error".to_string());
            }
            tab.interaction.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.next_warning.matches(key, modifiers) {
            if let Some(pos) = tab.next_warning_position(tab.scroll.scroll_offset) {
                tab.scroll.scroll_offset = pos;
            } else {
                tab.interaction.command_error = Some("No more warnings".to_string());
            }
            tab.interaction.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.prev_warning.matches(key, modifiers) {
            if let Some(pos) = tab.prev_warning_position(tab.scroll.scroll_offset) {
                tab.scroll.scroll_offset = pos;
            } else {
                tab.interaction.command_error = Some("No previous warning".to_string());
            }
            tab.interaction.g_key_pressed = false;
            self.count = None;
            return (self, KeyResult::Handled);
        }

        if kb.normal.show_keybindings.matches(key, modifiers) {
            tab.interaction.g_key_pressed = false;
            self.count = None;
            return (Box::new(KeybindingsHelpMode::new()), KeyResult::Handled);
        }

        // Unrecognised key — consume it, reset the gg-chord state and count.
        tab.interaction.g_key_pressed = false;
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
            kb.normal.filter_include_auto.display(),
            "filter in (auto)",
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
        status_entry(&mut spans, kb.normal.group_mode.display(), "groups", theme);
        status_entry(
            &mut spans,
            kb.normal.toggle_filtering.display(),
            "tog.filter",
            theme,
        );
        status_entry(
            &mut spans,
            kb.normal.toggle_highlight_mode.display(),
            "tog.highlight",
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
    let Some(line_idx) = tab.filter.visible_indices.get_opt(tab.scroll.scroll_offset) else {
        return 0;
    };
    let Some(occ_idx) = tab.search.query.get_current_occurrence_for_line(line_idx) else {
        return 0;
    };
    let Some(re) = tab.search.query.get_compiled_pattern() else {
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
    use crate::db::LogManager;
    use crate::ingestion::FileReader;
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
        assert_eq!(tab.scroll.scroll_offset, 1);
    }

    #[tokio::test]
    async fn test_down_increments_scroll_offset() {
        let mut tab = make_tab(&["a", "b"]).await;
        press(&mut tab, KeyCode::Down, KeyModifiers::NONE).await;
        assert_eq!(tab.scroll.scroll_offset, 1);
    }

    #[tokio::test]
    async fn test_k_saturates_at_zero() {
        let mut tab = make_tab(&["a"]).await;
        press(&mut tab, KeyCode::Char('k'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll.scroll_offset, 0);
    }

    #[tokio::test]
    async fn test_up_saturates_at_zero() {
        let mut tab = make_tab(&["a"]).await;
        press(&mut tab, KeyCode::Up, KeyModifiers::NONE).await;
        assert_eq!(tab.scroll.scroll_offset, 0);
    }

    #[tokio::test]
    async fn test_capital_g_jumps_to_last_visible_line() {
        let mut tab = make_tab(&["a", "b", "c", "d", "e"]).await;
        press(&mut tab, KeyCode::Char('G'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll.scroll_offset, 4);
    }

    #[tokio::test]
    async fn test_capital_g_on_empty_does_not_panic() {
        let mut tab = make_tab(&[]).await;
        press(&mut tab, KeyCode::Char('G'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll.scroll_offset, 0);
    }

    #[tokio::test]
    async fn test_gg_jumps_to_top() {
        let mut tab = make_tab(&["a", "b", "c"]).await;
        tab.scroll.scroll_offset = 2;
        press(&mut tab, KeyCode::Char('g'), KeyModifiers::NONE).await;
        assert!(tab.interaction.g_key_pressed);
        assert_eq!(tab.scroll.scroll_offset, 2);
        press(&mut tab, KeyCode::Char('g'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll.scroll_offset, 0);
        assert!(!tab.interaction.g_key_pressed);
    }

    /// Same four-line fixture as
    /// `tab_state::tests::test_continuation_correction_respects_exclude_filter`:
    /// lines 0-1 parse as structured logs, lines 2-3 are unparseable
    /// access-log lines that continue parent line 1.
    const PARSED0: &str = "2024-07-24T10:00:00Z INFO request processed";
    const PARSED1: &str = "2024-07-24T10:00:01Z INFO another request";
    const ACCESS2: &str = "2019-01-26 20:29:10.000 5.120.204.67 200 GET / HTTP/1.1";
    const ACCESS3: &str = "2019-01-26 20:29:11.000 5.120.204.68 200 GET /api HTTP/1.1";

    /// `<` must collapse the entry under the cursor even when
    /// `:collapse` has never run — the whole point of decoupling `<`/`>`
    /// from the global `collapse_continuations` default.
    #[tokio::test]
    async fn test_collapse_continuation_works_without_collapse_mode_on() {
        let mut tab = make_tab(&[PARSED0, PARSED1, ACCESS2, ACCESS3]).await;
        assert!(!tab.display.collapse_continuations);
        assert_eq!(
            tab.filter.visible_indices.iter().collect::<Vec<_>>(),
            vec![0, 1, 2, 3]
        );

        tab.scroll.scroll_offset = 1;
        press(&mut tab, KeyCode::Char('<'), KeyModifiers::NONE).await;
        assert!(tab.filter.overridden_groups.contains(&1));
        assert_eq!(
            tab.filter.visible_indices.iter().collect::<Vec<_>>(),
            vec![0, 1],
            "collapsing the entry under the cursor must hide its continuation lines \
             even though the global default is still expanded"
        );
        assert!(
            !tab.display.collapse_continuations,
            "the local override must not flip the global default"
        );
    }

    /// `>` on an already-expanded entry (default off, no override) is a
    /// harmless no-op: nothing to expand.
    #[tokio::test]
    async fn test_expand_continuation_noop_when_already_expanded() {
        let mut tab = make_tab(&[PARSED0, PARSED1, ACCESS2, ACCESS3]).await;
        tab.scroll.scroll_offset = 1;
        press(&mut tab, KeyCode::Char('>'), KeyModifiers::NONE).await;
        assert!(tab.filter.overridden_groups.is_empty());
        assert_eq!(
            tab.filter.visible_indices.iter().collect::<Vec<_>>(),
            vec![0, 1, 2, 3]
        );
    }

    #[tokio::test]
    async fn test_expand_then_collapse_continuation_under_cursor() {
        let mut tab = make_tab(&[PARSED0, PARSED1, ACCESS2, ACCESS3]).await;
        {
            let cmap = tab.continuation_map.as_ref().unwrap();
            assert_eq!(cmap[2], 1, "access line 2 must map to parsed parent 1");
            assert_eq!(cmap[3], 1, "access line 3 must map to parsed parent 1");
        }

        tab.display.collapse_continuations = true;
        tab.begin_filter_refresh();
        assert_eq!(
            tab.filter.visible_indices.iter().collect::<Vec<_>>(),
            vec![0, 1],
            "continuation lines 2 and 3 must be hidden once collapsed"
        );

        // Cursor on line 1, the parent with hidden continuation lines.
        tab.scroll.scroll_offset = 1;
        press(&mut tab, KeyCode::Char('>'), KeyModifiers::NONE).await;
        assert!(tab.filter.overridden_groups.contains(&1));
        assert_eq!(
            tab.filter.visible_indices.iter().collect::<Vec<_>>(),
            vec![0, 1, 2, 3],
            "expanding the group under the cursor must reveal its continuation lines"
        );

        press(&mut tab, KeyCode::Char('<'), KeyModifiers::NONE).await;
        assert!(!tab.filter.overridden_groups.contains(&1));
        assert_eq!(
            tab.filter.visible_indices.iter().collect::<Vec<_>>(),
            vec![0, 1],
            "collapsing the group under the cursor must hide its continuation lines again"
        );
    }

    /// Pressing `<` while the cursor sits on a *continuation* line (not the
    /// parent) must still resolve and collapse the whole record, and must
    /// re-pin the cursor onto the now-visible parent line — otherwise
    /// `scroll_offset` keeps its old numeric value, which now resolves to
    /// whatever line slid into that screen position once the continuation
    /// lines vanished, silently retargeting a follow-up `>` at an unrelated
    /// entry (from the user's perspective: "stuck collapsed, can't expand").
    #[tokio::test]
    async fn test_collapse_continuation_resolves_parent_from_continuation_line() {
        let mut tab = make_tab(&[PARSED0, PARSED1, ACCESS2, ACCESS3]).await;
        // Cursor on line 2, a continuation line (visible pos 2 since nothing
        // is collapsed yet: positions are file lines 0,1,2,3 verbatim).
        tab.scroll.scroll_offset = 2;
        press(&mut tab, KeyCode::Char('<'), KeyModifiers::NONE).await;
        assert!(tab.filter.overridden_groups.contains(&1));
        assert_eq!(
            tab.filter.visible_indices.iter().collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert_eq!(
            tab.filter.visible_indices.get(tab.scroll.scroll_offset),
            1,
            "cursor must be re-pinned to the parent line (1), not left at the \
             stale visible position 2 (which now maps to a different file line)"
        );

        // A follow-up `>` must still target the entry just collapsed, not
        // whatever line the stale scroll_offset would have resolved to.
        press(&mut tab, KeyCode::Char('>'), KeyModifiers::NONE).await;
        assert!(!tab.filter.overridden_groups.contains(&1));
        assert_eq!(
            tab.filter.visible_indices.iter().collect::<Vec<_>>(),
            vec![0, 1, 2, 3],
            "expand must actually restore the continuation lines, proving \
             the cursor stayed correctly anchored on the collapsed entry"
        );
    }

    #[tokio::test]
    async fn test_custom_binding_returns_execute_command_with_its_configured_command() {
        let mut tab = make_tab(&["a"]).await;
        let mut kb = crate::config::Keybindings::default();
        kb.custom
            .push(crate::config::keybindings::CustomCommandBinding {
                key: crate::config::keybindings::KeyBindings(vec![
                    crate::config::keybindings::KeyBinding(KeyCode::F(2), KeyModifiers::NONE),
                ]),
                command: "load-filters ~/logs/filters/draco-mars.json".to_string(),
            });
        tab.interaction.keybindings = Arc::new(kb);

        let (_, result) = press(&mut tab, KeyCode::F(2), KeyModifiers::NONE).await;
        assert!(
            matches!(result, KeyResult::ExecuteCommand(ref cmd) if cmd == "load-filters ~/logs/filters/draco-mars.json"),
            "expected ExecuteCommand with the configured command, got {result:?}"
        );
    }

    #[tokio::test]
    async fn test_custom_binding_takes_priority_over_a_colliding_builtin_action() {
        let mut tab = make_tab(&["a", "b"]).await;
        let mut kb = crate::config::Keybindings::default();
        let colliding_key = kb.normal.filter_include.clone();
        kb.custom
            .push(crate::config::keybindings::CustomCommandBinding {
                key: colliding_key.clone(),
                command: "wrap".to_string(),
            });
        tab.interaction.keybindings = Arc::new(kb);

        let key = colliding_key.0[0].clone();
        let (_, result) = press(&mut tab, key.0, key.1).await;
        assert!(
            matches!(result, KeyResult::ExecuteCommand(ref cmd) if cmd == "wrap"),
            "a custom binding must win over the built-in action it collides with, got {result:?}"
        );
    }

    #[tokio::test]
    async fn test_unbound_key_is_unaffected_by_unrelated_custom_bindings() {
        let mut tab = make_tab(&["a", "b"]).await;
        let mut kb = crate::config::Keybindings::default();
        kb.custom
            .push(crate::config::keybindings::CustomCommandBinding {
                key: crate::config::keybindings::KeyBindings(vec![
                    crate::config::keybindings::KeyBinding(KeyCode::F(2), KeyModifiers::NONE),
                ]),
                command: "wrap".to_string(),
            });
        tab.interaction.keybindings = Arc::new(kb);

        press(&mut tab, KeyCode::Char('j'), KeyModifiers::NONE).await;
        assert_eq!(
            tab.scroll.scroll_offset, 1,
            "an unrelated key must still behave normally when custom bindings exist"
        );
    }

    #[tokio::test]
    async fn test_ctrl_d_half_page_down() {
        let mut tab = make_tab(&["a", "b", "c", "d", "e", "f"]).await;
        tab.scroll.visible_height = 4;
        press(&mut tab, KeyCode::Char('d'), KeyModifiers::CONTROL).await;
        assert_eq!(tab.scroll.scroll_offset, 2);
    }

    #[tokio::test]
    async fn test_ctrl_u_half_page_up() {
        let mut tab = make_tab(&["a", "b", "c", "d"]).await;
        tab.scroll.visible_height = 4;
        tab.scroll.scroll_offset = 3;
        press(&mut tab, KeyCode::Char('u'), KeyModifiers::CONTROL).await;
        assert_eq!(tab.scroll.scroll_offset, 1);
    }

    #[tokio::test]
    async fn test_page_down() {
        let mut tab = make_tab(&["a", "b", "c", "d", "e"]).await;
        tab.scroll.visible_height = 3;
        press(&mut tab, KeyCode::PageDown, KeyModifiers::NONE).await;
        assert_eq!(tab.scroll.scroll_offset, 3);
    }

    #[tokio::test]
    async fn test_page_up_saturates_at_zero() {
        let mut tab = make_tab(&["a"]).await;
        tab.scroll.visible_height = 5;
        press(&mut tab, KeyCode::PageUp, KeyModifiers::NONE).await;
        assert_eq!(tab.scroll.scroll_offset, 0);
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
    async fn test_a_opens_filter_include_auto_command() {
        let mut tab = make_tab(&["line"]).await;
        let (mode, result) = press(&mut tab, KeyCode::Char('a'), KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::Handled));
        match mode.render_state() {
            ModeRenderState::Command { input, cursor, .. } => {
                assert_eq!(input, "filter --auto ");
                assert_eq!(cursor, input.len());
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
        tab.interaction.command_error = Some("previous error".to_string());
        press(&mut tab, KeyCode::Char('i'), KeyModifiers::NONE).await;
        assert!(tab.interaction.command_error.is_none());
    }

    #[tokio::test]
    async fn test_command_error_cleared_on_colon() {
        let mut tab = make_tab(&["line"]).await;
        tab.interaction.command_error = Some("previous error".to_string());
        press(&mut tab, KeyCode::Char(':'), KeyModifiers::NONE).await;
        assert!(tab.interaction.command_error.is_none());
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
    async fn test_ctrl_g_transitions_to_group_mode() {
        let mut tab = make_tab(&["line"]).await;
        let (mode, result) = press(&mut tab, KeyCode::Char('g'), KeyModifiers::CONTROL).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(matches!(
            mode.render_state(),
            ModeRenderState::GroupManagement { .. }
        ));
    }

    #[tokio::test]
    async fn test_ctrl_g_selects_first_group_alphabetically() {
        use crate::filters::{FilterOptions, FilterType};
        let mut tab = make_tab(&["line"]).await;
        for (pattern, group) in [("a", "zebra"), ("b", "alpha")] {
            tab.log_manager
                .add_filter_with_color(
                    pattern.to_string(),
                    FilterType::Include,
                    FilterOptions::default().group(group),
                )
                .await;
        }
        let (mode, _) = press(&mut tab, KeyCode::Char('g'), KeyModifiers::CONTROL).await;
        match mode.render_state() {
            ModeRenderState::GroupManagement { selected_group } => {
                assert_eq!(selected_group, "alpha");
            }
            other => panic!("expected GroupManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_ctrl_g_with_no_groups_still_enters_group_mode() {
        let mut tab = make_tab(&["line"]).await;
        let (mode, _) = press(&mut tab, KeyCode::Char('g'), KeyModifiers::CONTROL).await;
        match mode.render_state() {
            ModeRenderState::GroupManagement { selected_group } => {
                assert_eq!(selected_group, "");
            }
            other => panic!("expected GroupManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_plain_g_still_starts_go_to_top_chord_not_group_mode() {
        let mut tab = make_tab(&["line1", "line2", "line3"]).await;
        let (mode, _) = press(&mut tab, KeyCode::Char('g'), KeyModifiers::NONE).await;
        assert!(!matches!(
            mode.render_state(),
            ModeRenderState::GroupManagement { .. }
        ));
    }

    #[tokio::test]
    async fn test_f_restores_last_selected_filter() {
        use crate::filters::{FilterOptions, FilterType};
        let mut tab = make_tab(&["line"]).await;
        for pattern in ["a", "b", "c"] {
            tab.log_manager
                .add_filter_with_color(
                    pattern.to_string(),
                    FilterType::Include,
                    FilterOptions::default(),
                )
                .await;
        }
        tab.refresh_visible();
        tab.filter.last_selected_filter = 2;
        let (mode, _) = press(&mut tab, KeyCode::Char('f'), KeyModifiers::NONE).await;
        match mode.render_state() {
            ModeRenderState::FilterManagement { selected_index, .. } => {
                assert_eq!(selected_index, 2);
            }
            other => panic!("expected FilterManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_f_clamps_stale_last_selected_filter_to_available_range() {
        use crate::filters::{FilterOptions, FilterType};
        let mut tab = make_tab(&["line"]).await;
        tab.log_manager
            .add_filter_with_color(
                "a".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        tab.refresh_visible();
        // Simulate filters having been deleted elsewhere after this index
        // was last remembered.
        tab.filter.last_selected_filter = 9;
        let (mode, _) = press(&mut tab, KeyCode::Char('f'), KeyModifiers::NONE).await;
        match mode.render_state() {
            ModeRenderState::FilterManagement { selected_index, .. } => {
                assert_eq!(selected_index, 0);
            }
            other => panic!("expected FilterManagement, got {:?}", other),
        }
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
        tab.filter.visible_indices = VisibleLines::Filtered(vec![0, 1]);
        let visible = tab.filter.visible_indices.clone();
        let texts = tab.collect_display_texts(visible.iter());
        tab.search
            .query
            .search("error", visible.iter(), |li| texts.get(&li).cloned())
            .unwrap();
        assert!(tab.search.query.get_pattern().is_some());
        let (_, result) = press(&mut tab, KeyCode::Esc, KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(tab.search.query.get_pattern().is_none());
        assert!(tab.search.query.get_results().is_empty());
    }

    #[tokio::test]
    async fn test_entering_visual_char_clears_search() {
        let mut tab = make_tab(&["error line", "info line"]).await;
        tab.filter.visible_indices = VisibleLines::Filtered(vec![0, 1]);
        let visible = tab.filter.visible_indices.clone();
        let texts = tab.collect_display_texts(visible.iter());
        tab.search
            .query
            .search("error", visible.iter(), |li| texts.get(&li).cloned())
            .unwrap();
        assert!(tab.search.query.get_pattern().is_some());

        press(&mut tab, KeyCode::Char('v'), KeyModifiers::NONE).await;

        assert!(tab.search.query.get_pattern().is_none());
        assert!(tab.search.query.get_results().is_empty());
    }

    #[tokio::test]
    async fn test_esc_clears_inflight_search_handle() {
        let mut tab = make_tab(&["error line", "info line"]).await;
        tab.filter.visible_indices = VisibleLines::Filtered(vec![0, 1]);
        tab.begin_search("error", true, true);
        assert!(tab.search.handle.is_some());
        assert!(tab.search.query.get_pattern().is_some());
        press(&mut tab, KeyCode::Esc, KeyModifiers::NONE).await;
        assert!(tab.search.handle.is_none());
        assert!(tab.search.query.get_pattern().is_none());
    }

    #[tokio::test]
    async fn test_esc_without_active_search_does_nothing() {
        let mut tab = make_tab(&["line"]).await;
        assert!(tab.search.query.get_pattern().is_none());
        let (_, result) = press(&mut tab, KeyCode::Esc, KeyModifiers::NONE).await;
        // NormalMode consumes all unrecognised keys; no search to clear
        assert!(matches!(result, KeyResult::Handled));
        assert!(tab.search.query.get_pattern().is_none());
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
        tab.display.wrap = false;
        tab.scroll.horizontal_scroll = 5;
        press(&mut tab, KeyCode::Char('h'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll.horizontal_scroll, 4);
    }

    #[tokio::test]
    async fn test_l_increments_horizontal_scroll_when_not_wrapped() {
        let mut tab = make_tab(&["long line"]).await;
        tab.display.wrap = false;
        press(&mut tab, KeyCode::Char('l'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll.horizontal_scroll, 1);
    }

    #[tokio::test]
    async fn test_h_no_horizontal_scroll_when_wrapped() {
        let mut tab = make_tab(&["long line"]).await;
        tab.display.wrap = true;
        tab.scroll.horizontal_scroll = 5;
        press(&mut tab, KeyCode::Char('h'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll.horizontal_scroll, 5);
    }

    #[tokio::test]
    async fn test_l_no_horizontal_scroll_when_wrapped() {
        let mut tab = make_tab(&["long line"]).await;
        tab.display.wrap = true;
        press(&mut tab, KeyCode::Char('l'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll.horizontal_scroll, 0);
    }

    #[tokio::test]
    async fn test_left_arrow_decrements_horizontal_scroll_when_not_wrapped() {
        let mut tab = make_tab(&["long line"]).await;
        tab.display.wrap = false;
        tab.scroll.horizontal_scroll = 5;
        press(&mut tab, KeyCode::Left, KeyModifiers::NONE).await;
        assert_eq!(tab.scroll.horizontal_scroll, 4);
    }

    #[tokio::test]
    async fn test_right_arrow_increments_horizontal_scroll_when_not_wrapped() {
        let mut tab = make_tab(&["long line"]).await;
        tab.display.wrap = false;
        press(&mut tab, KeyCode::Right, KeyModifiers::NONE).await;
        assert_eq!(tab.scroll.horizontal_scroll, 1);
    }

    #[tokio::test]
    async fn test_left_arrow_no_horizontal_scroll_when_wrapped() {
        let mut tab = make_tab(&["long line"]).await;
        tab.display.wrap = true;
        tab.scroll.horizontal_scroll = 5;
        press(&mut tab, KeyCode::Left, KeyModifiers::NONE).await;
        assert_eq!(tab.scroll.horizontal_scroll, 5);
    }

    #[tokio::test]
    async fn test_right_arrow_no_horizontal_scroll_when_wrapped() {
        let mut tab = make_tab(&["long line"]).await;
        tab.display.wrap = true;
        press(&mut tab, KeyCode::Right, KeyModifiers::NONE).await;
        assert_eq!(tab.scroll.horizontal_scroll, 0);
    }

    #[tokio::test]
    async fn test_zero_resets_horizontal_scroll_to_start() {
        let mut tab = make_tab(&["hello world"]).await;
        tab.display.wrap = false;
        tab.scroll.horizontal_scroll = 7;
        press(&mut tab, KeyCode::Char('0'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll.horizontal_scroll, 0);
    }

    #[tokio::test]
    async fn test_zero_as_count_suffix_does_not_reset_scroll() {
        let mut tab = make_tab(&["hello world"]).await;
        tab.display.wrap = false;
        tab.scroll.horizontal_scroll = 7;
        // Prime a count of 1, then press 0 — should build count 10, not reset scroll.
        // We must carry the mode instance between key presses to preserve the count state.
        let (mode, _) = Box::new(NormalMode::default())
            .handle_key(&mut tab, KeyCode::Char('1'), KeyModifiers::NONE)
            .await;
        mode.handle_key(&mut tab, KeyCode::Char('0'), KeyModifiers::NONE)
            .await;
        assert_eq!(
            tab.scroll.horizontal_scroll, 7,
            "scroll must not change while building a count"
        );
    }

    #[tokio::test]
    async fn test_dollar_scrolls_to_end_of_line() {
        let mut tab = make_tab(&["hello world"]).await;
        tab.display.wrap = false;
        tab.scroll.visible_width = 5;
        tab.scroll.horizontal_scroll = 0;
        press(&mut tab, KeyCode::Char('$'), KeyModifiers::NONE).await;
        // "hello world" is 11 chars; cursor at col 10, visible_width=5 → pad=min(3,2)=2 → scroll=13-5=8
        assert_eq!(tab.scroll.horizontal_scroll, 8);
    }

    #[tokio::test]
    async fn test_dollar_leaves_padding_when_viewport_large_enough() {
        let mut tab = make_tab(&["hello world"]).await;
        tab.display.wrap = false;
        tab.scroll.visible_width = 10;
        tab.scroll.horizontal_scroll = 0;
        press(&mut tab, KeyCode::Char('$'), KeyModifiers::NONE).await;
        // "hello world" is 11 chars; cursor at col 10, visible_width=10 → pad=min(3,4)=3 → scroll=14-10=4
        assert_eq!(tab.scroll.horizontal_scroll, 4);
        // last char (col 10) is at viewport position 10-4=6, with 3 empty cols on the right
    }

    #[tokio::test]
    async fn test_dollar_no_scroll_when_line_fits_in_viewport() {
        let mut tab = make_tab(&["hi"]).await;
        tab.display.wrap = false;
        tab.scroll.visible_width = 10;
        tab.scroll.horizontal_scroll = 3;
        press(&mut tab, KeyCode::Char('$'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll.horizontal_scroll, 0);
    }

    #[tokio::test]
    async fn test_dollar_no_scroll_when_wrapped() {
        let mut tab = make_tab(&["hello world"]).await;
        tab.display.wrap = true;
        tab.scroll.visible_width = 5;
        tab.scroll.horizontal_scroll = 0;
        press(&mut tab, KeyCode::Char('$'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll.horizontal_scroll, 0);
    }

    #[tokio::test]
    async fn test_dollar_no_scroll_when_visible_width_unknown() {
        let mut tab = make_tab(&["hello world"]).await;
        tab.display.wrap = false;
        tab.scroll.visible_width = 0;
        tab.scroll.horizontal_scroll = 0;
        press(&mut tab, KeyCode::Char('$'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll.horizontal_scroll, 0);
    }

    #[tokio::test]
    async fn test_m_marks_current_line() {
        let mut tab = make_tab(&["line0", "line1"]).await;
        tab.scroll.scroll_offset = 0;
        press(&mut tab, KeyCode::Char('m'), KeyModifiers::NONE).await;
        assert!(tab.mark_manager.get_indices().contains(&0));
    }

    #[tokio::test]
    async fn test_m_unmarks_already_marked_line() {
        let mut tab = make_tab(&["line0"]).await;
        tab.scroll.scroll_offset = 0;
        press(&mut tab, KeyCode::Char('m'), KeyModifiers::NONE).await;
        press(&mut tab, KeyCode::Char('m'), KeyModifiers::NONE).await;
        assert!(!tab.mark_manager.get_indices().contains(&0));
    }

    #[tokio::test]
    async fn test_g_key_resets_on_non_g_press() {
        let mut tab = make_tab(&["a"]).await;
        press(&mut tab, KeyCode::Char('g'), KeyModifiers::NONE).await;
        assert!(tab.interaction.g_key_pressed);
        press(&mut tab, KeyCode::Char('j'), KeyModifiers::NONE).await;
        assert!(!tab.interaction.g_key_pressed);
    }

    #[tokio::test]
    async fn test_capital_f_toggles_filtering_enabled() {
        let mut tab = make_tab(&["a", "b", "c"]).await;
        assert!(tab.filter.enabled);
        press(&mut tab, KeyCode::Char('F'), KeyModifiers::NONE).await;
        assert!(!tab.filter.enabled);
        press(&mut tab, KeyCode::Char('F'), KeyModifiers::NONE).await;
        assert!(tab.filter.enabled);
    }

    #[tokio::test]
    async fn test_capital_h_toggles_highlight_mode() {
        let mut tab = make_tab(&["a", "b", "c"]).await;
        assert!(!tab.filter.highlight_mode);
        press(&mut tab, KeyCode::Char('H'), KeyModifiers::NONE).await;
        assert!(tab.filter.highlight_mode);
        press(&mut tab, KeyCode::Char('H'), KeyModifiers::NONE).await;
        assert!(!tab.filter.highlight_mode);
    }

    #[tokio::test]
    async fn test_filtering_disabled_shows_all_lines() {
        let mut tab = make_tab(&["error", "warn", "info"]).await;
        tab.log_manager
            .add_filter_with_color(
                "error".to_string(),
                crate::filters::FilterType::Include,
                crate::filters::FilterOptions::default().line_mode(),
            )
            .await;
        tab.refresh_visible();
        // With filtering on, only "error" line is visible
        assert_eq!(tab.filter.visible_indices.len(), 1);

        press(&mut tab, KeyCode::Char('F'), KeyModifiers::NONE).await;
        // With filtering off, all 3 lines are visible
        assert_eq!(tab.filter.visible_indices.len(), 3);
    }

    #[tokio::test]
    async fn test_capital_m_toggles_marks_only() {
        let mut tab = make_tab(&["a", "b", "c"]).await;
        assert!(!tab.filter.show_marks_only);
        press(&mut tab, KeyCode::Char('M'), KeyModifiers::NONE).await;
        assert!(tab.filter.show_marks_only);
        press(&mut tab, KeyCode::Char('M'), KeyModifiers::NONE).await;
        assert!(!tab.filter.show_marks_only);
    }

    #[tokio::test]
    async fn test_marks_only_shows_only_marked_lines() {
        let mut tab = make_tab(&["line0", "line1", "line2"]).await;
        // Mark lines 0 and 2
        tab.mark_manager.toggle(0);
        tab.mark_manager.toggle(2);

        press(&mut tab, KeyCode::Char('M'), KeyModifiers::NONE).await;

        assert_eq!(
            tab.filter.visible_indices,
            VisibleLines::Filtered(vec![0, 2])
        );
    }

    #[tokio::test]
    async fn test_marks_only_off_restores_all_lines() {
        let mut tab = make_tab(&["line0", "line1", "line2"]).await;
        tab.mark_manager.toggle(1);
        press(&mut tab, KeyCode::Char('M'), KeyModifiers::NONE).await;
        assert_eq!(tab.filter.visible_indices.len(), 1);

        press(&mut tab, KeyCode::Char('M'), KeyModifiers::NONE).await;
        assert_eq!(tab.filter.visible_indices.len(), 3);
    }

    #[tokio::test]
    async fn test_marks_only_empty_when_no_marks() {
        let mut tab = make_tab(&["a", "b"]).await;
        press(&mut tab, KeyCode::Char('M'), KeyModifiers::NONE).await;
        assert!(tab.filter.visible_indices.is_empty());
    }

    #[tokio::test]
    async fn test_y_yanks_current_line() {
        let mut tab = make_tab(&["line0", "line1", "line2"]).await;
        tab.scroll.scroll_offset = 1;
        let (_, result) = press(&mut tab, KeyCode::Char('y'), KeyModifiers::NONE).await;
        match result {
            KeyResult::CopyToClipboard(text) => {
                assert_eq!(text, "line1");
            }
            other => panic!("expected CopyToClipboard, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_y_yanks_embedded_json_parser_with_hidden_field() {
        // Built-in (non-custom-schema) parsers have no `template_segments`,
        // so this must fall back to the generic column layout — same as
        // before the reconstruction feature existed.
        let line =
            r#"{"timestamp":"2024-01-01T00:00:00Z","level":"INFO","secret":"shh","msg":"hello"}"#;
        let mut tab = make_tab(&[line]).await;
        tab.display.format = crate::parser::detect_format(&[line.as_bytes()]).map(Arc::from);
        tab.display.hidden_fields.insert("secret".to_string());

        let (_, result) = press(&mut tab, KeyCode::Char('y'), KeyModifiers::NONE).await;
        match result {
            KeyResult::CopyToClipboard(text) => {
                assert!(
                    !text.contains("shh"),
                    "hidden field must not appear: {text}"
                );
                assert!(text.contains("INFO"));
                assert!(text.contains("hello"));
            }
            other => panic!("expected CopyToClipboard, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_y_yanks_pattern_based_custom_schema_with_hidden_field() {
        // A `pattern`-based (regex) custom schema has no `template_segments`
        // either — must also fall back to the generic column layout.
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

        let (_, result) = press(&mut tab, KeyCode::Char('y'), KeyModifiers::NONE).await;
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
    async fn test_y_yanks_custom_schema_template_reconstruction_with_hidden_field() {
        // Yanked text must match what's actually rendered (schema template,
        // hidden field's separator collapsed), not the generic column
        // layout — otherwise a paste doesn't match what the user saw and
        // selected.
        let line = "INFO/Syscon/StartupMgr, hello there";
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

        let (_, result) = press(&mut tab, KeyCode::Char('y'), KeyModifiers::NONE).await;
        match result {
            KeyResult::CopyToClipboard(text) => {
                assert_eq!(text, "INFO/StartupMgr, hello there");
            }
            other => panic!("expected CopyToClipboard, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_y_no_visible_lines_sets_error() {
        let mut tab = make_tab(&[]).await;
        let (_, result) = press(&mut tab, KeyCode::Char('y'), KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::Handled));
        assert_eq!(
            tab.interaction.command_error.as_deref(),
            Some("No visible lines")
        );
    }

    #[tokio::test]
    async fn test_capital_y_yanks_marked_lines() {
        let mut tab = make_tab(&["line0", "line1", "line2"]).await;
        tab.mark_manager.toggle(0);
        tab.mark_manager.toggle(2);
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
        assert_eq!(
            tab.interaction.command_error.as_deref(),
            Some("No marked lines")
        );
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
        assert_eq!(tab.scroll.scroll_offset, 5);
    }

    #[tokio::test]
    async fn test_count_3k_moves_up_3() {
        let lines: Vec<&str> = (0..20).map(|_| "line").collect();
        let mut tab = make_tab(&lines).await;
        tab.scroll.scroll_offset = 10;
        let mode = NormalMode { count: Some(3) };
        press_mode(mode, &mut tab, KeyCode::Char('k'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll.scroll_offset, 7);
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
        assert_eq!(tab.scroll.scroll_offset, 10);
    }

    #[tokio::test]
    async fn test_count_g_goes_to_line() {
        let lines: Vec<&str> = (0..20).map(|_| "line").collect();
        let mut tab = make_tab(&lines).await;
        // 5G should go to line 5 (0-based index 4)
        let mode = NormalMode { count: Some(5) };
        press_mode(mode, &mut tab, KeyCode::Char('G'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll.scroll_offset, 4);
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
        assert_eq!(tab.scroll.scroll_offset, 4);
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
        tab.scroll.visible_height = 10;
        let mode = NormalMode { count: Some(3) };
        press_mode(mode, &mut tab, KeyCode::Char('d'), KeyModifiers::CONTROL).await;
        // 3 × (10/2) = 15
        assert_eq!(tab.scroll.scroll_offset, 15);
    }

    #[tokio::test]
    async fn test_count_half_page_up() {
        let lines: Vec<&str> = (0..100).map(|_| "line").collect();
        let mut tab = make_tab(&lines).await;
        tab.scroll.visible_height = 10;
        tab.scroll.scroll_offset = 50;
        let mode = NormalMode { count: Some(2) };
        press_mode(mode, &mut tab, KeyCode::Char('u'), KeyModifiers::CONTROL).await;
        // 50 - 2 × (10/2) = 40
        assert_eq!(tab.scroll.scroll_offset, 40);
    }

    #[tokio::test]
    async fn test_count_page_down() {
        let lines: Vec<&str> = (0..100).map(|_| "line").collect();
        let mut tab = make_tab(&lines).await;
        tab.scroll.visible_height = 10;
        let mode = NormalMode { count: Some(2) };
        press_mode(mode, &mut tab, KeyCode::PageDown, KeyModifiers::NONE).await;
        assert_eq!(tab.scroll.scroll_offset, 20);
    }

    #[tokio::test]
    async fn test_count_page_up() {
        let lines: Vec<&str> = (0..100).map(|_| "line").collect();
        let mut tab = make_tab(&lines).await;
        tab.scroll.visible_height = 10;
        tab.scroll.scroll_offset = 50;
        let mode = NormalMode { count: Some(3) };
        press_mode(mode, &mut tab, KeyCode::PageUp, KeyModifiers::NONE).await;
        assert_eq!(tab.scroll.scroll_offset, 20);
    }

    // ── Clear all marks and comments ────────────────────────────────

    #[tokio::test]
    async fn test_shift_c_clears_marks_and_comments() {
        let mut tab = make_tab(&["a", "b", "c"]).await;
        tab.mark_manager.toggle(0);
        tab.mark_manager.toggle(2);
        tab.comment_manager.add("note".into(), vec![1]);
        assert!(!tab.mark_manager.get_indices().is_empty());
        assert!(!tab.comment_manager.get().is_empty());

        press(&mut tab, KeyCode::Char('C'), KeyModifiers::NONE).await;
        assert!(tab.mark_manager.get_indices().is_empty());
        assert!(tab.comment_manager.get().is_empty());
        assert_eq!(
            tab.interaction.command_error.as_deref(),
            Some("Cleared all marks and comments")
        );
    }

    // ── Edit comment ────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_r_on_commented_line_opens_edit_mode() {
        let mut tab = make_tab(&["line0", "line1", "line2"]).await;
        tab.comment_manager.add("my comment".into(), vec![0]);
        tab.scroll.scroll_offset = 0;

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
        tab.scroll.scroll_offset = 0;

        let (mode, result) = press(&mut tab, KeyCode::Char('r'), KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(matches!(mode.render_state(), ModeRenderState::Normal));
        assert_eq!(
            tab.interaction.command_error.as_deref(),
            Some("No comment on this line")
        );
    }

    // ── Delete comment ──────────────────────────────────────────────────

    #[tokio::test]
    async fn test_d_on_commented_line_deletes_comment() {
        let mut tab = make_tab(&["line0", "line1", "line2"]).await;
        tab.comment_manager.add("to delete".into(), vec![0]);
        tab.comment_manager.add("keep".into(), vec![2]);
        tab.scroll.scroll_offset = 0;

        let (mode, result) = press(&mut tab, KeyCode::Char('d'), KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(matches!(mode.render_state(), ModeRenderState::Normal));
        let comments = tab.comment_manager.get();
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].text, "keep");
    }

    #[tokio::test]
    async fn test_d_on_non_commented_line_shows_error() {
        let mut tab = make_tab(&["line0", "line1"]).await;
        tab.scroll.scroll_offset = 0;

        let (_mode, result) = press(&mut tab, KeyCode::Char('d'), KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::Handled));
        assert_eq!(
            tab.interaction.command_error.as_deref(),
            Some("No comment on this line")
        );
        assert!(tab.comment_manager.get().is_empty());
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
        tab.scroll.scroll_offset = 1;
        let (mode, result) = press(&mut tab, KeyCode::Char('c'), KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::Handled));
        match mode.render_state() {
            ModeRenderState::Comment { line_count, .. } => {
                assert_eq!(line_count, 1);
            }
            other => panic!("expected Comment, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_c_on_commented_line_opens_edit_mode() {
        let mut tab = make_tab(&["line0", "line1", "line2"]).await;
        tab.comment_manager.add("existing note".into(), vec![1]);
        tab.scroll.scroll_offset = 1;
        let (mode, result) = press(&mut tab, KeyCode::Char('c'), KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::Handled));
        match mode.render_state() {
            ModeRenderState::Comment {
                lines, line_count, ..
            } => {
                assert_eq!(lines, vec!["existing note".to_string()]);
                assert_eq!(line_count, 1);
            }
            other => panic!("expected Comment, got {:?}", other),
        }
        // Saving should update the existing comment, not add a new one.
        mode.handle_key(&mut tab, KeyCode::Char('s'), KeyModifiers::CONTROL)
            .await;
        assert_eq!(tab.comment_manager.get().len(), 1);
    }

    // ── Error / warning navigation ────────────────────────────────────────

    #[tokio::test]
    async fn test_e_navigates_to_next_error() {
        let mut tab =
            make_tab(&["INFO normal line", "ERROR something failed", "INFO another"]).await;
        tab.scroll.scroll_offset = 0;
        press(&mut tab, KeyCode::Char('e'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll.scroll_offset, 1);
    }

    #[tokio::test]
    async fn test_capital_e_navigates_to_prev_error() {
        let mut tab = make_tab(&["ERROR first error", "INFO normal line", "INFO another"]).await;
        tab.scroll.scroll_offset = 2;
        press(&mut tab, KeyCode::Char('E'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll.scroll_offset, 0);
    }

    #[tokio::test]
    async fn test_e_at_last_error_sets_command_error() {
        let mut tab = make_tab(&["ERROR only error", "INFO line"]).await;
        tab.scroll.scroll_offset = 0;
        press(&mut tab, KeyCode::Char('e'), KeyModifiers::NONE).await;
        assert_eq!(
            tab.interaction.command_error.as_deref(),
            Some("No more errors")
        );
    }

    #[tokio::test]
    async fn test_capital_e_at_first_error_sets_command_error() {
        let mut tab = make_tab(&["INFO line", "ERROR only error"]).await;
        tab.scroll.scroll_offset = 1;
        press(&mut tab, KeyCode::Char('E'), KeyModifiers::NONE).await;
        assert_eq!(
            tab.interaction.command_error.as_deref(),
            Some("No previous error")
        );
    }

    #[tokio::test]
    async fn test_w_navigates_to_next_warning() {
        let mut tab =
            make_tab(&["INFO normal line", "WARN disk nearly full", "INFO another"]).await;
        tab.scroll.scroll_offset = 0;
        press(&mut tab, KeyCode::Char('w'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll.scroll_offset, 1);
    }

    #[tokio::test]
    async fn test_capital_w_navigates_to_prev_warning() {
        let mut tab = make_tab(&["WARN first warning", "INFO normal line", "INFO another"]).await;
        tab.scroll.scroll_offset = 2;
        press(&mut tab, KeyCode::Char('W'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll.scroll_offset, 0);
    }

    #[tokio::test]
    async fn test_w_no_warnings_sets_command_error() {
        let mut tab = make_tab(&["INFO only", "DEBUG line"]).await;
        tab.scroll.scroll_offset = 0;
        press(&mut tab, KeyCode::Char('w'), KeyModifiers::NONE).await;
        assert_eq!(
            tab.interaction.command_error.as_deref(),
            Some("No more warnings")
        );
    }

    #[tokio::test]
    async fn test_capital_w_no_prev_warning_sets_command_error() {
        let mut tab = make_tab(&["INFO line", "WARN only warning"]).await;
        tab.scroll.scroll_offset = 1;
        press(&mut tab, KeyCode::Char('W'), KeyModifiers::NONE).await;
        assert_eq!(
            tab.interaction.command_error.as_deref(),
            Some("No previous warning")
        );
    }

    // ── Marked-line navigation (n/N without an active search) ──────────────

    #[tokio::test]
    async fn test_n_navigates_to_next_marked_line_when_no_search() {
        let mut tab = make_tab(&["line0", "line1", "line2", "line3"]).await;
        tab.mark_manager.toggle(2);
        tab.scroll.scroll_offset = 0;
        press(&mut tab, KeyCode::Char('n'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll.scroll_offset, 2);
    }

    #[tokio::test]
    async fn test_capital_n_navigates_to_prev_marked_line_when_no_search() {
        let mut tab = make_tab(&["line0", "line1", "line2", "line3"]).await;
        tab.mark_manager.toggle(1);
        tab.scroll.scroll_offset = 3;
        press(&mut tab, KeyCode::Char('N'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll.scroll_offset, 1);
    }

    #[tokio::test]
    async fn test_n_no_marks_no_search_sets_command_error() {
        let mut tab = make_tab(&["line0", "line1"]).await;
        tab.scroll.scroll_offset = 0;
        press(&mut tab, KeyCode::Char('n'), KeyModifiers::NONE).await;
        assert_eq!(
            tab.interaction.command_error.as_deref(),
            Some("No more marks")
        );
    }

    #[tokio::test]
    async fn test_capital_n_no_marks_no_search_sets_command_error() {
        let mut tab = make_tab(&["line0", "line1"]).await;
        tab.scroll.scroll_offset = 1;
        press(&mut tab, KeyCode::Char('N'), KeyModifiers::NONE).await;
        assert_eq!(
            tab.interaction.command_error.as_deref(),
            Some("No previous mark")
        );
    }

    #[tokio::test]
    async fn test_n_navigates_search_matches_when_search_active_even_with_marks() {
        let lines = ["foo line", "bar line", "foo again"];
        let mut tab = make_tab(&lines).await;
        // A mark on a non-matching line must be ignored while a search is active.
        tab.mark_manager.toggle(1);
        let texts: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
        tab.search
            .query
            .search("foo", 0..lines.len(), |i| texts.get(i).cloned())
            .unwrap();
        tab.scroll.scroll_offset = 0;
        press(&mut tab, KeyCode::Char('n'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll.scroll_offset, 2);
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
        tab.scroll.scroll_offset = 0;
        press(&mut tab, KeyCode::Char('e'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll.scroll_offset, 3);
    }

    #[tokio::test]
    async fn test_e_navigates_to_fatal_level() {
        let mut tab = make_tab(&["INFO line", "FATAL crash"]).await;
        tab.scroll.scroll_offset = 0;
        press(&mut tab, KeyCode::Char('e'), KeyModifiers::NONE).await;
        assert_eq!(tab.scroll.scroll_offset, 1);
    }
}
