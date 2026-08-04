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

/// Theme picker popup (`:theme`) listing every available theme, narrowed by
/// a live fuzzy-typeahead query — modeled on
/// [`crate::mode::file_switcher_mode::FileSwitcherMode`], with one addition:
/// every selection change previews the theme immediately (see
/// `KeyResult::PreviewTheme`), reverted on `Esc` back to `original_theme`.
#[derive(Debug)]
pub struct ThemePickerMode {
    /// Available theme names, snapshotted when the popup opened.
    entries: Vec<String>,
    /// Index into the *visible* (filtered) entries.
    selected: usize,
    search: String,
    /// The theme active when the popup opened — restored on `Esc`.
    original_theme: Theme,
}

impl ThemePickerMode {
    pub fn new(entries: Vec<String>, original_theme: Theme) -> Self {
        Self {
            entries,
            selected: 0,
            search: String::new(),
            original_theme,
        }
    }

    /// Indices into `entries` matching the current search query — every
    /// entry when the query is empty, mirroring
    /// `FileSwitcherMode::visible_entries`.
    fn visible_entries(&self) -> Vec<usize> {
        if self.search.is_empty() {
            return (0..self.entries.len()).collect();
        }
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, name)| fuzzy_match(&self.search, name))
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

    /// The name of the currently-highlighted entry, if any — `None` when the
    /// search query matches nothing.
    fn selected_name(&self) -> Option<String> {
        let visible = self.visible_entries();
        visible
            .get(self.selected)
            .map(|&idx| self.entries[idx].clone())
    }

    /// `KeyResult` to preview the currently-highlighted entry, or `Handled`
    /// when nothing is highlighted (empty filtered list).
    fn preview_result(&self) -> KeyResult {
        match self.selected_name() {
            Some(name) => KeyResult::PreviewTheme(name),
            None => KeyResult::Handled,
        }
    }
}

#[async_trait]
impl Mode for ThemePickerMode {
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
            let result = self.preview_result();
            return (self, result);
        }
        if kb.navigation.scroll_up.matches(key, modifiers) {
            self.selected = self.selected.saturating_sub(1);
            let result = self.preview_result();
            return (self, result);
        }
        if matches!(key, KeyCode::Enter) {
            return match self.selected_name() {
                Some(name) => (
                    Box::new(NormalMode::default()),
                    KeyResult::ConfirmTheme(name),
                ),
                None => (
                    Box::new(NormalMode::default()),
                    KeyResult::RevertTheme(Box::new(self.original_theme)),
                ),
            };
        }
        if matches!(key, KeyCode::Esc) {
            if !self.search.is_empty() {
                self.search.clear();
                self.selected = 0;
                let result = self.preview_result();
                return (self, result);
            }
            return (
                Box::new(NormalMode::default()),
                KeyResult::RevertTheme(Box::new(self.original_theme)),
            );
        }
        match key {
            KeyCode::Char(c) if !modifiers.contains(KeyModifiers::CONTROL) => {
                self.search.push(c);
                self.selected = 0;
                self.clamp_selected();
                let result = self.preview_result();
                (self, result)
            }
            KeyCode::Backspace => {
                self.search.pop();
                self.selected = 0;
                self.clamp_selected();
                let result = self.preview_result();
                (self, result)
            }
            _ => (self, KeyResult::Ignored),
        }
    }

    fn mode_bar_content(&self, _kb: &Keybindings, theme: &Theme) -> Line<'static> {
        let mut spans: Vec<Span<'static>> = vec![Span::styled(
            "[THEME]  ",
            Style::default()
                .fg(theme.text_highlight_fg)
                .add_modifier(Modifier::BOLD),
        )];
        status_entry(&mut spans, "Enter".to_string(), "apply", theme);
        status_entry(&mut spans, "Esc".to_string(), "cancel", theme);
        Line::from(spans)
    }

    fn render_state(&self) -> ModeRenderState {
        ModeRenderState::ThemePicker {
            entries: self.entries.clone(),
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

    fn entries(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    async fn press(
        mode: ThemePickerMode,
        tab: &mut TabState,
        code: KeyCode,
    ) -> (Box<dyn Mode>, KeyResult) {
        Box::new(mode)
            .handle_key(tab, code, KeyModifiers::NONE)
            .await
    }

    fn extract(state: ModeRenderState) -> (Vec<String>, usize, String) {
        match state {
            ModeRenderState::ThemePicker {
                entries,
                selected,
                search,
            } => (entries, selected, search),
            other => panic!("expected ThemePicker, got {:?}", other),
        }
    }

    #[test]
    fn test_new_initializes_at_zero_with_empty_search() {
        let mode = ThemePickerMode::new(entries(&["dracula", "nord"]), Theme::default());
        let (e, selected, search) = extract(mode.render_state());
        assert_eq!(e, vec!["dracula".to_string(), "nord".to_string()]);
        assert_eq!(selected, 0);
        assert_eq!(search, "");
    }

    #[tokio::test]
    async fn test_scroll_down_previews_next_entry() {
        let mut tab = make_tab().await;
        let mode = ThemePickerMode::new(entries(&["dracula", "nord"]), Theme::default());
        let (mode2, result) = press(mode, &mut tab, KeyCode::Char('j')).await;
        assert!(matches!(result, KeyResult::PreviewTheme(ref n) if n == "nord"));
        let (_, selected, _) = extract(mode2.render_state());
        assert_eq!(selected, 1);
    }

    #[tokio::test]
    async fn test_scroll_down_clamped_at_last_still_previews_last() {
        let mut tab = make_tab().await;
        let mut mode = ThemePickerMode::new(entries(&["dracula", "nord"]), Theme::default());
        mode.selected = 1;
        let (_, result) = press(mode, &mut tab, KeyCode::Char('j')).await;
        assert!(matches!(result, KeyResult::PreviewTheme(ref n) if n == "nord"));
    }

    #[tokio::test]
    async fn test_scroll_up_previews_previous_entry() {
        let mut tab = make_tab().await;
        let mut mode = ThemePickerMode::new(entries(&["dracula", "nord"]), Theme::default());
        mode.selected = 1;
        let (_, result) = press(mode, &mut tab, KeyCode::Char('k')).await;
        assert!(matches!(result, KeyResult::PreviewTheme(ref n) if n == "dracula"));
    }

    #[tokio::test]
    async fn test_typing_narrows_and_previews_sole_match() {
        let mut tab = make_tab().await;
        let mode = ThemePickerMode::new(entries(&["dracula", "nord"]), Theme::default());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('n')).await;
        let (_, result) = mode2
            .handle_key(&mut tab, KeyCode::Char('o'), KeyModifiers::NONE)
            .await;
        assert!(matches!(result, KeyResult::PreviewTheme(ref n) if n == "nord"));
    }

    #[tokio::test]
    async fn test_typing_with_no_match_returns_handled_not_preview() {
        let mut tab = make_tab().await;
        let mode = ThemePickerMode::new(entries(&["dracula", "nord"]), Theme::default());
        let (_, result) = press(mode, &mut tab, KeyCode::Char('z')).await;
        assert!(matches!(result, KeyResult::Handled));
    }

    #[tokio::test]
    async fn test_backspace_re_previews_after_widening_matches() {
        let mut tab = make_tab().await;
        let mode = ThemePickerMode::new(entries(&["dracula", "nord"]), Theme::default());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('n')).await;
        let (_, result) = mode2
            .handle_key(&mut tab, KeyCode::Backspace, KeyModifiers::NONE)
            .await;
        assert!(matches!(result, KeyResult::PreviewTheme(ref n) if n == "dracula"));
    }

    #[tokio::test]
    async fn test_enter_confirms_selected_theme() {
        let mut tab = make_tab().await;
        let mut mode = ThemePickerMode::new(entries(&["dracula", "nord"]), Theme::default());
        mode.selected = 1;
        let (_, result) = press(mode, &mut tab, KeyCode::Enter).await;
        assert!(matches!(result, KeyResult::ConfirmTheme(ref n) if n == "nord"));
    }

    #[tokio::test]
    async fn test_enter_with_empty_visible_list_reverts_instead_of_confirming() {
        let mut tab = make_tab().await;
        let mode = ThemePickerMode::new(entries(&["dracula"]), Theme::default());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('z')).await;
        let (_, result) = mode2
            .handle_key(&mut tab, KeyCode::Enter, KeyModifiers::NONE)
            .await;
        assert!(matches!(result, KeyResult::RevertTheme(_)));
    }

    #[tokio::test]
    async fn test_esc_clears_search_first_and_previews() {
        let mut tab = make_tab().await;
        let mode = ThemePickerMode::new(entries(&["dracula", "nord"]), Theme::default());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('n')).await;
        let (mode3, result) = mode2
            .handle_key(&mut tab, KeyCode::Esc, KeyModifiers::NONE)
            .await;
        assert!(matches!(result, KeyResult::PreviewTheme(ref n) if n == "dracula"));
        let (_, _, search) = extract(mode3.render_state());
        assert_eq!(search, "");
    }

    #[tokio::test]
    async fn test_esc_with_empty_search_reverts_to_original_theme() {
        let mut tab = make_tab().await;
        let (_, result) = press(
            ThemePickerMode::new(entries(&["dracula"]), Theme::default()),
            &mut tab,
            KeyCode::Esc,
        )
        .await;
        assert!(matches!(result, KeyResult::RevertTheme(t) if *t == Theme::default()));
    }

    #[tokio::test]
    async fn test_unknown_key_returns_ignored() {
        let mut tab = make_tab().await;
        let (_, result) = press(
            ThemePickerMode::new(entries(&["dracula"]), Theme::default()),
            &mut tab,
            KeyCode::F(5),
        )
        .await;
        assert!(matches!(result, KeyResult::Ignored));
    }

    #[test]
    fn test_mode_bar_content_contains_theme_label() {
        let mode = ThemePickerMode::new(entries(&["dracula"]), Theme::default());
        let kb = Keybindings::default();
        let theme = Theme::default();
        let line = mode.mode_bar_content(&kb, &theme);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("THEME"));
    }
}
