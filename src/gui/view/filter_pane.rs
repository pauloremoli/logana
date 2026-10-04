use crate::filters::{FilterDef, FilterType, GroupDef};
use crate::gui::app::App;
use crate::gui::color::{filter_row_bg_color, filter_row_color, group_tag_color};
use crate::gui::message::Message;
use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::{ActiveTheme, Icon};
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{Context, Empty, Window, div, px};

/// Native filter controls: add, edit, enable, remove, and drag to reorder.
pub fn filter_pane(
    tab_idx: usize,
    filter_defs: &[FilterDef],
    group_defs: &[GroupDef],
    cx: &mut Context<App>,
) -> impl IntoElement {
    let mut col = div().flex().flex_col().gap_2().child(
        div()
            .flex()
            .items_center()
            .justify_between()
            .child(format!("Active Filters ({})", filter_defs.len()))
            .child(
                Button::new("clear-all-filters")
                    .ghost()
                    .label("Clear all")
                    .tooltip("Remove every active filter")
                    .on_click(
                        cx.listener(move |app: &mut App, _, window: &mut Window, cx| {
                            app.dispatch(Message::ClearAllFilters(tab_idx), window, cx);
                        }),
                    ),
            ),
    );
    col = col.child(
        Button::new("add-filter")
            .ghost()
            .label("Add filter")
            .on_click(cx.listener(|app, _, window, cx| {
                app.dispatch(
                    Message::GuiAction(crate::gui::actions::Action::Filter),
                    window,
                    cx,
                )
            })),
    );
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
            .map(|(k, v)| format!("{k} equals {v}"))
            .collect();
        if let Some(text) = expr.1 {
            parts.push(format!("text contains {text}"));
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
    let pattern_color = filter_row_color(def, group_defs);
    let pattern_bg = filter_row_bg_color(def, group_defs);
    let tag_color = group_tag_color(def, group_defs);
    let mut pattern_text = div().child(display_pattern(&def.pattern));
    if let Some(bg) = pattern_bg {
        pattern_text = pattern_text.bg(bg);
    }
    if let Some(color) = pattern_color {
        pattern_text = pattern_text.text_color(color);
    }
    let mut tag_text = div().child(group_tag_text(def));
    if let Some(color) = tag_color {
        tag_text = tag_text.text_color(color);
    }
    div()
        .id(("filter-row", id))
        .on_drop(
            cx.listener(move |app: &mut App, drag: &FilterDrag, window, cx| {
                if drag.tab_idx == tab_idx {
                    app.dispatch(Message::FilterReordered(tab_idx, drag.id, id), window, cx);
                }
            }),
        )
        .drag_over::<FilterDrag>(|style, _, _, cx| style.border_color(cx.theme().primary))
        .flex()
        .flex_wrap()
        .items_center()
        .gap_2()
        .p(px(10.))
        .rounded(px(6.))
        .border_1()
        .border_color(if selected {
            cx.theme().primary
        } else {
            cx.theme().border
        })
        .bg(cx.theme().secondary)
        .child(
            div()
                .id(("filter-drag", id))
                .cursor_pointer()
                .child(Icon::new(IconName::GripVertical))
                .on_drag(FilterDrag { tab_idx, id }, |_, _, _, cx| cx.new(|_| Empty)),
        )
        .child(
            Checkbox::new(("filter-toggle", id))
                .checked(def.enabled)
                .on_click(
                    cx.listener(move |app: &mut App, _, window: &mut Window, cx| {
                        app.dispatch(Message::FilterToggled(tab_idx, id), window, cx);
                    }),
                ),
        )
        .child(
            Button::new(("filter-edit", id))
                .ghost()
                .small()
                .label("Edit")
                .on_click(cx.listener(move |app, _, window, cx| {
                    app.dispatch(Message::EditFilter(id), window, cx)
                })),
        )
        .child(filter_type_label(&def.filter_type))
        .child(tag_text)
        .child(div().flex_1().min_w(px(0.)).child(pattern_text))
        .child(ignore_case_tag(def.ignore_case))
        .child(
            Button::new(("filter-remove", id))
                .ghost()
                .icon(Icon::new(IconName::X))
                .tooltip("Remove filter")
                .on_click(
                    cx.listener(move |app: &mut App, _, window: &mut Window, cx| {
                        app.dispatch(Message::FilterRemoved(tab_idx, id), window, cx);
                    }),
                ),
        )
}

#[derive(Clone)]
struct FilterDrag {
    tab_idx: usize,
    id: usize,
}

/// Abbreviated filter-type label, matching the TUI sidebar's row format.
fn filter_type_label(filter_type: &FilterType) -> &'static str {
    match filter_type {
        FilterType::Include => "In",
        FilterType::Exclude => "Out",
        FilterType::Highlight => "H",
    }
}

/// A filter's `[group] ` tag, or empty when it isn't grouped.
fn group_tag_text(def: &FilterDef) -> String {
    def.group
        .as_deref()
        .map(|name| format!("[{name}] "))
        .unwrap_or_default()
}

fn ignore_case_tag(ignore_case: bool) -> &'static str {
    if ignore_case { " [i]" } else { "" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_type_label_abbreviates_like_the_tui() {
        assert_eq!(filter_type_label(&FilterType::Include), "In");
        assert_eq!(filter_type_label(&FilterType::Exclude), "Out");
        assert_eq!(filter_type_label(&FilterType::Highlight), "H");
    }

    fn filter_def(group: Option<&str>) -> FilterDef {
        FilterDef {
            id: 1,
            pattern: "ERROR".to_string(),
            filter_type: FilterType::Include,
            enabled: true,
            color_config: None,
            use_regex: false,
            ignore_case: false,
            group: group.map(str::to_string),
        }
    }

    #[test]
    fn group_tag_text_wraps_the_group_name() {
        assert_eq!(group_tag_text(&filter_def(Some("errors"))), "[errors] ");
    }

    #[test]
    fn group_tag_text_is_empty_without_a_group() {
        assert_eq!(group_tag_text(&filter_def(None)), "");
    }

    #[test]
    fn ignore_case_tag_reflects_the_flag() {
        assert_eq!(ignore_case_tag(true), " [i]");
        assert_eq!(ignore_case_tag(false), "");
    }
}
