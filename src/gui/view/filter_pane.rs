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
use gpui_kit::component::tag::Tag;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Icon};
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{Context, Empty, div, px};

/// Native filter controls: add, edit, enable, remove, and drag to reorder.
pub fn filter_pane(
    tab_idx: usize,
    filter_defs: &[FilterDef],
    group_defs: &[GroupDef],
    selected: Option<usize>,
    cx: &mut Context<App>,
) -> impl IntoElement + use<> {
    let entity = cx.entity();
    let empty = filter_defs.is_empty();
    let enabled = filter_defs.iter().filter(|def| def.enabled).count();
    let mut col = div().flex().flex_col().gap_2().child(
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                div()
                    .flex_1()
                    .font_weight(gpui_kit::gpui::FontWeight::SEMIBOLD)
                    .child("Filters")
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui_kit::gpui::FontWeight::NORMAL)
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("{enabled} of {} enabled", filter_defs.len())),
                    ),
            )
            .child(
                Button::new("add-filter")
                    .primary()
                    .small()
                    .icon(IconName::Plus)
                    .label("Add filter")
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
                .flex()
                .flex_col()
                .items_center()
                .text_center()
                .gap_2()
                .py_6()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(Icon::new(IconName::Funnel).size(px(24.)))
                .child("No filters yet")
                .child(
                    div()
                        .text_xs()
                        .child("Add a rule to include, exclude, or highlight logs."),
                ),
        );
    }
    for (index, def) in filter_defs.iter().enumerate() {
        col = col.child(filter_row(
            tab_idx,
            def,
            group_defs,
            selected == Some(index),
            cx,
        ));
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
        .on_mouse_down(
            gpui_kit::gpui::MouseButton::Left,
            cx.listener(move |app, _, _, cx| {
                app.state.keyboard_sidebar = Some(crate::gui::state::SidebarTab::Filters);
                app.state.sidebar_cursor = app.state.tabs[tab_idx]
                    .log_manager
                    .get_filters()
                    .iter()
                    .position(|filter| filter.id == id)
                    .unwrap_or(0);
                cx.notify();
            }),
        )
        .when(selected, |row| row.bg(cx.theme().secondary))
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
        .py_2()
        .border_b_1()
        .border_color(cx.theme().border)
        .min_w(px(0.))
        .hover(|d| d.bg(cx.theme().secondary))
        .child(
            div()
                .id(("filter-drag", id))
                .cursor_pointer()
                .opacity(0.5)
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
                .id(("filter-condition", id))
                .test_support()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .items_start()
                .gap_1()
                .text_sm()
                .child(
                    div()
                        .id(("filter-type", id))
                        .test_support()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_1()
                        .child(Tag::secondary().small().child(type_label))
                        .when(def.use_regex, |d| {
                            d.child(Tag::secondary().small().child("Regex"))
                        })
                        .when(def.ignore_case, |d| {
                            d.child(Tag::secondary().small().child("Ignore case"))
                        })
                        .when_some(def.group.clone(), |d, group| {
                            d.child(Tag::secondary().small().child(format!("Group: {group}")))
                        })
                        .tooltip(move |window, cx| Tooltip::new(type_label).build(window, cx)),
                )
                .cursor_pointer()
                .child(
                    div()
                        .max_w_full()
                        .truncate()
                        .when_some(background, |d, bg| d.bg(bg))
                        .text_color(if def.enabled {
                            foreground.map(Into::into).unwrap_or(cx.theme().foreground)
                        } else {
                            cx.theme().muted_foreground
                        })
                        .child(condition),
                )
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
    fn filter_types_have_explicit_labels() {
        for (kind, label) in [
            (FilterType::Include, "Include"),
            (FilterType::Exclude, "Exclude"),
            (FilterType::Highlight, "Highlight"),
        ] {
            assert_eq!(filter_type_label(&kind), label);
        }
    }
}
