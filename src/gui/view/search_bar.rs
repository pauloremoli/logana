use crate::gui::app::App;
use crate::gui::message::Message;
use crate::gui::state::GuiState;
use crate::gui::time_range::{ALL_PRESETS, preset_label};
use crate::gui::update::is_tab_live;
use crate::gui::view::tab_bar::TAB_BAR_HEIGHT;
use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme, Icon, TITLE_BAR_HEIGHT};
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{
    AnyElement, Context, Entity, Pixels, Size, Window, anchored, deferred, div, point, px,
};

pub(crate) const SEARCH_BAR_HEIGHT: f32 = 64.0;
/// Width of the time-range dropdown's floating panel, and the right-edge
/// inset used to roughly line it up under `time_range_control`'s toggle —
/// an approximation, not real bounds tracking (see `time_range_overlay`'s
/// doc comment for why).
const TIME_RANGE_PANEL_WIDTH: f32 = 180.0;
const TIME_RANGE_RIGHT_INSET: f32 = 96.0;

/// The search/filter bar row: the search input, a time-range dropdown
/// (drives the real `:date-filter` command), the active-filter count, and
/// a Live indicator + Pause/Play toggle (drives the real `:pause`/
/// `:resume` commands). `.flex()`/`.h()`/`.overflow_hidden()` on every
/// sub-element are load-bearing, not cosmetic — see `log_pane.rs`'s
/// `styled_line_row` doc comment.
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
        .bg(cx.theme().title_bar)
        .px(px(12.))
        .overflow_hidden()
        .border_b_1()
        .border_color(cx.theme().border)
        .child(div().flex_1().min_w(px(0.)).children(input.map(|input| {
            Input::new(input)
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
        .child(time_range_control(state, cx))
        .child(filter_count_badge(state, cx))
        .child(live_indicator(state, cx))
}

fn time_range_control(state: &GuiState, cx: &mut Context<App>) -> impl IntoElement {
    div()
        .id("time-range-toggle")
        .h(px(38.))
        .px(px(12.))
        .rounded(px(6.))
        .border_1()
        .border_color(cx.theme().border)
        .cursor_pointer()
        .flex()
        .items_center()
        .gap_1()
        .overflow_hidden()
        .child(Icon::new(IconName::Clock))
        .child(preset_label(state.time_range_preset))
        .child(Icon::new(IconName::ChevronDown))
        .on_click(cx.listener(|app: &mut App, _, window: &mut Window, cx| {
            app.dispatch(Message::TimeRangeToggled, window, cx);
        }))
}

/// The time-range dropdown's option list, as a true floating overlay —
/// `deferred(anchored())` paints it in a later pass so it escapes the
/// search bar's `overflow_hidden()` instead of being clipped/squashed
/// inline (the bug a screenshot caught: the list used to render inline
/// inside the search bar's fixed-height row and overlapped the other
/// controls). Positioned with a fixed top offset (title bar + tab bar +
/// search bar heights) and a right inset, approximating the toggle
/// button's location rather than tracking its real bounds — gpui doesn't
/// give element bounds back to a plain render function, only through
/// bounds-tracking APIs (`on_children_prepainted` et al.) this codebase
/// doesn't use elsewhere yet. Good enough for a right-side toggle in a
/// fixed-height bar; revisit with real bounds tracking if the search bar
/// layout grows more dynamic.
pub fn time_range_overlay(
    state: &GuiState,
    viewport: Size<Pixels>,
    cx: &mut Context<App>,
) -> Option<AnyElement> {
    if !state.time_range_open {
        return None;
    }
    let top_offset = f32::from(TITLE_BAR_HEIGHT) + TAB_BAR_HEIGHT + SEARCH_BAR_HEIGHT;
    let mut list = div()
        .w(px(TIME_RANGE_PANEL_WIDTH))
        .flex()
        .flex_col()
        .rounded(px(8.))
        .bg(cx.theme().popover)
        .text_color(cx.theme().popover_foreground)
        .border_1()
        .border_color(cx.theme().border)
        .shadow_md()
        .p(px(4.));
    for preset in ALL_PRESETS {
        list = list.child(
            div()
                .id(preset_label(preset))
                .px(px(12.))
                .py(px(4.))
                .rounded(px(4.))
                .overflow_hidden()
                .whitespace_nowrap()
                .hover(|style| style.bg(cx.theme().accent))
                .child(preset_label(preset))
                .on_click(
                    cx.listener(move |app: &mut App, _, window: &mut Window, cx| {
                        app.dispatch(Message::TimeRangeSelected(preset), window, cx);
                    }),
                ),
        );
    }
    Some(
        deferred(
            anchored().position(point(px(0.), px(0.))).child(
                div()
                    .w(viewport.width)
                    .flex()
                    .justify_end()
                    .pt(px(top_offset))
                    .pr(px(TIME_RANGE_RIGHT_INSET
                        + state
                            .active_tab()
                            .filter(|tab| {
                                tab.display.show_sidebar && !tab.display.sidebar_side.is_left()
                            })
                            .map(|_| 360.)
                            .unwrap_or(0.)))
                    .child(list),
            ),
        )
        .into_any_element(),
    )
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
