use crate::gui::state::{GuiState, InteractionMode};
use gpui_kit::gpui::div;
use gpui_kit::gpui::prelude::*;

/// Shows the `:` prompt and its current input while Command mode is
/// active; empty otherwise. Typing is handled entirely by
/// `update::handle_command_mode_key` — this just reflects `state.mode`.
pub fn command_bar(state: &GuiState) -> impl IntoElement {
    let text = match &state.mode {
        InteractionMode::Command { input } => format!(":{input}"),
        InteractionMode::Normal => String::new(),
    };
    div().p_1().child(text)
}
