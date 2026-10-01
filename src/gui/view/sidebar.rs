use super::{filter_pane, group_pane};
use crate::gui::app::App;
use crate::gui::state::TabState;
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{Context, div, px};

const SIDEBAR_WIDTH: f32 = 320.0;

pub fn sidebar(tab_idx: usize, tab: &TabState, cx: &mut Context<App>) -> impl IntoElement {
    let names = tab.log_manager.group_names();
    div()
        .flex()
        .flex_col()
        .gap_4()
        .w(px(SIDEBAR_WIDTH))
        .p_2()
        .child(filter_pane::filter_pane(
            tab_idx,
            &tab.filter_defs,
            &tab.group_defs,
            cx,
        ))
        .child(group_pane::group_pane(
            tab_idx,
            &names,
            &tab.group_defs,
            &tab.filter_defs,
            cx,
        ))
}
