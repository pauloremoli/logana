use crate::filters::FilterDef;
use crate::gui::app::App;
use crate::gui::message::Message;
use crate::gui::update::filter_id_for_field_equality;
use crate::shared::FieldValueCounts;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::InputState;
use gpui_kit::component::{ActiveTheme, Icon};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{Context, Entity, Window, div, px};
use std::collections::HashMap;

const VALUES_PER_PAGE: usize = 40;

/// "Available Filters": one collapsible category per field `counts`
/// reports, each listing its values with a checkbox and occurrence count.
/// Reflects and controls real filters (`filter_id_for_field_equality` /
/// `Message::FacetValueToggled`) — toggling a checkbox adds or removes a
/// real `--field name=value` filter via the normal command pipeline, same
/// as typing `:filter --field name=value` in the TUI.
#[allow(clippy::too_many_arguments)]
pub fn field_facets(
    tab_idx: usize,
    counts: &FieldValueCounts,
    filter_defs: &[FilterDef],
    expanded: &HashMap<String, bool>,
    pages: &HashMap<String, usize>,
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
                .child(
                    "Checked values add Include filters. Multiple Include filters match any of them.",
                ),
        );
    let query = search
        .map(|input| input.read(cx).value().to_lowercase())
        .unwrap_or_default();
    if let Some(input) = search {
        col = col.child(
            super::input(input)
                .id("facet-search-input")
                .aria_label("Search fields and values")
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
            pages.get(name).copied().unwrap_or(0),
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

#[allow(clippy::too_many_arguments)]
fn facet_category(
    tab_idx: usize,
    field: &str,
    values: &[(String, usize)],
    filter_defs: &[FilterDef],
    expanded: bool,
    page: usize,
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
        let page = page.min(values.len().saturating_sub(1) / VALUES_PER_PAGE);
        let start = page * VALUES_PER_PAGE;
        let end = (start + VALUES_PER_PAGE).min(values.len());
        let mut list = div().flex().flex_col();
        if values.len() > VALUES_PER_PAGE {
            list = list.child(facet_pagination(field, page, end, values.len(), cx));
        }
        for (value, count) in &values[start..end] {
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

fn facet_pagination(
    field: &str,
    page: usize,
    end: usize,
    total: usize,
    cx: &mut Context<App>,
) -> impl IntoElement {
    let previous_field = field.to_owned();
    let next_field = field.to_owned();
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_1()
        .pb_2()
        .child(
            Button::new(format!("facet-previous:{field}"))
                .small()
                .ghost()
                .icon(IconName::ChevronLeft)
                .accessibility_label(format!("Previous values for {field}"))
                .disabled(page == 0)
                .on_click(cx.listener(move |app, _, _, cx| {
                    app.facet_pages
                        .insert(previous_field.clone(), page.saturating_sub(1));
                    cx.notify();
                })),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(format!("{}–{end} of {total}", page * VALUES_PER_PAGE + 1)),
        )
        .child(
            Button::new(format!("facet-next:{field}"))
                .small()
                .ghost()
                .icon(IconName::ChevronRight)
                .accessibility_label(format!("Next values for {field}"))
                .disabled(end == total)
                .on_click(cx.listener(move |app, _, _, cx| {
                    app.facet_pages.insert(next_field.clone(), page + 1);
                    cx.notify();
                })),
        )
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
                        _ => cx.theme().muted_foreground,
                    },
                ))
                .child(
                    Checkbox::new(format!("facet-value:{field}:{value}"))
                        .checked(checked)
                        .accessibility_label(format!("Filter {field} by {value}"))
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
