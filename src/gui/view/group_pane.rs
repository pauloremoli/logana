use crate::filters::{FilterDef, GroupDef, group_enabled};
use crate::gui::app::App;
use crate::gui::color::{group_row_bg_color, group_row_color};
use crate::gui::message::Message;
use gpui_kit::component::button::Button;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{Context, Window, div};

/// `GroupManagementMode`'s live state, mirroring `FilterManagementView`.
pub struct GroupManagementView {
    pub selected_group: String,
    pub search: String,
    pub searching: bool,
}

/// Every known group name — filters' `--group` values plus predefined
/// styles with no members yet — plus `GroupManagementMode`'s selection
/// cursor and live search narrowing when `management` is `Some`.
pub fn group_pane(
    tab_idx: usize,
    names: &[String],
    group_defs: &[GroupDef],
    filter_defs: &[FilterDef],
    management: Option<&GroupManagementView>,
    cx: &mut Context<App>,
) -> impl IntoElement {
    let mut col = div()
        .flex()
        .flex_col()
        .gap_2()
        .child(format!("Groups [{}]", names.len()));
    if let Some(mgmt) = management {
        col = col.child(if mgmt.searching {
            format!("/{}", mgmt.search)
        } else {
            "[GROUP MODE]".to_string()
        });
    }
    for name in names {
        let selected = management.is_some_and(|mgmt| mgmt.selected_group == *name);
        col = col.child(group_row(
            tab_idx,
            name.clone(),
            group_defs,
            filter_defs,
            selected,
            cx,
        ));
    }
    col
}

fn group_row(
    tab_idx: usize,
    name: String,
    group_defs: &[GroupDef],
    filter_defs: &[FilterDef],
    selected: bool,
    cx: &mut Context<App>,
) -> impl IntoElement {
    let count = member_count(&name, filter_defs);
    let enabled = group_enabled(group_defs, &name);
    let group_def = group_defs.iter().find(|g| g.name == name);
    let color = group_def.and_then(group_row_color);
    let bg = group_def.and_then(group_row_bg_color);
    let mut label = div().child(format!("{name} ({count})"));
    if let Some(bg) = bg {
        label = label.bg(bg);
    }
    if let Some(color) = color {
        label = label.text_color(color);
    }
    let toggle_name = name.clone();
    let delete_name = name.clone();
    div()
        .flex()
        .gap_2()
        .child(if selected { "> " } else { "  " })
        .child(
            Checkbox::new(format!("group-toggle-{name}"))
                .checked(enabled)
                .on_click(
                    cx.listener(move |app: &mut App, _, window: &mut Window, cx| {
                        app.dispatch(
                            Message::GroupToggled(tab_idx, toggle_name.clone()),
                            window,
                            cx,
                        );
                    }),
                ),
        )
        .child(label)
        .child(
            Button::new(format!("group-delete-{name}"))
                .label("Delete")
                .on_click(
                    cx.listener(move |app: &mut App, _, window: &mut Window, cx| {
                        app.dispatch(
                            Message::GroupDeleted(tab_idx, delete_name.clone()),
                            window,
                            cx,
                        );
                    }),
                ),
        )
}

/// How many filters belong to group `name` — matches the TUI sidebar's
/// `"{name} ({count})"` group row format.
fn member_count(name: &str, filter_defs: &[FilterDef]) -> usize {
    filter_defs
        .iter()
        .filter(|f| f.group.as_deref() == Some(name))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filters::FilterType;

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
    fn member_count_counts_only_matching_group() {
        let filters = vec![
            filter_def(Some("errors")),
            filter_def(Some("errors")),
            filter_def(Some("warnings")),
        ];
        assert_eq!(member_count("errors", &filters), 2);
        assert_eq!(member_count("warnings", &filters), 1);
        assert_eq!(member_count("missing", &filters), 0);
    }
}
