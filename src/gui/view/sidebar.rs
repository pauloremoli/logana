use super::field_facets;
use super::filter_pane;
use super::group_pane;
use crate::gui::app::App;
use crate::gui::message::Message;
use crate::gui::state::SidebarTab;
use crate::shared::TabState;
use gpui_kit::assets::IconName;
use gpui_kit::base::Selectable;
use gpui_kit::component::Icon;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::InputState;
use gpui_kit::component::{ActiveTheme, Sizable};
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{Context, Entity, Window, div, px};
use std::collections::HashMap;

pub(super) const SIDEBAR_WIDTH: f32 = 360.0;
const SIDEBAR_HEADER_HEIGHT: f32 = 40.0;

pub fn sidebar(
    tab_idx: usize,
    tab: &TabState,
    sidebar_tab: SidebarTab,
    keyboard_selection: Option<(SidebarTab, usize)>,
    facet_browser_open: bool,
    facet_expanded: &HashMap<String, bool>,
    facet_search: Option<&Entity<InputState>>,
    facet_counts: Option<&crate::shared::FieldValueCounts>,
    cx: &mut Context<App>,
) -> impl IntoElement {
    let names = tab.log_manager.group_names();
    let filter_defs = tab.log_manager.get_filters();
    let group_defs = tab.log_manager.get_group_styles();
    let mut col = div()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .gap_4()
        .w(px(SIDEBAR_WIDTH))
        .p_4()
        .bg(cx.theme().sidebar)
        .id("sidebar-scroll")
        .overflow_y_scroll()
        .when(tab.display.sidebar_side.is_left(), |d| d.border_r_1())
        .when(!tab.display.sidebar_side.is_left(), |d| d.border_l_1())
        .border_color(cx.theme().border)
        .child(sidebar_header(sidebar_tab, tab, cx));
    col = match sidebar_tab {
        SidebarTab::Filters => {
            let col = col.child(filter_pane::filter_pane(
                tab_idx,
                filter_defs,
                group_defs,
                keyboard_selection
                    .filter(|(scope, _)| *scope == SidebarTab::Filters)
                    .map(|(_, index)| index),
                cx,
            ));
            if !facet_browser_open {
                return col;
            }
            let col = col.child(
                Button::new("close-field-browser")
                    .ghost()
                    .small()
                    .label("Close field picker")
                    .on_click(cx.listener(|app, _, window, cx| {
                        app.dispatch(Message::FacetBrowserToggled, window, cx)
                    })),
            );
            match facet_counts {
                Some(counts) => col.child(field_facets::field_facets(
                    tab_idx,
                    counts,
                    filter_defs,
                    facet_expanded,
                    facet_search,
                    cx,
                )),
                None => col.child("Loading available filters…"),
            }
        }
        SidebarTab::Groups => col.child(group_pane::group_pane(
            tab_idx,
            &names,
            group_defs,
            filter_defs,
            keyboard_selection
                .filter(|(scope, _)| *scope == SidebarTab::Groups)
                .map(|(_, index)| index),
            cx,
        )),
    };
    col
}

fn sidebar_header(active: SidebarTab, tab: &TabState, cx: &mut Context<App>) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .h(px(SIDEBAR_HEADER_HEIGHT))
        .flex_shrink_0()
        .border_b_1()
        .border_color(cx.theme().border)
        .overflow_hidden()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(sidebar_tab_button(
                    SidebarTabButtonArgs {
                        icon: IconName::Funnel,
                        label: "Filters",
                        tab: SidebarTab::Filters,
                        active: active == SidebarTab::Filters,
                    },
                    cx,
                ))
                .child(sidebar_tab_button(
                    SidebarTabButtonArgs {
                        icon: IconName::Group,
                        label: "Groups",
                        tab: SidebarTab::Groups,
                        active: active == SidebarTab::Groups,
                    },
                    cx,
                ))
                .child(
                    Button::new("sidebar-filtering-toggle")
                        .ghost()
                        .small()
                        .icon(IconName::ListFilter)
                        .selected(tab.filter.enabled)
                        .tooltip("Toggle filtering (F)")
                        .on_click(cx.listener(|app: &mut App, _, window: &mut Window, cx| {
                            app.dispatch(
                                Message::GuiAction(crate::gui::actions::Action::ToggleFiltering),
                                window,
                                cx,
                            );
                        })),
                ),
        )
        .child(
            Button::new("sidebar-close")
                .ghost()
                .icon(Icon::new(IconName::X))
                .tooltip("Hide sidebar")
                .on_click(cx.listener(|app: &mut App, _, window: &mut Window, cx| {
                    app.dispatch(Message::SidebarToggled, window, cx);
                })),
        )
}

struct SidebarTabButtonArgs {
    icon: IconName,
    label: &'static str,
    tab: SidebarTab,
    active: bool,
}

fn sidebar_tab_button(args: SidebarTabButtonArgs, cx: &mut Context<App>) -> impl IntoElement {
    let SidebarTabButtonArgs {
        icon,
        label,
        tab,
        active,
    } = args;
    let mut button = Button::new(match tab {
        SidebarTab::Filters => "sidebar-filters",
        SidebarTab::Groups => "sidebar-groups",
    })
    .ghost()
    .small()
    .icon(icon)
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
