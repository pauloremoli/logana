use crate::gui::{actions::Action, app::App, message::Message};
use gpui_kit::component::button::Button;
use gpui_kit::component::link::Link;
use gpui_kit::component::{ActiveTheme, StyledExt};
use gpui_kit::gpui::{Context, div, prelude::*, px};

pub(super) fn page(cx: &mut Context<App>) -> impl IntoElement {
    div()
        .id("about-page")
        .flex_1()
        .min_w(px(0.))
        .overflow_y_scroll()
        .p_6()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_4()
                .max_w(px(680.))
                .text_color(cx.theme().foreground)
                .child(div().text_xl().font_semibold().child("About logana"))
                .child(
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("Version {}", env!("CARGO_PKG_VERSION"))),
                )
                .child(env!("CARGO_PKG_DESCRIPTION"))
                .child(format!("Created by {}", env!("CARGO_PKG_AUTHORS")))
                .child(format!("License: {}", env!("CARGO_PKG_LICENSE")))
                .child(
                    Link::new("about-project")
                        .href(env!("CARGO_PKG_REPOSITORY"))
                        .child("Project on GitHub"),
                )
                .child(
                    Link::new("about-issues")
                        .href(concat!(env!("CARGO_PKG_REPOSITORY"), "/issues"))
                        .child("Report an issue"),
                )
                .child(
                    Button::new("about-back")
                        .outline()
                        .label("Back to logs")
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.dispatch(Message::GuiAction(Action::ShowLogs), window, cx);
                        })),
                ),
        )
}
