use crate::gui::app::App;
use crate::gui::status::{
    status_bar_info, status_bar_left_text, status_bar_right_text, status_bar_schema_text,
};
use crate::ui::TabState;
use gpui_kit::component::ActiveTheme;
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{Context, div, px};

/// The bottom status bar: total/showing/filtered counts on the left, the
/// detected schema name in the middle, cursor position on the right.
/// `.flex()`/`.h()`/`.overflow_hidden()` are load-bearing here, not
/// cosmetic — see `log_pane.rs`'s `styled_line_row` doc comment for why a
/// flat row like this must always set them.
pub fn status_bar(tab: Option<&TabState>, cx: &mut Context<App>) -> impl IntoElement {
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
    div()
        .flex()
        .justify_between()
        .items_center()
        .gap_4()
        .h(px(24.))
        .px(px(8.))
        .overflow_hidden()
        .border_t_1()
        .border_color(cx.theme().border)
        .child(left)
        .child(schema)
        .child(right)
}
