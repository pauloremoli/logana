use crate::filters::{FilterDef, GroupDef, group_enabled};
use crate::gui::app::App;
use crate::gui::color::{group_row_bg_color, group_row_color};
use crate::gui::message::Message;
use gpui_kit::assets::IconName;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{Context, Window, div, px};

/// The group list's own header row occupies child index 0 of the
/// scrollable container (see `group_pane`), so a selected group at
/// `index` sits at `index + GROUP_LIST_HEADER_ROWS` for `scroll_to_item`.
const GROUP_LIST_HEADER_ROWS: usize = 1;

/// Native group management, including groups without member filters.
pub fn group_pane(
    tab_idx: usize,
    names: &[String],
    group_defs: &[GroupDef],
    filter_defs: &[FilterDef],
    selected: Option<usize>,
    scroll: &gpui_kit::gpui::ScrollHandle,
    cx: &mut Context<App>,
) -> impl IntoElement {
    if let Some(index) = selected {
        scroll.scroll_to_item(index + GROUP_LIST_HEADER_ROWS);
    }
    let mut col = div()
        .id("sidebar-groups-scroll")
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .gap_1()
        .overflow_y_scroll()
        .track_scroll(scroll)
        .px_4()
        .pt_4()
        .pb_4()
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .flex_1()
                        .font_weight(gpui_kit::gpui::FontWeight::SEMIBOLD)
                        .child(format!("Groups [{}]", names.len())),
                )
                .child(
                    Button::new("add-group")
                        .ghost()
                        .small()
                        .icon(IconName::Plus)
                        .label("Add")
                        .tooltip("Add group")
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.dispatch(
                                Message::GuiAction(crate::gui::actions::Action::Group),
                                window,
                                cx,
                            )
                        })),
                ),
        );
    if names.is_empty() {
        col = col.child(
            div()
                .py_3()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("No groups"),
        );
    }
    for (index, name) in names.iter().enumerate() {
        col = col.child(group_row(
            tab_idx,
            name.clone(),
            group_defs,
            filter_defs,
            selected == Some(index),
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
    let full_label = format!("{name} ({count})");
    let label_name = name.clone();
    let label = div()
        .id(format!("group-label-{name}"))
        .flex_1()
        .min_w(px(0.))
        .flex()
        .items_center()
        .text_sm()
        .cursor_pointer()
        .text_color(if enabled {
            color.map(Into::into).unwrap_or(cx.theme().foreground)
        } else {
            cx.theme().muted_foreground
        })
        .child(
            div()
                .max_w_full()
                .truncate()
                .when_some(bg, |d, bg| d.bg(bg))
                .child(full_label.clone()),
        )
        .tooltip(move |window, cx| Tooltip::new(full_label.clone()).build(window, cx))
        .on_click(cx.listener(move |app, _, window, cx| {
            app.dispatch(Message::EditGroup(label_name.clone()), window, cx)
        }));
    let toggle_name = name.clone();
    let delete_name = name.clone();
    let edit_name = name.clone();
    let selected_name = name.clone();
    div()
        .id(gpui_kit::gpui::SharedString::from(format!(
            "group-row-{name}"
        )))
        .on_mouse_down(
            gpui_kit::gpui::MouseButton::Left,
            cx.listener(move |app, _, _, cx| {
                app.state.keyboard_sidebar = Some(crate::gui::state::SidebarTab::Groups);
                app.state.sidebar_cursor = app.state.tabs[tab_idx]
                    .log_manager
                    .group_names()
                    .iter()
                    .position(|name| *name == selected_name)
                    .unwrap_or(0);
                cx.notify();
            }),
        )
        .flex()
        .items_center()
        .gap_2()
        .h(px(36.))
        .min_w(px(0.))
        .when(selected, |d| d.bg(cx.theme().secondary))
        .child(
            Checkbox::new(format!("group-toggle-{name}"))
                .checked(enabled)
                .tooltip("Enable or disable group")
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
            Button::new(format!("group-edit-{name}"))
                .ghost()
                .small()
                .icon(IconName::Pencil)
                .tooltip("Edit group")
                .on_click(cx.listener(move |app, _, window, cx| {
                    app.dispatch(Message::EditGroup(edit_name.clone()), window, cx)
                })),
        )
        .child(
            Button::new(format!("group-delete-{name}"))
                .ghost()
                .small()
                .icon(IconName::X)
                .tooltip(format!("Delete group {name}"))
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
