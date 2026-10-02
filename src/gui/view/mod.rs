pub mod command_bar;
pub mod filter_pane;
pub mod group_pane;
pub mod log_pane;
pub mod mode_bar;
pub mod nav_rail;
pub mod search_bar;
pub mod sidebar;
pub mod status_bar;
pub mod tab_bar;

use crate::gui::app::App;
use crate::gui::state::NavPage;
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{Context, Window, div, px};

const APP_TITLE_HEIGHT: f32 = 32.0;

pub fn view(app: &mut App, window: &mut Window, cx: &mut Context<App>) -> impl IntoElement {
    div()
        .flex()
        .size_full()
        .child(nav_rail::nav_rail(&app.state, cx))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .h(px(APP_TITLE_HEIGHT))
                        .overflow_hidden()
                        .child("logana"),
                )
                .child(tab_bar::tab_bar(&app.state, cx))
                .child(body(app, window, cx))
                .child(command_bar::command_bar(&app.state))
                .child(search_bar::search_bar(&app.state, cx))
                .child(mode_bar::mode_bar(&app.state))
                .child(status_bar::status_bar(app.state.active_tab())),
        )
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
            .child({
                // Re-borrow after the match's immutable borrow of `app.state`
                // ends, so `log_pane`'s `cx.entity()` call (which needs
                // `&mut Context<App>`) and `sidebar`'s panes aren't fighting
                // the outer borrow.
                let tab = &app.state.tabs[active_tab];
                log_pane::log_pane(active_tab, tab, &app.log_scroll, cx)
            })
            .child({
                let sidebar_tab = app.state.sidebar_tab;
                let tab = &app.state.tabs[active_tab];
                sidebar::sidebar(active_tab, tab, sidebar_tab, cx)
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
