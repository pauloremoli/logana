pub mod command_palette;
pub mod field_facets;
pub mod filter_pane;
pub mod group_pane;
pub mod log_pane;
pub mod nav_rail;
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
                        // Matches the mockup's actual order: search/filter
                        // bar sits directly under the tabs, above the
                        // table — it used to stay in the TUI's original
                        // bottom-of-screen position (after `body`), which
                        // is why it rendered next to the status bar
                        // instead of up here.
                        .child(search_bar::search_bar(&app.state, cx))
                        .child(body(app, window, cx))
                        .child(status_bar::status_bar(app.state.active_tab(), cx)),
                ),
        );
    if let Some(palette) = command_palette::command_palette(&app.state, viewport, cx) {
        root = root.child(palette);
    }
    root
}

fn body(app: &mut App, _window: &mut Window, cx: &mut Context<App>) -> impl IntoElement {
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
        Some(_) => div()
            .flex()
            .flex_1()
            .min_h(px(0.))
            .child({
                // Re-borrow after the match's immutable borrow of `app.state`
                // ends, so `log_pane`'s `cx.entity()` call (which needs
                // `&mut Context<App>`) and `sidebar`'s panes aren't fighting
                // the outer borrow.
                let tab = &app.state.tabs[active_tab];
                div()
                    .flex()
                    .flex_1()
                    .min_w(px(0.))
                    .px(px(16.))
                    .py(px(8.))
                    .child(log_pane::log_pane(active_tab, tab, &app.log_scroll, cx))
            })
            .child({
                let sidebar_tab = app.state.sidebar_tab;
                let tab = &app.state.tabs[active_tab];
                sidebar::sidebar(active_tab, tab, sidebar_tab, &app.state.facet_expanded, cx)
            })
            .into_any_element(),
        None => div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .child("Open a file to get started")
            .into_any_element(),
    }
}
