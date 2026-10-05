use crate::gui::app::App;
use crate::gui::message::Message;
use crate::gui::state::{GuiState, NavPage};
use crate::gui::update::is_tab_live;
use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::InputState;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{ActiveTheme, Icon};
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{Context, Entity, Window, div, px};

pub(crate) const SEARCH_BAR_HEIGHT: f32 = 64.0;
/// Search, view selection, filters, and live-stream controls.
pub fn search_bar(
    state: &GuiState,
    input: Option<&Entity<InputState>>,
    cx: &mut Context<App>,
) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .h(px(SEARCH_BAR_HEIGHT))
        .flex_shrink_0()
        .bg(cx.theme().title_bar)
        .px(px(12.))
        .overflow_hidden()
        .border_b_1()
        .border_color(cx.theme().border)
        .child(div().flex_1().min_w(px(0.)).children(input.map(|input| {
            super::input(input)
                .id("log-search-input")
                .bg(cx.theme().secondary)
                .text_color(cx.theme().foreground)
                .prefix(Icon::new(IconName::Search))
        })))
        .child(
            Button::new("search-previous")
                .ghost()
                .icon(IconName::ChevronUp)
                .tooltip("Previous match")
                .on_click(cx.listener(|app, _, window, cx| {
                    app.dispatch(Message::SearchNext(false), window, cx)
                })),
        )
        .child(
            Button::new("search-next")
                .ghost()
                .icon(IconName::ChevronDown)
                .tooltip("Next match")
                .on_click(cx.listener(|app, _, window, cx| {
                    app.dispatch(Message::SearchNext(true), window, cx)
                })),
        )
        .child(view_mode_control(state, cx))
        .child(filter_count_badge(state, cx))
        .child(live_indicator(state, cx))
}

fn view_mode_control(state: &GuiState, cx: &mut Context<App>) -> impl IntoElement {
    let entity = cx.entity();
    Button::new("view-mode")
        .label(match state.nav_page {
            NavPage::Logs => "Logs",
            NavPage::Bookmarks => "Bookmarks",
            _ => "View",
        })
        .icon(IconName::ChevronDown)
        .tooltip("View mode")
        .dropdown_menu(move |mut menu, _, _| {
            for (label, page) in [("Logs", NavPage::Logs), ("Bookmarks", NavPage::Bookmarks)] {
                let entity = entity.clone();
                menu = menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
                    entity.update(cx, |app, cx| {
                        app.dispatch(Message::NavPageSelected(page), window, cx);
                    });
                }));
            }
            menu
        })
}

fn filter_count_badge(state: &GuiState, cx: &mut Context<App>) -> impl IntoElement {
    let count = state
        .active_tab()
        .map(|tab| {
            tab.log_manager
                .get_filters()
                .iter()
                .filter(|f| f.enabled)
                .count()
        })
        .unwrap_or(0);
    Button::new("filter-panel-toggle")
        .ghost()
        .icon(Icon::new(IconName::Funnel))
        .label(count.to_string())
        .tooltip("Show or hide filters")
        .on_click(cx.listener(|app: &mut App, _, window: &mut Window, cx| {
            app.dispatch(Message::SidebarToggled, window, cx);
        }))
}

/// A colored dot (live = green, idle/paused = gray) plus a Pause/Play
/// toggle button — real: reflects and controls `tab.stream.watch`/
/// `paused` via `is_tab_live`/`Message::StreamToggled`. Disabled when the
/// tab has no watcher at all (nothing to pause), as opposed to "watched
/// but paused" which is still interactive.
fn live_indicator(state: &GuiState, cx: &mut Context<App>) -> impl IntoElement {
    let Some(tab) = state.active_tab() else {
        return div().into_any_element();
    };
    let tab_idx = state.active_tab;
    let live = is_tab_live(tab);
    let has_watch = tab.stream.watch.is_some()
        || tab
            .merged
            .as_ref()
            .is_some_and(|m| !m.stopped && !m.source_tab_ids.is_empty());
    let dot_color = if live {
        cx.theme().success
    } else {
        cx.theme().muted_foreground
    };
    let icon = if tab.stream.paused {
        IconName::Play
    } else {
        IconName::Pause
    };
    div()
        .flex()
        .items_center()
        .gap_1()
        .overflow_hidden()
        .h(px(38.))
        .px(px(10.))
        .rounded(px(6.))
        .border_1()
        .border_color(cx.theme().border)
        .child(div().w(px(8.)).h(px(8.)).rounded_full().bg(dot_color))
        .child(if live { "Live" } else { "Paused" })
        .child(
            Button::new("stream-toggle")
                .ghost()
                .icon(Icon::new(icon))
                .disabled(!has_watch)
                .tooltip(if tab.stream.paused { "Resume" } else { "Pause" })
                .on_click(
                    cx.listener(move |app: &mut App, _, window: &mut Window, cx| {
                        app.dispatch(Message::StreamToggled(tab_idx), window, cx);
                    }),
                ),
        )
        .into_any_element()
}
