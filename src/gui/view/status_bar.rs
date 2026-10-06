use crate::gui::app::App;
use crate::gui::status::{
    status_bar_info, status_bar_left_text, status_bar_right_text, status_bar_schema_text,
};
use crate::shared::TabState;
use gpui_kit::component::ActiveTheme;
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{Context, div, px};

const STATUS_ROW_HEIGHT: f32 = 40.0;

/// Log counts, schema, cursor position, and active filtering progress.
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
    let mut col = div().flex().flex_col();
    if let Some(handle) = tab.and_then(|tab| tab.filter.handle.as_ref()) {
        let progress = handle.displayed_progress.clamp(0.0, 1.0);
        col = col.child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .px(px(16.))
                .py(px(4.))
                .bg(cx.theme().status_bar)
                .child(format!("Applying filters… {:.0}%", progress * 100.0))
                .child(
                    div().h(px(3.)).w_full().bg(cx.theme().secondary).child(
                        div()
                            .h_full()
                            .w(gpui_kit::gpui::relative(progress as f32))
                            .bg(cx.theme().primary),
                    ),
                ),
        );
    }
    col.child(
        div()
            .flex()
            .justify_between()
            .items_center()
            .gap_4()
            .h(px(STATUS_ROW_HEIGHT))
            .bg(cx.theme().status_bar)
            .px(px(16.))
            .overflow_hidden()
            .border_t_1()
            .border_color(cx.theme().border)
            .text_size(gpui_kit::gpui::rems(12. / 13.))
            .text_color(cx.theme().muted_foreground)
            .child(left.replace("   ", "   |   "))
            .child(schema)
            .child(right),
    )
}
