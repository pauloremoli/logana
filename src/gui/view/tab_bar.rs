use crate::gui::app::App;
use crate::gui::message::Message;
use crate::gui::state::GuiState;
use gpui_kit::component::button::Button;
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{Context, Window, div};

pub fn tab_bar(state: &GuiState, cx: &mut Context<App>) -> impl IntoElement {
    let mut bar = div().flex().gap_1().p_1();
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
            .label("+ Open")
            .on_click(cx.listener(|app: &mut App, _, window: &mut Window, cx| {
                app.dispatch(Message::OpenFileDialog, window, cx);
            })),
    )
}

fn tab_label(idx: usize, title: String, active: bool, cx: &mut Context<App>) -> impl IntoElement {
    let label = if active { format!("[{title}]") } else { title };
    div()
        .flex()
        .gap_1()
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
                .label("x")
                .on_click(
                    cx.listener(move |app: &mut App, _, window: &mut Window, cx| {
                        app.dispatch(Message::TabClosed(idx), window, cx);
                    }),
                ),
        )
}
