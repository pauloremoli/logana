use crate::commands::auto_complete::fuzzy_match;
use crate::gui::app::App;
use crate::gui::state::GuiState;
use crate::mode::app_mode::ModeRenderState;
use crate::ui::TabId;
use gpui_kit::component::ActiveTheme;
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{
    AnyElement, Context, FontWeight, Pixels, Size, anchored, deferred, div, point, px,
};

const PANEL_WIDTH: f32 = 420.0;
const PANEL_TOP_OFFSET: f32 = 48.0;
const VISIBLE_ROWS: usize = 14;

/// `Ctrl+P`'s quick-open popup, as a floating overlay — same pattern as
/// the other mode overlays. `entries` in `ModeRenderState::FileSwitcher`
/// is the full, unfiltered tab list (same situation as `ThemePicker`'s
/// `entries`), so this re-derives the fuzzy-filtered list the same way
/// `FileSwitcherMode::visible_entries` does. `None` when
/// `FileSwitcherMode` isn't active.
pub fn file_switcher_overlay(
    state: &GuiState,
    viewport: Size<Pixels>,
    cx: &mut Context<App>,
) -> Option<AnyElement> {
    let tab = state.active_tab()?;
    let ModeRenderState::FileSwitcher {
        entries,
        active_tab,
        selected,
        search,
    } = tab.interaction.mode.render_state()
    else {
        return None;
    };
    let visible: Vec<&(TabId, String)> = if search.is_empty() {
        entries.iter().collect()
    } else {
        entries
            .iter()
            .filter(|(_, title)| fuzzy_match(&search, title))
            .collect()
    };
    let scroll = selected
        .saturating_sub(VISIBLE_ROWS.saturating_sub(1))
        .min(visible.len().saturating_sub(VISIBLE_ROWS));

    let title = if search.is_empty() {
        "Switch File  (type to filter, Enter to switch, Esc to cancel)".to_string()
    } else {
        format!("Switch File — /{search}")
    };
    let mut list = div().flex().flex_col().p(px(4.));
    for (idx, (tab_id, label)) in visible.iter().enumerate().skip(scroll).take(VISIBLE_ROWS) {
        let mut row = div().flex().items_center().gap_2().px(px(8.)).py(px(4.));
        if *tab_id == active_tab {
            row = row.child("●").font_weight(FontWeight::BOLD);
        } else {
            row = row.child(" ");
        }
        row = row.child(label.clone());
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
