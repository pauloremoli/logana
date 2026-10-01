use super::filter_pane::{self, FilterManagementView};
use super::group_pane::{self, GroupManagementView};
use crate::gui::app::App;
use crate::mode::app_mode::ModeRenderState;
use crate::ui::TabState;
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{Context, div, px};

const SIDEBAR_WIDTH: f32 = 320.0;

pub fn sidebar(tab_idx: usize, tab: &TabState, cx: &mut Context<App>) -> impl IntoElement {
    let names = tab.log_manager.group_names();
    let filter_defs = tab.log_manager.get_filters();
    let group_defs = tab.log_manager.get_group_styles();
    let render_state = tab.interaction.mode.render_state();

    let filter_management = match &render_state {
        ModeRenderState::FilterManagement {
            selected_index,
            search,
            searching,
        } => Some(FilterManagementView {
            selected_index: *selected_index,
            search: search.clone(),
            searching: *searching,
        }),
        _ => None,
    };
    let group_management = match render_state {
        ModeRenderState::GroupManagement {
            selected_group,
            search,
            searching,
        } => Some(GroupManagementView {
            selected_group,
            search,
            searching,
        }),
        _ => None,
    };

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
            filter_management.as_ref(),
            cx,
        ))
        .child(group_pane::group_pane(
            tab_idx,
            &names,
            group_defs,
            filter_defs,
            group_management.as_ref(),
            cx,
        ))
}
