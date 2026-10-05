use crate::filters::{FilterDef, FilterType, GroupDef};
use crate::gui::app::App;
use crate::gui::color::{filter_row_bg_color, filter_row_color};
use crate::gui::message::Message;
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Icon};
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{Context, Empty, div, px};

/// Native filter controls: add, edit, enable, remove, and drag to reorder.
pub fn filter_pane(
    tab_idx: usize,
    filter_defs: &[FilterDef],
    group_defs: &[GroupDef],
    cx: &mut Context<App>,
) -> impl IntoElement {
    let entity = cx.entity();
    let empty = filter_defs.is_empty();
    let mut col = div().flex().flex_col().gap_1().child(
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                div()
                    .flex_1()
                    .font_weight(gpui_kit::gpui::FontWeight::SEMIBOLD)
                    .child("Filters"),
            )
            .child(
                Button::new("add-filter")
                    .ghost()
                    .small()
                    .icon(IconName::Plus)
                    .label("Add")
                    .on_click(cx.listener(|app, _, window, cx| {
                        app.dispatch(
                            Message::GuiAction(crate::gui::actions::Action::Filter),
                            window,
                            cx,
                        )
                    })),
            )
            .child(
                Button::new("filter-menu")
                    .ghost()
                    .small()
                    .icon(IconName::Ellipsis)
                    .tooltip("Filter options")
                    .dropdown_menu(move |menu, _, _| {
                        let fields = entity.clone();
                        let clear = entity.clone();
                        menu.item(PopupMenuItem::new("Add from field").on_click(
                            move |_, window, cx| {
                                fields.update(cx, |app, cx| {
                                    app.dispatch(Message::FacetBrowserToggled, window, cx)
                                });
                            },
                        ))
                        .item(
                            PopupMenuItem::new("Clear all").disabled(empty).on_click(
                                move |_, window, cx| {
                                    clear.update(cx, |app, cx| {
                                        app.dispatch(Message::ClearAllFilters(tab_idx), window, cx)
                                    });
                                },
                            ),
                        )
                    }),
            ),
    );
    if empty {
        col = col.child(
            div()
                .py_3()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("No filters"),
        );
    }
    for def in filter_defs {
        col = col.child(filter_row(tab_idx, def, group_defs, false, cx));
    }
    col
}

pub(crate) fn display_pattern(pattern: &str) -> String {
    if let Some(expr) = pattern
        .strip_prefix(crate::filters::FIELD_PREFIX)
        .and_then(|e| crate::filters::parse_field_filter_expr(e).ok())
    {
        let mut parts: Vec<String> = expr
            .0
            .into_iter()
            .map(|(k, v)| format!("{k}: {v}"))
            .collect();
        if let Some(text) = expr.1 {
            parts.push(format!("text: {text}"));
        }
        parts.join(" · ")
    } else if let Some(date) = pattern.strip_prefix(crate::filters::DATE_PREFIX) {
        format!("Time {date}")
    } else {
        pattern.into()
    }
}
fn filter_row(
    tab_idx: usize,
    def: &FilterDef,
    group_defs: &[GroupDef],
    selected: bool,
    cx: &mut Context<App>,
) -> impl IntoElement {
    let id = def.id;
    let condition = display_pattern(&def.pattern);
    let full_condition = condition.clone();
    let type_label = filter_type_label(&def.filter_type);
    let foreground = filter_row_color(def, group_defs);
    let background = filter_row_bg_color(def, group_defs);
    div()
        .id(("filter-row", id))
        .group("filter-row")
        .on_drop(
            cx.listener(move |app: &mut App, drag: &FilterDrag, window, cx| {
                if drag.tab_idx == tab_idx {
                    app.dispatch(Message::FilterReordered(tab_idx, drag.id, id), window, cx);
                }
            }),
        )
        .drag_over::<FilterDrag>(|style, _, _, cx| style.bg(cx.theme().secondary))
        .flex()
        .items_center()
        .gap_2()
        .h(px(36.))
        .min_w(px(0.))
        .when_some(background, |d, bg| d.bg(bg))
        .when(selected, |d| d.bg(cx.theme().secondary))
        .child(
            div()
                .id(("filter-drag", id))
                .cursor_pointer()
                .opacity(0.)
                .group_hover("filter-row", |d| d.opacity(1.))
                .text_color(cx.theme().muted_foreground)
                .child(Icon::new(IconName::GripVertical).size(px(12.)))
                .on_drag(FilterDrag { tab_idx, id }, |_, _, _, cx| cx.new(|_| Empty)),
        )
        .child(
            Checkbox::new(("filter-toggle", id))
                .checked(def.enabled)
                .tooltip("Enable or disable filter")
                .on_click(cx.listener(move |app, _, window, cx| {
                    app.dispatch(Message::FilterToggled(tab_idx, id), window, cx)
                })),
        )
        .child(
            div()
                .id(("filter-type", id))
                .test_support()
                .w(px(12.))
                .flex_shrink_0()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(filter_type_symbol(&def.filter_type))
                .tooltip(move |window, cx| Tooltip::new(type_label).build(window, cx)),
        )
        .child(
            div()
                .id(("filter-condition", id))
                .test_support()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .text_sm()
                .cursor_pointer()
                .text_color(if def.enabled {
                    foreground.map(Into::into).unwrap_or(cx.theme().foreground)
                } else {
                    cx.theme().muted_foreground
                })
                .child(condition)
                .tooltip(move |window, cx| Tooltip::new(full_condition.clone()).build(window, cx))
                .on_click(cx.listener(move |app, _, window, cx| {
                    app.dispatch(Message::EditFilter(id), window, cx)
                })),
        )
        .child(
            Button::new(("filter-edit", id))
                .ghost()
                .small()
                .icon(IconName::Pencil)
                .tooltip("Edit filter")
                .on_click(cx.listener(move |app, _, window, cx| {
                    app.dispatch(Message::EditFilter(id), window, cx)
                })),
        )
        .child(
            Button::new(("filter-remove", id))
                .ghost()
                .small()
                .icon(IconName::X)
                .tooltip("Delete filter")
                .on_click(cx.listener(move |app, _, window, cx| {
                    app.dispatch(Message::FilterRemoved(tab_idx, id), window, cx)
                })),
        )
}

fn filter_type_symbol(kind: &FilterType) -> &'static str {
    match kind {
        FilterType::Include => "+",
        FilterType::Exclude => "−",
        FilterType::Highlight => "◇",
    }
}

#[derive(Clone)]
struct FilterDrag {
    tab_idx: usize,
    id: usize,
}

/// Explicit labels keep rule behavior readable in the GUI.
fn filter_type_label(filter_type: &FilterType) -> &'static str {
    match filter_type {
        FilterType::Include => "Include",
        FilterType::Exclude => "Exclude",
        FilterType::Highlight => "Highlight",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn field_filters_display_compact_field_value_labels() {
        let pattern = crate::filters::encode_field_filter(
            &[
                ("service".into(), "payments".into()),
                ("level".into(), "ERROR".into()),
            ],
            Some("request timeout"),
        );
        assert_eq!(
            display_pattern(&pattern),
            "service: payments · level: ERROR · text: request timeout"
        );
        assert_eq!(display_pattern("timeout"), "timeout");
    }

    #[test]
    fn filter_types_have_discrete_symbols_and_explicit_tooltips() {
        for (kind, symbol, label) in [
            (FilterType::Include, "+", "Include"),
            (FilterType::Exclude, "−", "Exclude"),
            (FilterType::Highlight, "◇", "Highlight"),
        ] {
            assert_eq!(filter_type_symbol(&kind), symbol);
            assert_eq!(filter_type_label(&kind), label);
        }
    }
}
