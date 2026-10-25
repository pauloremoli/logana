use crate::gui::app::App;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::link::Link;
use gpui_kit::gpui::{Context, div, prelude::*};

pub(super) fn content(cx: &Context<App>) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(
            div()
                .text_color(cx.theme().muted_foreground)
                .child(format!("Version {}", env!("CARGO_PKG_VERSION"))),
        )
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
}
