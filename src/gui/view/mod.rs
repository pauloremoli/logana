pub mod action_editor;
pub mod annotations;
pub mod bookmarks;
pub mod command_palette;
pub mod field_facets;
pub mod filter_pane;
pub mod group_pane;
pub mod loading;
pub mod log_pane;
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

/// Leave enough room for the native editor's line height plus both borders.
/// The component's default 32px height with 8px vertical padding clips glyphs.
pub(super) fn input(
    state: &gpui_kit::gpui::Entity<gpui_kit::component::input::InputState>,
) -> gpui_kit::component::input::Input {
    gpui_kit::component::input::Input::new(state)
        .min_h(px(36.))
        .py(px(4.))
        .flex_shrink_0()
}

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
        .child(
            TitleBar::new()
                .on_close_window(cx.listener(|app, _, _, cx| app.request_quit(cx)))
                .child("logana"),
        )
        .child(div().h(px(32.)).flex_shrink_0().child(app.menu_bar.clone()))
        .child(
            div().flex().flex_1().min_h(px(0.)).child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.))
                    .child(tab_bar::tab_bar(&app.state, cx))
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_h(px(0.))
                            .child(body(app, window, cx))
                            .children(loading::picker(app, cx)),
                    ),
            ),
        );
    if let Some(operations) = loading::operations(app, cx) {
        root = root.child(operations);
    }
    if let Some(search) = search_bar::search_popup(app, cx) {
        root = root.child(search);
    }
    if let Some(palette) = command_palette::command_palette(app, viewport, cx) {
        root = root.child(palette);
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
    if app.state.nav_page == NavPage::Annotations {
        return annotations::page(app, cx).into_any_element();
    }
    if !matches!(app.state.nav_page, NavPage::Logs | NavPage::Bookmarks) {
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
            let mut widths = crate::gui::column_widths::table_widths(
                &mut app.field_widths,
                tab,
                &fields,
                app.state.column_widths,
            );
            if let Some(saved) = app
                .state
                .preferences
                .column_widths
                .get(&crate::gui::column_widths::format_key(tab))
            {
                widths.restore(saved);
            }
            let available_width = f32::from(window.viewport_size().width)
                - if tab.display.show_sidebar {
                    sidebar::SIDEBAR_WIDTH
                } else {
                    0.
                }
                - if app
                    .loading
                    .selected_picker
                    .and_then(|id| app.loading.operations.iter().find(|op| op.id == id))
                    .is_some_and(|op| op.picker.is_some())
                {
                    420_f32.min(f32::from(window.viewport_size().width) * 0.5)
                } else {
                    0.
                }
                - 24.; // Table container's horizontal padding.
            let message_width = log_pane::message_width(tab, &fields, &widths, available_width);
            log_pane::prepare_scaled_list(
                &mut app.table_layout,
                &app.log_scroll,
                &app.state.tabs[active_tab],
                &widths,
                &fields,
                message_width,
                f32::from(window.rem_size()) / 13.,
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
                            &app.horizontal_scroll,
                            &fields,
                            message_width,
                            f32::from(window.rem_size()) / 13.,
                            app.text_handle.clone(),
                            app.text_document.clone(),
                            cx,
                        )),
                )
                .child(status_bar::status_bar(Some(tab), app.state.nav_page, cx))
                .into_any_element();
            let sidebar = {
                let sidebar_tab = app.state.sidebar_tab;
                let tab = &app.state.tabs[active_tab];
                sidebar::sidebar(
                    active_tab,
                    tab,
                    sidebar_tab,
                    app.state
                        .keyboard_sidebar
                        .map(|scope| (scope, app.state.sidebar_cursor)),
                    app.state.facet_browser_open,
                    &app.state.facet_expanded,
                    app.facet_search.as_ref(),
                    app.facet_counts
                        .get(&tab.id)
                        .and_then(|entry| entry.counts.as_ref()),
                    &app.filter_sidebar_scroll,
                    &app.group_sidebar_scroll,
                    cx,
                )
                .into_any_element()
            };
            let mut row = div().flex().flex_1().min_w(px(0.)).min_h(px(0.));
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
