use crate::gui::app::App;
use crate::gui::state::StatusMessage;
use crate::gui::status::{
    status_bar_info, status_bar_left_text, status_bar_right_text, status_bar_schema_text,
};
use crate::ui::TabState;
use gpui_kit::component::ActiveTheme;
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{Context, div, px, rgb};

const STATUS_ROW_HEIGHT: f32 = 24.0;

/// The bottom status bar: an optional error toast row above the real
/// status row (total/showing/filtered counts on the left, the detected
/// schema name in the middle, cursor position on the right). The toast
/// shows `state.status` — set by several command interceptions
/// (`:set-theme`/`:schema`/etc. on a bad name) that previously had
/// nowhere to actually display it; nothing currently clears `state.status`
/// back to `None`; it just stays until the next status-setting event
/// overwrites it. `.flex()`/`.h()`/`.overflow_hidden()` are load-bearing
/// here, not cosmetic — see `log_pane.rs`'s `styled_line_row` doc comment
/// for why a flat row like this must always set them.
pub fn status_bar(
    tab: Option<&TabState>,
    status: Option<&StatusMessage>,
    cx: &mut Context<App>,
) -> impl IntoElement {
    let (left, schema, right) = match tab {
        Some(tab) => {
            let info = status_bar_info(tab);
            (
                status_bar_left_text(&info),
                status_bar_schema_text(tab),
                status_bar_right_text(&info),
            )
        }
        None => (String::new(), String::new(), String::new()),
    };
    let mut col = div().flex().flex_col();
    if let Some((message, bg, fg)) = status.map(|s| match s {
        StatusMessage::Error(message) => (message, rgb(0x3a1d1d), rgb(0xff6b6b)),
        StatusMessage::Info(message) => (message, rgb(0x1d2a3a), rgb(0x6bb3ff)),
    }) {
        col = col.child(
            div()
                .flex()
                .items_center()
                .h(px(STATUS_ROW_HEIGHT))
                .px(px(8.))
                .overflow_hidden()
                .whitespace_nowrap()
                .bg(bg)
                .text_color(fg)
                .border_t_1()
                .border_color(cx.theme().border)
                .child(message.clone()),
        );
    }
    col.child(
        div()
            .flex()
            .justify_between()
            .items_center()
            .gap_4()
            .h(px(STATUS_ROW_HEIGHT))
            .px(px(8.))
            .overflow_hidden()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(left)
            .child(schema)
            .child(right),
    )
}
