use crate::gui::{app::App, message::Message, state::StatusMessage};
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{AnyElement, Context, MouseButton, Pixels, Size, deferred, div, px};

/// Floats above the app without moving the table or taking keyboard focus.
pub fn notification(
    app: &App,
    viewport: Size<Pixels>,
    cx: &mut Context<App>,
) -> Option<AnyElement> {
    let status = app.state.status.as_ref()?;
    let revision = app.state.notification_revision;
    let (title, message, accent) = match status {
        StatusMessage::Error(message) => ("Error", message, cx.theme().danger),
        StatusMessage::Info(message) => ("Notification", message, cx.theme().primary),
    };
    Some(
        deferred(
            div()
                .id("notification-popup")
                .test_support()
                .absolute()
                .bottom(px(16.))
                .right(px(16.))
                .w(px((f32::from(viewport.width) - 32.).clamp(0., 420.)))
                .max_h(px((f32::from(viewport.height) - 32.).clamp(0., 240.)))
                .flex()
                .items_start()
                .gap_3()
                .p_3()
                .rounded(px(8.))
                .bg(cx.theme().popover)
                .text_color(cx.theme().popover_foreground)
                .border_1()
                .border_color(accent)
                .shadow_lg()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .max_h(px((f32::from(viewport.height) - 64.).clamp(0., 208.)))
                        .id("notification-output")
                        .overflow_y_scroll()
                        .child(div().text_color(accent).child(title))
                        .child(div().child(message.clone())),
                )
                .child(
                    Button::new("notification-dismiss")
                        .ghost()
                        .icon(IconName::X)
                        .tooltip("Dismiss notification")
                        .on_click(cx.listener(move |app, _, window, cx| {
                            app.dispatch(Message::DismissNotification(revision), window, cx)
                        })),
                ),
        )
        .into_any_element(),
    )
}
