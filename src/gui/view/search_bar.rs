use crate::gui::app::App;
use crate::gui::message::Message;
use crate::gui::state::GuiState;
use crate::gui::time_range::{ALL_PRESETS, preset_label};
use crate::gui::update::is_tab_live;
use crate::mode::app_mode::ModeRenderState;
use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::Icon;
use gpui_kit::component::badge::Badge;
use gpui_kit::component::button::Button;
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{Context, Window, div, px};

const SEARCH_BAR_HEIGHT: f32 = 32.0;
const PLACEHOLDER: &str = "Search logs... (regex, field:value, or free text)";

/// The search/filter bar row: the search input, a time-range dropdown
/// (drives the real `:date-filter` command), the active-filter count, and
/// a Live indicator + Pause/Play toggle (drives the real `:pause`/
/// `:resume` commands). `.flex()`/`.h()`/`.overflow_hidden()` on every
/// sub-element are load-bearing, not cosmetic — see `log_pane.rs`'s
/// `styled_line_row` doc comment.
pub fn search_bar(state: &GuiState, cx: &mut Context<App>) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .h(px(SEARCH_BAR_HEIGHT))
        .px(px(8.))
        .overflow_hidden()
        .border_b_1()
        .border_color(cx.theme().border)
        .child(search_input(state))
        .child(time_range_control(state, cx))
        .child(filter_count_badge(state))
        .child(live_indicator(state, cx))
}

/// Shows the `/` (forward) or `?` (backward) search prompt and its live
/// query while `SearchMode` is active; a static placeholder otherwise.
/// Typing, matching, and navigation are all handled by the TUI's real
/// `SearchMode`/`Search` — this only reflects its `render_state`.
fn search_input(state: &GuiState) -> impl IntoElement {
    let text = match state
        .active_tab()
        .map(|tab| tab.interaction.mode.render_state())
    {
        Some(ModeRenderState::Search { query, forward, .. }) => {
            let prompt = if forward { "/" } else { "?" };
            format!("{prompt}{query}")
        }
        // Command mode (`:filter ERROR`, etc.) shares this one input
        // slot too, rather than a separate command-bar row — the mockup
        // has a single search/command input, not two.
        Some(ModeRenderState::Command { input, .. }) => format!(":{input}"),
        _ => PLACEHOLDER.to_string(),
    };
    div().flex().flex_1().overflow_hidden().child(text)
}

fn time_range_control(state: &GuiState, cx: &mut Context<App>) -> impl IntoElement {
    let mut control = div().flex().flex_col();
    let toggle = div()
        .id("time-range-toggle")
        .flex()
        .items_center()
        .gap_1()
        .overflow_hidden()
        .child(Icon::new(IconName::Clock))
        .child(preset_label(state.time_range_preset))
        .child(Icon::new(IconName::ChevronDown))
        .on_click(cx.listener(|app: &mut App, _, window: &mut Window, cx| {
            app.dispatch(Message::TimeRangeToggled, window, cx);
        }));
    control = control.child(toggle);
    if state.time_range_open {
        let mut list = div().flex().flex_col();
        for preset in ALL_PRESETS {
            list = list.child(
                div()
                    .id(preset_label(preset))
                    .overflow_hidden()
                    .child(preset_label(preset))
                    .on_click(
                        cx.listener(move |app: &mut App, _, window: &mut Window, cx| {
                            app.dispatch(Message::TimeRangeSelected(preset), window, cx);
                        }),
                    ),
            );
        }
        control = control.child(list);
    }
    control
}

fn filter_count_badge(state: &GuiState) -> impl IntoElement {
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
    div()
        .flex()
        .items_center()
        .gap_1()
        .overflow_hidden()
        .child(Icon::new(IconName::Funnel))
        .child(Badge::new().count(count))
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
    let has_watch = tab.stream.watch.is_some();
    let dot_color = if live {
        gpui_kit::gpui::rgb(0x3fb950)
    } else {
        gpui_kit::gpui::rgb(0x6e7681)
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
        .child(div().w(px(8.)).h(px(8.)).bg(dot_color))
        .child(
            Button::new("stream-toggle")
                .icon(Icon::new(icon))
                .disabled(!has_watch)
                .on_click(
                    cx.listener(move |app: &mut App, _, window: &mut Window, cx| {
                        app.dispatch(Message::StreamToggled(tab_idx), window, cx);
                    }),
                ),
        )
        .into_any_element()
}
