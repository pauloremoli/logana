use crate::config::Keybindings;
use crate::mode::app_mode::{Mode, ModeRenderState, status_entry, status_entry_dyn};
use crate::mode::normal_mode::NormalMode;
use crate::theme::Theme;
use crate::ui::{KeyResult, TabState};
use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

#[derive(Debug)]
pub struct UiMode {
    pub sidebar: bool,
    pub mode_bar: bool,
    pub borders: bool,
    pub wrap: bool,
    pub relative_line_numbers: bool,
}

impl UiMode {
    pub fn from_tab(tab: &TabState) -> Self {
        Self {
            sidebar: tab.display.show_sidebar,
            mode_bar: tab.display.show_mode_bar,
            borders: tab.display.show_borders,
            wrap: tab.display.wrap,
            relative_line_numbers: tab.display.relative_line_numbers,
        }
    }
}

#[async_trait]
impl Mode for UiMode {
    async fn handle_key(
        self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        let kb = tab.interaction.keybindings.clone();

        if kb.ui.exit.matches(key, modifiers) {
            return (Box::new(NormalMode::default()), KeyResult::Handled);
        }

        if kb.ui.toggle_sidebar.matches(key, modifiers) {
            return (Box::new(NormalMode::default()), KeyResult::ToggleSidebar);
        }

        if kb.ui.toggle_mode_bar.matches(key, modifiers) {
            return (Box::new(NormalMode::default()), KeyResult::ToggleModeBar);
        }

        if kb.ui.toggle_borders.matches(key, modifiers) {
            return (Box::new(NormalMode::default()), KeyResult::ToggleBorders);
        }

        if kb.ui.toggle_wrap.matches(key, modifiers) {
            return (Box::new(NormalMode::default()), KeyResult::ToggleWrap);
        }

        if kb.ui.toggle_relative_line_numbers.matches(key, modifiers) {
            return (
                Box::new(NormalMode::default()),
                KeyResult::ToggleRelativeLineNumbers,
            );
        }

        // Pass global keys (quit, tab switch) through to App.
        (self, KeyResult::Ignored)
    }

    fn render_state(&self) -> ModeRenderState {
        ModeRenderState::Ui
    }

    fn mode_bar_content(&self, kb: &Keybindings, theme: &Theme) -> Line<'static> {
        let mut spans = vec![Span::styled(
            "[UI]  ",
            Style::default()
                .fg(theme.text_highlight_fg)
                .add_modifier(Modifier::BOLD),
        )];

        let on_off = |on: bool| if on { "[ON]" } else { "[OFF]" };
        status_entry_dyn(
            &mut spans,
            kb.ui.toggle_sidebar.display(),
            format!("sidebar{}", on_off(self.sidebar)),
            theme,
        );
        status_entry_dyn(
            &mut spans,
            kb.ui.toggle_mode_bar.display(),
            format!("mode bar{}", on_off(self.mode_bar)),
            theme,
        );
        status_entry_dyn(
            &mut spans,
            kb.ui.toggle_borders.display(),
            format!("borders{}", on_off(self.borders)),
            theme,
        );
        status_entry_dyn(
            &mut spans,
            kb.ui.toggle_wrap.display(),
            format!("wrap{}", on_off(self.wrap)),
            theme,
        );
        status_entry_dyn(
            &mut spans,
            kb.ui.toggle_relative_line_numbers.display(),
            format!("relative numbers{}", on_off(self.relative_line_numbers)),
            theme,
        );
        status_entry(&mut spans, kb.ui.exit.display(), "back", theme);

        Line::from(spans)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::db::LogManager;
    use crate::ingestion::FileReader;
    use std::sync::Arc;

    async fn make_tab() -> TabState {
        let file_reader = FileReader::from_bytes(b"line".to_vec());
        let db = Arc::new(Database::in_memory().await.unwrap());
        let lm = LogManager::new(db, None).await;
        TabState::new(file_reader, lm, "test".to_string())
    }

    async fn press(
        tab: &mut TabState,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        let mode = Box::new(UiMode::from_tab(tab));
        mode.handle_key(tab, code, modifiers).await
    }

    #[tokio::test]
    async fn test_esc_returns_to_normal_mode() {
        let mut tab = make_tab().await;
        let (mode, result) = press(&mut tab, KeyCode::Esc, KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(matches!(mode.render_state(), ModeRenderState::Normal));
        // Must be NormalMode (not UiMode) — NormalMode::default() debug name
        assert!(!format!("{:?}", mode).contains("UiMode"));
    }

    #[tokio::test]
    async fn test_s_returns_toggle_sidebar() {
        let mut tab = make_tab().await;
        let (_, result) = press(&mut tab, KeyCode::Char('s'), KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::ToggleSidebar));
    }

    #[tokio::test]
    async fn test_b_returns_toggle_mode_bar() {
        let mut tab = make_tab().await;
        let (_, result) = press(&mut tab, KeyCode::Char('b'), KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::ToggleModeBar));
    }

    #[tokio::test]
    async fn test_capital_b_returns_toggle_borders() {
        let mut tab = make_tab().await;
        let (_, result) = press(&mut tab, KeyCode::Char('B'), KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::ToggleBorders));
    }

    #[tokio::test]
    async fn test_w_returns_toggle_wrap() {
        let mut tab = make_tab().await;
        let (_, result) = press(&mut tab, KeyCode::Char('w'), KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::ToggleWrap));
    }

    #[tokio::test]
    async fn test_r_returns_toggle_relative_line_numbers() {
        let mut tab = make_tab().await;
        let (_, result) = press(&mut tab, KeyCode::Char('r'), KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::ToggleRelativeLineNumbers));
    }

    #[tokio::test]
    async fn test_returns_to_normal_mode_after_toggle() {
        let mut tab = make_tab().await;
        for key in [
            KeyCode::Char('s'),
            KeyCode::Char('B'),
            KeyCode::Char('w'),
            KeyCode::Char('r'),
        ] {
            let (mode, _) = press(&mut tab, key, KeyModifiers::NONE).await;
            assert!(
                matches!(mode.render_state(), ModeRenderState::Normal),
                "Expected NormalMode after pressing {:?}, got {:?}",
                key,
                mode.render_state()
            );
        }
    }

    #[tokio::test]
    async fn test_unknown_key_returns_ignored() {
        let mut tab = make_tab().await;
        let (_, result) = press(&mut tab, KeyCode::Char('z'), KeyModifiers::NONE).await;
        assert!(matches!(result, KeyResult::Ignored));
    }

    #[tokio::test]
    async fn test_snapshot_reflects_state() {
        let mut tab = make_tab().await;
        tab.display.show_sidebar = false;
        let mode = UiMode::from_tab(&tab);
        assert!(!mode.sidebar);
        tab.display.show_sidebar = true;
        let mode = UiMode::from_tab(&tab);
        assert!(mode.sidebar);
    }

    #[tokio::test]
    async fn test_render_state_is_ui() {
        let tab = make_tab().await;
        let mode = UiMode::from_tab(&tab);
        assert!(matches!(mode.render_state(), ModeRenderState::Ui));
    }
}
