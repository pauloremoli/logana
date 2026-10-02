use crate::commands::auto_complete::fuzzy_match;
use crate::gui::app::App;
use crate::gui::state::GuiState;
use crate::mode::app_mode::ModeRenderState;
use gpui_kit::component::ActiveTheme;
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{AnyElement, Context, Pixels, Size, anchored, deferred, div, point, px};

const PANEL_WIDTH: f32 = 320.0;
const PANEL_TOP_OFFSET: f32 = 48.0;
const VISIBLE_ROWS: usize = 14;

/// The `:theme` picker, as a floating overlay — same `deferred(anchored())`
/// pattern as `command_palette`/`keybindings_help_overlay`. `entries` in
/// `ModeRenderState::ThemePicker` is the full, unfiltered theme list (per
/// its own doc comment), so this re-derives the same fuzzy-filtered,
/// `selected`-indexed subset `ThemePickerMode::visible_entries` computes —
/// via the same `fuzzy_match` it uses — rather than assuming `entries` is
/// already filtered. `None` when `ThemePickerMode` isn't active.
pub fn theme_picker_overlay(
    state: &GuiState,
    viewport: Size<Pixels>,
    cx: &mut Context<App>,
) -> Option<AnyElement> {
    let tab = state.active_tab()?;
    let ModeRenderState::ThemePicker {
        entries,
        selected,
        search,
    } = tab.interaction.mode.render_state()
    else {
        return None;
    };
    let visible: Vec<&String> = if search.is_empty() {
        entries.iter().collect()
    } else {
        entries
            .iter()
            .filter(|name| fuzzy_match(&search, name))
            .collect()
    };
    let scroll = selected
        .saturating_sub(VISIBLE_ROWS - 1)
        .min(visible.len().saturating_sub(VISIBLE_ROWS));

    let title = if search.is_empty() {
        "Theme  (type to filter, Enter to apply, Esc to cancel)".to_string()
    } else {
        format!("Theme — /{search}")
    };
    let mut list = div().flex().flex_col().p(px(4.));
    for (idx, name) in visible.iter().enumerate().skip(scroll).take(VISIBLE_ROWS) {
        let mut row = div()
            .px(px(8.))
            .py(px(4.))
            .rounded(px(4.))
            .child((*name).clone());
        if idx == selected {
            row = row.bg(cx.theme().accent);
        }
        list = list.child(row);
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
