use super::SelectionButton;
use crate::gui::{actions::Action, app::App, message::Message, preview};
use gpui_kit::{
    assets::IconName,
    base::{Disableable, TestSupportExt},
    component::{
        ActiveTheme, Sizable,
        button::{Button, ButtonGroup, ButtonVariants},
    },
    gpui::{AnyElement, ClipboardItem, Context, div, prelude::*, px},
};

pub fn panel(app: &App, viewport: f32, cx: &mut Context<App>) -> Option<AnyElement> {
    let (source, line) = app.inspected_row?;
    let tab = app.state.tabs.iter().find(|tab| tab.id == source)?;
    if line >= tab.file_reader.line_count() {
        return None;
    }
    let (bytes, _) = tab
        .file_reader
        .get_raw_line_prefix(line, preview::MAX_LINE_BYTES + 1);
    let visible = tab.filter.visible_indices.position_of(line).is_some();
    let fields = tab
        .display
        .format
        .as_deref()
        .map(|parser| preview::fields(parser, &bytes))
        .unwrap_or_default();
    let mut content = div()
        .id("inspector-fields")
        .test_support()
        .flex()
        .flex_col()
        .gap_3();
    if fields.is_empty() {
        content = content.child(if bytes.len() > preview::MAX_LINE_BYTES {
            "This line is too large for a parsed preview. Use Raw or copy the full message."
        } else {
            "No parsed fields for this line. Use Raw to read its text."
        });
    }
    for (name, value) in fields {
        let copy = value.clone();
        content = content.child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(name.clone())
                        .child(
                            Button::new(format!("inspector-copy-field:{name}"))
                                .ghost()
                                .small()
                                .icon(IconName::Copy)
                                .accessibility_label(format!("Copy {name}"))
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                                }),
                        ),
                )
                .child(
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .child(preview::text(&value)),
                ),
        );
    }
    let raw = app.inspector_raw;
    Some(div().id("row-inspector").test_support().w(px(420.)).max_w(gpui_kit::gpui::relative(0.9))
        .h_full().min_w(px(0.)).flex_shrink_0().flex().flex_col().gap_3().p_3()
        .bg(cx.theme().background).border_l_1().border_color(cx.theme().border)
        .when(viewport < 1100., |d| d.absolute().right_0().top_0().bottom_0().occlude().shadow_lg())
        .child(div().flex().items_center().justify_between().gap_2().child("Row inspector")
            .child(Button::new("inspector-close").ghost().icon(IconName::X).accessibility_label("Close row inspector")
                .on_click(cx.listener(|app, _, window, cx| { app.inspected_row = None; app.root_focus.focus(window, cx); cx.notify(); }))))
        .child(div().text_xs().text_color(cx.theme().muted_foreground).child(format!("{} · source row {}", tab.title, line + 1)))
        .child(ButtonGroup::new("inspector-view").small()
            .child(Button::new("inspector-parsed").label("Parsed").selected_accent(!raw)
                .on_click(cx.listener(|app, _, _, cx| { app.inspector_raw = false; cx.notify(); })))
            .child(Button::new("inspector-raw").label("Raw").selected_accent(raw)
                .on_click(cx.listener(|app, _, _, cx| { app.inspector_raw = true; cx.notify(); }))))
        .child(div().id("inspector-content").flex_1().min_h(px(0.)).overflow_y_scroll()
            .child(if raw {
                div().id("inspector-raw-text").test_support().child(preview::text(&String::from_utf8_lossy(&bytes[..bytes.len().min(preview::MAX_LINE_BYTES)]))).into_any_element()
            } else { content.into_any_element() }))
        .child(div().text_xs().text_color(cx.theme().muted_foreground)
            .child("Preview: up to 32 fields and 4,096 characters per value. Copy raw includes the full line."))
        .child(div().flex().flex_wrap().gap_2()
            .child(Button::new("inspector-copy-raw").label("Copy raw")
                .on_click(cx.listener(move |app, _, _, cx| {
                    if let Some(tab) = app.state.tabs.iter().find(|tab| tab.id == source) {
                        cx.write_to_clipboard(ClipboardItem::new_string(super::log_pane::raw_log_message(&tab.file_reader, line)));
                    }
                })))
            .child(Button::new("inspector-bookmark").disabled(!visible).label(if tab.mark_manager.is_marked(line) { "Unbookmark" } else { "Bookmark" })
                .on_click(cx.listener(move |app, _, window, cx| app.dispatch(Message::LogRowAction(source, line, Action::Bookmark), window, cx))))
            .child(Button::new("inspector-annotate").disabled(!visible).label("Add note")
                .on_click(cx.listener(move |app, _, window, cx| app.dispatch(Message::LogRowAction(source, line, Action::Annotation), window, cx)))))
        .into_any_element())
}
