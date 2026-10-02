use crate::commands::auto_complete::fuzzy_match;
use crate::gui::app::App;
use crate::gui::color::ratatui_color_to_gpui;
use crate::gui::state::GuiState;
use crate::mode::app_mode::ModeRenderState;
use crate::mode::value_colors_mode::{ValueColorEntry, ValueColorGroup, ValueColorRow};
use gpui_kit::component::ActiveTheme;
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{
    AnyElement, Context, FontWeight, Pixels, Size, anchored, deferred, div, point, px,
};

const PANEL_WIDTH: f32 = 360.0;
const PANEL_TOP_OFFSET: f32 = 48.0;
const VISIBLE_ROWS: usize = 16;

/// The `:value-colors`/`:level-colors` popups, as a floating overlay —
/// same pattern as the other mode overlays. Both share `ValueColorsMode`
/// (see `ModeRenderState::ValueColors`/`LevelColors`), distinguished only
/// by title here. `groups` is the full, unfiltered list (same situation
/// as `ThemePicker`'s `entries`), so this re-derives the search-filtered
/// row list the same way the TUI's own `ValueColorsPopup` widget does —
/// inline, not via `ValueColorsMode::visible_rows` (an instance method
/// the cloned render-state data can't call either; the TUI's own popup
/// widget duplicates this same filter for the same reason). `None` when
/// neither mode is active.
pub fn value_colors_overlay(
    state: &GuiState,
    viewport: Size<Pixels>,
    cx: &mut Context<App>,
) -> Option<AnyElement> {
    let tab = state.active_tab()?;
    let (groups, search, selected, title) = match tab.interaction.mode.render_state() {
        ModeRenderState::ValueColors {
            groups,
            search,
            selected,
        } => (groups, search, selected, "Value Colors"),
        ModeRenderState::LevelColors {
            groups,
            search,
            selected,
        } => (groups, search, selected, "Level Colors"),
        _ => return None,
    };
    let rows = visible_rows(&groups, &search);
    let scroll = selected
        .saturating_sub(VISIBLE_ROWS.saturating_sub(1))
        .min(rows.len().saturating_sub(VISIBLE_ROWS));

    let subtitle = if search.is_empty() {
        format!("{title}  (space to toggle, a/n all/none, Enter to apply, Esc to cancel)")
    } else {
        format!("{title} — /{search}")
    };
    let mut list = div().flex().flex_col().p(px(4.));
    for (idx, row) in rows.iter().enumerate().skip(scroll).take(VISIBLE_ROWS) {
        list = list.child(value_color_row(&groups, row, idx == selected, cx));
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
                .child(subtitle),
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

fn visible_rows(groups: &[ValueColorGroup], search: &str) -> Vec<ValueColorRow> {
    let mut rows = Vec::new();
    for (gi, group) in groups.iter().enumerate() {
        if search.is_empty() {
            rows.push(ValueColorRow::Group(gi));
            for (ei, _) in group.children.iter().enumerate() {
                rows.push(ValueColorRow::Entry(gi, ei));
            }
        } else {
            let matching: Vec<usize> = group
                .children
                .iter()
                .enumerate()
                .filter(|(_, e)| fuzzy_match(search, &format!("{} {}", group.label, e.label)))
                .map(|(i, _)| i)
                .collect();
            if !matching.is_empty() {
                rows.push(ValueColorRow::Group(gi));
                for ei in matching {
                    rows.push(ValueColorRow::Entry(gi, ei));
                }
            }
        }
    }
    rows
}

fn value_color_row(
    groups: &[ValueColorGroup],
    row: &ValueColorRow,
    is_selected: bool,
    cx: &Context<App>,
) -> AnyElement {
    let mut el = match row {
        ValueColorRow::Group(gi) => {
            let group = &groups[*gi];
            let all = group.children.iter().all(|e| e.enabled);
            let none = group.children.iter().all(|e| !e.enabled);
            let mark = if all {
                "[x]"
            } else if none {
                "[ ]"
            } else {
                "[~]"
            };
            div()
                .flex()
                .items_center()
                .gap_2()
                .px(px(8.))
                .py(px(2.))
                .font_weight(FontWeight::BOLD)
                .child(mark)
                .child(group.label.clone())
        }
        ValueColorRow::Entry(gi, ei) => {
            let entry: &ValueColorEntry = &groups[*gi].children[*ei];
            let mark = if entry.enabled { "[x]" } else { "[ ]" };
            let mut row = div()
                .flex()
                .items_center()
                .gap_2()
                .pl(px(24.))
                .pr(px(8.))
                .py(px(2.))
                .child(mark);
            if let Some(color) = ratatui_color_to_gpui(entry.color) {
                row = row.child(div().flex_shrink_0().w(px(8.)).h(px(8.)).bg(color));
            }
            row.child(entry.label.clone())
        }
    };
    if is_selected {
        el = el.bg(cx.theme().accent);
    }
    el.into_any_element()
}
