use crate::gui::app::App;
use gpui_kit::{
    base::TestSupportExt,
    component::{
        ActiveTheme,
        button::{Button, ButtonVariants},
    },
    gpui::{ClipboardItem, Context, div, prelude::*, px},
};

pub fn panel(app: &App, cx: &mut Context<App>) -> impl IntoElement {
    let mut panel = div()
        .id("export-preview")
        .test_support()
        .flex()
        .flex_col()
        .gap_2()
        .pt_3()
        .child("Document preview");
    panel = panel.child(div().text_xs().text_color(cx.theme().muted_foreground)
        .child("Sample: up to 3 notes, 8 rows per note, 8 bookmarks, and 4,096 output characters. Oversized rows are skipped. The full export includes all notes and bookmarks."));
    match &app.export_preview {
        None => panel.child("Preparing preview…"),
        Some(Err(error)) => panel.child(div().text_color(cx.theme().danger).child(error.clone())),
        Some(Ok(sample)) => {
            let output = sample.output.clone();
            panel
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!(
                            "{} notes · {} bookmarked rows · {} rows skipped",
                            sample.notes, sample.bookmarks, sample.skipped
                        )),
                )
                .child(
                    div()
                        .id("export-preview-output")
                        .test_support()
                        .max_h(px(280.))
                        .overflow_y_scroll()
                        .p_3()
                        .bg(cx.theme().secondary)
                        .child(output.clone()),
                )
                .child(
                    Button::new("export-preview-copy")
                        .ghost()
                        .label("Copy preview")
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(output.clone()))
                        }),
                )
        }
    }
}
