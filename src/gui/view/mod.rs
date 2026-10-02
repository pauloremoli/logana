pub mod command_bar;
pub mod filter_pane;
pub mod group_pane;
pub mod log_pane;
pub mod mode_bar;
pub mod search_bar;
pub mod sidebar;
pub mod status_bar;
pub mod tab_bar;

use crate::gui::app::App;
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{Context, Window, div};

pub fn view(app: &mut App, window: &mut Window, cx: &mut Context<App>) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .size_full()
        .child(tab_bar::tab_bar(&app.state, cx))
        .child(body(app, window, cx))
        .child(command_bar::command_bar(&app.state))
        .child(search_bar::search_bar(&app.state))
        .child(mode_bar::mode_bar(&app.state))
        .child(status_bar::status_bar(app.state.active_tab()))
}

fn body(app: &mut App, _window: &mut Window, cx: &mut Context<App>) -> impl IntoElement {
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
                let tab = &app.state.tabs[active_tab];
                sidebar::sidebar(active_tab, tab, cx)
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
