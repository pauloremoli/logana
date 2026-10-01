use crate::gui::state::{GuiState, StatusMessage};
use gpui_kit::gpui::div;
use gpui_kit::gpui::prelude::*;

/// The bottom bar: the active mode and its keybindings (reusing the TUI's
/// own `Mode::mode_bar_content`, so the hints always match the mode's
/// real, live-rebindable keybindings), plus any pending status.
pub fn mode_bar(state: &GuiState) -> impl IntoElement {
    div().flex().gap_5().child(mode_bar_text(state))
}

/// Plain-text content of the mode bar, independent of any gpui type so
/// it's directly unit-testable. `Mode::mode_bar_content` returns a styled
/// `ratatui::text::Line` (colors, bold); only the text is kept — gpui
/// rendering doesn't reuse ratatui's `Style` type.
pub fn mode_bar_text(state: &GuiState) -> String {
    let mode = state
        .active_tab()
        .map(|tab| {
            flatten_line(
                &tab.interaction
                    .mode
                    .mode_bar_content(&tab.interaction.keybindings, &state.theme),
            )
        })
        .unwrap_or_default();
    match &state.status {
        Some(StatusMessage::Error(err)) => format!("{mode}   {err}"),
        None => mode,
    }
}

fn flatten_line(line: &ratatui::text::Line<'static>) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use ratatui::style::Style;
    use ratatui::text::{Line, Span};
    use std::sync::Arc;

    async fn state() -> GuiState {
        let db = Arc::new(Database::in_memory().await.unwrap());
        GuiState::new(db)
    }

    #[tokio::test]
    async fn no_tabs_renders_an_empty_mode_bar() {
        let state = state().await;
        assert_eq!(mode_bar_text(&state), "");
    }

    #[tokio::test]
    async fn an_error_status_is_appended_after_the_mode_text() {
        let mut state = state().await;
        state.status = Some(StatusMessage::Error("bad command".to_string()));
        assert!(mode_bar_text(&state).ends_with("bad command"));
    }

    #[test]
    fn flatten_line_concatenates_span_text_ignoring_style() {
        let line = Line::from(vec![
            Span::styled("[NORMAL]", Style::default()),
            Span::raw("  "),
            Span::styled("<q>", Style::default()),
            Span::raw(" quit"),
        ]);
        assert_eq!(flatten_line(&line), "[NORMAL]  <q> quit");
    }

    #[test]
    fn flatten_line_of_an_empty_line_is_empty() {
        assert_eq!(flatten_line(&Line::default()), "");
    }
}
