use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyModifiers};

use crate::{
    config::Keybindings,
    mode::{app_mode::Mode, normal_mode::NormalMode},
    ui::{KeyResult, TabState},
};

// ---------------------------------------------------------------------------
// HelpRow — one entry in the keybindings table
// ---------------------------------------------------------------------------

/// A single row in the keybindings popup: section header OR action+key pair.
#[derive(Clone)]
pub enum HelpRow {
    Header(String),
    Entry { action: String, keys: String },
}

/// Build the full list of rows from the current keybindings.
pub fn build_help_rows(kb: &Keybindings) -> Vec<HelpRow> {
    let n = &kb.normal;
    let g = &kb.global;
    let f = &kb.filter;

    let mut rows: Vec<HelpRow> = Vec::new();

    // ── Normal Mode ──────────────────────────────────────────────────────────
    rows.push(HelpRow::Header("Normal Mode".to_string()));
    rows.push(HelpRow::Entry {
        action: "Scroll down".into(),
        keys: n.scroll_down.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Scroll up".into(),
        keys: n.scroll_up.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Scroll left".into(),
        keys: n.scroll_left.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Scroll right".into(),
        keys: n.scroll_right.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Half page down".into(),
        keys: n.half_page_down.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Half page up".into(),
        keys: n.half_page_up.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Page down".into(),
        keys: n.page_down.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Page up".into(),
        keys: n.page_up.display(),
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
        action: "Visual select".into(),
        keys: n.visual_mode.display(),
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
        action: "Filter mode".into(),
        keys: n.filter_mode.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Toggle filtering".into(),
        keys: n.toggle_filtering.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Toggle sidebar".into(),
        keys: n.toggle_sidebar.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Toggle wrap".into(),
        keys: n.toggle_wrap.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Command mode".into(),
        keys: n.command_mode.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Show keybindings".into(),
        keys: n.show_keybindings.display(),
    });

    // ── Global ───────────────────────────────────────────────────────────────
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

    // ── Filter Mode ──────────────────────────────────────────────────────────
    rows.push(HelpRow::Header("Filter Mode".to_string()));
    rows.push(HelpRow::Entry {
        action: "Navigate".into(),
        keys: format!("{}/{}", f.select_up.display(), f.select_down.display()),
    });
    rows.push(HelpRow::Entry {
        action: "Add include".into(),
        keys: f.add_include.display(),
    });
    rows.push(HelpRow::Entry {
        action: "Add exclude".into(),
        keys: f.add_exclude.display(),
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
        action: "Exit filter mode".into(),
        keys: f.exit_mode.display(),
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

// ---------------------------------------------------------------------------
// KeybindingsHelpMode
// ---------------------------------------------------------------------------

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

        // Close: Esc clears search first, then exits.
        if key == KeyCode::Esc {
            if !self.search.is_empty() {
                self.search.clear();
                self.scroll = 0;
                return (self, KeyResult::Handled);
            }
            return (Box::new(NormalMode), KeyResult::Handled);
        }

        // q closes (only when not typing in search)
        if (key == KeyCode::Char('q') || key == KeyCode::Char('Q')) && self.search.is_empty() {
            return (Box::new(NormalMode), KeyResult::Handled);
        }

        // The show_keybindings key (F1 by default) also closes
        if kb.normal.show_keybindings.matches(key, modifiers) && self.search.is_empty() {
            return (Box::new(NormalMode), KeyResult::Handled);
        }

        // Ctrl+d / Ctrl+u — fast scroll (works in both modes)
        if key == KeyCode::Char('d') && modifiers.contains(KeyModifiers::CONTROL) {
            self.scroll = self.scroll.saturating_add(10);
            return (self, KeyResult::Handled);
        }
        if key == KeyCode::Char('u') && modifiers.contains(KeyModifiers::CONTROL) {
            self.scroll = self.scroll.saturating_sub(10);
            return (self, KeyResult::Handled);
        }

        // j/k/arrows scroll only when not typing
        if self.search.is_empty() {
            match key {
                KeyCode::Char('j') | KeyCode::Down => {
                    self.scroll = self.scroll.saturating_add(1);
                    return (self, KeyResult::Handled);
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.scroll = self.scroll.saturating_sub(1);
                    return (self, KeyResult::Handled);
                }
                _ => {}
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

    fn status_line(&self) -> &str {
        "[HELP] type to search | j/k scroll | Esc clear/close | q close"
    }

    fn keybindings_help_scroll(&self) -> Option<usize> {
        Some(self.scroll)
    }

    fn keybindings_help_search(&self) -> Option<&str> {
        Some(&self.search)
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
        assert!(mode2.keybindings_help_scroll().is_none()); // NormalMode
    }

    #[tokio::test]
    async fn test_esc_clears_search_first() {
        let mut tab = make_tab().await;
        let mut mode = KeybindingsHelpMode::new();
        mode.search = "foo".to_string();
        let (mode2, _) = press(mode, &mut tab, KeyCode::Esc).await;
        // Still in help mode but search cleared
        assert!(mode2.keybindings_help_scroll().is_some());
        assert!(
            mode2
                .keybindings_help_search()
                .map(|s| s.is_empty())
                .unwrap_or(true)
        );
    }

    #[tokio::test]
    async fn test_q_closes() {
        let mut tab = make_tab().await;
        let mode = KeybindingsHelpMode::new();
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('q')).await;
        assert!(mode2.keybindings_help_scroll().is_none());
    }

    #[tokio::test]
    async fn test_j_scrolls_down() {
        let mut tab = make_tab().await;
        let mode = KeybindingsHelpMode::new();
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('j')).await;
        assert_eq!(mode2.keybindings_help_scroll(), Some(1));
    }

    #[tokio::test]
    async fn test_k_clamps_at_zero() {
        let mut tab = make_tab().await;
        let mode = KeybindingsHelpMode::new();
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('k')).await;
        assert_eq!(mode2.keybindings_help_scroll(), Some(0));
    }

    #[tokio::test]
    async fn test_typing_updates_search_and_resets_scroll() {
        let mut tab = make_tab().await;
        let mut mode = KeybindingsHelpMode::new();
        mode.scroll = 5;
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('f')).await;
        assert_eq!(mode2.keybindings_help_scroll(), Some(0));
        assert_eq!(mode2.keybindings_help_search(), Some("f"));
    }

    #[tokio::test]
    async fn test_backspace_removes_search_char() {
        let mut tab = make_tab().await;
        let mut mode = KeybindingsHelpMode::new();
        mode.search = "fo".to_string();
        let (mode2, _) = press(mode, &mut tab, KeyCode::Backspace).await;
        assert_eq!(mode2.keybindings_help_search(), Some("f"));
    }

    #[tokio::test]
    async fn test_status_line_contains_help() {
        let mode = KeybindingsHelpMode::new();
        assert!(mode.status_line().contains("[HELP]"));
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
