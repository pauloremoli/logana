use super::{filter_pane, group_pane};
use crate::gui::app::App;
use crate::ui::TabState;
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{Context, div, px};

const SIDEBAR_WIDTH: f32 = 320.0;

pub fn sidebar(tab_idx: usize, tab: &TabState, cx: &mut Context<App>) -> impl IntoElement {
    let names = tab.log_manager.group_names();
    let filter_defs = tab.log_manager.get_filters();
    let group_defs = tab.log_manager.get_group_styles();
    div()
        .flex()
        .flex_col()
        .gap_4()
        .w(px(SIDEBAR_WIDTH))
        .p_2()
        .child(filter_pane::filter_pane(
            tab_idx,
            filter_defs,
            group_defs,
            cx,
        ))
        .child(group_pane::group_pane(
            tab_idx,
            &names,
            group_defs,
            filter_defs,
            cx,
        ))
}
