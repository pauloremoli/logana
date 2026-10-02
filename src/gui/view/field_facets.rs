use crate::filters::FilterDef;
use crate::gui::app::App;
use crate::gui::message::Message;
use crate::gui::update::filter_id_for_field_equality;
use crate::ui::FieldValueCounts;
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{Context, Rgba, Window, div, px};
use std::collections::HashMap;

/// "Available Filters": one collapsible category per field `counts`
/// reports, each listing its values with a checkbox and occurrence count.
/// Reflects and controls real filters (`filter_id_for_field_equality` /
/// `Message::FacetValueToggled`) — toggling a checkbox adds or removes a
/// real `--field name=value` filter via the normal command pipeline, same
/// as typing `:filter --field name=value` in the TUI.
pub fn field_facets(
    tab_idx: usize,
    counts: &FieldValueCounts,
    filter_defs: &[FilterDef],
    expanded: &HashMap<String, bool>,
    cx: &mut Context<App>,
) -> impl IntoElement {
    let mut col = div().flex().flex_col().gap_1().child("Available Filters");
    for name in &counts.names {
        let Some(values) = counts.counts.get(name) else {
            continue;
        };
        let is_expanded = expanded.get(name).copied().unwrap_or(false);
        col = col.child(facet_category(
            tab_idx,
            name,
            values,
            filter_defs,
            is_expanded,
            cx,
        ));
    }
    col
}

fn facet_category(
    tab_idx: usize,
    field: &str,
    values: &[(String, usize)],
    filter_defs: &[FilterDef],
    expanded: bool,
    cx: &mut Context<App>,
) -> impl IntoElement {
    let chevron = if expanded {
        IconName::ChevronDown
    } else {
        IconName::ChevronRight
    };
    let field_owned = field.to_string();
    let mut category = div().flex().flex_col().child(
        div()
            .id(format!("facet-category:{field}"))
            .flex()
            .items_center()
            .gap_1()
            .overflow_hidden()
            .child(Icon::new(chevron))
            .child(field.to_string())
            .on_click(
                cx.listener(move |app: &mut App, _, window: &mut Window, cx| {
                    app.dispatch(Message::FacetToggled(field_owned.clone()), window, cx);
                }),
            ),
    );
    if expanded {
        let mut list = div().flex().flex_col();
        for (value, count) in values {
            list = list.child(facet_value_row(
                tab_idx,
                field,
                value,
                *count,
                filter_defs,
                cx,
            ));
        }
        category = category.child(list);
    }
    category
}

fn facet_value_row(
    tab_idx: usize,
    field: &str,
    value: &str,
    count: usize,
    filter_defs: &[FilterDef],
    cx: &mut Context<App>,
) -> impl IntoElement {
    let checked = filter_id_for_field_equality(filter_defs, field, value).is_some();
    let field_owned = field.to_string();
    let value_owned = value.to_string();
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .overflow_hidden()
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .overflow_hidden()
                .child(div().w(px(8.)).h(px(8.)).bg(facet_dot_color(value)))
                .child(
                    Checkbox::new(format!("facet-value:{field}:{value}"))
                        .checked(checked)
                        .on_click(
                            cx.listener(move |app: &mut App, _, window: &mut Window, cx| {
                                app.dispatch(
                                    Message::FacetValueToggled(
                                        tab_idx,
                                        field_owned.clone(),
                                        value_owned.clone(),
                                    ),
                                    window,
                                    cx,
                                );
                            }),
                        ),
                )
                .child(value.to_string()),
        )
        .child(count.to_string())
}

/// A deterministic color for a facet value's dot — these values have no
/// `FilterDef`/`GroupDef` color to borrow from (unlike every other color
/// helper in `gui::color`), so this derives a stable, readable RGB triplet
/// from a hash of the value's own text instead.
fn facet_dot_color(value: &str) -> Rgba {
    let hash = value.bytes().fold(5381u32, |acc, b| {
        acc.wrapping_mul(33).wrapping_add(b as u32)
    });
    // Keep each channel in a mid-to-bright range (80-255) so the dot is
    // never near-black or washed-out white regardless of the hash.
    let r = 80 + (hash & 0xFF) % 176;
    let g = 80 + ((hash >> 8) & 0xFF) % 176;
    let b = 80 + ((hash >> 16) & 0xFF) % 176;
    gpui_kit::gpui::rgb((r << 16) | (g << 8) | b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facet_dot_color_is_deterministic() {
        assert_eq!(facet_dot_color("error"), facet_dot_color("error"));
    }

    #[test]
    fn facet_dot_color_differs_for_different_values() {
        assert_ne!(facet_dot_color("error"), facet_dot_color("info"));
    }
}
