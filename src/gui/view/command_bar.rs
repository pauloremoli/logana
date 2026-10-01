use crate::gui::state::GuiState;
use crate::mode::app_mode::ModeRenderState;
use gpui_kit::gpui::div;
use gpui_kit::gpui::prelude::*;

/// Shows the `:` prompt and its current input while Command mode is
/// active; empty otherwise. Typing is handled entirely by the active
/// tab's real `Mode::handle_key` — this just reflects its `render_state`.
pub fn command_bar(state: &GuiState) -> impl IntoElement {
    let text = match state
        .active_tab()
        .map(|tab| tab.interaction.mode.render_state())
    {
        Some(ModeRenderState::Command { input, .. }) => format!(":{input}"),
        _ => String::new(),
    };
    div().p_1().child(text)
}
