use crate::gui::app::App;
use crate::gui::color::resolve_line_color;
use crate::gui::state::TabState;
use gpui_kit::base::{VirtualListScrollHandle, v_virtual_list};
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{Context, Size, div, px};
use std::rc::Rc;

/// Fixed row height, used for every visible log line — the virtual list
/// only needs height to lay out rows; width is inferred from content.
const ROW_HEIGHT_PX: f32 = 20.0;

/// Renders only the currently-visible window of `tab.visible_lines`,
/// via gpui-component's virtual-scrolling list — no manual viewport math
/// or rendered-line cap, unlike the discarded iced prototype. `log_scroll`
/// is `App`'s own handle (not reachable through `cx`, which derefs to
/// gpui's own `App` platform type, not ours), passed in so
/// `Effect::Scroll` can drive the same handle from `apply_effect`.
pub fn log_pane(
    tab_idx: usize,
    tab: &TabState,
    log_scroll: &VirtualListScrollHandle,
    cx: &mut Context<App>,
) -> impl IntoElement {
    let total = tab.visible_lines.len();
    let item_sizes = Rc::new(vec![Size::new(px(0.0), px(ROW_HEIGHT_PX)); total]);
    let view = cx.entity();
    v_virtual_list(
        view,
        "log-pane",
        item_sizes,
        move |app: &mut App, range, _window, _cx| {
            let Some(tab) = app.state.tabs.get(tab_idx) else {
                return Vec::new();
            };
            range
                .filter_map(|pos| tab.visible_lines.get(pos).copied())
                .map(|line_idx| line_row(tab, line_idx))
                .collect()
        },
    )
    .track_scroll(log_scroll)
    .size_full()
}

/// Each row renders as a single line, matching the TUI's default (`:wrap`
/// is off until a later phase implements it): wrapping a long line onto
/// multiple visual lines without the virtual list reserving extra height
/// for it just makes rows overlap, so overflow is clipped with an ellipsis
/// instead.
fn line_row(tab: &TabState, line_idx: usize) -> gpui_kit::gpui::AnyElement {
    let bytes = tab.reader.get_line_zero_copy(line_idx);
    let text = String::from_utf8_lossy(bytes).into_owned();
    let mut row = div().truncate().child(text);
    if let Some(color) = resolve_line_color(bytes, &tab.filter_defs, &tab.group_defs) {
        row = row.text_color(color);
    }
    row.into_any_element()
}
