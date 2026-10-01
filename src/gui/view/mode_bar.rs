use crate::gui::state::{GuiState, InteractionMode, StatusMessage};
use gpui_kit::gpui::div;
use gpui_kit::gpui::prelude::*;

/// The bottom bar: the active mode and its keybindings, plus any pending
/// status (e.g. a bad `:` command).
pub fn mode_bar(state: &GuiState) -> impl IntoElement {
    div().flex().gap_5().child(mode_bar_text(state))
}

/// Plain-text content of the mode bar, independent of any gpui type so
/// it's directly unit-testable.
pub fn mode_bar_text(state: &GuiState) -> String {
    let mode = match &state.mode {
        InteractionMode::Normal => {
            "[NORMAL]  j/k scroll  gg/G top/bottom  Tab/Shift-Tab switch tab  : command  q quit"
                .to_string()
        }
        InteractionMode::Command { input } => format!("[COMMAND]  :{input}"),
    };
    match &state.status {
        Some(StatusMessage::Error(err)) => format!("{mode}   {err}"),
        None => mode,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use std::sync::Arc;

    async fn state() -> GuiState {
        let db = Arc::new(Database::in_memory().await.unwrap());
        GuiState::new(db)
    }

    #[tokio::test]
    async fn normal_mode_shows_the_keybinding_hints() {
        let state = state().await;
        assert!(mode_bar_text(&state).starts_with("[NORMAL]"));
    }

    #[tokio::test]
    async fn command_mode_shows_the_prompt_and_input() {
        let mut state = state().await;
        state.mode = InteractionMode::Command {
            input: "filter ERROR".to_string(),
        };
        assert_eq!(mode_bar_text(&state), "[COMMAND]  :filter ERROR");
    }

    #[tokio::test]
    async fn an_error_status_is_appended_after_the_mode_text() {
        let mut state = state().await;
        state.status = Some(StatusMessage::Error("bad command".to_string()));
        assert!(mode_bar_text(&state).ends_with("bad command"));
    }
}
