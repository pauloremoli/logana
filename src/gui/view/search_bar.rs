use crate::gui::app::App;
use crate::gui::message::Message;
use crate::gui::state::GuiState;
use crate::gui::time_range::{ALL_PRESETS, preset_label};
use crate::gui::update::is_tab_live;
use crate::input::{KeyCode, KeyModifiers};
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
        .child(search_input(state, cx))
        .child(time_range_control(state, cx))
        .child(filter_count_badge(state))
        .child(live_indicator(state, cx))
}

/// Shows the `/` (forward) or `?` (backward) search prompt and its live
/// query while `SearchMode` is active; the placeholder otherwise (Command
/// mode has its own floating overlay — see `command_palette` — rather
/// than sharing this slot). Styled to look like a real text input field
/// (bordered, rounded, its own background) even though it isn't a
/// focusable native input: typing is handled entirely by the TUI's real
/// `SearchMode`, same as every other mode, via the root's capture-phase
/// key handler — this just reflects its `render_state`. Clicking it while
/// not already searching dispatches the same `/` keypress `NormalMode`'s
/// `search_forward` binding handles, so it enters `SearchMode` through the
/// real keybinding-driven transition rather than a GUI-only shortcut; once
/// already searching, clicking does nothing (routing another `/` through
/// `SearchMode` would type a literal `/` into the query).
fn search_input(state: &GuiState, cx: &mut Context<App>) -> impl IntoElement {
    let render_state = state
        .active_tab()
        .map(|tab| tab.interaction.mode.render_state());
    let (text, is_placeholder) = match &render_state {
        Some(ModeRenderState::Search { query, forward, .. }) => {
            let prompt = if *forward { "/" } else { "?" };
            (format!("{prompt}{query}"), false)
        }
        _ => (PLACEHOLDER.to_string(), true),
    };
    let already_searching = matches!(render_state, Some(ModeRenderState::Search { .. }));
    let mut input = div()
        .id("search-input")
        .flex()
        .flex_1()
        .h(px(24.))
        .items_center()
        .px(px(8.))
        .rounded(px(6.))
        .bg(cx.theme().input)
        .overflow_hidden()
        .whitespace_nowrap()
        .child(text);
    if is_placeholder {
        input = input.text_color(cx.theme().muted_foreground);
    }
    if !already_searching {
        input = input.on_click(cx.listener(|app: &mut App, _, window: &mut Window, cx| {
            app.dispatch(
                Message::KeyPressed(KeyCode::Char('/'), KeyModifiers::NONE),
                window,
                cx,
            );
        }));
    }
    input
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
                .tooltip(if tab.stream.paused { "Resume" } else { "Pause" })
                .on_click(
                    cx.listener(move |app: &mut App, _, window: &mut Window, cx| {
                        app.dispatch(Message::StreamToggled(tab_idx), window, cx);
                    }),
                ),
        )
        .into_any_element()
}
