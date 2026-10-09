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

pub(crate) const SIDEBAR_WIDTH: f32 = 360.0;

/// Wraps `content` in its own independently-scrollable region — the header
/// stays put while only this list scrolls. `id` must be unique among
/// sibling scroll regions (gpui keys scroll offsets by element id).
fn scroll_region(id: &'static str, content: impl IntoElement) -> impl IntoElement {
    div()
        .id(id)
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scroll()
        .px_4()
        .pt_4()
        .pb_4()
        .child(content)
}

#[allow(clippy::too_many_arguments)]
pub fn sidebar(
    tab_idx: usize,
    tab: &TabState,
    sidebar_tab: SidebarTab,
    keyboard_selection: Option<(SidebarTab, usize)>,
    facet_browser_open: bool,
    facet_expanded: &HashMap<String, bool>,
    facet_search: Option<&Entity<InputState>>,
    facet_counts: Option<&crate::shared::FieldValueCounts>,
    filter_scroll: &gpui_kit::gpui::ScrollHandle,
    group_scroll: &gpui_kit::gpui::ScrollHandle,
    cx: &mut Context<App>,
) -> impl IntoElement {
    let names = tab.log_manager.group_names();
    let filter_defs = tab.log_manager.get_filters();
    let group_defs = tab.log_manager.get_group_styles();
    let outer = div()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .min_h(px(0.))
        .w_full()
        .bg(cx.theme().sidebar)
        .when(tab.display.sidebar_side.is_left(), |d| d.border_r_1())
        .when(!tab.display.sidebar_side.is_left(), |d| d.border_l_1())
        .border_color(cx.theme().border)
        .child(sidebar_header(sidebar_tab, facet_browser_open, tab, cx));
    let body = div().flex().flex_col().flex_1().min_h(px(0.));
    let body = match sidebar_tab {
        SidebarTab::Filters => {
            let filters_pane = filter_pane::filter_pane(
                tab_idx,
                filter_defs,
                group_defs,
                keyboard_selection
                    .filter(|(scope, _)| *scope == SidebarTab::Filters)
                    .map(|(_, index)| index),
                filter_scroll,
                cx,
            );
            if !facet_browser_open {
                return outer.child(filters_pane);
            }
            let facets = match facet_counts {
                Some(counts) => field_facets::field_facets(
                    tab_idx,
                    counts,
                    filter_defs,
                    facet_expanded,
                    facet_search,
                    cx,
                )
                .into_any_element(),
                None => "Loading available filters…".into_any_element(),
            };
            let facets_pane = div()
                .child(
                    Button::new("close-field-browser")
                        .ghost()
                        .small()
                        .label("Close field picker")
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.dispatch(Message::FacetBrowserToggled, window, cx)
                        })),
                )
                .child(facets);
            body.child(scroll_region("sidebar-facets-scroll", facets_pane))
        }
        SidebarTab::Groups => body.child(group_pane::group_pane(
            tab_idx,
            &names,
            group_defs,
            filter_defs,
            keyboard_selection
                .filter(|(scope, _)| *scope == SidebarTab::Groups)
                .map(|(_, index)| index),
            group_scroll,
            cx,
        )),
    };
    outer.child(body)
}

fn sidebar_header(
    active: SidebarTab,
    fields_open: bool,
    tab: &TabState,
    cx: &mut Context<App>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .px_3()
        .py_2()
        .border_b_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_1()
                        .child(sidebar_tab_button(
                            SidebarTabButtonArgs {
                                icon: IconName::Funnel,
                                label: "Filters",
                                tab: SidebarTab::Filters,
                                active: active == SidebarTab::Filters && !fields_open,
                            },
                            cx,
                        ))
                        .child(
                            Button::new("sidebar-fields")
                                .ghost()
                                .small()
                                .label("Fields")
                                .selected(active == SidebarTab::Filters && fields_open)
                                .on_click(cx.listener(|app, _, window, cx| {
                                    app.dispatch(
                                        Message::SidebarTabSelected(SidebarTab::Filters),
                                        window,
                                        cx,
                                    );
                                    if !app.state.facet_browser_open {
                                        app.dispatch(Message::FacetBrowserToggled, window, cx);
                                    }
                                })),
                        )
                        .child(sidebar_tab_button(
                            SidebarTabButtonArgs {
                                icon: IconName::Group,
                                label: "Groups",
                                tab: SidebarTab::Groups,
                                active: active == SidebarTab::Groups,
                            },
                            cx,
                        )),
                )
                .child(
                    Button::new("sidebar-close")
                        .ghost()
                        .icon(Icon::new(IconName::X))
                        .accessibility_label("Hide sidebar")
                        .tooltip("Hide sidebar")
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.dispatch(Message::SidebarToggled, window, cx)
                        })),
                ),
        )
        .child(
            Button::new("sidebar-filtering-toggle")
                .ghost()
                .small()
                .icon(IconName::ListFilter)
                .label(if tab.filter.enabled {
                    "Filtering on"
                } else {
                    "Filtering off"
                })
                .selected(tab.filter.enabled)
                .accessibility_label("Toggle filtering")
                .tooltip("Toggle filtering (F)")
                .on_click(cx.listener(|app, _, window, cx| {
                    app.dispatch(
                        Message::GuiAction(crate::gui::actions::Action::ToggleFiltering),
                        window,
                        cx,
                    )
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
            if app.state.facet_browser_open && tab == SidebarTab::Filters {
                app.dispatch(Message::FacetBrowserToggled, window, cx);
            }
            app.dispatch(Message::SidebarTabSelected(tab), window, cx);
        }),
    );
    if active {
        button = button.selected(true);
    }
    button
}
