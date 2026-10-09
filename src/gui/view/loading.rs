//! Progress stays visible without preventing interaction with existing tabs.
use crate::{
    gui::app::App,
    ingestion::archive_tree::{CheckState, NodeKind},
};
use gpui_kit::{
    assets::IconName,
    base::{Disableable, TestSupportExt},
    component::{
        ActiveTheme,
        button::{Button, ButtonVariants},
        checkbox::Checkbox,
        progress::Progress,
    },
    gpui::{AnyElement, Context, MouseButton, div, prelude::*, px},
};

pub fn operations(app: &App, cx: &mut Context<App>) -> Option<AnyElement> {
    if app.loading.operations.is_empty() {
        return None;
    }
    Some(
        div()
            .id("loading-operations")
            .test_support()
            .max_h(px(180.))
            .overflow_y_scroll()
            .border_t_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .p_3()
            .flex()
            .flex_col()
            .gap_3()
            .children(app.loading.operations.iter().map(|op| {
                let id = op.id;
                let progress = &op.progress;
                let counts = if progress.total_files > 0 {
                    format!(" · {}/{} files", progress.completed, progress.total_files)
                } else {
                    String::new()
                };
                let amount = match progress.total_bytes {
                    Some(total) => {
                        format!("{} / {} MiB", progress.bytes / 1_048_576, total / 1_048_576)
                    }
                    None if progress.bytes > 0 => {
                        format!("{} MiB processed", progress.bytes / 1_048_576)
                    }
                    _ => String::new(),
                };
                let choosing = op.picker.is_some();
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(div().truncate().child(format!(
                                "{} — {}{counts}",
                                op.path.file_name().unwrap_or_default().to_string_lossy(),
                                progress.phase
                            )))
                            .child(
                                div()
                                    .truncate()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(if progress.entry.is_empty() {
                                        amount.clone()
                                    } else {
                                        format!("{}  {amount}", progress.entry)
                                    }),
                            )
                            .when(!choosing, |el| {
                                el.child(
                                    div().id(("load-progress", id)).test_support().child(
                                        Progress::new(("load-progress-value", id))
                                            .accessibility_label(format!(
                                                "{} progress",
                                                progress.phase
                                            ))
                                            .loading(progress.fraction.is_none())
                                            .value(progress.fraction.unwrap_or(0.) as f32 * 100.),
                                    ),
                                )
                            }),
                    )
                    .when(choosing, |el| {
                        el.child(
                            Button::new(("archive-choose", id))
                                .label("Choose files")
                                .on_click(cx.listener(move |app, _, _, cx| {
                                    app.loading.selected_picker = Some(id);
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        Button::new(("load-cancel", id))
                            .label("Cancel")
                            .disabled(op.control.cancelled())
                            .on_click(cx.listener(move |app, _, _, cx| app.cancel_loading(id, cx))),
                    )
            }))
            .into_any_element(),
    )
}

pub fn picker(app: &App, cx: &mut Context<App>) -> Option<AnyElement> {
    let id = app.loading.selected_picker?;
    let op = app.loading.operations.iter().find(|op| op.id == id)?;
    let picker = op.picker.as_ref()?;
    let selected = picker
        .tree
        .nodes
        .iter()
        .filter(|n| n.selected && matches!(n.kind, NodeKind::File))
        .count();
    let sizes = picker.sizes.clone();
    let list = gpui_kit::base::v_virtual_list(
        cx.entity(),
        ("archive-entries", id),
        sizes,
        move |app: &mut App, range, _window, cx| {
            let Some(picker) = app
                .loading
                .operations
                .iter()
                .find(|op| op.id == id)
                .and_then(|op| op.picker.as_ref())
            else {
                return Vec::new();
            };
            range
                .filter_map(|row| picker.rows.get(row).copied().map(|node| (row, node)))
                .map(|(row, node)| {
                    let item = &picker.tree.nodes[node];
                    let container = matches!(
                        item.kind,
                        NodeKind::Container { .. } | NodeKind::LazyContainer { .. }
                    );
                    let lazy = matches!(item.kind, NodeKind::LazyContainer { .. });
                    let error = match &item.kind {
                        NodeKind::UnreadableContainer { error } => Some(error.clone()),
                        _ => None,
                    };
                    let check = picker.checks[node];
                    div()
                        .id(("archive-row", node))
                        .test_support()
                        .h(px(32.))
                        .flex()
                        .items_center()
                        .gap_1()
                        .pl(px((item.depth.min(8) * 12) as f32))
                        .pr_2()
                        .bg(if picker.cursor == row {
                            cx.theme().secondary
                        } else {
                            cx.theme().background
                        })
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |app, _, window, cx| {
                                if let Some(picker) = app
                                    .loading
                                    .operations
                                    .iter_mut()
                                    .find(|op| op.id == id)
                                    .and_then(|op| op.picker.as_mut())
                                {
                                    picker.cursor = row;
                                }
                                app.loading.focus.focus(window, cx);
                                cx.notify();
                            }),
                        )
                        .child(
                            Button::new(("archive-expand", node))
                                .ghost()
                                .icon(if lazy || item.collapsed {
                                    IconName::ChevronRight
                                } else {
                                    IconName::ChevronDown
                                })
                                .disabled(!container)
                                .accessibility_label("Expand or collapse archive")
                                .tooltip("Expand or collapse archive")
                                .on_click(cx.listener(move |app, _, _, cx| {
                                    app.picker_expand(id, node, cx)
                                })),
                        )
                        .child(
                            Checkbox::new(("archive-check", node))
                                .checked(check == CheckState::Checked)
                                .disabled(lazy || error.is_some())
                                .tooltip(if check == CheckState::Partial {
                                    "Some files selected"
                                } else {
                                    "Select files"
                                })
                                .label(item.name.clone())
                                .on_click(cx.listener(move |app, _, _, cx| {
                                    app.picker_toggle(id, node, cx)
                                })),
                        )
                        .when_some(error, |el, error| {
                            el.child(div().truncate().text_color(cx.theme().danger).child(error))
                        })
                })
                .collect::<Vec<_>>()
        },
    )
    .track_scroll(&app.loading.scroll);
    Some(
        div()
            .id("archive-picker")
            .test_support()
            .track_focus(&app.loading.focus)
            .w(px(420.))
            .max_w(gpui_kit::gpui::relative(0.5))
            .min_w(px(0.))
            .h_full()
            .border_l_1()
            .border_color(cx.theme().border)
            .p_3()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().child("Choose archive files"))
                    .child(
                        Button::new("archive-picker-hide")
                            .ghost()
                            .icon(IconName::X)
                            .accessibility_label("Hide picker")
                            .tooltip("Hide picker")
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.loading.selected_picker = None;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .truncate()
                    .text_color(cx.theme().muted_foreground)
                    .child(op.path.display().to_string()),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("archive-select-all")
                            .label("Select all")
                            .on_click(
                                cx.listener(move |app, _, _, cx| app.picker_all(id, true, cx)),
                            ),
                    )
                    .child(
                        Button::new("archive-clear").label("Clear").on_click(
                            cx.listener(move |app, _, _, cx| app.picker_all(id, false, cx)),
                        ),
                    ),
            )
            .child(div().flex_1().min_h(px(0.)).child(list))
            .child(div().text_color(cx.theme().muted_foreground).child(format!(
                "{selected} selected · Space selects · arrows navigate"
            )))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("archive-extract-open")
                            .label("Extract and open")
                            .disabled(selected == 0)
                            .on_click(cx.listener(move |app, _, _, cx| app.picker_open(id, cx))),
                    )
                    .child(
                        Button::new("archive-extract-merge")
                            .label("Extract and merge")
                            .disabled(selected < 2)
                            .on_click(cx.listener(move |app, _, _, cx| app.merge_archive(id, cx))),
                    )
                    .child(
                        Button::new("archive-cancel")
                            .label("Cancel")
                            .on_click(cx.listener(move |app, _, _, cx| app.cancel_loading(id, cx))),
                    ),
            )
            .into_any_element(),
    )
}
