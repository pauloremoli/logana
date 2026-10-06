use crate::gui::app::App;
use crate::gui::message::Message;
use gpui_kit::assets::IconName;
use gpui_kit::base::{FocusTrapElement, TestSupportExt};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::{ActiveTheme, Icon};
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{
    AnyElement, Context, MouseButton, Pixels, Size, anchored, deferred, div, point, px,
};

pub fn command_palette(
    app: &App,
    viewport: Size<Pixels>,
    cx: &mut Context<App>,
) -> Option<AnyElement> {
    let palette = &app.state.palette;
    if !palette.visible() {
        return None;
    }
    let width = (f32::from(viewport.width) - 48.).clamp(0., 720.);
    let height = (f32::from(viewport.height) * 0.8).min((f32::from(viewport.height) - 48.).max(0.));
    let mut content = div()
        .id("command-panel")
        .w(px(width))
        .max_h(px(height))
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
        .rounded(px(10.))
        .bg(cx.theme().popover)
        .text_color(cx.theme().popover_foreground)
        .border_1()
        .border_color(cx.theme().border)
        .shadow_lg()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(div().child("Actions").text_size(px(15.)))
                .child(
                    Button::new("command-close")
                        .ghost()
                        .icon(IconName::X)
                        .tooltip("Close (Escape)")
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.dispatch(Message::PaletteClose, window, cx)
                        })),
                ),
        )
        .children(app.command_input.as_ref().map(|input| {
            super::input(input)
                .id("command-input")
                .prefix(Icon::new(IconName::Search))
                .bg(cx.theme().secondary)
                .text_color(cx.theme().foreground)
        }));

    let mut list = div()
        .id("command-suggestions")
        .max_h(px(height * 0.45))
        .overflow_y_scroll()
        .track_scroll(&app.command_scroll)
        .test_support();
    let suggestions = crate::gui::actions::matching(&palette.input);
    for (index, descriptor) in suggestions.iter().enumerate() {
        let missing = descriptor.unavailable(&app.state).is_some();
        list = list.child(
            ListItem::new(("command-suggestion", index))
                .disabled(missing)
                .selected(index == palette.selected)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .w_full()
                        .child(
                            div()
                                .flex()
                                .justify_between()
                                .child(descriptor.label)
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(cx.theme().muted_foreground)
                                        .child(crate::gui::keyboard::shortcut_for_action(
                                            &crate::gui::keyboard::bindings(&app.state),
                                            descriptor.action,
                                        )),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(cx.theme().muted_foreground)
                                .child(if missing {
                                    descriptor.unavailable(&app.state).unwrap()
                                } else {
                                    descriptor.description
                                }),
                        ),
                )
                .on_click(cx.listener(move |app, _, window, cx| {
                    app.dispatch(Message::PaletteInsert(index), window, cx)
                })),
        );
    }
    if suggestions.is_empty() {
        list = list.child(div().p_2().child("No matching actions."));
    }
    content = content.child(list);
    Some(
        deferred(
            anchored().position(point(px(0.), px(0.))).child(
                div()
                    .id("command-backdrop")
                    .occlude()
                    .w(viewport.width)
                    .h(viewport.height)
                    .flex()
                    .items_center()
                    .justify_center()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|app, _, window, cx| {
                            cx.stop_propagation();
                            app.dispatch(Message::PaletteClose, window, cx);
                        }),
                    )
                    .child(
                        content
                            .test_support()
                            .focus_trap("launcher-focus", &app.interface_focus),
                    )
                    .test_support(),
            ),
        )
        .into_any_element(),
    )
}
