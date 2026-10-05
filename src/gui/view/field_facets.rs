use crate::filters::FilterDef;
use crate::gui::app::App;
use crate::gui::message::Message;
use crate::gui::update::filter_id_for_field_equality;
use crate::shared::FieldValueCounts;
use gpui_kit::assets::IconName;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme, Icon};
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{Context, Entity, Rgba, Window, div, px};
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
    search: Option<&Entity<InputState>>,
    cx: &mut Context<App>,
) -> impl IntoElement {
    let mut col = div()
        .flex()
        .flex_col()
        .gap_2()
        .pt_3()
        .border_t_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .font_weight(gpui_kit::gpui::FontWeight::SEMIBOLD)
                .child("Filter by field"),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Choose values to add include filters."),
        );
    let query = search
        .map(|input| input.read(cx).value().to_lowercase())
        .unwrap_or_default();
    if let Some(input) = search {
        col = col.child(
            Input::new(input)
                .id("facet-search-input")
                .bg(cx.theme().secondary)
                .text_color(cx.theme().foreground)
                .prefix(Icon::new(IconName::Search)),
        );
    }
    let mut matches = 0;
    for name in &counts.names {
        let Some(values) = counts.counts.get(name) else {
            continue;
        };
        let values: Vec<_> = values
            .iter()
            .filter(|(value, _)| {
                query.is_empty()
                    || name.to_lowercase().contains(&query)
                    || value.to_lowercase().contains(&query)
            })
            .cloned()
            .collect();
        if values.is_empty() {
            continue;
        }
        let is_expanded = !query.is_empty()
            || expanded
                .get(name)
                .copied()
                .unwrap_or(name == "level" || name == "target" || name == "service");
        matches += 1;
        col = col.child(facet_category(
            tab_idx,
            name,
            &values,
            filter_defs,
            is_expanded,
            cx,
        ));
    }
    if matches == 0 {
        col = col.child(
            div()
                .py_3()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(if query.is_empty() {
                    "No structured fields found in these logs."
                } else {
                    "No matching fields or values. Try another search."
                }),
        );
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
    let mut category = div()
        .flex()
        .flex_col()
        .py(px(8.))
        .border_t_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .id(format!("facet-category:{field}"))
                .flex()
                .items_center()
                .gap_2()
                .h(px(32.))
                .cursor_pointer()
                .overflow_hidden()
                .child(Icon::new(chevron))
                .child(div().flex_1().min_w(px(0.)).child(field.to_string()))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("{} values", values.len())),
                )
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
        .min_h(px(32.))
        .py_1()
        .px_2()
        .rounded(px(4.))
        .when(checked, |d| d.bg(cx.theme().secondary))
        .text_color(cx.theme().muted_foreground)
        .overflow_hidden()
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .flex_1()
                .min_w(px(0.))
                .child(div().w(px(8.)).h(px(8.)).rounded_full().bg(
                    match value.to_lowercase().as_str() {
                        "error" | "fatal" => cx.theme().danger,
                        "warn" | "warning" => cx.theme().warning,
                        "info" | "notice" => cx.theme().success,
                        "debug" | "trace" => cx.theme().primary,
                        _ => facet_dot_color(value).into(),
                    },
                ))
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
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_sm()
                        .text_color(cx.theme().foreground)
                        .child(if value.is_empty() {
                            "(empty)".to_string()
                        } else {
                            value.to_string()
                        }),
                ),
        )
        .child(div().flex_shrink_0().text_xs().child(count.to_string()))
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
