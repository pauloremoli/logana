pub mod action_editor;
pub mod bookmarks;
pub mod command_palette;
pub mod field_facets;
pub mod filter_pane;
pub mod group_pane;
pub mod log_pane;
pub mod nav_rail;
pub mod notification;
pub mod search_bar;
pub mod sidebar;
pub mod status_bar;
pub mod tab_bar;

use crate::gui::app::App;
use crate::gui::state::NavPage;
use gpui_kit::component::TitleBar;
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{Context, Window, div, px};

pub fn view(app: &mut App, window: &mut Window, cx: &mut Context<App>) -> impl IntoElement {
    let viewport = window.viewport_size();
    let mut root = div()
        .relative()
        .flex()
        .flex_col()
        .size_full()
        // Draws the real minimize/maximize/close controls (platform-gated
        // internally — native traffic lights on macOS, skipped entirely
        // under server-side decorations) and owns window dragging; see
        // `TitleBar::window_options()` in `logana_gui.rs`'s window setup.
        .child(TitleBar::new().child("logana"))
        .child(
            div()
                .flex()
                .flex_1()
                .min_h(px(0.))
                .child(nav_rail::nav_rail(&app.state, cx))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(px(0.))
                        .child(tab_bar::tab_bar(&app.state, cx))
                        .child(body(app, window, cx)),
                ),
        );
    if let Some(palette) = command_palette::command_palette(app, viewport, cx) {
        root = root.child(palette);
    }
    if let Some(overlay) = search_bar::time_range_overlay(&app.state, viewport, cx) {
        root = root.child(overlay);
    }
    if let Some(editor) = action_editor::editor(app, viewport, cx) {
        root = root.child(editor);
    }
    if let Some(notification) = notification::notification(app, viewport, cx) {
        root = root.child(notification);
    }
    root
}

fn body(app: &mut App, window: &mut Window, cx: &mut Context<App>) -> impl IntoElement {
    if app.state.nav_page == NavPage::Bookmarks {
        return bookmarks::page(app, cx).into_any_element();
    }
    if app.state.nav_page != NavPage::Logs {
        return div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .child("Not yet implemented")
            .into_any_element();
    }
    let active_tab = app.state.active_tab;
    match app.state.tabs.get(active_tab) {
        Some(_) => {
            // Re-borrow after the match's immutable borrow of `app.state`
            // ends, so `log_pane`'s `cx.entity()` call (which needs
            // `&mut Context<App>`) and `sidebar`'s panes aren't fighting
            // the outer borrow.
            let fields = app.state.tabs[active_tab].collect_field_names();
            let tab = &app.state.tabs[active_tab];
            let widths = crate::gui::column_widths::table_widths(
                &mut app.field_widths,
                tab,
                &fields,
                app.state.column_widths,
            );
            let available_width = f32::from(window.viewport_size().width)
                - nav_rail::NAV_RAIL_WIDTH
                - if tab.display.show_sidebar {
                    sidebar::SIDEBAR_WIDTH
                } else {
                    0.
                }
                - 24.; // Table container's horizontal padding.
            let message_width = log_pane::message_width(tab, &fields, &widths, available_width);
            let item_sizes = log_pane::item_sizes(
                &mut app.table_layout,
                &app.state.tabs[active_tab],
                &widths,
                &fields,
                message_width,
            );
            let tab = &app.state.tabs[active_tab];
            let sidebar_on_left = tab.display.sidebar_side.is_left();
            let show_sidebar = tab.display.show_sidebar;
            let table = div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.))
                .min_h(px(0.))
                .child(search_bar::search_bar(
                    &app.state,
                    app.log_search.as_ref(),
                    cx,
                ))
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_h(px(0.))
                        .px(px(12.))
                        .pb(px(8.))
                        .child(log_pane::log_pane(
                            active_tab,
                            tab,
                            &widths,
                            &app.log_scroll,
                            &fields,
                            item_sizes,
                            message_width,
                            cx,
                        )),
                )
                .child(status_bar::status_bar(Some(tab), cx))
                .into_any_element();
            let sidebar = {
                let sidebar_tab = app.state.sidebar_tab;
                let tab = &app.state.tabs[active_tab];
                sidebar::sidebar(
                    active_tab,
                    tab,
                    sidebar_tab,
                    &app.state.facet_expanded,
                    app.facet_search.as_ref(),
                    app.facet_counts
                        .get(&tab.id)
                        .and_then(|entry| entry.counts.as_ref()),
                    cx,
                )
                .into_any_element()
            };
            let mut row = div().flex().flex_1().min_h(px(0.));
            row = if !show_sidebar {
                row.child(table)
            } else if sidebar_on_left {
                row.child(sidebar).child(table)
            } else {
                row.child(table).child(sidebar)
            };
            row.into_any_element()
        }
        None => div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .child("Open a file to get started")
            .into_any_element(),
    }
}
