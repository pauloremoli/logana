use crate::gui::app::App;
use crate::gui::state::GuiState;
use crate::mode::app_mode::ModeRenderState;
use crate::mode::keybindings_help_mode::{HelpRow, build_help_rows, filter_rows};
use gpui_kit::component::ActiveTheme;
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{
    AnyElement, Context, FontWeight, Pixels, Size, anchored, deferred, div, point, px,
};

const PANEL_WIDTH: f32 = 480.0;
const PANEL_TOP_OFFSET: f32 = 48.0;
/// How many rows show at once — `KeybindingsHelpMode`'s own `scroll` (an
/// index into the filtered row list, advanced by the mode's own
/// `j`/`k`/`PageUp`/`PageDown` handling) picks the window, same as the
/// TUI's popup; this just needs a fixed window size instead of deriving
/// one from a terminal's row count.
const VISIBLE_ROWS: usize = 20;

/// `?`'s keybindings-help popup, as a floating overlay — same
/// `deferred(anchored())` pattern as `command_palette`. Reuses
/// `mode::keybindings_help_mode::{build_help_rows, filter_rows}`, the
/// exact pure functions the TUI's own `KeybindingsHelpPopup` widget
/// calls, so the GUI's row text/grouping/fuzzy-search can never drift
/// from the TUI's. `None` when `KeybindingsHelpMode` isn't active.
pub fn keybindings_help_overlay(
    state: &GuiState,
    viewport: Size<Pixels>,
    cx: &mut Context<App>,
) -> Option<AnyElement> {
    let tab = state.active_tab()?;
    let ModeRenderState::KeybindingsHelp { scroll, search } = tab.interaction.mode.render_state()
    else {
        return None;
    };
    let all_rows = build_help_rows(&tab.interaction.keybindings);
    let rows = filter_rows(&all_rows, &search);
    let scroll = scroll.min(rows.len().saturating_sub(VISIBLE_ROWS));

    let title = if search.is_empty() {
        "Keybindings  (type to filter, Esc to close)".to_string()
    } else {
        format!("Keybindings — /{search}")
    };
    let mut list = div().flex().flex_col().p(px(8.));
    for row in rows.iter().skip(scroll).take(VISIBLE_ROWS) {
        list = list.child(help_row(row, cx));
    }

    let panel = div()
        .w(px(PANEL_WIDTH))
        .overflow_hidden()
        .rounded(px(8.))
        .bg(cx.theme().background)
        .border_1()
        .border_color(cx.theme().border)
        .shadow_md()
        .flex()
        .flex_col()
        .child(
            div()
                .px(px(12.))
                .py(px(8.))
                .border_b_1()
                .border_color(cx.theme().border)
                .child(title),
        )
        .child(list);

    Some(
        deferred(
            anchored().position(point(px(0.), px(0.))).child(
                div()
                    .w(viewport.width)
                    .flex()
                    .justify_center()
                    .pt(px(PANEL_TOP_OFFSET))
                    .child(panel),
            ),
        )
        .into_any_element(),
    )
}

fn help_row(row: &HelpRow, cx: &Context<App>) -> AnyElement {
    match row {
        HelpRow::Header(title) => div()
            .pt(px(8.))
            .pb(px(2.))
            .font_weight(FontWeight::BOLD)
            .text_color(cx.theme().muted_foreground)
            .child(title.clone())
            .into_any_element(),
        HelpRow::Entry { action, keys } => div()
            .flex()
            .items_center()
            .gap_4()
            .child(
                div()
                    .flex_shrink_0()
                    .w(px(96.))
                    .text_color(cx.theme().muted_foreground)
                    .child(keys.clone()),
            )
            .child(div().flex_1().overflow_hidden().child(action.clone()))
            .into_any_element(),
    }
}
