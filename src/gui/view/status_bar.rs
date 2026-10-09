use crate::gui::app::App;
use crate::gui::state::NavPage;
use crate::gui::status::{
    status_bar_info, status_bar_left_text, status_bar_right_text, status_bar_schema_text,
};
use crate::shared::TabState;
use gpui_kit::component::{ActiveTheme, progress::Progress};
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{Context, div, px};

const STATUS_ROW_HEIGHT: f32 = 40.0;

/// Log counts, schema, cursor position, and active filtering progress.
pub fn status_bar(
    tab: Option<&TabState>,
    nav_page: NavPage,
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
                    Progress::new("filter-status-progress")
                        .accessibility_label("Applying filters")
                        .value(progress as f32 * 100.),
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
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .min_w(px(0.))
                    .flex_1()
                    .child(
                        div()
                            .flex_shrink_0()
                            .child(super::search_bar::view_mode_control(nav_page, cx)),
                    )
                    .child(
                        div()
                            .min_w(px(0.))
                            .truncate()
                            .child(left.replace("   ", "   |   ")),
                    ),
            )
            .child(div().max_w(px(140.)).truncate().child(schema))
            .child(div().flex_shrink_0().child(right)),
    )
}
