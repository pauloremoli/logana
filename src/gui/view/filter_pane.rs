use crate::filters::{FilterDef, FilterType, GroupDef};
use crate::gui::app::App;
use crate::gui::color::{filter_row_bg_color, filter_row_color, group_tag_color};
use crate::gui::message::Message;
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::button::Button;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{Context, Window, div};

/// `FilterManagementMode`'s live state, needed to render the sidebar's
/// selection cursor and live search narrowing while that mode is active.
pub struct FilterManagementView {
    pub selected_index: usize,
    pub search: String,
    pub searching: bool,
}

/// The active tab's filter list — a passive display (managed via the `:`
/// command bar and each row's own checkbox) unless `management` is
/// `Some`, in which case it also shows `FilterManagementMode`'s selection
/// cursor and search narrowing.
pub fn filter_pane(
    tab_idx: usize,
    filter_defs: &[FilterDef],
    group_defs: &[GroupDef],
    management: Option<&FilterManagementView>,
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
                    .label("Clear all")
                    .tooltip("Remove every active filter")
                    .on_click(
                        cx.listener(move |app: &mut App, _, window: &mut Window, cx| {
                            app.dispatch(Message::ClearAllFilters(tab_idx), window, cx);
                        }),
                    ),
            ),
    );
    if let Some(mgmt) = management {
        col = col.child(if mgmt.searching {
            format!("/{}", mgmt.search)
        } else {
            "[FILTER MODE]".to_string()
        });
    }
    for (idx, def) in filter_defs.iter().enumerate() {
        let selected = management.is_some_and(|mgmt| mgmt.selected_index == idx);
        col = col.child(filter_row(tab_idx, def, group_defs, selected, cx));
    }
    col
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
    let mut pattern_text = div().child(def.pattern.clone());
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
        .flex()
        .gap_2()
        .child(if selected { "> " } else { "  " })
        .child(
            Checkbox::new(("filter-toggle", id))
                .checked(def.enabled)
                .on_click(
                    cx.listener(move |app: &mut App, _, window: &mut Window, cx| {
                        app.dispatch(Message::FilterToggled(tab_idx, id), window, cx);
                    }),
                ),
        )
        .child(filter_type_label(&def.filter_type))
        .child(tag_text)
        .child(pattern_text)
        .child(ignore_case_tag(def.ignore_case))
        .child(
            Button::new(("filter-remove", id))
                .icon(Icon::new(IconName::X))
                .tooltip("Remove filter")
                .on_click(
                    cx.listener(move |app: &mut App, _, window: &mut Window, cx| {
                        app.dispatch(Message::FilterRemoved(tab_idx, id), window, cx);
                    }),
                ),
        )
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
