use crate::gui::app::App;
use crate::gui::message::Message;
use crate::gui::state::{GuiState, NavPage};
use crate::gui::update::is_tab_live;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Disableable, Selectable};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{ActiveTheme, Icon, Sizable};
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{Context, Window, div, px};

/// Search is transient; closing the popup keeps matches highlighted.
pub fn search_popup(app: &App, cx: &mut Context<App>) -> Option<gpui_kit::gpui::AnyElement> {
    if !app.state.search_open {
        return None;
    }
    let tab = app.state.active_tab()?;
    let total = tab.search.query.get_total_match_count();
    let searching = tab.search.handle.is_some();
    let query = app
        .log_search
        .as_ref()
        .map(|input| input.read(cx).value().to_string())
        .unwrap_or_default();
    let summary = if query.is_empty() {
        "Search raw log text in visible rows · regular expressions supported".to_string()
    } else if searching {
        format!("Searching… {total} matches found")
    } else if total == 0 {
        "No matches in visible rows".to_string()
    } else {
        format!(
            "{} of {total} matches · visible rows",
            tab.search.query.get_current_occurrence_number()
        )
    };
    Some(
        div()
            .id("search-popup")
            .occlude()
            .on_mouse_down(gpui_kit::gpui::MouseButton::Left, |_, _, cx| {
                cx.stop_propagation()
            })
            .absolute()
            .top(
                gpui_kit::component::TITLE_BAR_HEIGHT
                    + px(32. + super::tab_bar::TAB_BAR_HEIGHT + 8.),
            )
            .right(px(16.))
            .w(px(480.))
            .max_w(gpui_kit::gpui::relative(0.9))
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().border)
            .rounded(px(8.))
            .shadow_md()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .children(app.log_search.as_ref().map(|input| {
                        div().flex_1().min_w(px(0.)).child(
                            super::input(input)
                                .id("log-search-input")
                                .aria_label("Search visible logs")
                                .prefix(Icon::new(IconName::Search)),
                        )
                    }))
                    .child(
                        Button::new("search-previous")
                            .ghost()
                            .icon(IconName::ChevronUp)
                            .disabled(total == 0)
                            .accessibility_label("Previous match")
                            .tooltip("Previous match")
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.dispatch(Message::SearchNext(false), window, cx)
                            })),
                    )
                    .child(
                        Button::new("search-next")
                            .ghost()
                            .icon(IconName::ChevronDown)
                            .disabled(total == 0)
                            .accessibility_label("Next match")
                            .tooltip("Next match")
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.dispatch(Message::SearchNext(true), window, cx)
                            })),
                    )
                    .child(
                        Button::new("search-close")
                            .ghost()
                            .icon(IconName::X)
                            .accessibility_label("Close search")
                            .tooltip("Close search (Escape)")
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.dispatch(Message::SearchPopup(false), window, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(summary),
                    )
                    .child(
                        Button::new("search-clear")
                            .ghost()
                            .small()
                            .label("Clear")
                            .disabled(query.is_empty())
                            .on_click(cx.listener(|app, _, window, cx| {
                                if let Some(input) = &app.log_search {
                                    input.update(cx, |input, cx| {
                                        input.set_value(String::new(), window, cx)
                                    });
                                }
                                app.dispatch(Message::SearchEdited(String::new()), window, cx);
                            })),
                    ),
            )
            .into_any_element(),
    )
}

pub fn search_button(cx: &mut Context<App>) -> impl IntoElement {
    Button::new("open-search")
        .ghost()
        .icon(IconName::Search)
        .accessibility_label("Search")
        .tooltip("Search (/)")
        .on_click(
            cx.listener(|app, _, window, cx| app.dispatch(Message::SearchPopup(true), window, cx)),
        )
}

pub fn filter_button(cx: &mut Context<App>) -> impl IntoElement {
    Button::new("filter-panel-toggle")
        .ghost()
        .icon(IconName::Funnel)
        .accessibility_label("Show or hide filters")
        .tooltip("Show or hide filters")
        .on_click(
            cx.listener(|app, _, window, cx| app.dispatch(Message::SidebarToggled, window, cx)),
        )
}

pub fn view_mode_control(nav_page: NavPage, cx: &mut Context<App>) -> impl IntoElement {
    let entity = cx.entity();
    div()
        .flex()
        .items_center()
        .h(px(32.))
        .px(px(4.))
        .rounded(px(6.))
        .bg(cx.theme().secondary)
        .child(
            Button::new("view-mode")
                .ghost()
                .small()
                .label(match nav_page {
                    NavPage::Logs => "Logs",
                    NavPage::Bookmarks => "Bookmarks",
                    NavPage::Annotations => "Notes",
                    _ => "View",
                })
                .icon(IconName::ChevronDown)
                .tooltip("View mode")
                .dropdown_menu(move |mut menu, _, _| {
                    for (label, page) in
                        [("Logs", NavPage::Logs), ("Bookmarks", NavPage::Bookmarks)]
                    {
                        let entity = entity.clone();
                        menu =
                            menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
                                entity.update(cx, |app, cx| {
                                    app.dispatch(Message::NavPageSelected(page), window, cx);
                                });
                            }));
                    }
                    menu
                }),
        )
}

/// A colored dot (live = green, idle/paused = gray) plus a Pause/Play
/// toggle button — real: reflects and controls `tab.stream.watch`/
/// `paused` via `is_tab_live`/`Message::StreamToggled`. Disabled when the
/// tab has no watcher at all (nothing to pause), as opposed to "watched
/// but paused" which is still interactive.
pub fn live_indicator(state: &GuiState, cx: &mut Context<App>) -> impl IntoElement {
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
        .h(px(32.))
        .px(px(6.))
        .child(div().w(px(8.)).h(px(8.)).rounded_full().bg(dot_color))
        .child(if live {
            "Live"
        } else if has_watch {
            "Paused"
        } else if tab.stream.no_retry || tab.merged.as_ref().is_some_and(|m| m.stopped) {
            "Stopped"
        } else {
            "File"
        })
        .when(has_watch, |d| {
            d.child(
                Button::new("stream-toggle")
                    .ghost()
                    .icon(Icon::new(icon))
                    .disabled(!has_watch)
                    .accessibility_label(if tab.stream.paused {
                        "Resume stream"
                    } else {
                        "Pause stream"
                    })
                    .tooltip(if tab.stream.paused { "Resume" } else { "Pause" })
                    .on_click(
                        cx.listener(move |app: &mut App, _, window: &mut Window, cx| {
                            app.dispatch(Message::StreamToggled(tab_idx), window, cx);
                        }),
                    ),
            )
        })
        .when(has_watch, |d| {
            d.child(
                Button::new("follow-latest")
                    .ghost()
                    .small()
                    .label("Follow")
                    .selected(tab.stream.tail_mode)
                    .accessibility_label("Follow latest rows")
                    .tooltip("Keep the latest rows in view")
                    .on_click(cx.listener(|app, _, window, cx| {
                        app.dispatch(
                            Message::GuiAction(crate::gui::actions::Action::Tail),
                            window,
                            cx,
                        );
                    })),
            )
        })
        .into_any_element()
}
