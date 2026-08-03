use crate::{
    commands::auto_complete::fuzzy_match,
    config::Keybindings,
    mode::app_mode::{Mode, ModeRenderState, status_entry},
    mode::normal_mode::NormalMode,
    theme::Theme,
    ui::{KeyResult, TabState},
};
use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

/// Quick-open popup (`Ctrl+P`) listing every open tab, narrowed by a live
/// fuzzy-typeahead query — always in "typing" mode, unlike
/// [`crate::mode::archive_picker_mode::ArchivePickerMode`], since there's no
/// other action here for a bare key to conflict with.
#[derive(Debug)]
pub struct FileSwitcherMode {
    /// (`App::tabs` index, tab title) for every open tab, snapshotted when
    /// the popup opened.
    entries: Vec<(usize, String)>,
    /// The tab that was active when the popup opened — used to highlight
    /// the current file in the rendered list.
    active_tab: usize,
    /// Index into the *visible* (filtered) entries.
    selected: usize,
    search: String,
}

impl FileSwitcherMode {
    pub fn new(entries: Vec<(usize, String)>, active_tab: usize) -> Self {
        Self {
            entries,
            active_tab,
            selected: 0,
            search: String::new(),
        }
    }

    /// Indices into `entries` matching the current search query — every
    /// entry when the query is empty, mirroring
    /// `DefaultFiltersMode::visible_rows`.
    fn visible_entries(&self) -> Vec<usize> {
        if self.search.is_empty() {
            return (0..self.entries.len()).collect();
        }
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, (_, title))| fuzzy_match(&self.search, title))
            .map(|(i, _)| i)
            .collect()
    }

    fn clamp_selected(&mut self) {
        let count = self.visible_entries().len();
        if count == 0 {
            self.selected = 0;
        } else if self.selected >= count {
            self.selected = count - 1;
        }
    }
}

#[async_trait]
impl Mode for FileSwitcherMode {
    async fn handle_key(
        mut self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        let kb = &tab.interaction.keybindings;

        if kb.navigation.scroll_down.matches(key, modifiers) {
            let count = self.visible_entries().len();
            if count > 0 {
                self.selected = (self.selected + 1).min(count - 1);
            }
            return (self, KeyResult::Handled);
        }
        if kb.navigation.scroll_up.matches(key, modifiers) {
            self.selected = self.selected.saturating_sub(1);
            return (self, KeyResult::Handled);
        }
        if matches!(key, KeyCode::Enter) {
            let visible = self.visible_entries();
            let Some(&entry_idx) = visible.get(self.selected) else {
                return (Box::new(NormalMode::default()), KeyResult::Handled);
            };
            let tab_idx = self.entries[entry_idx].0;
            return (
                Box::new(NormalMode::default()),
                KeyResult::SwitchToTab(tab_idx),
            );
        }
        if matches!(key, KeyCode::Esc) || kb.global.file_switcher.matches(key, modifiers) {
            if !self.search.is_empty() && matches!(key, KeyCode::Esc) {
                self.search.clear();
                self.selected = 0;
                return (self, KeyResult::Handled);
            }
            return (Box::new(NormalMode::default()), KeyResult::Handled);
        }
        match key {
            KeyCode::Char(c) if !modifiers.contains(KeyModifiers::CONTROL) => {
                self.search.push(c);
                self.selected = 0;
                self.clamp_selected();
                (self, KeyResult::Handled)
            }
            KeyCode::Backspace => {
                self.search.pop();
                self.selected = 0;
                self.clamp_selected();
                (self, KeyResult::Handled)
            }
            _ => (self, KeyResult::Ignored),
        }
    }

    fn mode_bar_content(&self, _kb: &Keybindings, theme: &Theme) -> Line<'static> {
        let mut spans: Vec<Span<'static>> = vec![Span::styled(
            "[SWITCH FILE]  ",
            Style::default()
                .fg(theme.text_highlight_fg)
                .add_modifier(Modifier::BOLD),
        )];
        status_entry(&mut spans, "Enter".to_string(), "switch", theme);
        status_entry(&mut spans, "Esc".to_string(), "cancel", theme);
        Line::from(spans)
    }

    fn render_state(&self) -> ModeRenderState {
        ModeRenderState::FileSwitcher {
            entries: self.entries.clone(),
            active_tab: self.active_tab,
            selected: self.selected,
            search: self.search.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Database, LogManager};
    use crate::ingestion::FileReader;
    use std::sync::Arc;

    async fn make_tab() -> TabState {
        let reader = FileReader::from_bytes(b"line1\nline2\n".to_vec());
        let db = Arc::new(Database::in_memory().await.unwrap());
        let lm = LogManager::new(db, None).await;
        TabState::new(reader, lm, "test".to_string())
    }

    fn entries(n: usize) -> Vec<(usize, String)> {
        (0..n).map(|i| (i, format!("file{i}.log"))).collect()
    }

    async fn press(
        mode: FileSwitcherMode,
        tab: &mut TabState,
        code: KeyCode,
    ) -> (Box<dyn Mode>, KeyResult) {
        Box::new(mode)
            .handle_key(tab, code, KeyModifiers::NONE)
            .await
    }

    fn extract(state: ModeRenderState) -> (Vec<(usize, String)>, usize, usize, String) {
        match state {
            ModeRenderState::FileSwitcher {
                entries,
                active_tab,
                selected,
                search,
            } => (entries, active_tab, selected, search),
            other => panic!("expected FileSwitcher, got {:?}", other),
        }
    }

    #[test]
    fn test_new_initializes_at_zero_with_empty_search() {
        let mode = FileSwitcherMode::new(entries(3), 1);
        let (e, active, selected, search) = extract(mode.render_state());
        assert_eq!(e.len(), 3);
        assert_eq!(active, 1);
        assert_eq!(selected, 0);
        assert_eq!(search, "");
    }

    #[tokio::test]
    async fn test_typing_narrows_selection_by_fuzzy_match() {
        let mut tab = make_tab().await;
        let mode = FileSwitcherMode::new(
            vec![(0, "app.log".to_string()), (1, "server.log".to_string())],
            0,
        );
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('s')).await;
        let (mode3, _) = mode2
            .handle_key(&mut tab, KeyCode::Char('v'), KeyModifiers::NONE)
            .await;
        // Only "server.log" matches "sv" (fuzzy subsequence) — "app.log"
        // must have been filtered out, so Enter on the sole remaining
        // (auto-reselected) entry lands on tab index 1, not 0.
        let (_, result) = mode3
            .handle_key(&mut tab, KeyCode::Enter, KeyModifiers::NONE)
            .await;
        assert!(matches!(result, KeyResult::SwitchToTab(1)));
    }

    #[tokio::test]
    async fn test_scroll_down_moves_selection() {
        let mut tab = make_tab().await;
        let (mode2, result) = press(
            FileSwitcherMode::new(entries(3), 0),
            &mut tab,
            KeyCode::Char('j'),
        )
        .await;
        assert!(matches!(result, KeyResult::Handled));
        let (_, _, selected, _) = extract(mode2.render_state());
        assert_eq!(selected, 1);
    }

    #[tokio::test]
    async fn test_scroll_down_clamped_at_last() {
        let mut tab = make_tab().await;
        let mut mode = FileSwitcherMode::new(entries(2), 0);
        mode.selected = 1;
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('j')).await;
        let (_, _, selected, _) = extract(mode2.render_state());
        assert_eq!(selected, 1);
    }

    #[tokio::test]
    async fn test_scroll_up_clamped_at_zero() {
        let mut tab = make_tab().await;
        let (mode2, _) = press(
            FileSwitcherMode::new(entries(3), 0),
            &mut tab,
            KeyCode::Char('k'),
        )
        .await;
        let (_, _, selected, _) = extract(mode2.render_state());
        assert_eq!(selected, 0);
    }

    #[tokio::test]
    async fn test_enter_switches_to_selected_tab() {
        let mut tab = make_tab().await;
        let mut mode = FileSwitcherMode::new(entries(3), 0);
        mode.selected = 2;
        let (_, result) = press(mode, &mut tab, KeyCode::Enter).await;
        assert!(matches!(result, KeyResult::SwitchToTab(2)));
    }

    #[tokio::test]
    async fn test_enter_on_narrowed_list_switches_to_correct_underlying_tab_index() {
        let mut tab = make_tab().await;
        let mode = FileSwitcherMode::new(
            vec![(5, "app.log".to_string()), (9, "server.log".to_string())],
            5,
        );
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('s')).await;
        let (_, result) = mode2
            .handle_key(&mut tab, KeyCode::Enter, KeyModifiers::NONE)
            .await;
        assert!(matches!(result, KeyResult::SwitchToTab(9)));
    }

    #[tokio::test]
    async fn test_enter_with_empty_visible_list_returns_to_normal_without_switching() {
        let mut tab = make_tab().await;
        let mode = FileSwitcherMode::new(vec![(0, "app.log".to_string())], 0);
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('z')).await;
        let (_, result) = mode2
            .handle_key(&mut tab, KeyCode::Enter, KeyModifiers::NONE)
            .await;
        assert!(matches!(result, KeyResult::Handled));
    }

    #[tokio::test]
    async fn test_esc_clears_search_first() {
        let mut tab = make_tab().await;
        let (mode2, _) = press(
            FileSwitcherMode::new(entries(3), 0),
            &mut tab,
            KeyCode::Char('x'),
        )
        .await;
        let (mode3, result) = mode2
            .handle_key(&mut tab, KeyCode::Esc, KeyModifiers::NONE)
            .await;
        assert!(matches!(result, KeyResult::Handled));
        let (_, _, _, search) = extract(mode3.render_state());
        assert_eq!(search, "");
    }

    #[tokio::test]
    async fn test_esc_with_empty_search_returns_to_normal_mode() {
        let mut tab = make_tab().await;
        let (_, result) = press(FileSwitcherMode::new(entries(3), 0), &mut tab, KeyCode::Esc).await;
        assert!(matches!(result, KeyResult::Handled));
    }

    #[tokio::test]
    async fn test_ctrl_p_closes_popup_even_with_search_active() {
        let mut tab = make_tab().await;
        let (mode2, _) = press(
            FileSwitcherMode::new(entries(3), 0),
            &mut tab,
            KeyCode::Char('x'),
        )
        .await;
        let (mode3, result) = mode2
            .handle_key(&mut tab, KeyCode::Char('p'), KeyModifiers::CONTROL)
            .await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(!matches!(
            mode3.render_state(),
            ModeRenderState::FileSwitcher { .. }
        ));
    }

    #[tokio::test]
    async fn test_backspace_removes_last_search_char() {
        let mut tab = make_tab().await;
        let (mode2, _) = press(
            FileSwitcherMode::new(entries(3), 0),
            &mut tab,
            KeyCode::Char('a'),
        )
        .await;
        let (mode3, _) = mode2
            .handle_key(&mut tab, KeyCode::Backspace, KeyModifiers::NONE)
            .await;
        let (_, _, _, search) = extract(mode3.render_state());
        assert_eq!(search, "");
    }

    #[tokio::test]
    async fn test_unknown_key_returns_ignored() {
        let mut tab = make_tab().await;
        let (_, result) = press(
            FileSwitcherMode::new(entries(3), 0),
            &mut tab,
            KeyCode::F(5),
        )
        .await;
        assert!(matches!(result, KeyResult::Ignored));
    }

    #[test]
    fn test_mode_bar_content_contains_switch_label() {
        let mode = FileSwitcherMode::new(entries(2), 0);
        let kb = Keybindings::default();
        let theme = crate::theme::Theme::default();
        let line = mode.mode_bar_content(&kb, &theme);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("SWITCH FILE"));
    }
}
