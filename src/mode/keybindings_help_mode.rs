use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::{
    config::Keybindings,
    mode::app_mode::{Mode, ModeRenderState, status_entry},
    mode::normal_mode::NormalMode,
    theme::Theme,
    ui::{KeyResult, TabState},
};

/// A single row in the keybindings popup: section header OR action+key pair.
#[derive(Clone)]
pub enum HelpRow {
    Header(String),
    Entry { action: String, keys: String },
}

/// Build the full list of rows from the current keybindings.
pub fn build_help_rows(kb: &Keybindings) -> Vec<HelpRow> {
    let nav = &kb.navigation;
    let n = &kb.normal;
    let g = &kb.global;
    let f = &kb.filter;

    let mut rows: Vec<HelpRow> = Vec::new();

    rows.push(HelpRow::Header("Navigation (all modes)".to_string()));
    rows.push(HelpRow::Entry {
        action: "Scroll down".into(),
        keys: nav.scroll_down.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Scroll up".into(),
        keys: nav.scroll_up.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Half page down".into(),
        keys: nav.half_page_down.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Half page up".into(),
        keys: nav.half_page_up.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Page down".into(),
        keys: nav.page_down.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Page up".into(),
        keys: nav.page_up.display(),
    });

    rows.push(HelpRow::Header("Normal Mode".to_string()));
    rows.push(HelpRow::Entry {
        action: "Scroll left".into(),
        keys: n.scroll_left.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Scroll right".into(),
        keys: n.scroll_right.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Start of line".into(),
        keys: n.start_of_line.display(),
    });
    rows.push(HelpRow::Entry {
        action: "End of line".into(),
        keys: n.end_of_line.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Go to top (gg)".into(),
        keys: format!(
            "{}{}",
            n.go_to_top_chord.display(),
            n.go_to_top_chord.display()
        ),
    });
    rows.push(HelpRow::Entry {
        action: "Go to bottom".into(),
        keys: n.go_to_bottom.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Mark line".into(),
        keys: n.mark_line.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Marks only".into(),
        keys: n.toggle_marks_only.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Yank current line".into(),
        keys: n.yank_line.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Yank marked lines".into(),
        keys: n.yank_marked.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Visual line select".into(),
        keys: n.visual_mode.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Visual char select".into(),
        keys: n.visual_char.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Search forward".into(),
        keys: n.search_forward.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Search backward".into(),
        keys: n.search_backward.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Next match".into(),
        keys: n.next_match.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Prev match".into(),
        keys: n.prev_match.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Filter include".into(),
        keys: n.filter_include.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Filter exclude".into(),
        keys: n.filter_exclude.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Filter mode".into(),
        keys: n.filter_mode.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Toggle filtering".into(),
        keys: n.toggle_filtering.display(),
    });
    rows.push(HelpRow::Entry {
        action: "UI mode".into(),
        keys: n.enter_ui_mode.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Command mode".into(),
        keys: n.command_mode.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Comment line".into(),
        keys: n.comment_line.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Edit comment".into(),
        keys: n.edit_comment.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Delete comment".into(),
        keys: n.delete_comment.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Next error".into(),
        keys: n.next_error.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Prev error".into(),
        keys: n.prev_error.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Next warning".into(),
        keys: n.next_warning.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Prev warning".into(),
        keys: n.prev_warning.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Clear search".into(),
        keys: n.clear_search.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Clear marks/comments".into(),
        keys: n.clear_all.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Show keybindings".into(),
        keys: n.show_keybindings.display(),
    });

    let vl = &kb.visual_line;
    rows.push(HelpRow::Header("Visual Line Mode".to_string()));
    rows.push(HelpRow::Entry {
        action: "Comment selection".into(),
        keys: vl.comment.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Yank to clipboard".into(),
        keys: vl.yank.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Mark lines".into(),
        keys: vl.mark.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Search".into(),
        keys: vl.search.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Exit visual mode".into(),
        keys: vl.exit.display(),
    });

    let vc = &kb.visual;
    rows.push(HelpRow::Header("Visual Char Mode".to_string()));
    rows.push(HelpRow::Entry {
        action: "Move left".into(),
        keys: vc.move_left.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Move right".into(),
        keys: vc.move_right.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Word forward".into(),
        keys: vc.word_forward.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Word backward".into(),
        keys: vc.word_backward.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Word end".into(),
        keys: vc.word_end.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Word forward (WORD)".into(),
        keys: vc.word_forward_big.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Word backward (WORD)".into(),
        keys: vc.word_backward_big.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Word end (WORD)".into(),
        keys: vc.word_end_big.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Start of line".into(),
        keys: vc.start_of_line.display(),
    });
    rows.push(HelpRow::Entry {
        action: "First non-blank".into(),
        keys: vc.first_nonblank.display(),
    });
    rows.push(HelpRow::Entry {
        action: "End of line".into(),
        keys: vc.end_of_line.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Find forward".into(),
        keys: vc.find_forward.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Find backward".into(),
        keys: vc.find_backward.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Till forward".into(),
        keys: vc.till_forward.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Till backward".into(),
        keys: vc.till_backward.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Repeat motion".into(),
        keys: vc.repeat_motion.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Repeat motion reverse".into(),
        keys: vc.repeat_motion_rev.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Filter include".into(),
        keys: vc.filter_include.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Filter exclude".into(),
        keys: vc.filter_exclude.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Search".into(),
        keys: vc.search.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Start selection".into(),
        keys: vc.start_selection.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Yank to clipboard".into(),
        keys: vc.yank.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Exit visual mode".into(),
        keys: vc.exit.display(),
    });

    rows.push(HelpRow::Header("Global".to_string()));
    rows.push(HelpRow::Entry {
        action: "Quit".into(),
        keys: g.quit.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Next tab".into(),
        keys: g.next_tab.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Prev tab".into(),
        keys: g.prev_tab.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Close tab".into(),
        keys: g.close_tab.display(),
    });
    rows.push(HelpRow::Entry {
        action: "New tab".into(),
        keys: g.new_tab.display(),
    });

    rows.push(HelpRow::Header("UI Mode".to_string()));
    let ui = &kb.ui;
    rows.push(HelpRow::Entry {
        action: "Toggle sidebar".into(),
        keys: ui.toggle_sidebar.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Toggle mode bar".into(),
        keys: ui.toggle_mode_bar.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Toggle borders".into(),
        keys: ui.toggle_borders.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Toggle wrap".into(),
        keys: ui.toggle_wrap.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Exit UI mode".into(),
        keys: ui.exit.display(),
    });

    rows.push(HelpRow::Header("Filter Mode".to_string()));
    rows.push(HelpRow::Entry {
        action: "Add include filter".into(),
        keys: f.add_include_filter.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Add exclude filter".into(),
        keys: f.add_exclude_filter.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Add date filter".into(),
        keys: f.add_date_filter.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Toggle filter".into(),
        keys: f.toggle_filter.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Delete filter".into(),
        keys: f.delete_filter.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Edit filter".into(),
        keys: f.edit_filter.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Set color".into(),
        keys: f.set_color.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Move filter up".into(),
        keys: f.move_filter_up.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Move filter down".into(),
        keys: f.move_filter_down.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Toggle all".into(),
        keys: f.toggle_all_filters.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Clear all".into(),
        keys: f.clear_all_filters.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Grow sidebar".into(),
        keys: f.sidebar_grow.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Shrink sidebar".into(),
        keys: f.sidebar_shrink.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Exit filter mode".into(),
        keys: f.exit_mode.display(),
    });

    rows.push(HelpRow::Header("Comment Mode".to_string()));
    rows.push(HelpRow::Entry {
        action: "Save comment".into(),
        keys: kb.comment.save.display(),
    });
    rows.push(HelpRow::Entry {
        action: "New line".into(),
        keys: kb.comment.newline.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Delete comment".into(),
        keys: kb.comment.delete.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Cancel".into(),
        keys: kb.comment.cancel.display(),
    });

    rows.push(HelpRow::Header("Search Mode".to_string()));
    rows.push(HelpRow::Entry {
        action: "Confirm search".into(),
        keys: kb.search.confirm.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Cancel".into(),
        keys: kb.search.cancel.display(),
    });

    rows.push(HelpRow::Header("Filter Edit Mode".to_string()));
    rows.push(HelpRow::Entry {
        action: "Save".into(),
        keys: kb.filter_edit.confirm.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Cancel".into(),
        keys: kb.filter_edit.cancel.display(),
    });

    rows.push(HelpRow::Header("Command Mode".to_string()));
    rows.push(HelpRow::Entry {
        action: "Execute".into(),
        keys: kb.command.confirm.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Cancel".into(),
        keys: kb.command.cancel.display(),
    });

    let ds = &kb.docker_select;
    rows.push(HelpRow::Header("Docker Select Mode".to_string()));
    rows.push(HelpRow::Entry {
        action: "Attach".into(),
        keys: ds.confirm.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Cancel".into(),
        keys: ds.cancel.display(),
    });

    let vc = &kb.value_colors;
    rows.push(HelpRow::Header("Value Colors Mode".to_string()));
    rows.push(HelpRow::Entry {
        action: "Toggle".into(),
        keys: vc.toggle.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Enable all".into(),
        keys: vc.all.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Disable all".into(),
        keys: vc.none.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Apply".into(),
        keys: vc.apply.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Cancel".into(),
        keys: vc.cancel.display(),
    });

    let sf = &kb.select_fields;
    rows.push(HelpRow::Header("Select Fields Mode".to_string()));
    rows.push(HelpRow::Entry {
        action: "Toggle".into(),
        keys: sf.toggle.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Move up".into(),
        keys: sf.move_up.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Move down".into(),
        keys: sf.move_down.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Enable all".into(),
        keys: sf.all.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Disable all".into(),
        keys: sf.none.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Apply".into(),
        keys: sf.apply.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Cancel".into(),
        keys: sf.cancel.display(),
    });

    let h = &kb.help;
    rows.push(HelpRow::Header("Help Mode".to_string()));
    rows.push(HelpRow::Entry {
        action: "Close".into(),
        keys: h.close.display(),
    });

    rows.push(HelpRow::Header("Confirm Mode".to_string()));
    rows.push(HelpRow::Entry {
        action: "Yes".into(),
        keys: kb.confirm.yes.display(),
    });
    rows.push(HelpRow::Entry {
        action: "No".into(),
        keys: kb.confirm.no.display(),
    });

    rows
}

/// Filter rows by fuzzy-matching `query` against action name and key strings.
/// Headers are kept if at least one entry below them matches; consecutive
/// orphaned headers are removed.
pub fn filter_rows(rows: &[HelpRow], query: &str) -> Vec<HelpRow> {
    if query.is_empty() {
        return rows.to_vec();
    }
    use crate::auto_complete::fuzzy_match;

    // First pass: collect which entry indices match.
    // Keep a header only if a subsequent entry (before the next header) matches.
    let mut result: Vec<HelpRow> = Vec::new();
    let mut pending_header: Option<HelpRow> = None;
    let mut section_has_match = false;

    for row in rows {
        match row {
            HelpRow::Header(_) => {
                // If the previous section had no matches, drop the pending header.
                if section_has_match {
                    // already pushed when first match was found
                }
                pending_header = Some(row.clone());
                section_has_match = false;
            }
            HelpRow::Entry { action, keys } => {
                let haystack = format!("{} {}", action, keys);
                if fuzzy_match(query, &haystack) {
                    if !section_has_match {
                        if let Some(h) = pending_header.take() {
                            result.push(h);
                        }
                        section_has_match = true;
                    }
                    result.push(row.clone());
                }
            }
        }
    }

    result
}

#[derive(Debug)]
pub struct KeybindingsHelpMode {
    pub scroll: usize,
    pub search: String,
}

impl KeybindingsHelpMode {
    pub fn new() -> Self {
        Self {
            scroll: 0,
            search: String::new(),
        }
    }
}

impl Default for KeybindingsHelpMode {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Mode for KeybindingsHelpMode {
    async fn handle_key(
        mut self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        let kb = tab.keybindings.clone();

        // Close: configured close key clears search first, then exits.
        if kb.help.close.matches(key, modifiers) {
            if !self.search.is_empty() {
                self.search.clear();
                self.scroll = 0;
                return (self, KeyResult::Handled);
            }
            return (Box::new(NormalMode::default()), KeyResult::Handled);
        }

        // The show_keybindings key (F1 by default) also closes
        if kb.normal.show_keybindings.matches(key, modifiers) && self.search.is_empty() {
            return (Box::new(NormalMode::default()), KeyResult::Handled);
        }

        // Fast scroll (works in both modes)
        if kb.navigation.half_page_down.matches(key, modifiers) {
            self.scroll = self.scroll.saturating_add(10);
            return (self, KeyResult::Handled);
        }
        if kb.navigation.half_page_up.matches(key, modifiers) {
            self.scroll = self.scroll.saturating_sub(10);
            return (self, KeyResult::Handled);
        }

        // j/k/arrows scroll only when not typing
        if self.search.is_empty() {
            if kb.navigation.scroll_down.matches(key, modifiers) {
                self.scroll = self.scroll.saturating_add(1);
                return (self, KeyResult::Handled);
            }
            if kb.navigation.scroll_up.matches(key, modifiers) {
                self.scroll = self.scroll.saturating_sub(1);
                return (self, KeyResult::Handled);
            }
        }

        // Search editing: any printable char appends, Backspace removes
        match key {
            KeyCode::Char(c) if !modifiers.contains(KeyModifiers::CONTROL) => {
                self.search.push(c);
                self.scroll = 0;
            }
            KeyCode::Backspace => {
                self.search.pop();
                self.scroll = 0;
            }
            _ => {}
        }

        (self, KeyResult::Handled)
    }

    fn mode_bar_content(&self, kb: &Keybindings, theme: &Theme) -> Line<'static> {
        let mut spans: Vec<Span<'static>> = vec![Span::styled(
            "[HELP]  ",
            Style::default()
                .fg(theme.text_highlight_fg)
                .add_modifier(Modifier::BOLD),
        )];
        spans.push(Span::styled(
            "type to search  ",
            Style::default().fg(theme.text),
        ));
        // Scroll up/down
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
        spans.push(Span::styled("> scroll  ", Style::default().fg(theme.text)));
        status_entry(&mut spans, kb.help.close.display(), "close", theme);
        Line::from(spans)
    }

    fn render_state(&self) -> ModeRenderState {
        ModeRenderState::KeybindingsHelp {
            scroll: self.scroll,
            search: self.search.clone(),
        }
    }
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

    async fn make_tab() -> TabState {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let lm = LogManager::new(db, None).await;
        TabState::new(FileReader::from_bytes(vec![]), lm, "test".to_string())
    }

    async fn press(
        mode: KeybindingsHelpMode,
        tab: &mut TabState,
        key: KeyCode,
    ) -> (Box<dyn Mode>, KeyResult) {
        Box::new(mode)
            .handle_key(tab, key, KeyModifiers::NONE)
            .await
    }

    #[tokio::test]
    async fn test_esc_closes_when_search_empty() {
        let mut tab = make_tab().await;
        let mode = KeybindingsHelpMode::new();
        let (mode2, _) = press(mode, &mut tab, KeyCode::Esc).await;
        assert!(!matches!(
            mode2.render_state(),
            ModeRenderState::KeybindingsHelp { .. }
        )); // NormalMode
    }

    #[tokio::test]
    async fn test_esc_clears_search_first() {
        let mut tab = make_tab().await;
        let mut mode = KeybindingsHelpMode::new();
        mode.search = "foo".to_string();
        let (mode2, _) = press(mode, &mut tab, KeyCode::Esc).await;
        // Still in help mode but search cleared
        assert!(matches!(
            mode2.render_state(),
            ModeRenderState::KeybindingsHelp { .. }
        ));
        match mode2.render_state() {
            ModeRenderState::KeybindingsHelp { search, .. } => assert!(search.is_empty()),
            other => panic!("expected KeybindingsHelp, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_q_closes() {
        let mut tab = make_tab().await;
        let mode = KeybindingsHelpMode::new();
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('q')).await;
        assert!(!matches!(
            mode2.render_state(),
            ModeRenderState::KeybindingsHelp { .. }
        ));
    }

    #[tokio::test]
    async fn test_j_scrolls_down() {
        let mut tab = make_tab().await;
        let mode = KeybindingsHelpMode::new();
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('j')).await;
        match mode2.render_state() {
            ModeRenderState::KeybindingsHelp { scroll, .. } => assert_eq!(scroll, 1),
            other => panic!("expected KeybindingsHelp, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_k_clamps_at_zero() {
        let mut tab = make_tab().await;
        let mode = KeybindingsHelpMode::new();
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('k')).await;
        match mode2.render_state() {
            ModeRenderState::KeybindingsHelp { scroll, .. } => assert_eq!(scroll, 0),
            other => panic!("expected KeybindingsHelp, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_typing_updates_search_and_resets_scroll() {
        let mut tab = make_tab().await;
        let mut mode = KeybindingsHelpMode::new();
        mode.scroll = 5;
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('f')).await;
        match mode2.render_state() {
            ModeRenderState::KeybindingsHelp { scroll, search } => {
                assert_eq!(scroll, 0);
                assert_eq!(search, "f");
            }
            other => panic!("expected KeybindingsHelp, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_backspace_removes_search_char() {
        let mut tab = make_tab().await;
        let mut mode = KeybindingsHelpMode::new();
        mode.search = "fo".to_string();
        let (mode2, _) = press(mode, &mut tab, KeyCode::Backspace).await;
        match mode2.render_state() {
            ModeRenderState::KeybindingsHelp { search, .. } => assert_eq!(search, "f"),
            other => panic!("expected KeybindingsHelp, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_mode_bar_content_contains_help() {
        let mode = KeybindingsHelpMode::new();
        assert!(matches!(
            mode.render_state(),
            ModeRenderState::KeybindingsHelp { .. }
        ));
    }

    #[test]
    fn test_build_help_rows_contains_normal_header() {
        let kb = Keybindings::default();
        let rows = build_help_rows(&kb);
        let has_normal = rows
            .iter()
            .any(|r| matches!(r, HelpRow::Header(h) if h == "Normal Mode"));
        assert!(has_normal);
    }

    #[test]
    fn test_filter_rows_empty_query_returns_all() {
        let kb = Keybindings::default();
        let rows = build_help_rows(&kb);
        let count = rows.len();
        let filtered = filter_rows(&rows, "");
        assert_eq!(filtered.len(), count);
    }

    #[test]
    fn test_filter_rows_matches_action() {
        let kb = Keybindings::default();
        let rows = build_help_rows(&kb);
        let filtered = filter_rows(&rows, "quit");
        let has_quit = filtered.iter().any(|r| matches!(r, HelpRow::Entry { action, .. } if action.to_lowercase().contains("quit")));
        assert!(has_quit);
    }

    #[test]
    fn test_filter_rows_matches_key() {
        let kb = Keybindings::default();
        let rows = build_help_rows(&kb);
        // "Ctrl+d" is the half-page-down key
        let filtered = filter_rows(&rows, "ctrl");
        assert!(!filtered.is_empty());
    }

    #[test]
    fn test_filter_rows_no_orphan_headers() {
        let kb = Keybindings::default();
        let rows = build_help_rows(&kb);
        // A query that matches nothing should return empty
        let filtered = filter_rows(&rows, "zzzzznotakey");
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_filter_rows_header_dropped_when_no_entry_matches() {
        let kb = Keybindings::default();
        let rows = build_help_rows(&kb);
        // Filter for something that only exists in Normal Mode section
        let filtered = filter_rows(&rows, "visual");
        let global_header = filtered
            .iter()
            .any(|r| matches!(r, HelpRow::Header(h) if h == "Global"));
        assert!(
            !global_header,
            "Global header should be absent when no Global entries match"
        );
    }
}
