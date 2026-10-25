//! Progress stays visible without preventing interaction with existing tabs.
use super::SelectionButton;
use crate::{
    gui::app::App,
    ingestion::archive_tree::{CheckState, NodeKind},
};
use gpui_kit::{
    assets::IconName,
    base::{Disableable, StyledExt, TestSupportExt},
    component::{
        ActiveTheme, Icon, Sizable,
        button::{Button, ButtonVariants},
        collapsible::Collapsible,
        h_resizable,
        progress::Progress,
        resizable_panel,
        switch::Switch,
    },
    gpui::{AnyElement, Context, MouseButton, div, prelude::*, px},
};

pub fn operations(app: &App, cx: &mut Context<App>) -> Option<AnyElement> {
    if app.loading.operations.is_empty() {
        return None;
    }
    let details = div()
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
                            if choosing {
                                "Choose files"
                            } else {
                                progress.phase
                            }
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
                        .when(op.shows_progress(), |el| {
                            el.child(
                                div().id(("load-progress", id)).test_support().child(
                                    Progress::new(("load-progress-value", id))
                                        .accessibility_label(format!("{} progress", progress.phase))
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
                                app.show_archive_picker(id, cx);
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
        .into_any_element();
    let first = &app.loading.operations[0];
    let active = app.loading.operations.iter().find(|op| op.shows_progress());
    let count = app.loading.operations.len();
    let summary = format!(
        "{count} background {} · {}",
        if count == 1 { "task" } else { "tasks" },
        if let Some(active) = active {
            active.progress.phase
        } else if first.picker.is_some() {
            "Choose files"
        } else {
            first.progress.phase
        }
    );
    Some(
        Collapsible::new()
            .open(app.loading_details_open)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_1()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        Button::new("loading-details-toggle")
                            .ghost()
                            .small()
                            .label(summary)
                            .selected_accent(app.loading_details_open)
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.loading_details_open = !app.loading_details_open;
                                cx.notify();
                            })),
                    )
                    .when_some(active, |row, active| {
                        row.child(
                            div()
                                .id("loading-summary-progress")
                                .test_support()
                                .flex_1()
                                .child(
                                    Progress::new("loading-summary-progress-value")
                                        .accessibility_label("Background task progress")
                                        .loading(active.progress.fraction.is_none())
                                        .value(
                                            active.progress.fraction.unwrap_or(0.) as f32 * 100.,
                                        ),
                                ),
                        )
                    }),
            )
            .content(details)
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
                                .accessibility_label(format!("Expand or collapse {}", item.name))
                                .tooltip("Expand or collapse archive")
                                .on_click(cx.listener(move |app, _, _, cx| {
                                    app.picker_expand(id, node, cx)
                                })),
                        )
                        .child(
                            gpui_kit::base::Checkbox::new(("archive-check", node))
                                .checked(check == CheckState::Checked)
                                .indeterminate(check == CheckState::Partial)
                                .disabled(lazy || error.is_some())
                                .accessibility_label(item.name.clone())
                                .flex()
                                .items_center()
                                .gap_2()
                                .styles(|styles| styles.disabled(|style| style.opacity(0.5)))
                                .child(
                                    div()
                                        .size(px(16.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .border_1()
                                        .border_color(cx.theme().border)
                                        .rounded(px(3.))
                                        .when(check != CheckState::Unchecked, |box_| {
                                            box_.bg(cx.theme().primary)
                                                .text_color(cx.theme().primary_foreground)
                                                .child(
                                                    Icon::new(if check == CheckState::Partial {
                                                        IconName::Minus
                                                    } else {
                                                        IconName::Check
                                                    })
                                                    .size(px(12.)),
                                                )
                                        }),
                                )
                                .child(item.name.clone())
                                .on_change({
                                    let entity = cx.entity();
                                    move |_, _, _, cx| {
                                        entity.update(cx, |app, cx| app.picker_toggle(id, node, cx))
                                    }
                                }),
                        )
                        .when_some(error, |el, error| {
                            el.child(div().truncate().text_color(cx.theme().danger).child(error))
                        })
                })
                .collect::<Vec<_>>()
        },
    )
    .track_scroll(&app.loading.scroll);
    let groups = if app.state.preferences.archive_merge_rotations {
        crate::shared::archive_selection::rotation_groups(&picker.tree)
    } else {
        Vec::new()
    };
    let grouped: std::collections::HashSet<_> = groups
        .iter()
        .flat_map(|group| group.members.iter().copied())
        .collect();
    let mut preview = Vec::new();
    for group in groups {
        preview.push((None, group.relative, false));
        for node in group.members {
            preview.push((
                Some(node),
                crate::shared::archive_selection::entry_path(&picker.tree, node),
                true,
            ));
        }
    }
    preview.extend(
        picker
            .selected_rows
            .iter()
            .filter(|node| !grouped.contains(node))
            .map(|&node| {
                (
                    Some(node),
                    crate::shared::archive_selection::entry_path(&picker.tree, node),
                    false,
                )
            }),
    );
    let selected_list = div()
        .id(("archive-selected-files", id))
        .size_full()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .children(preview.into_iter().map(|(node, path, child)| {
            div()
                .id(node
                    .map(|node| gpui_kit::gpui::ElementId::from(("archive-selected-row", node)))
                    .unwrap_or_else(|| format!("archive-preview-row:{path}").into()))
                .test_support()
                .w_full()
                .min_w(px(0.))
                .min_h(px(32.))
                .flex_shrink_0()
                .flex()
                .items_start()
                .gap_2()
                .py_1()
                .when(child, |d| d.pl_4())
                .when(node.is_none(), |d| d.font_semibold())
                .child(
                    div()
                        .id(format!(
                            "archive-preview-{}:{path}",
                            if node.is_none() { "output" } else { "file" }
                        ))
                        .test_support()
                        .flex_1()
                        .min_w(px(0.))
                        .whitespace_normal()
                        .child(path.clone()),
                )
                .when_some(node, |d, node| {
                    d.child(
                        Button::new(("archive-remove-file", node))
                            .ghost()
                            .small()
                            .flex_shrink_0()
                            .icon(IconName::X)
                            .accessibility_label(format!("Remove {path}"))
                            .on_click(
                                cx.listener(move |app, _, _, cx| app.picker_toggle(id, node, cx)),
                            ),
                    )
                })
        }));
    let files = div()
        .h_full()
        .flex()
        .flex_col()
        .min_w(px(0.))
        .gap_2()
        .pr_3()
        .child(div().font_semibold().child("All files"))
        .children(app.archive_search.as_ref().map(|input| {
            super::input(input)
                .id("archive-search")
                .aria_label("Search filenames")
                .cleanable(true)
        }))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Includes all nested archives. Search does not change selection."),
        )
        .child(
            div()
                .flex()
                .gap_2()
                .child(
                    Button::new("archive-select-all")
                        .small()
                        .label("Select all")
                        .on_click(cx.listener(move |app, _, _, cx| app.picker_all(id, true, cx))),
                )
                .child(
                    Button::new("archive-clear")
                        .small()
                        .label("Clear selection")
                        .on_click(cx.listener(move |app, _, _, cx| app.picker_all(id, false, cx))),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .when(picker.rows.is_empty(), |d| {
                    d.child("No matching files. Try another search.")
                })
                .child(list),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Space selects · arrows navigate · Enter opens"),
        );
    let patterns = div().flex_1().min_h(px(0.)).flex().flex_col().gap_2()
        .child(div().font_semibold().child("Saved glob patterns"))
        .child(div().text_xs().text_color(cx.theme().muted_foreground)
            .child("Reused for every archive. Adding or removing a pattern reselects matching files."))
        .children(app.archive_pattern.as_ref().map(|input| super::input(input)
            .id("archive-pattern-input").aria_label("Glob pattern")))
        .child(Button::new("archive-pattern-add").small().label("Add pattern")
            .on_click(cx.listener(move |app, _, window, cx| {
                if let Some(input) = app.archive_pattern.clone() {
                    let value = input.read(cx).value().to_string();
                    if app.picker_add_pattern(id, value, cx) {
                        input.update(cx, |input, cx| input.set_value("", window, cx));
                    }
                }
            })))
        .when_some(picker.pattern_error.clone(), |d, error| d.child(div().text_sm().text_color(cx.theme().danger).child(error)))
        .child(div().id("archive-patterns").flex_1().min_h(px(0.)).overflow_y_scroll()
            .when(app.state.preferences.archive_patterns.is_empty(), |d| d.child(div().text_sm()
                .text_color(cx.theme().muted_foreground).child("No saved patterns. Select files manually or add a glob.")))
            .children(app.state.preferences.archive_patterns.iter().enumerate().map(|(index, pattern)| {
                div().flex().items_center().gap_2().py_1()
                    .child(div().flex_1().min_w(px(0.)).truncate().child(pattern.clone()))
                    .child(Button::new(("archive-pattern-remove", index)).ghost().small().icon(IconName::X)
                        .accessibility_label(format!("Remove pattern {pattern}"))
                        .on_click(cx.listener(move |app, _, _, cx| app.picker_remove_pattern(index, cx))))
            })));
    let selection =
        div()
            .h_full()
            .min_w(px(0.))
            .pl_3()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex_1()
                    .min_h(px(80.))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .font_semibold()
                            .child(format!("Selected files ({selected})")),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h(px(0.))
                            .when(selected == 0, |d| {
                                d.child(div().text_sm().text_color(cx.theme().muted_foreground)
                    .child("Choose files on the left, or add a glob pattern below."))
                            })
                            .child(selected_list),
                    ),
            )
            .child(
                div()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .pt_3()
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .flex_col()
                    .child(patterns),
            );
    let merge_rotations = app.state.preferences.archive_merge_rotations;
    let groups = if merge_rotations {
        crate::shared::archive_selection::rotation_groups(&picker.tree)
    } else {
        Vec::new()
    };
    let output_count = selected - groups.iter().map(|g| g.members.len() - 1).sum::<usize>();
    Some(div().id("archive-picker").test_support().track_focus(&app.loading.focus)
        .w_full()
        .min_w(px(0.)).h_full().bg(cx.theme().background).border_l_1().border_color(cx.theme().border)
        .p_4().flex().flex_col().gap_3()
        .child(div().flex().items_center().gap_2()
            .child(div().flex_1().font_semibold().child("Open files from archive"))
            .child(Button::new("archive-picker-hide").ghost().icon(IconName::X)
                .accessibility_label("Hide picker").tooltip("Hide picker")
                .on_click(cx.listener(|app, _, _, cx| { app.loading.selected_picker = None; cx.notify(); }))))
        .child(div().truncate().text_sm().text_color(cx.theme().muted_foreground).child(op.path.display().to_string()))
        .child(div().flex_1().min_h(px(0.)).child(h_resizable(("archive-panes", id))
            .child(resizable_panel().size(px(460.)).size_range(px(220.)..px(1000.)).child(files))
            .child(resizable_panel().size(px(440.)).size_range(px(220.)..px(1000.)).child(selection))))
        .child(div().border_t_1().border_color(cx.theme().border).pt_3().flex().flex_col().gap_2()
            .child(Switch::new("archive-merge-rotations").label("Auto merge rotated logs").checked(merge_rotations)
                .on_click(cx.listener(|app, _, _, cx| {
                    let mut prefs = app.state.preferences.clone();
                    prefs.archive_merge_rotations = !prefs.archive_merge_rotations;
                    app.save_preferences(prefs); cx.notify();
                })))
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child(if merge_rotations {
                format!("{selected} selected → {output_count} outputs. Each rotation family opens as one saved log; other files open individually.")
            } else { "runtime.log, runtime.log1, runtime.log2 → runtime-merged.log. Each log family stays separate.".into() })))
        .child(div().flex().items_center().gap_2()
            .child(div().flex_1().min_w(px(0.)).flex().flex_col()
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Output folder"))
                .child(div().id("archive-output-directory").test_support().truncate().text_sm().child(picker.output_dir.display().to_string())))
            .child(Button::new("archive-output-choose").small().label("Choose folder…")
                .on_click(cx.listener(move |app, _, _, cx| app.picker_choose_output(id, cx)))))
        .child(div().flex().flex_wrap().gap_2()
            .child(Button::new("archive-extract-open").primary().label(format!("Open {output_count} {}", if output_count == 1 { "file" } else { "files" }))
                .disabled(selected == 0).on_click(cx.listener(move |app, _, _, cx| app.picker_open(id, cx))))
            .child(Button::new("archive-cancel").label("Cancel")
                .on_click(cx.listener(move |app, _, _, cx| app.cancel_loading(id, cx)))))
        .into_any_element())
}
