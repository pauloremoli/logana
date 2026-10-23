use crate::gui::{app::App, preview};
use gpui_kit::{
    base::TestSupportExt,
    component::{ActiveTheme, description_list::DescriptionList},
    gpui::{Context, div, prelude::*},
};

pub fn panel(app: &App, cx: &mut Context<App>) -> impl IntoElement {
    let mut panel = div()
        .id("schema-preview")
        .test_support()
        .flex()
        .flex_col()
        .gap_2()
        .pt_3();
    panel = panel.child("Sample from the first 8 source rows");
    match &app.schema_preview {
        None => panel.child("Preparing sample…"),
        Some(Err(error)) => panel.child(div().text_color(cx.theme().danger).child(error.clone())),
        Some(Ok(sample)) => {
            panel = panel.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "{} rows examined · {} with fields · {} oversized rows skipped",
                        sample.examined, sample.matched, sample.skipped
                    )),
            );
            if sample.unstructured {
                panel.child(
                    div()
                        .id("schema-preview-raw")
                        .test_support()
                        .child(sample.raw.clone()),
                )
            } else if sample.fields.is_empty() {
                panel.child("No fields in this sample. Try another schema or inspect a different source row.")
            } else {
                let mut fields = DescriptionList::new().columns(1).bordered(false);
                for (name, value) in &sample.fields {
                    fields = fields.item(name.clone(), preview::text(value), 1);
                }
                panel.child(
                    div()
                        .id("schema-preview-fields")
                        .test_support()
                        .child(fields),
                )
            }
        }
    }
}
