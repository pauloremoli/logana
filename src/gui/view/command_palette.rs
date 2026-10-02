use crate::gui::app::App;
use crate::gui::state::GuiState;
use crate::mode::app_mode::ModeRenderState;
use gpui_kit::component::ActiveTheme;
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{AnyElement, Context, Pixels, Size, anchored, deferred, div, point, px};

const PALETTE_WIDTH: f32 = 600.0;
const PALETTE_TOP_OFFSET: f32 = 120.0;

/// A VS Code command-palette-style overlay for Command mode (`:filter
/// ERROR`, etc.): floats centered near the top of the window instead of
/// inline in the search bar. `deferred(anchored())` is what makes this a
/// true overlay — it paints in a later pass, escaping the normal layout
/// flow (and every ancestor's `overflow_hidden()`) so it renders over the
/// table/sidebar/nav rail instead of being clipped by them. `None` when
/// Command mode isn't active, so callers only add this child when there's
/// actually something to show.
pub fn command_palette(
    state: &GuiState,
    viewport: Size<Pixels>,
    cx: &mut Context<App>,
) -> Option<AnyElement> {
    let tab = state.active_tab()?;
    let ModeRenderState::Command { input, .. } = tab.interaction.mode.render_state() else {
        return None;
    };
    Some(
        deferred(
            anchored().position(point(px(0.), px(0.))).child(
                // Centering via a full-viewport-width flex row instead of
                // computing `(viewport.width - PALETTE_WIDTH) / 2` in
                // pixels directly — `Pixels` has no `Sub` impl in this
                // gpui version, only `Mul<f32>`/`Div<Pixels>`.
                div()
                    .w(viewport.width)
                    .flex()
                    .justify_center()
                    .pt(px(PALETTE_TOP_OFFSET))
                    .child(
                        div()
                            .w(px(PALETTE_WIDTH))
                            .rounded(px(8.))
                            .bg(cx.theme().background)
                            .border_1()
                            .border_color(cx.theme().border)
                            .shadow_md()
                            .p(px(12.))
                            .child(format!(":{input}")),
                    ),
            ),
        )
        .into_any_element(),
    )
}
