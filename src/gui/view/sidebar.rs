use super::filter_pane::{self, FilterManagementView};
use super::group_pane::{self, GroupManagementView};
use crate::gui::app::App;
use crate::gui::message::Message;
use crate::gui::state::SidebarTab;
use crate::mode::app_mode::ModeRenderState;
use crate::ui::TabState;
use gpui_kit::assets::IconName;
use gpui_kit::base::Selectable;
use gpui_kit::component::Icon;
use gpui_kit::component::button::Button;
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{Context, Window, div, px};

const SIDEBAR_WIDTH: f32 = 320.0;
const SIDEBAR_HEADER_HEIGHT: f32 = 32.0;

pub fn sidebar(
    tab_idx: usize,
    tab: &TabState,
    sidebar_tab: SidebarTab,
    cx: &mut Context<App>,
) -> impl IntoElement {
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

    let mut col = div()
        .flex()
        .flex_col()
        .gap_4()
        .w(px(SIDEBAR_WIDTH))
        .p_2()
        .child(sidebar_header(sidebar_tab, cx));
    col = match sidebar_tab {
        SidebarTab::Filters => col.child(filter_pane::filter_pane(
            tab_idx,
            filter_defs,
            group_defs,
            filter_management.as_ref(),
            cx,
        )),
        SidebarTab::Groups => col.child(group_pane::group_pane(
            tab_idx,
            &names,
            group_defs,
            filter_defs,
            group_management.as_ref(),
            cx,
        )),
    };
    col
}

/// Tab switcher (Filters/**Groups** — Groups replaces the mockup's
/// Annotations tab, no annotations UI exists yet) plus a close button
/// reusing the same `show_sidebar` toggle the keyboard-driven
/// `KeyResult::ToggleSidebar` already flips.
fn sidebar_header(active: SidebarTab, cx: &mut Context<App>) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .h(px(SIDEBAR_HEADER_HEIGHT))
        .overflow_hidden()
        .child(
            div()
                .flex()
                .gap_2()
                .child(sidebar_tab_button(
                    IconName::Funnel,
                    "Filters",
                    SidebarTab::Filters,
                    active == SidebarTab::Filters,
                    cx,
                ))
                .child(sidebar_tab_button(
                    IconName::Group,
                    "Groups",
                    SidebarTab::Groups,
                    active == SidebarTab::Groups,
                    cx,
                )),
        )
        .child(
            Button::new("sidebar-close")
                .icon(Icon::new(IconName::X))
                .on_click(cx.listener(|app: &mut App, _, window: &mut Window, cx| {
                    app.dispatch(Message::SidebarToggled, window, cx);
                })),
        )
}

fn sidebar_tab_button(
    icon: IconName,
    label: &'static str,
    tab: SidebarTab,
    active: bool,
    cx: &mut Context<App>,
) -> impl IntoElement {
    let mut button = Button::new(label)
        .icon(Icon::new(icon))
        .label(label)
        .on_click(
            cx.listener(move |app: &mut App, _, window: &mut Window, cx| {
                app.dispatch(Message::SidebarTabSelected(tab), window, cx);
            }),
        );
    if active {
        button = button.selected(true);
    }
    button
}
