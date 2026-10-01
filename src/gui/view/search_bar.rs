use crate::gui::state::GuiState;
use crate::mode::app_mode::ModeRenderState;
use gpui_kit::gpui::div;
use gpui_kit::gpui::prelude::*;

/// Shows the `/` (forward) or `?` (backward) search prompt and its live
/// query while `SearchMode` is active; empty otherwise. Typing, matching,
/// and navigation are all handled by the TUI's real `SearchMode`/
/// `Search` — this only reflects its `render_state`.
pub fn search_bar(state: &GuiState) -> impl IntoElement {
    let text = match state
        .active_tab()
        .map(|tab| tab.interaction.mode.render_state())
    {
        Some(ModeRenderState::Search { query, forward, .. }) => {
            let prompt = if forward { "/" } else { "?" };
            format!("{prompt}{query}")
        }
        _ => String::new(),
    };
    div().p_1().child(text)
}
