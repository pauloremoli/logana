use super::field_facets;
use super::filter_pane::{self, FilterManagementView};
use super::group_pane::{self, GroupManagementView};
use crate::gui::app::App;
use crate::gui::message::Message;
use crate::gui::state::SidebarTab;
use crate::input::{KeyCode, KeyModifiers};
use crate::mode::app_mode::ModeRenderState;
use crate::ui::TabState;
use gpui_kit::assets::IconName;
use gpui_kit::base::Selectable;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::Icon;
use gpui_kit::component::button::Button;
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{Context, Window, div, px};
use std::collections::HashMap;

const SIDEBAR_WIDTH: f32 = 320.0;
const SIDEBAR_HEADER_HEIGHT: f32 = 32.0;

pub fn sidebar(
    tab_idx: usize,
    tab: &TabState,
    sidebar_tab: SidebarTab,
    facet_expanded: &HashMap<String, bool>,
    cx: &mut Context<App>,
) -> impl IntoElement {
    let names = tab.log_manager.group_names();
    let filter_defs = tab.log_manager.get_filters();
    let group_defs = tab.log_manager.get_group_styles();
    let render_state = tab.interaction.mode.render_state();
    let already_filter_mgmt = matches!(&render_state, ModeRenderState::FilterManagement { .. });
    let already_group_mgmt = matches!(&render_state, ModeRenderState::GroupManagement { .. });

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
        .flex_shrink_0()
        .gap_4()
        .w(px(SIDEBAR_WIDTH))
        .p_2()
        .when(tab.display.sidebar_side.is_left(), |d| d.border_r_1())
        .when(!tab.display.sidebar_side.is_left(), |d| d.border_l_1())
        .border_color(cx.theme().border)
        .child(sidebar_header(
            sidebar_tab,
            already_filter_mgmt,
            already_group_mgmt,
            cx,
        ));
    col = match sidebar_tab {
        SidebarTab::Filters => {
            let col = col.child(filter_pane::filter_pane(
                tab_idx,
                filter_defs,
                group_defs,
                filter_management.as_ref(),
                cx,
            ));
            // Re-scans up to 5000 lines on every render while the Filters
            // tab is open (same cap as `build_field_index`, which this
            // mirrors) — fine for the file sizes exercised so far, but a
            // real cost on a large file; caching this per tab (recompute
            // only when the file or its filters change) is the natural
            // follow-up if it's ever visibly slow.
            let counts = tab.build_field_value_counts();
            col.child(field_facets::field_facets(
                tab_idx,
                &counts,
                filter_defs,
                facet_expanded,
                cx,
            ))
        }
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
/// `KeyResult::ToggleSidebar` already flips. `already_filter_mgmt`/
/// `already_group_mgmt` report whether the real `FilterManagementMode`/
/// `GroupManagementMode` is already engaged, so each tab button knows
/// whether clicking it also needs to enter that mode for real (see
/// `sidebar_tab_button`'s doc comment).
fn sidebar_header(
    active: SidebarTab,
    already_filter_mgmt: bool,
    already_group_mgmt: bool,
    cx: &mut Context<App>,
) -> impl IntoElement {
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
                    SidebarTabButtonArgs {
                        icon: IconName::Funnel,
                        label: "Filters",
                        tab: SidebarTab::Filters,
                        active: active == SidebarTab::Filters,
                        already_in_mgmt_mode: already_filter_mgmt,
                        mode_key: KeyCode::Char('f'),
                        mode_modifiers: KeyModifiers::NONE,
                    },
                    cx,
                ))
                .child(sidebar_tab_button(
                    SidebarTabButtonArgs {
                        icon: IconName::Group,
                        label: "Groups",
                        tab: SidebarTab::Groups,
                        active: active == SidebarTab::Groups,
                        already_in_mgmt_mode: already_group_mgmt,
                        mode_key: KeyCode::Char('g'),
                        mode_modifiers: KeyModifiers::CONTROL,
                    },
                    cx,
                )),
        )
        .child(
            Button::new("sidebar-close")
                .icon(Icon::new(IconName::X))
                .tooltip("Hide sidebar")
                .on_click(cx.listener(|app: &mut App, _, window: &mut Window, cx| {
                    app.dispatch(Message::SidebarToggled, window, cx);
                })),
        )
}

/// Bundles `sidebar_tab_button`'s per-tab config — kept as one struct
/// instead of individual parameters to stay under clippy's
/// `too_many_arguments` threshold.
struct SidebarTabButtonArgs {
    icon: IconName,
    label: &'static str,
    tab: SidebarTab,
    active: bool,
    already_in_mgmt_mode: bool,
    mode_key: KeyCode,
    mode_modifiers: KeyModifiers,
}

/// Switches which pane is visible (`Message::SidebarTabSelected`, a pure
/// GUI display toggle) and, when the real `Mode` state machine isn't
/// already in the matching management mode, also dispatches the same
/// keypress `NormalMode`'s `filter_mode`/`group_mode` bindings handle
/// (`f` / `Ctrl+g`) — exactly like the search bar's click-to-search fix —
/// so the sidebar's selection cursor and keyboard navigation (`j`/`k`,
/// `dd`, etc.) actually engage instead of just showing a static list.
/// Skipped when already active, since routing another `f`/`Ctrl+g`
/// through `FilterManagementMode`/`GroupManagementMode` itself could type
/// into its live search instead of re-entering it.
fn sidebar_tab_button(args: SidebarTabButtonArgs, cx: &mut Context<App>) -> impl IntoElement {
    let SidebarTabButtonArgs {
        icon,
        label,
        tab,
        active,
        already_in_mgmt_mode,
        mode_key,
        mode_modifiers,
    } = args;
    let mut button = Button::new(label)
        .icon(Icon::new(icon))
        .label(label)
        .tooltip(label)
        .on_click(
            cx.listener(move |app: &mut App, _, window: &mut Window, cx| {
                app.dispatch(Message::SidebarTabSelected(tab), window, cx);
                if !already_in_mgmt_mode {
                    app.dispatch(Message::KeyPressed(mode_key, mode_modifiers), window, cx);
                }
            }),
        );
    if active {
        button = button.selected(true);
    }
    button
}
