use crate::commands::Commands;
use crate::filters::FilterType;
use crate::gui::{
    actions::{Action, CATALOG, Editor},
    app::App,
    message::Message,
};
use gpui_kit::assets::IconName;
use gpui_kit::base::{Disableable, FocusTrapElement, Selectable, TestSupportExt};
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::color_picker::ColorPicker;
use gpui_kit::component::date_picker::DatePicker;
use gpui_kit::component::description_list::DescriptionList;
use gpui_kit::component::group_box::GroupBoxVariant;
use gpui_kit::component::input::Textarea;
use gpui_kit::component::list::ListItem;
use gpui_kit::component::select::Select;
use gpui_kit::component::setting::{SettingGroup, SettingItem, SettingPage, Settings};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::time_field::TimeField;
use gpui_kit::component::{ActiveTheme, Sizable};
use gpui_kit::gpui::{
    AnyElement, ClipboardItem, Context, Empty, Entity, MouseButton, Pixels, Size, Window, anchored,
    deferred, div, point, prelude::*, px,
};

#[derive(Clone)]
struct ColumnDrag {
    epoch: u64,
    name: String,
}

fn drop_column(
    app: &mut App,
    drag: &ColumnDrag,
    target: &str,
    window: &mut Window,
    cx: &mut Context<App>,
) {
    let Some(editor) = app
        .state
        .editor
        .as_ref()
        .filter(|editor| editor.action == Action::Columns && editor.epoch == drag.epoch)
    else {
        return;
    };
    let Some(mut from) = editor
        .fields
        .iter()
        .position(|(name, _)| name == &drag.name)
    else {
        return;
    };
    let Some(to) = editor.fields.iter().position(|(name, _)| name == target) else {
        return;
    };
    while from != to {
        let delta = if from < to { 1 } else { -1 };
        app.dispatch(Message::EditorMove(from, delta), window, cx);
        from = (from as isize + delta) as usize;
    }
}

/// Chunks `keyboard::editable_bindings`'s "group.field" flat list into
/// contiguous per-group runs of indices, for display as [`SettingGroup`]s —
/// entries are already emitted group-by-group by that function, so a
/// changed prefix always starts a new run.
fn chunk_by_group(fields: &[(String, bool)]) -> Vec<(&str, Vec<usize>)> {
    let mut groups: Vec<(&str, Vec<usize>)> = Vec::new();
    for (index, (name, _)) in fields.iter().enumerate() {
        let group = name.split_once('.').map(|(g, _)| g).unwrap_or(name);
        match groups.last_mut() {
            Some((last_group, indices)) if *last_group == group => indices.push(index),
            _ => groups.push((group, vec![index])),
        }
    }
    groups
}

/// "docker_select" -> "Docker Select"; a couple of acronyms are
/// special-cased since title-casing alone renders them as "Ui"/"Dlt".
fn humanize(key: &str) -> String {
    key.split('_')
        .map(|word| match word {
            "ui" => "UI".to_string(),
            "dlt" => "DLT".to_string(),
            _ => {
                let mut chars = word.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    None => String::new(),
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// One capture-binding row (label + current/"Unbound"/"Press a key…"
/// button) as a [`SettingItem`], wired through `entity` instead of
/// `cx.listener` since `SettingItem::render`'s closure only hands back a
/// plain `&mut App`.
fn keybinding_item(entity: &Entity<App>, app: &App, e: &Editor, index: usize) -> SettingItem {
    let name = e.fields[index]
        .0
        .split_once('.')
        .map_or(e.fields[index].0.as_str(), |(_, field)| field);
    let label = humanize(name);
    let item_label = label.clone();
    let capturing = app.state.binding_capture == Some(index);
    let value = e.values[index].clone();
    let entity = entity.clone();
    SettingItem::render(move |_, _, _| {
        let entity = entity.clone();
        div()
            .flex()
            .justify_between()
            .items_center()
            .gap_3()
            .child(item_label.clone())
            .child(
                Button::new(("capture-binding", index))
                    .label(if capturing {
                        "Press a key…".to_string()
                    } else if value.is_empty() {
                        "Unbound".into()
                    } else {
                        value.clone()
                    })
                    .on_click(move |_, window, cx| {
                        entity.update(cx, |app, cx| {
                            app.state.binding_capture = Some(index);
                            app.interface_focus.focus(window, cx);
                            cx.notify();
                        });
                    }),
            )
    })
    .keywords([label])
}

/// One restore-policy row (title + always/ask/never button group).
fn restore_policy_row(
    app: &App,
    e: &Editor,
    index: usize,
    title: &str,
    key: crate::db::SettingsKey,
    cx: &mut Context<App>,
) -> impl IntoElement {
    let locked = app.state.preferences.locked.contains(&key);
    let mut choices = ButtonGroup::new(("restore-policy-group", index)).outline();
    for (choice, value) in ["always", "ask", "never"].into_iter().enumerate() {
        choices = choices.child(
            Button::new(("restore-policy", index * 3 + choice))
                .label(value)
                .selected(e.values[index] == value)
                .disabled(locked)
                .on_click(cx.listener(move |app, _, window, cx| {
                    app.dispatch(Message::EditorText(index, value.into()), window, cx)
                })),
        );
    }
    let mut row = div().flex().flex_col().gap_1().child(title.to_string());
    row = row.child(choices);
    if locked {
        row = row.child("Set in config.json");
    }
    row
}

fn text(app: &App, index: usize, label: &str, _cx: &mut Context<App>) -> AnyElement {
    if let Some(input) = app.editor_textareas.get(&index) {
        return div()
            .flex()
            .flex_col()
            .gap_1()
            .child(label.to_string())
            .child(Textarea::new(input).aria_label(label.to_string()))
            .into_any_element();
    }
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(label.to_string())
        .children(app.editor_inputs.get(index).map(|input| {
            super::input(input)
                .aria_label(label.to_string())
                .id(("editor-input", index))
                .disabled(
                    (app.state
                        .editor
                        .as_ref()
                        .is_some_and(|e| e.action == Action::Settings)
                        && index == 3
                        && app
                            .state
                            .preferences
                            .locked
                            .contains(&crate::db::SettingsKey::McpPort))
                        || index == 0
                            && app
                                .state
                                .editor
                                .as_ref()
                                .is_some_and(|e| e.group_name_locked),
                )
        }))
        .into_any_element()
}
/// Splits a `list_docker` label (`"name · image · status"`) back into its
/// container name and status, for display as separate elements.
fn docker_name_and_status(label: &str) -> (&str, &str) {
    let mut parts = label.splitn(3, " · ");
    let name = parts.next().unwrap_or(label);
    let status = parts.nth(1).unwrap_or("");
    (name, status)
}
fn button(id: &str, label: &str, message: fn() -> Message, cx: &mut Context<App>) -> Button {
    Button::new(id.to_string())
        .label(label.to_string())
        .on_click(cx.listener(move |app, _, window, cx| app.dispatch(message(), window, cx)))
}
fn timestamp_editor(app: &App, cx: &mut Context<App>) -> AnyElement {
    let pattern = &app.state.editor.as_ref().unwrap().values[0];
    let (operator, start, end) = crate::gui::time_range::timestamp_parts(pattern);
    let time_only = start.contains(':') && !start.contains('-') && !start.contains(' ');
    let mut operations = ButtonGroup::new("timestamp-operators")
        .outline()
        .small()
        .flex_wrap();
    for (index, (value, label)) in [
        (">", "After"),
        (">=", "On or after"),
        ("<", "Before"),
        ("<=", "On or before"),
        ("=", "Equals"),
        ("..", "Between"),
    ]
    .into_iter()
    .enumerate()
    {
        operations = operations.child(
            Button::new(("timestamp-operator", index))
                .small()
                .selected(operator == value)
                .label(label)
                .on_click(cx.listener(move |app, _, window, cx| {
                    app.dispatch(Message::EditorTimestampPart(0, value.into()), window, cx);
                })),
        );
    }
    let mut bounds = div().flex().flex_wrap().gap_3();
    for index in 0..if operator == ".." { 2 } else { 1 } {
        let mut bound = div().flex().flex_col().gap_1().child(if operator == ".." {
            if index == 0 {
                "From (inclusive)"
            } else {
                "To (inclusive)"
            }
        } else {
            "Date and time"
        });
        if time_only {
            bound = bound.child(
                div()
                    .id(("timestamp-time", index))
                    .test_support()
                    .child(TimeField::new(&app.editor_times[index])),
            );
        } else {
            bound = bound.child(
                div().id(("timestamp-date", index)).test_support().child(
                    DatePicker::new(&app.editor_dates[index])
                        .placeholder("Select date and time")
                        .cleanable(true),
                ),
            );
        }
        bounds = bounds.child(bound);
    }
    let mut section = div()
        .flex()
        .flex_col()
        .gap_3()
        .child(if time_only {
            "Clock-time condition"
        } else {
            "Date and time condition"
        })
        .child(
            div()
                .id("timestamp-comparison")
                .test_support()
                .child(operations),
        )
        .child(bounds)
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(if time_only {
                    "Match the time of day across all dates."
                } else {
                    "Choose a date, then set the time in the picker."
                }),
        );
    // Keep non-ISO expressions (such as yearless syslog dates) editable without rewriting them.
    if [start, end]
        .iter()
        .any(|bound| !bound.is_empty() && !time_only && !bound.as_bytes()[0].is_ascii_digit())
    {
        section = section.child(text(app, 0, "Timestamp expression", cx));
    }
    section.into_any_element()
}

fn dlt_editor(app: &App, width: f32, height: f32, cx: &mut Context<App>) -> AnyElement {
    let e = app.state.editor.as_ref().unwrap();
    let mut devices = div()
        .id("dlt-saved-device-list")
        .test_support()
        .flex()
        .flex_col()
        .gap_1()
        .max_h(px((height - 240.).clamp(120., 360.)))
        .overflow_y_scroll();
    let query = e.values[2].trim().to_lowercase();
    let mut count = 0;
    for (index, label, _) in &e.members {
        if let Some((host, port)) = e.conditions.get(*index) {
            if !format!("{label} {host} {port}")
                .to_lowercase()
                .contains(&query)
            {
                continue;
            }
            count += 1;
            let name = label.clone();
            let host = host.clone();
            let port = port.clone();
            devices = devices.child(
                ListItem::new(("dlt-device", *index))
                    .selected(e.values[0] == host && e.values[1] == port && e.values[3] == *label)
                    .child(
                        div().flex().flex_col().child(label.clone()).child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("{host}:{port}")),
                        ),
                    )
                    .on_click(cx.listener(move |app, _, window, cx| {
                        app.dispatch(Message::EditorText(0, host.clone()), window, cx);
                        app.dispatch(Message::EditorText(1, port.clone()), window, cx);
                        app.dispatch(Message::EditorText(3, name.clone()), window, cx);
                    })),
            );
        }
    }
    if count == 0 {
        devices = devices.child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(if e.members.is_empty() {
                    "No saved devices yet. Enter details on the right to save one."
                } else {
                    "No matching saved devices."
                }),
        );
    }
    let details = div().id("dlt-device-details").test_support()
                .flex().flex_col().flex_1().min_w(px(0.)).gap_3()
                .child(div().font_weight(gpui_kit::gpui::FontWeight::SEMIBOLD).child("Connection details"))
                .child(text(app, 3, "Device name", cx))
                .child(text(app, 0, "Device host", cx))
                .child(text(app, 1, "TCP port", cx))
                .child(div().text_sm().text_color(cx.theme().muted_foreground)
                    .child("Select a saved device to fill these fields. Edit them before connecting, or save a new name."))
                .child(
                    Button::new("dlt-save-device")
                        .label(if e.dlt_saving {
                            "Saving…"
                        } else {
                            "Save device"
                        })
                        .disabled(e.dlt_saving)
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.dispatch(Message::SaveDltDevice, window, cx)
                        })),
                ).into_any_element();
    let saved = div()
        .id("dlt-saved-devices")
        .test_support()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .w(px((width * 0.36).min(260.)))
        .min_w(px(0.))
        .gap_3()
        .pr_3()
        .border_r_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .font_weight(gpui_kit::gpui::FontWeight::SEMIBOLD)
                .child("Saved devices"),
        )
        .child(text(app, 2, "Search devices", cx))
        .child(devices)
        .into_any_element();
    div()
        .flex()
        .items_start()
        .gap_4()
        .min_w(px(0.))
        .child(saved)
        .child(details)
        .into_any_element()
}

pub fn editor(app: &App, viewport: Size<Pixels>, cx: &mut Context<App>) -> Option<AnyElement> {
    let e = app.state.editor.as_ref()?;
    let width = (f32::from(viewport.width) - 48.).clamp(
        0.,
        if e.action == Action::Filter {
            520.
        } else if matches!(e.action, Action::Settings | Action::Keybindings) {
            960.
        } else {
            720.
        },
    );
    let height = (f32::from(viewport.height) * 0.8).min((f32::from(viewport.height) - 48.).max(0.));
    let header = div()
        .flex()
        .justify_between()
        .items_center()
        .when(!app.state.editor_history.is_empty(), |header| {
            header.child(button("editor-back", "Back", || Message::EditorBack, cx))
        })
        .child(if e.action == Action::Filter && e.edit_id.is_some() {
            "Edit filter"
        } else {
            CATALOG.iter().find(|d| d.action == e.action).unwrap().label
        })
        .when(e.action == Action::Filter, |d| {
            d.pb_3()
                .border_b_1()
                .border_color(cx.theme().border)
                .font_weight(gpui_kit::gpui::FontWeight::SEMIBOLD)
        })
        .child(button(
            "editor-close",
            "Close",
            || Message::EditorCancel,
            cx,
        ));
    let mut body = div().flex().flex_col().flex_shrink_0().gap_3();
    match e.action {
        Action::Keybindings => {
            let entity = cx.entity();
            let groups = chunk_by_group(&e.fields)
                .into_iter()
                .map(|(group, indices)| {
                    SettingGroup::new().title(humanize(group)).items(
                        indices
                            .into_iter()
                            .map(|index| keybinding_item(&entity, app, e, index))
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>();
            body = body
                .child("Click a shortcut, then press its replacement. Escape cancels capture.")
                .child(
                    div().h(px((height - 180.).max(120.))).child(
                        Settings::new("keybindings-settings")
                            .with_group_variant(GroupBoxVariant::Outline)
                            .page(
                                SettingPage::new("Keyboard Shortcuts")
                                    .resettable(false)
                                    .groups(groups),
                            ),
                    ),
                );
        }
        Action::Settings => {
            use crate::db::SettingsKey;
            let entity = cx.entity();
            let mut general = SettingGroup::new().title("Session");
            for (index, label, key) in [
                (0, "Restore previous session", SettingsKey::RestoreSession),
                (
                    1,
                    "Restore file position, bookmarks, and display options",
                    SettingsKey::RestoreFileContext,
                ),
            ] {
                let entity = entity.clone();
                general = general.item(
                    SettingItem::render(move |_, _, cx| {
                        entity.update(cx, |app, cx| {
                            let Some(editor) = app.state.editor.as_ref() else {
                                return div().into_any_element();
                            };
                            restore_policy_row(app, editor, index, label, key, cx)
                                .into_any_element()
                        })
                    })
                    .keywords([label]),
                );
            }
            let mut display = SettingGroup::new().title("New file defaults");
            for (index, (label, on)) in e.fields.iter().enumerate() {
                let locked = app
                    .state
                    .preferences
                    .locked
                    .contains(&crate::gui::persistence::DISPLAY_KEYS[index]);
                let label = if locked {
                    format!("{label} (set in config.json)")
                } else {
                    label.clone()
                };
                let keywords = label.clone();
                let on = *on;
                let entity = entity.clone();
                display = display.item(
                    SettingItem::render(move |_, _, _| {
                        let entity = entity.clone();
                        Checkbox::new(("setting-default", index))
                            .label(label.clone())
                            .checked(on)
                            .disabled(locked)
                            .on_click(move |_, window, cx| {
                                entity.update(cx, |app, cx| {
                                    app.dispatch(Message::EditorField(index), window, cx)
                                })
                            })
                    })
                    .keywords([keywords]),
                );
            }
            let port_entity = entity.clone();
            let mcp = SettingGroup::new()
                .title("MCP server")
                .description("Default port used when starting the server.")
                .item(
                    SettingItem::render(move |_, _, cx| {
                        port_entity.update(cx, |app, cx| text(app, 3, "Default MCP port", cx))
                    })
                    .keywords(["port", "MCP"]),
                );
            let mut customize = SettingGroup::new().title("Customize");
            for (id, label, action) in [
                ("settings-appearance", "Appearance", Action::Appearance),
                (
                    "settings-default-filters",
                    "Schema default filters",
                    Action::DefaultFilters,
                ),
                (
                    "settings-shortcuts",
                    "Keyboard shortcuts",
                    Action::Keybindings,
                ),
            ] {
                let entity = entity.clone();
                customize = customize.item(
                    SettingItem::render(move |_, _, _| {
                        let entity = entity.clone();
                        Button::new(id)
                            .ghost()
                            .label(label)
                            .on_click(move |_, window, cx| {
                                entity.update(cx, |app, cx| {
                                    app.dispatch(Message::EditorNavigate(action), window, cx)
                                })
                            })
                    })
                    .keywords([label]),
                );
            }
            body = body
                .child("Changes apply to future opens. Values set in config.json are read-only.")
                .child(
                    div().h(px((height - 180.).max(120.))).child(
                        Settings::new("preferences-settings")
                            .sidebar_width(px(180.))
                            .with_group_variant(GroupBoxVariant::Outline)
                            .page(SettingPage::new("General").resettable(false).group(general))
                            .page(SettingPage::new("Display").resettable(false).group(display))
                            .page(
                                SettingPage::new("Integrations")
                                    .resettable(false)
                                    .group(mcp),
                            )
                            .page(
                                SettingPage::new("Customize")
                                    .resettable(false)
                                    .group(customize),
                            ),
                    ),
                );
        }
        Action::Help => {
            body = body.child(text(app, 0, "Search shortcuts", cx));
            let query = e.values[0].trim().to_lowercase();
            let shortcuts = crate::gui::keyboard::shortcuts(&app.state);
            let mut found = false;
            for category in ["Navigate logs", "Files", "Investigate", "General"] {
                let mut rows = DescriptionList::new().columns(1).bordered(false);
                let mut has_rows = false;
                for (group, label, keys) in &shortcuts {
                    if *group == category
                        && (label.to_lowercase().contains(&query)
                            || keys.to_lowercase().contains(&query)
                            || category.to_lowercase().contains(&query))
                    {
                        rows = rows.item(label.clone(), keys.clone(), 1);
                        has_rows = true;
                    }
                }
                if has_rows {
                    found = true;
                    body = body
                        .child(
                            div()
                                .pt_3()
                                .font_weight(gpui_kit::gpui::FontWeight::SEMIBOLD)
                                .child(category),
                        )
                        .child(rows);
                }
            }
            if !found {
                body = body.child("No shortcuts match this search.");
            }
            body = body.child(button(
                "help-customize",
                "Customize shortcuts…",
                || Message::EditorNavigate(Action::Keybindings),
                cx,
            ));
        }
        Action::Merge => {
            body = body.child("Select at least two files. Lines without timestamps are omitted; closed sources keep their last content.")
                .child(text(app, 0, "Search open files by name or path", cx))
                .child(div().flex().gap_2()
                    .child(button("merge-all", "Select all", || Message::EditorAll(true), cx))
                    .child(button("merge-none", "Select none", || Message::EditorAll(false), cx)));
            for (index, (id, on)) in e.merge_sources.iter().enumerate() {
                let Some(tab) = app.state.tabs.iter().find(|tab| tab.id == *id) else {
                    continue;
                };
                let query = e.values[0].trim().to_lowercase();
                if !tab.title.to_lowercase().contains(&query)
                    && !tab
                        .display_source_path()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&query)
                {
                    continue;
                }
                let id = *id;
                let schema = crate::gui::merge::source_parser(tab)
                    .map(|parser| parser.name().to_string())
                    .unwrap_or_else(|| "No timestamp format — rows will be omitted".into());
                body = body.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .p_2()
                        .border_1()
                        .border_color(cx.theme().border)
                        .rounded(px(4.))
                        .child(
                            Checkbox::new(("merge-source", index))
                                .label(tab.title.clone())
                                .checked(*on)
                                .on_click(cx.listener(move |app, _, window, cx| {
                                    app.dispatch(Message::MergeSource(id), window, cx)
                                })),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(
                                    tab.display_source_path()
                                        .unwrap_or("Temporary source")
                                        .to_string(),
                                ),
                        )
                        .child(div().text_sm().child(format!(
                            "{} source rows · {schema}",
                            tab.file_reader.line_count()
                        ))),
                );
            }
            body = body.child(format!(
                "{} selected",
                e.merge_sources.iter().filter(|(_, on)| *on).count()
            ));
        }
        Action::SwitchFile => {
            body = body.child(text(app, 0, "Search open files by name or path", cx));
            let query = e.values[0].trim().to_lowercase();
            let mut count = 0;
            for (index, tab) in app.state.tabs.iter().enumerate() {
                if !tab.title.to_lowercase().contains(&query)
                    && !tab
                        .display_source_path()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&query)
                {
                    continue;
                }
                count += 1;
                let id = tab.id;
                body = body.child(
                    ListItem::new(("switch-file", index))
                        .selected(index == app.state.active_tab)
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .w_full()
                                .child(
                                    div()
                                        .flex()
                                        .justify_between()
                                        .child(tab.title.clone())
                                        .when(index == app.state.active_tab, |row| {
                                            row.child("Active")
                                        }),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(
                                            tab.display_source_path()
                                                .unwrap_or("Temporary source")
                                                .to_string(),
                                        ),
                                ),
                        )
                        .on_click(cx.listener(move |app, _, window, cx| {
                            if let Some(index) = app.state.tabs.iter().position(|tab| tab.id == id)
                            {
                                crate::gui::actions::close(&mut app.state, true);
                                app.dispatch(Message::TabSelected(index), window, cx);
                            }
                        })),
                );
            }
            if count == 0 {
                body = body.child("No open files match this name or path. Try a shorter search.");
            }
        }
        Action::EnableMcp => {
            body = body.child(text(app, 0, "MCP HTTP port", cx));
        }
        Action::DefaultFilters => {
            body = body
                .child(format!("Schema: {}", e.values[0]))
                .child(text(
                    app,
                    1,
                    "Default filter file (empty removes the mapping)",
                    cx,
                ))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(button(
                            "default-filter-browse",
                            "Choose file…",
                            || Message::BrowseDefaultMapping,
                            cx,
                        ))
                        .child(
                            Button::new("default-filter-clear")
                                .ghost()
                                .label("Clear mapping")
                                .on_click(cx.listener(|app, _, window, cx| {
                                    app.dispatch(Message::EditorText(1, String::new()), window, cx)
                                })),
                        ),
                )
                .child("Mappings in config.json take precedence over saved settings.")
                .child(text(app, 2, "Search schemas or filter paths", cx));
            let custom = crate::config::custom_schemas()
                .iter()
                .map(|s| s.name.clone())
                .collect::<Vec<_>>();
            let mut names = crate::commands::auto_complete::schema_completion_names(&custom)
                .into_iter()
                .map(|(name, _)| name)
                .filter(|name| name != "none")
                .collect::<Vec<_>>();
            names.extend(e.conditions.iter().map(|(name, _)| name.clone()));
            names.sort();
            names.dedup();
            let query = e.values[2].trim().to_lowercase();
            let mut count = 0;
            for (index, name) in names.into_iter().enumerate() {
                let path = e
                    .conditions
                    .iter()
                    .find(|(schema, _)| schema == &name)
                    .map(|(_, path)| path.as_str())
                    .unwrap_or("Not configured");
                if !name.to_lowercase().contains(&query) && !path.to_lowercase().contains(&query) {
                    continue;
                }
                count += 1;
                body = body.child(
                    ListItem::new(("default-schema", index))
                        .child(
                            div().flex().flex_col().child(name.clone()).child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(path.to_string()),
                            ),
                        )
                        .selected(e.values[0] == name)
                        .on_click(cx.listener(move |app, _, window, cx| {
                            app.dispatch(Message::EditorText(0, name.clone()), window, cx)
                        })),
                );
            }
            if count == 0 {
                body = body.child("No schemas or filter paths match this search.");
            }
        }
        Action::Docker => {
            body = body
                .child("Follow container logs, or enter a container name or ID directly.")
                .child(text(app, 0, "Find a container by name or ID", cx));
            let query = e.values[0].trim().to_lowercase();
            let mut count = 0;
            for (index, (id, label)) in e.conditions.iter().enumerate() {
                if !id.to_lowercase().contains(&query) && !label.to_lowercase().contains(&query) {
                    continue;
                }
                count += 1;
                let id = id.clone();
                let (name, status) = docker_name_and_status(label);
                body = body.child(
                    ListItem::new(("docker-container", index))
                        .selected(e.values[0] == id)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .child(
                                    div().flex().flex_col().child(name.to_string()).child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(id.clone()),
                                    ),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .flex_shrink_0()
                                        .child(div().w(px(8.)).h(px(8.)).rounded_full().bg(
                                            if status.starts_with("Up") {
                                                cx.theme().success
                                            } else {
                                                cx.theme().muted_foreground
                                            },
                                        ))
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(status.to_string()),
                                        ),
                                ),
                        )
                        .on_click(cx.listener(move |app, _, window, cx| {
                            app.dispatch(Message::EditorText(0, id.clone()), window, cx)
                        })),
                );
            }
            if count == 0 {
                body = body.child("No matching container suggestions. You can still connect using the name or ID above.");
            }
        }
        Action::Dlt => {
            body = body.child(dlt_editor(app, width, height, cx));
        }
        Action::Otel => {
            body = body
                .child("Receive OpenTelemetry logs. Choose the transport used by your sender.")
                .child(text(app, 0, "Listen port", cx));
            let mut protocols = ButtonGroup::new("otel-protocol").outline();
            for (protocol, label) in [("http", "HTTP"), ("grpc", "gRPC")] {
                protocols = protocols.child(
                    Button::new(format!("otel-{protocol}"))
                        .label(label)
                        .selected(e.values[1] == protocol)
                        .on_click(cx.listener(move |app, _, window, cx| {
                            app.dispatch(Message::EditorText(1, protocol.into()), window, cx)
                        })),
                );
            }
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child("Transport")
                    .child(protocols),
            );
        }
        Action::Run => {
            body = body
                .child("Capture the process output as a new log source.")
                .child(text(app, 0, "Executable", cx))
                .child(text(
                    app,
                    1,
                    "Arguments (one argument per line; spaces stay inside an argument)",
                    cx,
                ));
        }
        Action::Export => {
            body = body
                .child("Export an analysis document from this source’s notes and bookmarks.")
                .child(div().flex().flex_col().gap_1().child("Template").children(
                    app.editor_choice_select.as_ref().map(|state| {
                        Select::new(state)
                            .id("export-template-select")
                            .w_full()
                            .placeholder("Choose a template")
                            .search_placeholder("Search templates…")
                            .accessibility_label("Export template")
                    }),
                ));
            if let Some(tab) = e
                .source
                .and_then(|id| app.state.tabs.iter().find(|tab| tab.id == id))
            {
                body = body.child(
                    DescriptionList::new()
                        .columns(1)
                        .bordered(false)
                        .item("Source", tab.title.clone(), 1)
                        .item("Notes", tab.comment_manager.get().len().to_string(), 1)
                        .item(
                            "Bookmarked rows",
                            tab.mark_manager.indices().len().to_string(),
                            1,
                        ),
                );
            }
            if !e.fields.is_empty() {
                body = body.child(div().pt_2().child("Optional document fields"));
            }
            for (index, (key, _)) in e.fields.iter().enumerate() {
                body = body.child(text(app, index + 6, &humanize(key), cx));
            }
            body = body
                .child("Choose the destination file after clicking Export.")
                .child(super::export_preview::panel(app, cx));
        }
        Action::Annotation => {
            body = body
                .child(format!("Note attached to {} source rows", e.members.len()))
                .child(text(app, 0, "Annotation", cx));
        }
        Action::Filter => {
            let timestamp = e.values[0].starts_with(crate::filters::DATE_PREFIX);
            if !timestamp {
                let mut kinds = ButtonGroup::new("filter-behavior").outline().small();
                for (i, (kind, label)) in [
                    (FilterType::Include, "Include"),
                    (FilterType::Exclude, "Exclude"),
                    (FilterType::Highlight, "Highlight"),
                ]
                .into_iter()
                .enumerate()
                {
                    kinds = kinds.child(
                        Button::new(("filter-kind", i))
                            .label(label)
                            .selected(e.kind == kind)
                            .on_click(cx.listener(move |app, _, window, cx| {
                                app.dispatch(Message::EditorKind(kind.clone()), window, cx)
                            })),
                    );
                }
                body = body.child(kinds);
                body = body.child(div().flex().flex_col().gap_1().child("Match in").children(
                    app.editor_field_select.as_ref().map(|state| {
                        Select::new(state)
                            .id("filter-field-select")
                            .w_full()
                            .placeholder("All text")
                            .search_placeholder("Search fields…")
                            .accessibility_label("Match in")
                            .cleanable(true)
                    }),
                ));
                body = body.child(if e.values[1].is_empty() {
                    text(app, 0, "Contains", cx)
                } else {
                    text(app, 2, "Contains", cx)
                });
                if e.conditions.len() > 1
                    || (!e.conditions.is_empty() && e.condition_edit.is_none())
                {
                    body = body.child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!(
                                "{} additional field conditions",
                                e.conditions.len() - usize::from(e.condition_edit.is_some())
                            )),
                    );
                }
            } else {
                body = body.child(timestamp_editor(app, cx));
            }
            if e.kind != FilterType::Exclude || timestamp {
                let (color, background) = crate::gui::actions::automatic_filter_colors(
                    &e.style_key,
                    &app.state.theme,
                    e.style_seed,
                );
                body = body.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(
                            Button::new("filter-auto-style")
                                .small()
                                .selected(e.auto_style)
                                .label("Auto style")
                                .on_click(cx.listener(|app, _, window, cx| {
                                    app.dispatch(Message::EditorStyle(true), window, cx)
                                })),
                        )
                        .when(e.auto_style, |d| {
                            d.child(
                                Button::new("filter-regenerate-style")
                                    .ghost()
                                    .small()
                                    .label("Generate another")
                                    .on_click(cx.listener(|app, _, window, cx| {
                                        app.dispatch(Message::EditorRegenerateStyle, window, cx)
                                    })),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .px_2()
                                    .py_1()
                                    .when_some(
                                        crate::gui::color::color_to_gpui(background),
                                        |d, bg| d.bg(bg),
                                    )
                                    .when_some(
                                        crate::gui::color::color_to_gpui(color),
                                        |d, color| d.text_color(color),
                                    )
                                    .child("Matched log text"),
                            )
                        })
                        .child(
                            Button::new("filter-custom-style")
                                .ghost()
                                .small()
                                .label("Custom…")
                                .on_click(cx.listener(|app, _, window, cx| {
                                    app.dispatch(Message::EditorStyle(false), window, cx)
                                })),
                        ),
                );
            }
            body = body.child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(
                        Button::new("filter-advanced")
                            .ghost()
                            .small()
                            .icon(if e.advanced {
                                IconName::ChevronUp
                            } else {
                                IconName::ChevronDown
                            })
                            .label(
                                if e.regex
                                    || e.ignore_case
                                    || !e.values[3].is_empty()
                                    || e.conditions.len() > 1
                                {
                                    "Advanced · active"
                                } else {
                                    "Advanced"
                                },
                            )
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.dispatch(Message::EditorAdvanced, window, cx)
                            })),
                    )
                    .child(
                        Button::new(if timestamp {
                            "filter-target-text"
                        } else {
                            "filter-target-timestamp"
                        })
                        .ghost()
                        .small()
                        .label(if timestamp {
                            "Use text filter"
                        } else {
                            "Use timestamp"
                        })
                        .on_click(cx.listener(
                            move |app, _, window, cx| {
                                app.dispatch(
                                    Message::EditorText(
                                        0,
                                        if timestamp {
                                            String::new()
                                        } else {
                                            format!("{}>= ", crate::filters::DATE_PREFIX)
                                        },
                                    ),
                                    window,
                                    cx,
                                )
                            },
                        )),
                    ),
            );
            if e.advanced {
                let mut advanced = div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .pt_3()
                    .border_t_1()
                    .border_color(cx.theme().border);
                if !timestamp {
                    if !e.values[1].is_empty() {
                        advanced =
                            advanced.child(text(app, 0, "Also contains text (optional)", cx));
                    }
                    let mut options = div().flex().flex_wrap().gap_4();
                    for (i, (label, on)) in [
                        ("Regular expression", e.regex),
                        ("Ignore case", e.ignore_case),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        options = options.child(
                            Switch::new(("filter-option", i))
                                .small()
                                .label(label)
                                .checked(on)
                                .disabled(!e.values[1].is_empty() || !e.conditions.is_empty())
                                .on_click(cx.listener(move |app, _, window, cx| {
                                    app.dispatch(Message::EditorToggle(i), window, cx)
                                })),
                        );
                    }
                    advanced = advanced.child(options);
                    if !e.values[1].is_empty() {
                        advanced = advanced.child(
                            button(
                                "condition-add",
                                "Keep condition and add another",
                                || Message::EditorConditionAdd,
                                cx,
                            )
                            .small()
                            .outline()
                            .icon(IconName::Plus)
                            .disabled(e.values[2].is_empty()),
                        );
                    }
                    for (i, (field, value)) in e.conditions.iter().enumerate() {
                        advanced = advanced.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.))
                                        .text_sm()
                                        .child(format!("{field} contains {value}")),
                                )
                                .child(
                                    Button::new(("condition-edit", i))
                                        .ghost()
                                        .small()
                                        .icon(IconName::Pencil)
                                        .accessibility_label("Edit condition")
                                        .tooltip("Edit condition")
                                        .on_click(cx.listener(move |app, _, window, cx| {
                                            app.dispatch(
                                                Message::EditorConditionEdit(i),
                                                window,
                                                cx,
                                            )
                                        })),
                                )
                                .child(
                                    Button::new(("condition-remove", i))
                                        .ghost()
                                        .small()
                                        .icon(IconName::X)
                                        .accessibility_label("Remove condition")
                                        .tooltip("Remove condition")
                                        .on_click(cx.listener(move |app, _, window, cx| {
                                            app.dispatch(
                                                Message::EditorConditionRemove(i),
                                                window,
                                                cx,
                                            )
                                        })),
                                ),
                        );
                    }
                }
                advanced = advanced.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child("Group (optional)")
                        .children(app.editor_group_select.as_ref().map(|state| {
                            Select::new(state)
                                .id("filter-group-select")
                                .w_full()
                                .placeholder("No group")
                                .search_placeholder("Search groups…")
                                .accessibility_label("Group")
                                .cleanable(true)
                        })),
                );
                body = body.child(advanced);
            }
        }
        Action::Group => {
            body = body
                .child(text(app, 0, "Group name", cx))
                .child("Member filters");
            body = body.children(app.editor_members.as_ref().map(|members| {
                gpui_kit::component::combobox::Combobox::new(members)
                    .placeholder("Choose member filters")
                    .cleanable(true)
            }));
        }
        Action::Columns | Action::ValueColors | Action::LevelColors => {
            let columns = e.action == Action::Columns;
            if columns {
                let visible: Vec<_> = e.fields.iter().filter(|(_, on)| *on).collect();
                let mut preview = div().flex().flex_wrap().gap_1();
                for (name, _) in &visible {
                    preview = preview.child(
                        div()
                            .px_2()
                            .py_1()
                            .border_1()
                            .border_color(cx.theme().border)
                            .rounded(px(3.))
                            .child(name.clone()),
                    );
                }
                body = body.child(format!("{} of {} columns visible", visible.len(), e.fields.len()))
                    .child("Column order preview")
                    .child(preview)
                    .child("Move columns with the arrow controls. Bulk actions affect all columns, including those outside the search results.");
            }
            body = body.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(button(
                        "editor-all",
                        if columns {
                            "Show all columns"
                        } else {
                            "Enable all"
                        },
                        || Message::EditorAll(true),
                        cx,
                    ))
                    .child(button(
                        "editor-none",
                        if columns {
                            "Hide all columns"
                        } else {
                            "Disable all"
                        },
                        || Message::EditorAll(false),
                        cx,
                    ))
                    .child(button(
                        "editor-reset",
                        if columns {
                            "Reset columns"
                        } else {
                            "Restore initial choices"
                        },
                        || Message::EditorReset,
                        cx,
                    )),
            );
            if e.action == Action::Columns {
                body = body.child(text(app, 0, "Search columns", cx));
            }
            let samples: Vec<_> = if e.action == Action::ValueColors {
                app.state
                    .theme
                    .value_colors
                    .grouped_categories(app.state.theme.process_colors.first().copied())
                    .into_iter()
                    .flat_map(|group| {
                        group
                            .children
                            .into_iter()
                            .map(move |(key, label, color)| (key, label, color, group.label))
                    })
                    .collect()
            } else {
                Vec::new()
            };
            let mut last_group = "";
            if !columns {
                body = body.child("Checked entries use the theme’s color. Unchecked entries use ordinary log text color.");
            }
            for (i, (name, on)) in e.fields.iter().enumerate() {
                if e.action == Action::Columns
                    && !name.to_lowercase().contains(&e.values[0].to_lowercase())
                {
                    continue;
                }
                let mut row = div()
                    .id(("column-row", i))
                    .test_support()
                    .flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Checkbox::new(("editor-field", i))
                            .checked(*on)
                            .label(if columns {
                                name.clone()
                            } else {
                                humanize(name)
                            })
                            .on_click(cx.listener(move |app, _, window, cx| {
                                app.dispatch(Message::EditorField(i), window, cx)
                            })),
                    );
                if e.action == Action::Columns {
                    let target = name.clone();
                    row = row
                        .on_drop(cx.listener(move |app, drag: &ColumnDrag, window, cx| {
                            drop_column(app, drag, &target, window, cx);
                        }))
                        .drag_over::<ColumnDrag>(|style, _, _, cx| style.bg(cx.theme().secondary))
                        .child(
                            div()
                                .id(("column-drag", i))
                                .test_support()
                                .cursor_pointer()
                                .child(
                                    gpui_kit::component::Icon::new(IconName::GripVertical)
                                        .size(px(14.)),
                                )
                                .on_drag(
                                    ColumnDrag {
                                        epoch: e.epoch,
                                        name: name.clone(),
                                    },
                                    |_, _, _, cx| cx.new(|_| Empty),
                                ),
                        )
                        .child(
                            Button::new(("column-up", i))
                                .ghost()
                                .small()
                                .label("↑")
                                .accessibility_label(format!("Move {name} up"))
                                .disabled(i == 0)
                                .on_click(cx.listener(move |app, _, window, cx| {
                                    app.dispatch(Message::EditorMove(i, -1), window, cx)
                                })),
                        )
                        .child(
                            Button::new(("column-down", i))
                                .ghost()
                                .small()
                                .label("↓")
                                .accessibility_label(format!("Move {name} down"))
                                .disabled(i + 1 == e.fields.len())
                                .on_click(cx.listener(move |app, _, window, cx| {
                                    app.dispatch(Message::EditorMove(i, 1), window, cx)
                                })),
                        );
                }
                if !columns {
                    let sample = samples.iter().find(|(key, _, _, _)| *key == name);
                    let (label, color, category) = if let Some((_, label, color, category)) = sample
                    {
                        (*label, *color, *category)
                    } else {
                        let theme = &app.state.theme;
                        let color = match name.as_str() {
                            "trace" => theme.trace_fg,
                            "debug" => theme.debug_fg,
                            "info" => theme.info_fg,
                            "notice" => theme.notice_fg,
                            "warning" => theme.warning_fg,
                            "error" => theme.error_fg,
                            "fatal" => theme.fatal_fg,
                            _ => theme.text,
                        };
                        (name.as_str(), color, "Severity")
                    };
                    if category != last_group {
                        body = body.child(
                            div()
                                .pt_2()
                                .font_weight(gpui_kit::gpui::FontWeight::SEMIBOLD)
                                .child(category.to_string()),
                        );
                        last_group = category;
                    }
                    row = row.justify_between().child(
                        div()
                            .px_2()
                            .py_1()
                            .font_family("monospace")
                            .when_some(
                                crate::gui::color::color_to_gpui(app.state.theme.root_bg),
                                |d, color| d.bg(color),
                            )
                            .when_some(
                                crate::gui::color::color_to_gpui(if *on {
                                    color
                                } else {
                                    app.state.theme.text
                                }),
                                |d, color| d.text_color(color),
                            )
                            .child(label.to_string()),
                    );
                }
                body = body.child(row);
            }
        }
        Action::Appearance => {
            body = body.child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("appearance-values")
                            .ghost()
                            .label("Value colors")
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.dispatch(
                                    Message::EditorNavigate(Action::ValueColors),
                                    window,
                                    cx,
                                )
                            })),
                    )
                    .child(
                        Button::new("appearance-levels")
                            .ghost()
                            .label("Level colors")
                            .disabled(app.state.active_tab().is_none())
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.dispatch(
                                    Message::EditorNavigate(Action::LevelColors),
                                    window,
                                    cx,
                                )
                            })),
                    ),
            );
            body = body
                .child(div().child(
                    "Choose a preset to preview the application. Save theme keeps it; Cancel restores the previous theme.",
                ))
                .child(text(app, 1, "Search themes", cx));
            for (i, name) in crate::theme::Theme::list_available_themes()
                .into_iter()
                .filter(|n| n.to_lowercase().contains(&e.values[1].to_lowercase()))
                .enumerate()
            {
                let mut row = div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_2()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(cx.theme().border);
                if let Ok(theme) = crate::theme::Theme::from_file(format!("{name}.json")) {
                    let mut sample = div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .p_3()
                        .rounded(px(4.))
                        .font_family("monospace")
                        .text_sm();
                    if let Some(color) = crate::gui::color::color_to_gpui(theme.root_bg) {
                        sample = sample.bg(color);
                    }
                    if let Some(color) = crate::gui::color::color_to_gpui(theme.text) {
                        sample = sample.text_color(color);
                    }
                    for (level, color, message) in [
                        ("INFO", theme.info_fg, "Request completed in 24 ms"),
                        ("ERROR", theme.error_fg, "Connection timed out"),
                        ("DEBUG", theme.debug_fg, "Retry scheduled · attempt=2"),
                    ] {
                        let mut severity = div().w(px(52.)).flex_shrink_0().child(level);
                        if let Some(color) = crate::gui::color::color_to_gpui(color) {
                            severity = severity.text_color(color);
                        }
                        sample = sample.child(
                            div()
                                .flex()
                                .gap_2()
                                .child("12:04:32")
                                .child(severity)
                                .child(message),
                        );
                    }
                    row = row.child(sample);
                }
                let chosen = name.clone();
                row = row.child(
                    Button::new(("theme-choice", i))
                        .ghost()
                        .selected(e.values[0] == name)
                        .label(name)
                        .on_click(cx.listener(move |app, _, window, cx| {
                            app.dispatch(Message::EditorPreview(chosen.clone()), window, cx)
                        })),
                );
                body = body.child(row);
            }
        }
        Action::Schema => {
            body = body
                .child("Choose how log fields are parsed. Use ‘none’ for unstructured text.")
                .child(div().flex().flex_col().gap_1().child("Schema").children(
                    app.editor_choice_select.as_ref().map(|state| {
                        Select::new(state)
                            .id("schema-select")
                            .w_full()
                            .placeholder("Choose a schema")
                            .search_placeholder("Search built-in and custom schemas…")
                            .accessibility_label("Schema")
                    }),
                ));
            let kind = if e.values[0] == "none" {
                "Unstructured text"
            } else if crate::config::custom_schemas()
                .iter()
                .any(|s| s.name == e.values[0])
            {
                "Custom schema"
            } else {
                "Built-in schema"
            };
            body = body
                .child(kind)
                .child(super::schema_preview::panel(app, cx));
            if let Some(tab) = app.state.active_tab() {
                body = body.child(format!(
                    "Current schema: {}",
                    tab.display
                        .format
                        .as_ref()
                        .map(|f| f.name())
                        .unwrap_or("none")
                ));
            }
        }
        Action::FileInfo => {
            let path = e
                .source
                .and_then(|id| app.state.tabs.iter().find(|t| t.id == id))
                .and_then(|t| t.display_source_path())
                .unwrap_or("No source path")
                .to_string();
            let mut details =
                DescriptionList::new()
                    .columns(1)
                    .bordered(false)
                    .item("Source", path.clone(), 1);
            if let Some(tab) = e
                .source
                .and_then(|id| app.state.tabs.iter().find(|tab| tab.id == id))
            {
                details = details
                    .item(
                        "Schema",
                        tab.display
                            .format
                            .as_ref()
                            .map(|format| format.name())
                            .unwrap_or("Unstructured"),
                        1,
                    )
                    .item("Source rows", tab.file_reader.line_count().to_string(), 1)
                    .item(
                        "Visible rows",
                        tab.filter.visible_indices.len().to_string(),
                        1,
                    )
                    .item("Size", format!("{} bytes", tab.file_reader.total_len()), 1)
                    .item("Bookmarks", tab.mark_manager.indices().len().to_string(), 1)
                    .item("Notes", tab.comment_manager.get().len().to_string(), 1);
                if let Some(merged) = &tab.merged {
                    details =
                        details.item("Merged sources", merged.source_tab_ids.len().to_string(), 1);
                }
            }
            body =
                body.child(details)
                    .child(Button::new("copy-path").label("Copy path").on_click(
                        move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(path.clone()))
                        },
                    ));
        }
        Action::View => {
            body = body.child("Changes apply immediately to this file.");
            let mut navigation = div().flex().flex_wrap().gap_2();
            for (id, label, action) in [
                ("view-columns", "Columns", Action::Columns),
                ("view-schema", "Schema", Action::Schema),
                ("view-theme", "Appearance", Action::Appearance),
                ("view-values", "Value colors", Action::ValueColors),
                ("view-levels", "Level colors", Action::LevelColors),
            ] {
                navigation = navigation.child(Button::new(id).ghost().label(label).on_click(
                    cx.listener(move |app, _, window, cx| {
                        app.dispatch(Message::EditorNavigate(action), window, cx)
                    }),
                ));
            }
            body = body.child(navigation);
            if let Some(tab) = app.state.active_tab() {
                for (i, (label, on)) in [
                    ("Wrap lines", tab.display.wrap),
                    ("Line numbers", tab.display.show_line_numbers),
                    ("Relative line numbers", tab.display.relative_line_numbers),
                    ("Show field keys", tab.display.show_keys),
                    ("Raw display", tab.display.raw_mode),
                    ("Collapse continuations", tab.display.collapse_continuations),
                    ("Sidebar on left", tab.display.sidebar_side.is_left()),
                    ("Show sidebar", tab.display.show_sidebar),
                ]
                .into_iter()
                .enumerate()
                {
                    body = body.child(
                        Checkbox::new(("view-option", i))
                            .label(label)
                            .checked(on)
                            .on_click(cx.listener(move |app, _, window, cx| {
                                let command = match i {
                                    0 => Commands::Wrap,
                                    1 => Commands::LineNumbers,
                                    2 => Commands::RelativeLineNumbers,
                                    3 => {
                                        if on {
                                            Commands::HideKeys
                                        } else {
                                            Commands::ShowKeys
                                        }
                                    }
                                    4 => Commands::Raw,
                                    5 => {
                                        if on {
                                            Commands::Expand
                                        } else {
                                            Commands::Collapse
                                        }
                                    }
                                    6 => {
                                        app.dispatch(Message::SidebarSideToggled, window, cx);
                                        return;
                                    }
                                    _ => {
                                        app.dispatch(Message::SidebarToggled, window, cx);
                                        return;
                                    }
                                };
                                app.dispatch(
                                    Message::GuiCommand(command, label.into()),
                                    window,
                                    cx,
                                );
                            })),
                    );
                }
            }
        }
        _ => {}
    }
    if matches!(e.action, Action::Filter | Action::Group)
        && (e.kind != FilterType::Exclude || e.values[0].starts_with(crate::filters::DATE_PREFIX))
        && (e.action == Action::Group || (e.advanced && !e.auto_style))
    {
        if e.action == Action::Filter {
            body = body.child(
                div()
                    .mt_3()
                    .font_weight(gpui_kit::gpui::FontWeight::SEMIBOLD)
                    .child("Match appearance"),
            );
        }
        let mut colors = div().flex().flex_wrap().gap_3();
        for (index, label) in [(0, "Text color"), (1, "Background color")] {
            if let Some(picker) = app.editor_colors.get(index) {
                colors = colors.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(ColorPicker::new(picker).label(label))
                        .child(
                            Button::new(("color-reset", index))
                                .ghost()
                                .small()
                                .label("Default")
                                .on_click(cx.listener(move |app, _, window, cx| {
                                    app.dispatch(
                                        Message::EditorText(index + 4, String::new()),
                                        window,
                                        cx,
                                    )
                                })),
                        ),
                );
            }
        }
        body = body.child(colors).child(
            Checkbox::new("whole-line")
                .label("Color whole line")
                .checked(e.whole_line)
                .on_click(cx.listener(|app, _, window, cx| {
                    app.dispatch(Message::EditorToggle(2), window, cx)
                })),
        );
    }
    if let Some(error) = &e.error {
        body = body.child(
            Alert::error(
                if e.action == Action::Filter {
                    "filter-validation"
                } else {
                    "editor-validation"
                },
                error.clone(),
            )
            .small(),
        );
    }
    let footer =
        div()
            .flex()
            .justify_end()
            .when(e.action == Action::Filter, |d| {
                d.pt_3().border_t_1().border_color(cx.theme().border)
            })
            .gap_2()
            .when(
                !matches!(e.action, Action::View | Action::Help | Action::FileInfo),
                |d| {
                    d.child(button(
                        "editor-cancel",
                        "Cancel",
                        || Message::EditorCancel,
                        cx,
                    ))
                },
            )
            .child(
                Button::new("editor-apply")
                    .primary()
                    .label(if matches!(e.action, Action::FileInfo | Action::View) {
                        "Done"
                    } else if e.action == Action::Merge {
                        "Merge files"
                    } else if e.action == Action::Filter {
                        if e.edit_id.is_some() {
                            "Save filter"
                        } else {
                            "Add filter"
                        }
                    } else {
                        match e.action {
                            Action::Help | Action::SwitchFile => "Done",
                            Action::Settings => "Save preferences",
                            Action::Keybindings => "Save shortcuts",
                            Action::Appearance => "Save theme",
                            Action::Schema => "Use schema",
                            Action::Columns => "Save columns",
                            Action::ValueColors | Action::LevelColors => "Save colors",
                            Action::Group => "Save group",
                            Action::DefaultFilters => "Save mapping",
                            Action::EnableMcp => "Start server",
                            Action::Docker => "Connect container logs",
                            Action::Dlt => "Connect device",
                            Action::Otel => "Start receiver",
                            Action::Run => "Start process",
                            Action::Export => "Export",
                            Action::Annotation => {
                                if e.edit_id.is_some() {
                                    "Save note"
                                } else {
                                    "Add note"
                                }
                            }
                            _ => "Apply",
                        }
                    })
                    .disabled(
                        app.state.palette.pending()
                            || (e.action == Action::Merge
                                && e.merge_sources.iter().filter(|(_, on)| *on).count() < 2)
                            || (e.action == Action::Appearance && e.values[0].is_empty()),
                    )
                    .on_click(cx.listener(|app, _, window, cx| {
                        app.dispatch(Message::EditorApply, window, cx)
                    })),
            );
    let panel = div()
        .id("action-editor")
        .w(px(width))
        .max_h(px(height))
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
        .rounded(px(10.))
        .bg(cx.theme().popover)
        .text_color(cx.theme().popover_foreground)
        .border_1()
        .border_color(cx.theme().border)
        .shadow_lg()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(header)
        .child(
            div()
                .id("editor-content")
                .max_h(px((height - 130.).max(48.)))
                .overflow_y_scroll()
                .track_scroll(&app.command_scroll)
                .test_support()
                .child(body),
        )
        .child(footer);
    Some(
        deferred(
            anchored().position(point(px(0.), px(0.))).child(
                div()
                    .id("editor-backdrop")
                    .occlude()
                    .w(viewport.width)
                    .h(viewport.height)
                    .flex()
                    .items_center()
                    .justify_center()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        panel
                            .test_support()
                            .focus_trap("editor-focus", &app.interface_focus),
                    ),
            ),
        )
        .into_any_element(),
    )
}
