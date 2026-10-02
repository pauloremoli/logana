use crate::gui::app::App;
use crate::gui::message::Message;
use crate::gui::state::GuiState;
use gpui_kit::assets::IconName;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::Icon;
use gpui_kit::component::button::Button;
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{Context, Window, div, px};

const TAB_BAR_HEIGHT: f32 = 36.0;
const TAB_HEIGHT: f32 = 28.0;

pub fn tab_bar(state: &GuiState, cx: &mut Context<App>) -> impl IntoElement {
    let mut bar = div()
        .flex()
        .items_center()
        .gap_1()
        .px(px(4.))
        .h(px(TAB_BAR_HEIGHT))
        .overflow_hidden()
        .border_b_1()
        .border_color(cx.theme().border);
    for (idx, tab) in state.tabs.iter().enumerate() {
        bar = bar.child(tab_label(
            idx,
            tab.title.clone(),
            idx == state.active_tab,
            cx,
        ));
    }
    bar.child(
        Button::new("open-file")
            .icon(Icon::new(IconName::Plus))
            .tooltip("Open file")
            .on_click(cx.listener(|app: &mut App, _, window: &mut Window, cx| {
                app.dispatch(Message::OpenFileDialog, window, cx);
            })),
    )
}

/// A tab chip: file icon, title, close button. The active tab is marked
/// with a background tint on the whole chip (matching the mockup) instead
/// of the TUI-era `[bracket]` convention this replaced.
fn tab_label(idx: usize, title: String, active: bool, cx: &mut Context<App>) -> impl IntoElement {
    let mut chip = div()
        .flex()
        .items_center()
        .gap_1()
        .h(px(TAB_HEIGHT))
        .px(px(6.))
        .rounded(px(4.))
        .overflow_hidden()
        .child(Icon::new(IconName::FileText))
        .child(
            Button::new(("tab-select", idx))
                .tooltip(format!("Switch to {title}"))
                .label(title)
                .on_click(
                    cx.listener(move |app: &mut App, _, window: &mut Window, cx| {
                        app.dispatch(Message::TabSelected(idx), window, cx);
                    }),
                ),
        )
        .child(
            Button::new(("tab-close", idx))
                .icon(Icon::new(IconName::X))
                .tooltip("Close tab")
                .on_click(
                    cx.listener(move |app: &mut App, _, window: &mut Window, cx| {
                        app.dispatch(Message::TabClosed(idx), window, cx);
                    }),
                ),
        );
    if active {
        chip = chip.bg(cx.theme().accent);
    }
    chip
}
