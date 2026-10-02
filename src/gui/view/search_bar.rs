use crate::gui::state::GuiState;
use crate::mode::app_mode::ModeRenderState;
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{div, px};

const SEARCH_BAR_HEIGHT: f32 = 32.0;
const PLACEHOLDER: &str = "Search logs... (regex, field:value, or free text)";

/// Shows the `/` (forward) or `?` (backward) search prompt and its live
/// query while `SearchMode` is active; a static placeholder otherwise.
/// Typing, matching, and navigation are all handled by the TUI's real
/// `SearchMode`/`Search` — this only reflects its `render_state`.
pub fn search_bar(state: &GuiState) -> impl IntoElement {
    let text = match state
        .active_tab()
        .map(|tab| tab.interaction.mode.render_state())
    {
        Some(ModeRenderState::Search { query, forward, .. }) => {
            let prompt = if forward { "/" } else { "?" };
            format!("{prompt}{query}")
        }
        _ => PLACEHOLDER.to_string(),
    };
    div()
        .flex()
        .items_center()
        .h(px(SEARCH_BAR_HEIGHT))
        .px(px(8.))
        .overflow_hidden()
        .child(text)
}
