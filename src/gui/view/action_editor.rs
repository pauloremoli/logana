use crate::commands::Commands;
use crate::filters::FilterType;
use crate::gui::{
    actions::{Action, CATALOG},
    app::App,
    message::Message,
};
use gpui_kit::base::{Disableable, FocusTrapElement, Selectable, TestSupportExt};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::color_picker::ColorPicker;
use gpui_kit::component::input::{Input, Textarea};
use gpui_kit::component::{ActiveTheme, Sizable};
use gpui_kit::gpui::{
    AnyElement, ClipboardItem, Context, MouseButton, Pixels, Size, anchored, deferred, div, point,
    prelude::*, px,
};

fn text(app: &App, index: usize, label: &str, _cx: &mut Context<App>) -> AnyElement {
    if let Some(input) = app.editor_textareas.get(&index) {
        return div()
            .flex()
            .flex_col()
            .gap_1()
            .child(label.to_string())
            .child(Textarea::new(input))
            .into_any_element();
    }
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(label.to_string())
        .children(app.editor_inputs.get(index).map(|input| {
            Input::new(input).id(("editor-input", index)).disabled(
                index == 0
                    && app
                        .state
                        .editor
                        .as_ref()
                        .is_some_and(|e| e.group_name_locked),
            )
        }))
        .into_any_element()
}
fn button(id: &str, label: &str, message: fn() -> Message, cx: &mut Context<App>) -> Button {
    Button::new(id.to_string())
        .label(label.to_string())
        .on_click(cx.listener(move |app, _, window, cx| app.dispatch(message(), window, cx)))
}
pub fn editor(app: &App, viewport: Size<Pixels>, cx: &mut Context<App>) -> Option<AnyElement> {
    let e = app.state.editor.as_ref()?;
    let width = (f32::from(viewport.width) - 48.).clamp(0., 720.);
    let height = (f32::from(viewport.height) * 0.8).min((f32::from(viewport.height) - 48.).max(0.));
    let header = div()
        .flex()
        .justify_between()
        .items_center()
        .child(if e.action == Action::Filter && e.edit_id.is_some() {
            "Edit filter"
        } else {
            CATALOG.iter().find(|d| d.action == e.action).unwrap().label
        })
        .child(button(
            "editor-close",
            "Close",
            || Message::EditorCancel,
            cx,
        ));
    let mut body = div()
        .id("editor-content")
        .max_h(px((height - 130.).max(48.)))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap_3();
    match e.action {
        Action::Help => {
            for (label, keys) in crate::gui::keyboard::shortcuts(&app.state) {
                body = body.child(format!("{keys} — {label}"));
            }
        }
        Action::Merge => {
            body = body.child("Select at least two files. Lines without timestamps are omitted; closed sources keep their last content.")
                .child(text(app, 0, "Search open files", cx))
                .child(div().flex().gap_2()
                    .child(button("merge-all", "Select all", || Message::EditorAll(true), cx))
                    .child(button("merge-none", "Select none", || Message::EditorAll(false), cx)));
            for (index, (id, on)) in e.merge_sources.iter().enumerate() {
                let Some(tab) = app.state.tabs.iter().find(|tab| tab.id == *id) else {
                    continue;
                };
                if !tab
                    .title
                    .to_lowercase()
                    .contains(&e.values[0].to_lowercase())
                {
                    continue;
                }
                let id = *id;
                let label = if crate::gui::merge::source_parser(tab).is_none() {
                    format!("{} — no timestamp format", tab.title)
                } else {
                    tab.title.clone()
                };
                body = body.child(
                    Checkbox::new(("merge-source", index))
                        .label(label)
                        .checked(*on)
                        .on_click(cx.listener(move |app, _, window, cx| {
                            app.dispatch(Message::MergeSource(id), window, cx)
                        })),
                );
            }
            body = body.child(format!(
                "{} selected",
                e.merge_sources.iter().filter(|(_, on)| *on).count()
            ));
        }
        Action::SwitchFile => {
            body = body.child(text(app, 0, "Search open files", cx));
            for (index, tab) in app.state.tabs.iter().enumerate() {
                if !tab
                    .title
                    .to_lowercase()
                    .contains(&e.values[0].to_lowercase())
                {
                    continue;
                }
                let id = tab.id;
                body = body.child(
                    Button::new(("switch-file", index))
                        .label(tab.title.clone())
                        .selected(index == app.state.active_tab)
                        .on_click(cx.listener(move |app, _, window, cx| {
                            if let Some(index) = app.state.tabs.iter().position(|tab| tab.id == id)
                            {
                                crate::gui::actions::close(&mut app.state, true);
                                app.dispatch(Message::TabSelected(index), window, cx);
                            }
                        })),
                );
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
                .child("Mappings in config.json take precedence over saved settings.");
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
            for (index, name) in names.into_iter().enumerate() {
                body = body.child(
                    Button::new(("default-schema", index))
                        .label(name.clone())
                        .selected(e.values[0] == name)
                        .on_click(cx.listener(move |app, _, window, cx| {
                            app.dispatch(Message::EditorText(0, name.clone()), window, cx)
                        })),
                );
            }
        }
        Action::Docker => {
            body = body.child(text(app, 0, "Container name or ID", cx));
            for (index, (id, label)) in e.conditions.iter().enumerate() {
                let id = id.clone();
                body = body.child(
                    Button::new(("docker-container", index))
                        .label(label.clone())
                        .selected(e.values[0] == id)
                        .on_click(cx.listener(move |app, _, window, cx| {
                            app.dispatch(Message::EditorText(0, id.clone()), window, cx)
                        })),
                );
            }
        }
        Action::Dlt => {
            for (index, label, _) in &e.members {
                if let Some((host, port)) = e.conditions.get(*index) {
                    let host = host.clone();
                    let port = port.clone();
                    body = body.child(
                        Button::new(("dlt-device", *index))
                            .label(label.clone())
                            .on_click(cx.listener(move |app, _, window, cx| {
                                app.dispatch(Message::EditorText(0, host.clone()), window, cx);
                                app.dispatch(Message::EditorText(1, port.clone()), window, cx);
                            })),
                    );
                }
            }
            body = body
                .child(text(app, 0, "Device host", cx))
                .child(text(app, 1, "TCP port", cx));
        }
        Action::Otel => {
            body = body.child(text(app, 0, "Listen port", cx));
            for protocol in ["http", "grpc"] {
                body = body.child(
                    Button::new(format!("otel-{protocol}"))
                        .label(protocol)
                        .selected(e.values[1] == protocol)
                        .on_click(cx.listener(move |app, _, window, cx| {
                            app.dispatch(Message::EditorText(1, protocol.into()), window, cx)
                        })),
                );
            }
        }
        Action::Run => {
            body = body.child(text(app, 0, "Executable", cx)).child(text(
                app,
                1,
                "Arguments (one argument per line; spaces stay inside an argument)",
                cx,
            ));
        }
        Action::Export => {
            body = body.child("Choose a template and fill its optional fields.");
            for (index, name) in crate::commands::list_templates().into_iter().enumerate() {
                body = body.child(
                    Button::new(("export-template", index))
                        .label(name.clone())
                        .selected(e.values[0] == name)
                        .on_click(cx.listener(move |app, _, window, cx| {
                            app.dispatch(Message::EditorText(0, name.clone()), window, cx)
                        })),
                );
            }
            for (index, (key, _)) in e.fields.iter().enumerate() {
                body = body.child(text(app, index + 6, key, cx));
            }
        }
        Action::Annotation => {
            body = body
                .child(format!("Note attached to {} source rows", e.members.len()))
                .child(text(app, 0, "Annotation", cx));
        }
        Action::Filter => {
            let mut kinds = div().flex().gap_2();
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
            body = body
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .font_weight(gpui_kit::gpui::FontWeight::SEMIBOLD)
                                .child("Filter behavior"),
                        )
                        .child(kinds)
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(match e.kind {
                                    FilterType::Include => "Keep matching lines in view.",
                                    FilterType::Exclude => "Hide matching lines from view.",
                                    FilterType::Highlight => {
                                        "Color matching lines without hiding other logs."
                                    }
                                }),
                        ),
                )
                .child(
                    div()
                        .mt_3()
                        .font_weight(gpui_kit::gpui::FontWeight::SEMIBOLD)
                        .child("Match conditions"),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("Text and field conditions must all match. Use either or both."),
                )
                .child(text(app, 0, "Text pattern", cx));
            let mut options = div().flex().flex_wrap().gap_4();
            for (i, (label, on)) in [
                ("Regular expression", e.regex),
                ("Ignore case", e.ignore_case),
            ]
            .into_iter()
            .enumerate()
            {
                options = options.child(
                    Checkbox::new(("filter-option", i))
                        .label(label)
                        .checked(on)
                        .on_click(cx.listener(move |app, _, window, cx| {
                            app.dispatch(Message::EditorToggle(i), window, cx)
                        })),
                );
            }
            body = body
                .child(options)
                .child(div().text_xs().text_color(cx.theme().muted_foreground)
                    .child("Regex and ignore case apply to text-only filters. Field conditions use case-sensitive substring matching."))
                .child(div().mt_2().child(text(app, 1, "Field name", cx)));
            let mut names = div().flex().flex_wrap().gap_1();
            for (i, name) in e
                .discovery
                .names
                .iter()
                .filter(|n| n.to_lowercase().contains(&e.values[1].to_lowercase()))
                .take(24)
                .enumerate()
            {
                let value = name.clone();
                names = names.child(
                    Button::new(("field-choice", i))
                        .ghost()
                        .small()
                        .label(name.clone())
                        .on_click(cx.listener(move |app, _, window, cx| {
                            app.dispatch(Message::EditorText(1, value.clone()), window, cx)
                        })),
                );
            }
            body = body.child(names).child(text(app, 2, "Contains value", cx));
            let mut values = div().flex().flex_wrap().gap_1();
            if let Some(discovered) = e.discovery.values.get(&e.values[1]) {
                for (i, value) in discovered
                    .iter()
                    .filter(|v| v.to_lowercase().contains(&e.values[2].to_lowercase()))
                    .take(24)
                    .enumerate()
                {
                    let value = value.clone();
                    values = values.child(
                        Button::new(("value-choice", i))
                            .ghost()
                            .small()
                            .label(value.clone())
                            .on_click(cx.listener(move |app, _, window, cx| {
                                app.dispatch(Message::EditorText(2, value.clone()), window, cx)
                            })),
                    );
                }
            }
            body = body.child(values).child(button(
                "condition-add",
                if e.condition_edit.is_some() {
                    "Update condition"
                } else {
                    "Add condition"
                },
                || Message::EditorConditionAdd,
                cx,
            ));
            for (i, (field, value)) in e.conditions.iter().enumerate() {
                body = body.child(
                    div()
                        .flex()
                        .justify_between()
                        .items_center()
                        .gap_2()
                        .py_2()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .child(format!("{field} contains {value}")),
                        )
                        .child(
                            Button::new(("condition-edit", i))
                                .ghost()
                                .label("Edit")
                                .on_click(cx.listener(move |app, _, window, cx| {
                                    app.dispatch(Message::EditorConditionEdit(i), window, cx)
                                })),
                        )
                        .child(
                            Button::new(("condition-remove", i))
                                .ghost()
                                .label("Remove")
                                .on_click(cx.listener(move |app, _, window, cx| {
                                    app.dispatch(Message::EditorConditionRemove(i), window, cx)
                                })),
                        ),
                );
            }
            body = body
                .child(
                    div()
                        .mt_3()
                        .font_weight(gpui_kit::gpui::FontWeight::SEMIBOLD)
                        .child("Organization"),
                )
                .child(text(app, 3, "Group (optional)", cx));
            if let Some(tab) = app.state.active_tab() {
                let mut groups = div().flex().flex_wrap().gap_1();
                for (i, name) in tab.log_manager.group_names().into_iter().enumerate() {
                    groups = groups.child(
                        Button::new(("group-choice", i))
                            .ghost()
                            .small()
                            .label(name.clone())
                            .on_click(cx.listener(move |app, _, window, cx| {
                                app.dispatch(Message::EditorText(3, name.clone()), window, cx)
                            })),
                    );
                }
                body = body.child(groups);
            }
        }
        Action::Group => {
            body = body
                .child(text(app, 0, "Group name", cx))
                .child("Member filters");
            for (index, (_, label, on)) in e.members.iter().enumerate() {
                body = body.child(
                    Checkbox::new(("group-member", index))
                        .label(label.clone())
                        .checked(*on)
                        .on_click(cx.listener(move |app, _, window, cx| {
                            app.dispatch(Message::EditorMember(index), window, cx)
                        })),
                );
            }
        }
        Action::Columns | Action::ValueColors | Action::LevelColors => {
            body = body.child(
                div()
                    .flex()
                    .gap_2()
                    .child(button("editor-all", "All", || Message::EditorAll(true), cx))
                    .child(button(
                        "editor-none",
                        "None",
                        || Message::EditorAll(false),
                        cx,
                    ))
                    .child(button("editor-reset", "Reset", || Message::EditorReset, cx)),
            );
            if e.action == Action::Columns {
                body = body.child(text(app, 0, "Search columns", cx));
            }
            for (i, (name, on)) in e.fields.iter().enumerate() {
                if e.action == Action::Columns
                    && !name.to_lowercase().contains(&e.values[0].to_lowercase())
                {
                    continue;
                }
                let mut row = div().flex().gap_2().items_center().child(
                    Checkbox::new(("editor-field", i))
                        .checked(*on)
                        .label(name.clone())
                        .on_click(cx.listener(move |app, _, window, cx| {
                            app.dispatch(Message::EditorField(i), window, cx)
                        })),
                );
                if e.action == Action::Columns {
                    row = row
                        .child(
                            Button::new(("column-up", i))
                                .ghost()
                                .small()
                                .label("↑")
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
                                .disabled(i + 1 == e.fields.len())
                                .on_click(cx.listener(move |app, _, window, cx| {
                                    app.dispatch(Message::EditorMove(i, 1), window, cx)
                                })),
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
                    "Click a preset to preview the whole application. Apply saves your selection.",
                ))
                .child(text(app, 1, "Search themes", cx));
            for (i, name) in crate::theme::Theme::list_available_themes()
                .into_iter()
                .filter(|n| n.to_lowercase().contains(&e.values[1].to_lowercase()))
                .enumerate()
            {
                let mut row = div().flex().items_center().gap_2();
                if let Ok(theme) = crate::theme::Theme::from_file(format!("{name}.json")) {
                    for color in [
                        theme.root_bg,
                        theme.text,
                        theme.info_fg,
                        theme.error_fg,
                        theme.debug_fg,
                    ] {
                        if let Some(color) = crate::gui::color::color_to_gpui(color) {
                            row = row.child(div().size(px(16.)).rounded(px(3.)).bg(color));
                        }
                    }
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
            body = body.child(text(app, 0, "Selected schema", cx));
            let mut names = vec![("none".to_string(), "Unstructured")];
            for name in crate::parser::builtin_format_names() {
                names.push((name, "Built-in"));
            }
            for schema in crate::config::custom_schemas() {
                names.push((schema.name.clone(), "Custom"));
            }
            for (i, (name, kind)) in names.into_iter().enumerate() {
                body = body.child(
                    Button::new(("schema-choice", i))
                        .ghost()
                        .label(format!("{name} · {kind}"))
                        .selected(e.values[0] == name)
                        .on_click(cx.listener(move |app, _, window, cx| {
                            app.dispatch(Message::EditorText(0, name.clone()), window, cx)
                        })),
                );
            }
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
            body = body.child(path.clone()).child(
                Button::new("copy-path")
                    .label("Copy path")
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(path.clone()))
                    }),
            );
        }
        Action::View => {
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
                                    6 => Commands::SidebarPosition {
                                        side: if on {
                                            crate::shared::SidebarSide::Right
                                        } else {
                                            crate::shared::SidebarSide::Left
                                        },
                                    },
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
    if matches!(e.action, Action::Filter | Action::Group) && e.kind != FilterType::Exclude {
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
        body = body.child(div().text_color(cx.theme().danger).child(error.clone()));
    }
    let footer =
        div()
            .flex()
            .justify_end()
            .gap_2()
            .child(button(
                "editor-cancel",
                "Cancel",
                || Message::EditorCancel,
                cx,
            ))
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
                        "Apply"
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
        .child(body)
        .child(footer);
    Some(
        deferred(
            anchored().position(point(px(0.), px(0.))).child(
                div()
                    .id("editor-backdrop")
                    .w(viewport.width)
                    .h(viewport.height)
                    .flex()
                    .items_center()
                    .justify_center()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|app, _, window, cx| {
                            cx.stop_propagation();
                            app.dispatch(Message::EditorCancel, window, cx);
                        }),
                    )
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
