use crate::gui::app::App;
use crate::gui::message::Message;
use crate::gui::state::GuiState;
use gpui_kit::assets::IconName;
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
        .h(px(TAB_BAR_HEIGHT))
        .overflow_hidden();
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
            .on_click(cx.listener(|app: &mut App, _, window: &mut Window, cx| {
                app.dispatch(Message::OpenFileDialog, window, cx);
            })),
    )
}

fn tab_label(idx: usize, title: String, active: bool, cx: &mut Context<App>) -> impl IntoElement {
    let label = if active { format!("[{title}]") } else { title };
    div()
        .flex()
        .items_center()
        .gap_1()
        .h(px(TAB_HEIGHT))
        .overflow_hidden()
        .child(Icon::new(IconName::FileText))
        .child(
            Button::new(("tab-select", idx))
                .label(label)
                .on_click(
                    cx.listener(move |app: &mut App, _, window: &mut Window, cx| {
                        app.dispatch(Message::TabSelected(idx), window, cx);
                    }),
                ),
        )
        .child(
            Button::new(("tab-close", idx))
                .icon(Icon::new(IconName::X))
                .on_click(
                    cx.listener(move |app: &mut App, _, window: &mut Window, cx| {
                        app.dispatch(Message::TabClosed(idx), window, cx);
                    }),
                ),
        )
}
