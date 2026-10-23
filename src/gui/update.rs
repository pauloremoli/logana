#[cfg(test)]
use crate::commands::CommandLine;
use crate::commands::Commands;
#[cfg(test)]
use crate::commands::auto_complete::shell_split;
use crate::db::{Database, LogManager};
use crate::filters::{
    DATE_PREFIX, FIELD_PREFIX, FilterDef, FilterOptions, FilterType, group_enabled,
    parse_date_filter, parse_field_filter_expr,
};
use crate::gui::effect::Effect;
use crate::gui::message::{FileLoaded, Message};
use crate::gui::runtime;
use crate::gui::state::{GuiState, StatusMessage};
use crate::ingestion::FileReader;
use crate::input::KeyCode;
use crate::shared::TabState;
#[cfg(test)]
use crate::shared::VisibleLines;
#[cfg(test)]
use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// The central reducer: every user intent (`Message`) flows through here,
/// mutating `state` directly and returning an `Effect` describing any
/// async work or imperative widget action the gpui glue layer (`App` in
/// `app.rs`) still needs to perform. Never touches a gpui type itself.
pub fn update(state: &mut GuiState, message: Message) -> Effect {
    let source = match &message {
        Message::PaletteExecuted(_, id, _) => Some(*id),
        Message::FiltersMutated(index, _)
        | Message::GroupsMutated(index, _)
        | Message::CommandExecuted(index, _)
        | Message::FilterToggled(index, _)
        | Message::FilterReordered(index, _, _)
        | Message::GroupToggled(index, _) => state.tabs.get(*index).map(|tab| tab.id),
        Message::GuiAction(action)
            if !matches!(
                action,
                crate::gui::actions::Action::Undo | crate::gui::actions::Action::Redo
            ) =>
        {
            state.active_tab().map(|tab| tab.id)
        }
        Message::EditorApply
        | Message::GuiCommand(..)
        | Message::SidebarToggled
        | Message::SidebarSideToggled => state
            .editor
            .as_ref()
            .and_then(|e| e.source)
            .or_else(|| state.active_tab().map(|t| t.id)),
        Message::KeyPressed(key, modifiers)
            if crate::gui::keyboard::mutates_tab(state, *key, *modifiers) =>
        {
            state.active_tab().map(|t| t.id)
        }
        Message::AnnotationRemoved(_) => state.active_tab().map(|t| t.id),
        Message::LogRowAction(id, _, _) => Some(*id),
        Message::FilterRemoved(index, _)
        | Message::ClearAllFilters(index)
        | Message::GroupDeleted(index, _)
        | Message::FacetValueToggled(index, _, _) => state.tabs.get(*index).map(|tab| tab.id),
        _ => None,
    };
    let before = source
        .and_then(|id| state.tabs.iter().find(|tab| tab.id == id))
        .map(crate::gui::history::Snapshot::new);
    let effect = update_inner(state, message);
    if let Some(editor) = state.editor.as_mut()
        && editor
            .validation_pending
            .as_ref()
            .is_some_and(|key| !editor.validation_is_current(key))
    {
        editor.validation_pending = None;
    }
    if let Some(before) = before
        && state
            .tabs
            .iter()
            .find(|tab| tab.id == before.id)
            .is_some_and(|tab| before.changed(tab))
    {
        state.history.entry(before.id).or_default().record(before);
    }
    effect
}
fn update_inner(state: &mut GuiState, message: Message) -> Effect {
    match message {
        Message::SchemaPrepared(request_id, tab_id, name, prepared) => {
            super::schema_preparation::install(state, request_id, tab_id, name, *prepared)
        }
        Message::FilterPatternValidated(epoch, pattern, timestamp, result) => {
            let key = (pattern, timestamp);
            let Some(editor) = state.editor.as_mut().filter(|e| {
                e.epoch == epoch
                    && e.action == crate::gui::actions::Action::Filter
                    && e.validation_pending.as_ref() == Some(&key)
                    && e.validation_is_current(&key)
            }) else {
                return Effect::None;
            };
            editor.validation_pending = None;
            match result {
                Ok(()) => {
                    editor.validated_pattern = Some(key);
                    crate::gui::actions::apply(state)
                }
                Err(error) => {
                    editor.error = Some(error);
                    Effect::None
                }
            }
        }
        Message::SelectDltDevice(index) => {
            if let Some(e) = state
                .editor
                .as_mut()
                .filter(|e| e.action == crate::gui::actions::Action::Dlt && !e.dlt_saving)
            {
                if let Some(index) = index {
                    if let Some((_, name, _)) = e.members.get(index)
                        && let Some((host, port)) = e.conditions.get(index)
                    {
                        e.values[0] = host.clone();
                        e.values[1] = port.clone();
                        e.values[3] = name.clone();
                        e.edit_id = Some(index);
                    }
                } else {
                    e.edit_id = None;
                    e.values[0] = "localhost".into();
                    e.values[1] = crate::config::DEFAULT_DLT_PORT.to_string();
                    e.values[3].clear();
                }
                e.error = None;
            }
            Effect::None
        }
        Message::RemoveDltDevice => {
            let Some(e) = state
                .editor
                .as_mut()
                .filter(|e| e.action == crate::gui::actions::Action::Dlt && !e.dlt_saving)
            else {
                return Effect::None;
            };
            let Some(index) = e.edit_id else {
                return Effect::None;
            };
            e.dlt_saving = true;
            e.error = None;
            Effect::UpdateDltDevice {
                epoch: e.epoch,
                original_name: e.members[index].1.clone(),
                device: None,
            }
        }
        Message::DltDeviceRemoved(epoch, result) => {
            if let Some(e) = state
                .editor
                .as_mut()
                .filter(|e| e.epoch == epoch && e.action == crate::gui::actions::Action::Dlt)
            {
                e.dlt_saving = false;
                if result.is_ok() {
                    if let Some(index) = e.edit_id.take() {
                        e.members.remove(index);
                        e.conditions.remove(index);
                        for (index, member) in e.members.iter_mut().enumerate() {
                            member.0 = index;
                        }
                        e.values[3].clear();
                    }
                } else {
                    e.error = result.clone().err();
                }
            }
            state.notify(match result {
                Ok(()) => StatusMessage::Info("Removed saved DLT device.".into()),
                Err(error) => StatusMessage::Error(error),
            });
            Effect::None
        }
        Message::SaveDltDevice => {
            let Some(e) = state
                .editor
                .as_mut()
                .filter(|e| e.action == crate::gui::actions::Action::Dlt && !e.dlt_saving)
            else {
                return Effect::None;
            };
            match crate::gui::actions::dlt_device(e, true) {
                Ok(device) => {
                    if e.members.iter().enumerate().any(|(index, (_, name, _))| {
                        Some(index) != e.edit_id && *name == device.name
                    }) {
                        e.error = Some(
                            "A saved device already uses this name. Choose another name.".into(),
                        );
                        return Effect::None;
                    }
                    e.dlt_saving = true;
                    e.error = None;
                    if let Some(index) = e.edit_id {
                        Effect::UpdateDltDevice {
                            epoch: e.epoch,
                            original_name: e.members[index].1.clone(),
                            device: Some(device),
                        }
                    } else {
                        Effect::SaveDltDevice {
                            epoch: e.epoch,
                            device,
                        }
                    }
                }
                Err(error) => {
                    e.error = Some(error);
                    Effect::None
                }
            }
        }
        Message::DltDeviceSaved(epoch, device, result) => {
            if let Some(e) = state
                .editor
                .as_mut()
                .filter(|e| e.epoch == epoch && e.action == crate::gui::actions::Action::Dlt)
            {
                e.dlt_saving = false;
                if result.is_ok() {
                    let index = e.edit_id.unwrap_or(e.conditions.len());
                    if e.edit_id.is_some() {
                        e.members[index].1 = device.name.clone();
                        e.conditions[index] = (
                            device.host.clone(),
                            device
                                .port
                                .unwrap_or(crate::config::DEFAULT_DLT_PORT)
                                .to_string(),
                        );
                    } else {
                        e.members.push((index, device.name.clone(), true));
                        e.conditions.push((
                            device.host,
                            device
                                .port
                                .unwrap_or(crate::config::DEFAULT_DLT_PORT)
                                .to_string(),
                        ));
                    }
                } else {
                    e.error = result.clone().err();
                }
            }
            state.notify(match result {
                Ok(()) => StatusMessage::Info(format!("Saved DLT device {}.", device.name)),
                Err(error) => StatusMessage::Error(error),
            });
            Effect::None
        }
        Message::AnnotationRemoved(index) => {
            if let Some(tab) = state.active_tab_mut()
                && index < tab.comment_manager.get().len()
            {
                tab.comment_manager.remove(index);
                state.notify(StatusMessage::Info(
                    "Note deleted. Use Undo to restore it.".into(),
                ));
            }
            Effect::None
        }
        Message::BrowseDefaultMapping => match &state.editor {
            Some(e) if e.action == crate::gui::actions::Action::DefaultFilters => {
                Effect::DefaultMappingDialog {
                    epoch: e.epoch,
                    format: e.values[0].clone(),
                }
            }
            _ => Effect::None,
        },
        Message::DefaultMappingPicked(epoch, format, result) => {
            if let Some(e) = &mut state.editor
                && e.action == crate::gui::actions::Action::DefaultFilters
                && e.epoch == epoch
                && e.values[0] == format
            {
                match result {
                    Ok(Some(path)) => {
                        e.values[1] = path.to_string_lossy().into_owned();
                        e.error = None;
                    }
                    Ok(None) => {}
                    Err(error) => e.error = Some(error),
                }
            }
            Effect::None
        }
        Message::MergeSource(id) => {
            if let Some(e) = &mut state.editor
                && let Some((_, on)) = e.merge_sources.iter_mut().find(|(source, _)| *source == id)
            {
                *on = !*on;
                e.error = None;
            }
            Effect::None
        }
        Message::DefaultMappingsLoaded(epoch, mappings) => {
            if let Some(editor) = &mut state.editor
                && editor.action == crate::gui::actions::Action::DefaultFilters
                && editor.epoch == epoch
            {
                editor.values[1] = mappings.get(&editor.values[0]).cloned().unwrap_or_default();
                editor.conditions = mappings.into_iter().collect();
            }
            Effect::None
        }
        Message::DockerListed(epoch, result) => {
            if let Some(editor) = &mut state.editor
                && editor.action == crate::gui::actions::Action::Docker
                && editor.epoch == epoch
            {
                match result {
                    Ok(containers) => editor.conditions = containers,
                    Err(error) => editor.error = Some(error),
                }
            }
            Effect::None
        }
        Message::SourceLoaded(result) => {
            match result {
                Ok(mut tab) => {
                    state.preferences.apply(&mut tab);
                    tab.begin_filter_refresh_background();
                    state.tabs.push(*tab);
                    state.active_tab = state.tabs.len() - 1;
                    state.nav_page = crate::gui::state::NavPage::Logs;
                }
                Err(error) => state.notify(StatusMessage::Error(error)),
            }
            Effect::None
        }
        Message::LogRowAction(source, line, action) => {
            if state.editor.is_some() || state.palette.pending() {
                return Effect::None;
            }
            let Some(index) = state.tabs.iter().position(|tab| tab.id == source) else {
                return Effect::None;
            };
            let Some(row) = state.tabs[index].filter.visible_indices.position_of(line) else {
                return Effect::None;
            };
            state.active_tab = index;
            if !state
                .log_selection
                .is_some_and(|(id, a, b)| id == source && (a.min(b)..=a.max(b)).contains(&row))
            {
                state.log_selection = Some((source, row, row));
            }
            crate::gui::actions::open(state, action, None, None)
        }
        Message::EditAnnotation(index) => crate::gui::actions::open(
            state,
            crate::gui::actions::Action::Annotation,
            Some(index),
            None,
        ),
        Message::DismissNotification(revision) => {
            state.dismiss_notification(revision);
            Effect::None
        }
        Message::GuiFinished(request, result) => {
            if state
                .palette
                .outcome
                .as_ref()
                .is_some_and(|o| o.request_id == request)
            {
                match result {
                    Ok(output) => {
                        if let Some(e) = &mut state.editor
                            && e.action == crate::gui::actions::Action::Appearance
                        {
                            e.original_theme = state.theme.clone();
                        }
                        finish_palette(
                            state,
                            request,
                            crate::gui::palette::ExecutionState::Success,
                            output,
                        )
                    }
                    Err(error) => {
                        if let Some(e) = &mut state.editor {
                            e.error = Some(error.clone());
                        }
                        finish_palette(
                            state,
                            request,
                            crate::gui::palette::ExecutionState::Error,
                            error,
                        )
                    }
                }
            }
            Effect::None
        }
        Message::GuiAction(action) => crate::gui::actions::open(state, action, None, None),
        Message::EditFilter(id) => {
            crate::gui::actions::open(state, crate::gui::actions::Action::Filter, Some(id), None)
        }
        Message::EditGroup(name) => {
            crate::gui::actions::open(state, crate::gui::actions::Action::Group, None, Some(name))
        }
        Message::EditorText(index, value) => {
            if let Some(e) = &mut state.editor {
                if e.values.get(index) == Some(&value) {
                    return Effect::None;
                }
                if e.action == crate::gui::actions::Action::DefaultFilters && index == 0 {
                    e.values[1] = e
                        .conditions
                        .iter()
                        .find(|(name, _)| name == &value)
                        .map(|(_, path)| path.clone())
                        .unwrap_or_default();
                    e.values[0] = value;
                } else if e.action == crate::gui::actions::Action::Export && index == 0 {
                    crate::gui::actions::configure_export(e, &value);
                } else if e.action == crate::gui::actions::Action::Otel && index == 1 {
                    if e.values[0] == crate::gui::actions::otel_default_port(&e.values[1]) {
                        e.values[0] = crate::gui::actions::otel_default_port(&value).into();
                    }
                    e.values[1] = value;
                } else if let Some(slot) = e.values.get_mut(index) {
                    *slot = value;
                }
                e.error = None;
            }
            Effect::None
        }
        Message::EditorAdvanced => {
            if let Some(e) = &mut state.editor {
                e.advanced = !e.advanced;
            }
            Effect::None
        }
        Message::EditorStyle(automatic) => {
            if let Some(e) = &mut state.editor {
                if automatic && !e.auto_style {
                    e.regenerate_auto_style();
                } else if !automatic {
                    e.disable_auto_style();
                }
            }
            Effect::None
        }
        Message::EditorRegenerateStyle => {
            if let Some(e) = &mut state.editor {
                e.regenerate_auto_style();
            }
            Effect::None
        }
        Message::EditorMatchField(value) => {
            if let Some(e) = &mut state.editor {
                if value.is_none()
                    && e.conditions.len() == 1
                    && e.condition_edit == Some(0)
                    && e.values[0].is_empty()
                {
                    e.conditions.clear();
                    e.condition_edit = None;
                }
                if e.conditions.is_empty() {
                    if value.is_some() && e.values[1].is_empty() {
                        e.values[2] = std::mem::take(&mut e.values[0]);
                    } else if value.is_none() {
                        e.values[0] = std::mem::take(&mut e.values[2]);
                    }
                }
                e.values[1] = value.unwrap_or_default();
                e.error = None;
            }
            Effect::None
        }
        Message::EditorTimestampPart(index, value) => {
            if let Some(e) = &mut state.editor {
                let (operator, start, end) = crate::gui::time_range::timestamp_parts(&e.values[0]);
                let mut parts = [operator.to_owned(), start.to_owned(), end.to_owned()];
                if let Some(part) = parts.get_mut(index) {
                    *part = value;
                }
                e.values[0] =
                    crate::gui::time_range::timestamp_pattern(&parts[0], &parts[1], &parts[2]);
                e.error = None;
            }
            Effect::None
        }
        Message::EditorToggle(index) => {
            if let Some(e) = &mut state.editor {
                match index {
                    0 => e.regex = !e.regex,
                    1 => e.ignore_case = !e.ignore_case,
                    _ => e.whole_line = !e.whole_line,
                };
                e.error = None;
            }
            Effect::None
        }
        Message::EditorKind(kind) => {
            if let Some(e) = &mut state.editor {
                e.kind = kind;
                e.error = None;
            }
            Effect::None
        }
        Message::EditorMembers(ids) => {
            if let Some(e) = &mut state.editor {
                for (id, _, on) in &mut e.members {
                    *on = ids.contains(id);
                }
            }
            Effect::None
        }
        Message::EditorMember(index) => {
            if let Some(e) = &mut state.editor
                && let Some((_, _, on)) = e.members.get_mut(index)
            {
                *on = !*on;
            }
            Effect::None
        }
        Message::EditorField(index) => {
            if let Some(e) = &mut state.editor
                && let Some((_, on)) = e.fields.get_mut(index)
            {
                *on = !*on;
            }
            Effect::None
        }
        Message::EditorMove(index, delta) => {
            if let Some(e) = &mut state.editor {
                let target = index as isize + delta;
                if target >= 0 && (target as usize) < e.fields.len() {
                    e.fields.swap(index, target as usize);
                }
            }
            Effect::None
        }
        Message::EditorAll(on) => {
            if let Some(e) = &mut state.editor {
                for (_, enabled) in &mut e.merge_sources {
                    *enabled = on;
                }
                e.error = None;
                for (_, enabled) in &mut e.fields {
                    *enabled = on;
                }
            }
            Effect::None
        }
        Message::EditorReset => {
            if let Some(e) = &mut state.editor {
                e.fields = e.defaults.clone();
            }
            Effect::None
        }
        Message::EditorConditionAdd => {
            if let Some(e) = &mut state.editor {
                if !e.values[1].trim().is_empty() {
                    let condition = (e.values[1].clone(), e.values[2].clone());
                    if let Some(index) = e
                        .condition_edit
                        .and_then(|index| e.conditions.get_mut(index))
                    {
                        *index = condition;
                    } else {
                        e.conditions.push(condition);
                    }
                    e.condition_edit = None;
                    e.values[1].clear();
                    e.values[2].clear();
                    e.error = None;
                } else {
                    e.error = Some("Choose a field first.".into());
                }
            }
            Effect::None
        }
        Message::EditorConditionEdit(index) => {
            if let Some(e) = &mut state.editor
                && let Some((field, value)) = e.conditions.get(index)
            {
                e.values[1] = field.clone();
                e.values[2] = value.clone();
                e.condition_edit = Some(index);
                e.error = None;
            }
            Effect::None
        }
        Message::EditorConditionRemove(index) => {
            if let Some(e) = &mut state.editor
                && index < e.conditions.len()
            {
                e.conditions.remove(index);
                if e.condition_edit == Some(index) {
                    e.condition_edit = None;
                    e.values[1].clear();
                    e.values[2].clear();
                } else if let Some(edit) = e.condition_edit.as_mut()
                    && *edit > index
                {
                    *edit -= 1;
                }
            }
            Effect::None
        }
        Message::EditorPreview(name) => {
            match set_theme_by_name(state, &name) {
                Ok(_) => {
                    if let Some(e) = &mut state.editor {
                        e.values[0] = name;
                        e.error = None;
                    }
                }
                Err(error) => {
                    if let Some(e) = &mut state.editor {
                        e.error = Some(error);
                    }
                }
            }
            Effect::None
        }
        Message::EditorBack => {
            if state.palette.pending() {
                return Effect::None;
            }
            if let Some(mut editor) = state.editor_history.pop() {
                let history = std::mem::take(&mut state.editor_history);
                crate::gui::actions::close(state, false);
                editor.epoch = state.palette.focus_epoch.wrapping_add(1);
                state.editor = Some(editor);
                state.editor_history = history;
            }
            Effect::None
        }
        Message::EditorNavigate(action) => {
            if state.palette.pending() {
                return Effect::None;
            }
            let previous = state.editor.clone();
            let mut history = std::mem::take(&mut state.editor_history);
            crate::gui::actions::close(state, false);
            let effect = crate::gui::actions::open(state, action, None, None);
            if state.editor.is_some() {
                if let Some(previous) = previous {
                    history.push(previous);
                }
                state.editor_history = history;
            }
            effect
        }
        Message::EditorApply => {
            let active = state.active_tab;
            let effect = crate::gui::actions::apply(state);
            state.active_tab = active;
            effect
        }
        Message::EditorCancel => {
            crate::gui::actions::close(state, false);
            Effect::None
        }
        Message::GuiCommand(command, label) => {
            let active = state.active_tab;
            if let Some(source) = state.editor.as_ref().and_then(|e| e.source) {
                let Some(index) = state.tabs.iter().position(|t| t.id == source) else {
                    return Effect::None;
                };
                state.active_tab = index;
            }
            let effect = perform_gui(state, command, label);
            state.active_tab = active;
            effect
        }
        Message::SearchNext(forward) => {
            if let Some(tab) = state.active_tab_mut() {
                let result = if forward {
                    tab.search.query.go_next()
                } else {
                    tab.search.query.go_prev()
                };
                if let Some(result) = result {
                    let line = result.line_idx;
                    tab.scroll.scroll_offset = tab
                        .filter
                        .visible_indices
                        .position_of(line)
                        .unwrap_or(tab.scroll.scroll_offset);
                    if let Some(handle) = &mut tab.filter.handle {
                        handle.scroll_anchor = Some(line);
                    }
                }
            }
            Effect::None
        }
        Message::SelectLog(index, extend) => {
            state.keyboard_sidebar = None;
            let existing_anchor = state.active_tab().and_then(|tab| {
                state
                    .log_selection
                    .filter(|(id, _, _)| *id == tab.id)
                    .map(|(_, anchor, _)| anchor)
            });
            if let Some(tab) = state.active_tab_mut() {
                let id = tab.id;
                // A mouse click moves the cursor the same way keyboard
                // navigation does, so the status bar's "Ln N" follows it.
                tab.scroll.scroll_offset =
                    index.min(tab.filter.visible_indices.len().saturating_sub(1));
                if let Some(handle) = &mut tab.filter.handle {
                    handle.scroll_anchor =
                        tab.filter.visible_indices.get_opt(tab.scroll.scroll_offset);
                }
                let anchor = if extend {
                    existing_anchor.unwrap_or(index)
                } else {
                    index
                };
                state.log_selection = Some((id, anchor, index));
            }
            Effect::None
        }
        Message::OpenCommands => {
            if state.editor.is_some() || state.time_range_open || state.palette.visible() {
                Effect::None
            } else {
                state.palette.input.clear();
                state.palette.collapsed = false;
                open_palette(state)
            }
        }
        Message::PaletteEdited(input) => {
            state.palette.edit(input);
            crate::gui::actions::select_best(state);
            Effect::None
        }
        Message::PaletteKey(code, _) => {
            match code {
                KeyCode::Esc => state.palette.open = false,
                KeyCode::Enter => {
                    return update(state, Message::PaletteInsert(state.palette.selected));
                }
                KeyCode::Up | KeyCode::Down => {
                    let enabled: Vec<_> = crate::gui::actions::matching(&state.palette.input)
                        .iter()
                        .enumerate()
                        .filter(|(_, d)| d.unavailable(state).is_none())
                        .map(|(i, _)| i)
                        .collect();
                    let next = if code == KeyCode::Up {
                        enabled
                            .into_iter()
                            .rev()
                            .find(|i| *i < state.palette.selected)
                    } else {
                        enabled.into_iter().find(|i| *i > state.palette.selected)
                    };
                    if let Some(next) = next {
                        state.palette.selected = next;
                    }
                }
                _ => {}
            }
            Effect::None
        }
        Message::PaletteInsert(index) => {
            let action = crate::gui::actions::matching(&state.palette.input)
                .get(index)
                .map(|d| d.action);
            action
                .map(|a| crate::gui::actions::open(state, a, None, None))
                .unwrap_or(Effect::None)
        }
        Message::PaletteSubmit => Effect::None,
        Message::PaletteClose => {
            state.palette.open = false;
            state.palette.generation = state.palette.generation.wrapping_add(1);
            Effect::None
        }
        Message::PaletteClear => {
            if !state.palette.pending() {
                state.palette.outcome = None;
            }
            Effect::None
        }
        Message::PaletteExecuted(request, source, result) => {
            palette_executed(state, request, source, result)
        }
        Message::OpenFileDialog => {
            if state.palette.pending() || state.editor.is_some() {
                Effect::None
            } else {
                state.palette.open = false;
                state.time_range_open = false;
                Effect::OpenFileDialog
            }
        }
        Message::FileDialogResult(Some(path)) => {
            // Background loading owns progress; a running palette outcome would
            // block subsequent actions without a palette completion callback.
            gui_outcome(
                state,
                "Open file".into(),
                crate::gui::palette::ExecutionState::Information,
                format!("Loading {}…", path.display()),
            );
            Effect::LoadFile(path)
        }
        Message::FileDialogResult(None) => {
            gui_outcome(
                state,
                "Open file".into(),
                crate::gui::palette::ExecutionState::Information,
                "File selection canceled.".into(),
            );
            Effect::None
        }
        Message::TabPrepared(tab, widths, activate) => {
            let was_empty = state.tabs.is_empty();
            state.tabs.push(*tab);
            if activate || was_empty {
                state.column_widths = widths;
                state.active_tab = state.tabs.len() - 1;
            }
            Effect::None
        }
        Message::FileLoaded(Ok(loaded)) => {
            push_tab(state, loaded);
            gui_outcome(
                state,
                "Open file".into(),
                crate::gui::palette::ExecutionState::Success,
                "File opened.".into(),
            );
            Effect::None
        }
        Message::FileLoaded(Err(err)) => {
            gui_outcome(
                state,
                "Open file".into(),
                crate::gui::palette::ExecutionState::Error,
                err.clone(),
            );
            state.notify(StatusMessage::Error(err));
            Effect::None
        }
        Message::TabSelected(idx) => {
            state.visual_lines = false;
            state.keyboard_sidebar = None;
            if idx < state.tabs.len() {
                state.active_tab = idx;
                state.column_widths = Default::default();
                state.column_resize_last_x = None;
            }
            Effect::None
        }
        Message::TabClosed(idx) => {
            close_tab_at(state, idx);
            Effect::None
        }
        Message::KeyPressed(key, modifiers) => crate::gui::keyboard::apply(state, key, modifiers),
        Message::SearchPopup(open) => {
            state.search_open = open
                && state.active_tab().is_some()
                && state.editor.is_none()
                && !state.palette.visible();
            Effect::None
        }
        Message::SearchEdited(query) => {
            if state.editor.is_none()
                && !state.palette.visible()
                && let Some(tab) = state.active_tab_mut()
            {
                if query.is_empty() {
                    tab.cancel_search();
                } else {
                    tab.begin_search(&query, tab.search.query.is_forward(), false);
                }
            }
            Effect::None
        }
        Message::CommandExecuted(tab_idx, result) => {
            match result {
                Ok(log_manager) => apply_log_manager(state, tab_idx, log_manager),
                Err(err) => state.notify(StatusMessage::Error(err)),
            }
            Effect::None
        }
        Message::FilterPersistenceFailed(error) => {
            state.notify(StatusMessage::Error(format!(
                "Could not save filter change: {error}"
            )));
            Effect::None
        }
        Message::FilterReordered(tab_idx, id, target) => {
            let Some(tab) = state.tabs.get_mut(tab_idx) else {
                return Effect::None;
            };
            let swaps = tab.log_manager.reorder_filter_local(id, target);
            if swaps.is_empty() {
                return Effect::None;
            }
            force_recompute(tab);
            Effect::PersistFilterEdit(crate::gui::effect::FilterEdit::Reorder {
                db: tab.log_manager.db.clone(),
                order: tab.log_manager.get_filters().iter().map(|f| f.id).collect(),
            })
        }
        Message::FilterToggled(tab_idx, id) => {
            let Some(tab) = state.tabs.get_mut(tab_idx) else {
                return Effect::None;
            };
            if !tab.log_manager.toggle_filter_local(id) {
                return Effect::None;
            }
            force_recompute(tab);
            Effect::PersistFilterEdit(crate::gui::effect::FilterEdit::Toggle {
                db: tab.log_manager.db.clone(),
                id,
                enabled: tab
                    .log_manager
                    .get_filters()
                    .iter()
                    .find(|f| f.id == id)
                    .unwrap()
                    .enabled,
            })
        }
        Message::FiltersMutated(tab_idx, log_manager) => {
            apply_log_manager(state, tab_idx, log_manager);
            Effect::None
        }
        Message::GroupToggled(tab_idx, name) => {
            let Some(tab) = state.tabs.get_mut(tab_idx) else {
                return Effect::None;
            };
            let enabled = tab.log_manager.toggle_group_local(&name);
            force_recompute(tab);
            Effect::PersistFilterEdit(crate::gui::effect::FilterEdit::Group {
                db: tab.log_manager.db.clone(),
                source: tab.log_manager.source_file().unwrap_or("").to_string(),
                name,
                enabled,
            })
        }
        Message::GroupDeleted(tab_idx, name) => match state.tabs.get(tab_idx) {
            Some(tab) => Effect::RemoveGroup {
                tab_idx,
                log_manager: tab.log_manager.clone(),
                name,
            },
            None => Effect::None,
        },
        Message::GroupsMutated(tab_idx, log_manager) => {
            apply_log_manager(state, tab_idx, log_manager);
            Effect::None
        }
        Message::NavPageSelected(page) => {
            if page == crate::gui::state::NavPage::Settings {
                return crate::gui::actions::open(
                    state,
                    crate::gui::actions::Action::Settings,
                    None,
                    None,
                );
            }
            crate::gui::view::bookmarks::set_view(state, page);
            Effect::None
        }
        Message::TimeRangeToggled => {
            if state.editor.is_some() || state.palette.visible() {
                return Effect::None;
            }
            state.time_range_open = !state.time_range_open;
            Effect::None
        }
        Message::TimeRangeSelected(preset) => {
            if state.palette.pending() {
                return Effect::None;
            }
            state.time_range_open = false;
            state.time_range_preset = preset;
            apply_time_range_preset(state, preset)
        }
        Message::StreamToggled(tab_idx) => {
            let cmd = match state.tabs.get(tab_idx) {
                Some(tab) if tab.stream.paused => "resume",
                Some(_) => "pause",
                None => return Effect::None,
            };
            state.active_tab = tab_idx;
            perform_gui(
                state,
                if cmd == "resume" {
                    Commands::Resume
                } else {
                    Commands::Pause
                },
                "Live stream".into(),
            )
        }
        Message::SidebarTabSelected(tab) => {
            state.sidebar_tab = tab;
            state.keyboard_sidebar = Some(tab);
            state.sidebar_cursor = 0;
            Effect::None
        }
        Message::FacetBrowserToggled => {
            state.facet_browser_open = !state.facet_browser_open;
            Effect::None
        }
        Message::SidebarSideToggled => {
            let source = state
                .editor
                .as_ref()
                .and_then(|e| e.source)
                .or_else(|| state.active_tab().map(|t| t.id));
            if let Some(tab) = state.tabs.iter_mut().find(|t| Some(t.id) == source) {
                tab.display.sidebar_side = if tab.display.sidebar_side.is_left() {
                    crate::shared::SidebarSide::Right
                } else {
                    crate::shared::SidebarSide::Left
                };
                tab.display.show_sidebar = true;
            }
            Effect::None
        }
        Message::SidebarToggled => {
            toggle_display(state, |d| d.show_sidebar = !d.show_sidebar);
            Effect::None
        }
        Message::ClearAllFilters(tab_idx) => {
            state.active_tab = tab_idx;
            perform_gui(state, Commands::ClearFilters, "Clear filters".into())
        }
        Message::FilterRemoved(tab_idx, id) => match state.tabs.get(tab_idx) {
            Some(tab) => Effect::RemoveFilter {
                tab_idx,
                log_manager: tab.log_manager.clone(),
                id,
            },
            None => Effect::None,
        },
        Message::FacetToggled(field) => {
            let expanded = state.facet_expanded.entry(field).or_insert(false);
            *expanded = !*expanded;
            Effect::None
        }
        Message::FacetValueToggled(tab_idx, field, value) => {
            state.active_tab = tab_idx;
            let Some(tab) = state.tabs.get(tab_idx) else {
                return Effect::None;
            };
            match filter_id_for_field_equality(tab.log_manager.get_filters(), &field, &value) {
                Some(id) => Effect::RemoveFilter {
                    tab_idx,
                    log_manager: tab.log_manager.clone(),
                    id,
                },
                None => perform_gui(
                    state,
                    Commands::Filter {
                        pattern: vec![],
                        field: vec![format!("{field}={value}")],
                        fg: None,
                        bg: None,
                        line_mode: false,
                        regex: false,
                        ignore_case: false,
                        group: None,
                        auto: false,
                        fga: false,
                    },
                    "Add field filter".into(),
                ),
            }
        }
        Message::ColumnResizeMoved(column, mouse_x) => {
            if let Some(last_x) = state.column_resize_last_x {
                if let crate::gui::column_widths::ResizableColumn::Field(field, initial) = column {
                    if let Some(tab) = state.active_tab() {
                        let format = crate::gui::column_widths::format_key(tab);
                        let width = state
                            .preferences
                            .column_widths
                            .entry(format)
                            .or_default()
                            .entry(field)
                            .or_insert(initial);
                        let next = *width + mouse_x - last_x;
                        if next.is_finite() {
                            *width = next.max(crate::gui::column_widths::MIN_COLUMN_WIDTH_PX);
                        }
                    }
                } else {
                    crate::gui::column_widths::apply_width_delta(
                        &mut state.column_widths,
                        column,
                        mouse_x - last_x,
                    );
                }
            }
            state.column_resize_last_x = Some(mouse_x);
            Effect::None
        }
        Message::ColumnResizeEnded => {
            state.column_resize_last_x = None;
            Effect::None
        }
    }
}

/// Finds the filter that's an exact `--field name=value` equality match
/// for `field`/`value` (no extra `--field` conditions, no trailing free
/// text) — used to read a facet checkbox's checked state and to decide
/// whether toggling it should add or remove that filter. Decodes the
/// `@field:`-prefixed stored pattern via the existing field-filter parser
/// rather than string-matching it.
pub fn filter_id_for_field_equality(
    filter_defs: &[FilterDef],
    field: &str,
    value: &str,
) -> Option<usize> {
    filter_defs.iter().find_map(|def| {
        let expr = def.pattern.strip_prefix(FIELD_PREFIX)?;
        let (conditions, text) = parse_field_filter_expr(expr).ok()?;
        if text.is_some() {
            return None;
        }
        match conditions.as_slice() {
            [(k, v)] if k == field && v == value => Some(def.id),
            _ => None,
        }
    })
}

/// Applies a time-range preset: runs the real `:date-filter` command for
/// every preset except `AllTime`, which instead clears the date filter —
/// but only when it's the *only* active filter, so switching to "All
/// time" can never silently drop unrelated filters the user still wants.
/// With other filters present, this just points the user at the Filters
/// panel instead of guessing what they meant.
fn apply_time_range_preset(
    state: &mut GuiState,
    preset: crate::gui::time_range::TimeRangePreset,
) -> Effect {
    if state.palette.pending() {
        return Effect::None;
    }
    let Some(tab) = state.active_tab() else {
        return Effect::None;
    };
    let expression =
        crate::gui::time_range::date_filter_expression(preset, time::OffsetDateTime::now_utc());
    if let Some(expr) = expression {
        return perform_gui(
            state,
            Commands::DateFilter {
                expr: vec![expr],
                fg: None,
                bg: None,
                line_mode: false,
            },
            "Time range".into(),
        );
    }
    if !tab
        .log_manager
        .get_filters()
        .iter()
        .any(|f| f.pattern.starts_with(DATE_PREFIX))
    {
        return Effect::None;
    }
    let tab_id = tab.id;
    let log_manager = tab.log_manager.clone();
    gui_outcome(
        state,
        "All time".into(),
        crate::gui::palette::ExecutionState::Running,
        "Removing time restriction…".into(),
    );
    Effect::ClearGuiDates {
        request_id: state.palette.next_request,
        tab_id,
        log_manager,
    }
}

fn open_palette(state: &mut GuiState) -> Effect {
    state.palette.open = true;
    crate::gui::actions::select_best(state);
    state.palette.focus_epoch = state.palette.focus_epoch.wrapping_add(1);
    state.palette.generation = state.palette.generation.wrapping_add(1);
    Effect::None
}

pub fn gui_outcome(
    state: &mut GuiState,
    label: String,
    status: crate::gui::palette::ExecutionState,
    output: String,
) {
    if status != crate::gui::palette::ExecutionState::Running {
        state.notify(if status == crate::gui::palette::ExecutionState::Error {
            StatusMessage::Error(output.clone())
        } else {
            StatusMessage::Info(output.clone())
        });
    }
    state.palette.next_request = state.palette.next_request.wrapping_add(1);
    state.palette.outcome = Some(crate::gui::palette::Outcome {
        request_id: state.palette.next_request,
        invocation: label,
        source: state
            .editor
            .as_ref()
            .map(|e| e.source)
            .unwrap_or_else(|| state.active_tab().map(|t| t.id)),
        source_label: state
            .editor
            .as_ref()
            .and_then(|e| e.source)
            .or_else(|| {
                if state.editor.is_none() {
                    state.active_tab().map(|t| t.id)
                } else {
                    None
                }
            })
            .and_then(|id| state.tabs.iter().find(|t| t.id == id))
            .map(|t| t.title.clone()),
        state: status,
        output,
        validation: false,
    });
}
pub fn perform_gui(state: &mut GuiState, command: Commands, label: String) -> Effect {
    use crate::gui::palette::ExecutionState;
    if state.palette.pending() {
        return Effect::None;
    }
    gui_outcome(state, label, ExecutionState::Running, "Running…".into());
    let request_id = state.palette.next_request;
    if let Commands::SetTheme { theme_name } = &command {
        if let Err(error) = set_theme_by_name(state, theme_name) {
            if let Some(e) = &mut state.editor {
                e.error = Some(error.clone());
            }
            finish_palette(state, request_id, ExecutionState::Error, error);
            return Effect::None;
        }
        return Effect::SaveGuiTheme {
            request_id,
            name: theme_name.clone(),
            db: state.db.clone(),
        };
    }
    if let Commands::Schema { name: Some(name) } = &command
        && let Some(tab) = state.active_tab()
    {
        return Effect::PrepareSchema {
            request_id,
            tab_id: tab.id,
            name: name.clone(),
        };
    }
    let result = match &command {
        Commands::SetTheme { theme_name } => Some(set_theme_by_name(state, theme_name)),
        Commands::Schema { .. } => Some(report_schema(state)),
        Commands::Path => Some(report_source_path(state)),
        _ => state.active_tab_mut().and_then(|tab| {
            if apply_stream_command(tab, &command) || apply_display_command(tab, &command) {
                Some(Ok(command_summary(&command, tab)))
            } else {
                None
            }
        }),
    };
    if let Some(result) = result {
        match result {
            Ok(output) => finish_palette(state, request_id, ExecutionState::Success, output),
            Err(error) => {
                if let Some(e) = &mut state.editor {
                    e.error = Some(error.clone());
                }
                finish_palette(state, request_id, ExecutionState::Error, error)
            }
        }
        return Effect::None;
    }
    let Some(tab) = state.active_tab() else {
        finish_palette(
            state,
            request_id,
            ExecutionState::Error,
            "Open a log tab first.".into(),
        );
        return Effect::None;
    };
    Effect::ExecutePalette {
        request_id,
        tab_id: tab.id,
        log_manager: tab.log_manager.clone(),
        command,
        theme_bg: crate::theme::color_to_rgb(state.theme.root_bg),
        filter_context: None,
    }
}
pub(super) fn finish_palette(
    state: &mut GuiState,
    request_id: u64,
    status: crate::gui::palette::ExecutionState,
    output: String,
) {
    if let Some(outcome) = &mut state.palette.outcome
        && outcome.request_id == request_id
    {
        outcome.state = status;
        outcome.output = output.clone();
        state.notify(if status == crate::gui::palette::ExecutionState::Error {
            StatusMessage::Error(output)
        } else {
            StatusMessage::Info(output)
        });
    }
}
fn command_summary(command: &Commands, tab: &TabState) -> String {
    match command {
        Commands::Wrap => format!(
            "Line wrapping {}.",
            if tab.display.wrap {
                "enabled"
            } else {
                "disabled"
            }
        ),
        Commands::LineNumbers => format!(
            "Line numbers {}.",
            if tab.display.show_line_numbers {
                "shown"
            } else {
                "hidden"
            }
        ),
        Commands::RelativeLineNumbers => format!(
            "Relative line numbers {}.",
            if tab.display.relative_line_numbers {
                "enabled"
            } else {
                "disabled"
            }
        ),
        Commands::ShowKeys => "Field keys shown.".into(),
        Commands::HideKeys => "Field keys hidden.".into(),
        Commands::Raw => format!(
            "Raw display {}.",
            if tab.display.raw_mode {
                "enabled"
            } else {
                "disabled"
            }
        ),
        Commands::Collapse => "Continuation lines collapsed.".into(),
        Commands::Expand => "Continuation lines expanded.".into(),
        Commands::SidebarPosition { side } => format!("Sidebar moved to {side:?}."),
        Commands::Pause => "Live stream paused.".into(),
        Commands::Resume => "Live stream resumed.".into(),
        Commands::Stop => "File watching stopped.".into(),
        Commands::Tail => format!(
            "Tail mode {}.",
            if tab.stream.tail_mode {
                "enabled"
            } else {
                "disabled"
            }
        ),
        _ => "Action completed.".into(),
    }
}
fn palette_executed(
    state: &mut GuiState,
    request_id: u64,
    source: crate::shared::TabId,
    result: Result<LogManager, String>,
) -> Effect {
    use crate::gui::palette::ExecutionState;
    if !state.palette.outcome.as_ref().is_some_and(|o| {
        o.request_id == request_id && o.source == Some(source) && o.state == ExecutionState::Running
    }) {
        return Effect::None;
    }
    // Stable identity prevents a closed tab's result landing in a reused index.
    match result {
        Ok(manager) => {
            let Some(index) = state.tabs.iter().position(|tab| tab.id == source) else {
                finish_palette(
                    state,
                    request_id,
                    ExecutionState::Information,
                    "Command completed; its source tab has been closed.".into(),
                );
                return Effect::None;
            };
            apply_log_manager(state, index, manager);
            let invocation = state
                .palette
                .outcome
                .as_ref()
                .filter(|o| o.request_id == request_id)
                .map(|o| o.invocation.clone())
                .unwrap_or_default();
            let output = match invocation.split_whitespace().next().unwrap_or("") {
                "filter" => "Include filter added.",
                "exclude" => "Exclude filter added.",
                "highlight" | "h" => "Highlight filter added.",
                "clear-filters" => "All filters removed.",
                "disable-filters" => "All filters disabled.",
                "enable-filters" => "All filters enabled.",
                "group" => "Group style updated.",
                "toggle-group" => "Group filters toggled.",
                "date-filter" => "Date filter added.",
                "set-color" => "Filter color updated.",
                _ => "Changes applied.",
            };
            finish_palette(state, request_id, ExecutionState::Success, output.into());
        }
        Err(error) => {
            if let Some(e) = &mut state.editor {
                e.error = Some(error.clone());
            }
            finish_palette(state, request_id, ExecutionState::Error, error)
        }
    }
    Effect::None
}

/// Mirrors the TUI's `App::cmd_set_theme` (`src/ui/commands/display.rs`)
/// for `:set-theme <name>` — unlike `apply_theme_by_name` (used for the
/// `:theme` picker's silent-on-failure live preview, where a bad name
/// should be unreachable since it only ever comes from `Theme::
/// list_available_themes`), a user-typed `:set-theme` name can easily be
/// wrong, so a load failure surfaces as a status message instead of
/// silently doing nothing. Doesn't persist to `AppSettingsStore` — the
/// same deliberate simplification as `apply_display_command`.
fn set_theme_by_name(state: &mut GuiState, theme_name: &str) -> Result<String, String> {
    let theme_filename = format!("{}.json", theme_name.to_lowercase());
    let theme = crate::theme::Theme::from_file(&theme_filename)
        .map_err(|e| format!("Failed to load theme '{theme_name}': {e}"))?;
    state.theme = theme;
    Ok(format!("Theme changed to {theme_name}."))
}

fn report_schema(state: &mut GuiState) -> Result<String, String> {
    let tab = state
        .active_tab()
        .ok_or("Open a log tab to use this command.")?;
    let name = tab
        .display
        .format
        .as_deref()
        .map(|format| format.name())
        .unwrap_or("none");
    Ok(format!("active schema: {name}"))
}
/// Mirrors the TUI's `App::cmd_path`: reports the active tab's source file
/// path (or a placeholder for a tab with none, e.g. stdin) via
/// the GUI result and temporary notification rather than TUI notification state.
fn report_source_path(state: &mut GuiState) -> Result<String, String> {
    let Some(tab) = state.active_tab() else {
        return Err("Open a log tab to use this command.".into());
    };
    let message = tab
        .display_source_path()
        .map(str::to_string)
        .unwrap_or_else(|| "This tab has no source file (e.g. stdin).".to_string());
    Ok(message)
}

fn toggle_display(state: &mut GuiState, f: impl FnOnce(&mut crate::shared::DisplayConfig)) {
    if let Some(tab) = state.active_tab_mut() {
        f(&mut tab.display);
    }
}

/// Handles `Commands::Pause`/`Resume`/`Stop`/`Tail` directly on
/// `tab.stream`, mirroring the TUI's `cmd_pause`/`cmd_resume`/`cmd_stop`/
/// `cmd_tail` (`src/ui/commands/stream.rs`) — these never touch
/// `LogManager`, so they skip the async `Effect::ExecuteCommand` round
/// trip entirely. `Tail` jumping to the last line is a one-time effect;
/// actually following new lines as they arrive needs live-tail growth
/// consumption, which the GUI doesn't have yet (see `GUI_PARITY_BACKLOG.md`).
/// Returns `true` when `command` was one of these and has been applied.
fn apply_stream_command(tab: &mut TabState, command: &Commands) -> bool {
    match command {
        Commands::Pause => tab.stream.paused = true,
        Commands::Resume => tab.stream.paused = false,
        Commands::Stop => {
            tab.stream.watch = None;
            if let Some(merged) = &mut tab.merged {
                merged.stopped = true;
            }
        }
        Commands::Tail => {
            tab.stream.tail_mode = !tab.stream.tail_mode;
            if tab.stream.tail_mode {
                let last = tab.filter.visible_indices.len().saturating_sub(1);
                tab.scroll.scroll_offset = last;
            }
        }
        _ => return false,
    }
    true
}

/// Handles the display-toggle commands directly on `tab.display`,
/// mirroring the TUI's `cmd_wrap`/`cmd_line_numbers`/etc. (`src/ui/
/// commands/display.rs`) for the active tab only — these never touch
/// `LogManager`, so (like `apply_stream_command`) they skip the async
/// `Effect::ExecuteCommand` round trip entirely. Returns `true` when
/// `command` was one of these and has been applied.
///
/// Deliberate simplification vs. the TUI: `Wrap`/`LineNumbers`/
/// `RelativeLineNumbers`/`Collapse`/`Expand` are app-wide settings in the
/// TUI (broadcast to every open tab and persisted to
/// `AppSettingsStore`) — the GUI applies them to the active tab only and
/// doesn't persist them, since `GuiState` doesn't load persisted display
/// settings on startup either. `ShowKeys`/`HideKeys`/`Raw` were already
/// active-tab-only, app-wide-setting-free commands in the TUI itself, so
/// those match exactly.
fn apply_display_command(tab: &mut TabState, command: &Commands) -> bool {
    match command {
        Commands::Wrap => tab.display.wrap = !tab.display.wrap,
        Commands::LineNumbers => tab.display.show_line_numbers = !tab.display.show_line_numbers,
        Commands::RelativeLineNumbers => {
            tab.display.relative_line_numbers = !tab.display.relative_line_numbers;
        }
        Commands::ShowKeys => {
            tab.display.show_keys = true;
            tab.invalidate_parse_cache();
        }
        Commands::HideKeys => {
            tab.display.show_keys = false;
            tab.invalidate_parse_cache();
        }
        Commands::Raw => {
            tab.display.raw_mode = !tab.display.raw_mode;
            force_recompute(tab);
        }
        Commands::Collapse => set_collapse_continuations(tab, true),
        Commands::Expand => set_collapse_continuations(tab, false),
        Commands::SidebarPosition { side } => tab.display.sidebar_side = *side,
        _ => return false,
    }
    true
}

fn set_collapse_continuations(tab: &mut TabState, enabled: bool) {
    tab.display.collapse_continuations = enabled;
    tab.filter.overridden_groups.clear();
    tab.begin_filter_refresh_background();
}

/// Whether `tab` is actively tailing its source: a watcher is running and
/// hasn't been paused. Feeds the search bar's "Live" dot/toggle.
pub fn is_tab_live(tab: &TabState) -> bool {
    (tab.stream.watch.is_some()
        || tab
            .merged
            .as_ref()
            .is_some_and(|m| !m.stopped && !m.source_tab_ids.is_empty()))
        && !tab.stream.paused
}

fn close_tab_at(state: &mut GuiState, idx: usize) {
    if idx < state.tabs.len() {
        let id = state.tabs[idx].id;
        state.history.remove(&id);
        state.tabs.remove(idx);
        state.active_tab = active_after_close(state.active_tab, idx, state.tabs.len());
    }
}

/// Which tab should be active after closing tab `closed`, given `active`
/// was active before the close and `len_after` tabs remain.
fn active_after_close(active: usize, closed: usize, len_after: usize) -> usize {
    if len_after == 0 {
        0
    } else if closed < active {
        active - 1
    } else {
        active.min(len_after - 1)
    }
}

/// Cycles the active tab index by `delta` (+1/-1 for Tab/Shift+Tab),
/// wrapping around `len` tabs.
#[cfg(test)]
fn next_active_tab(active: usize, delta: isize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let len = len as isize;
    (((active as isize + delta) % len) + len) as usize % len as usize
}

fn push_tab(state: &mut GuiState, loaded: FileLoaded) {
    if let Ok((tab, widths)) = prepare_tab(loaded, &state.preferences, &AtomicBool::new(false)) {
        state.column_widths = widths;
        state.tabs.push(tab);
        state.active_tab = state.tabs.len() - 1;
    }
}

pub(super) fn prepare_tab(
    loaded: FileLoaded,
    preferences: &crate::gui::persistence::Preferences,
    cancel: &AtomicBool,
) -> Result<(TabState, crate::gui::column_widths::ColumnWidths), String> {
    prepare_tab_with_merge(loaded, preferences, cancel, None)
}

pub(super) fn prepare_tab_with_merge(
    loaded: FileLoaded,
    preferences: &crate::gui::persistence::Preferences,
    cancel: &AtomicBool,
    merged: Option<crate::shared::MergedState>,
) -> Result<(TabState, crate::gui::column_widths::ColumnWidths), String> {
    let title = loaded
        .path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| loaded.path.display().to_string());
    let mut tab = TabState::new_controlled(loaded.reader, loaded.log_manager, title, cancel)
        .ok_or("Operation canceled")?;
    tab.interaction.keybindings = std::sync::Arc::new(
        crate::config::Config::load()
            .unwrap_or_default()
            .keybindings,
    );
    tab.display.hidden_fields.clear();
    preferences.apply(&mut tab);
    tab.stream.watch = Some(loaded.watch);
    if let Some(merged) = merged {
        tab.display.format = None;
        tab.continuation_map = None;
        tab.year_map = None;
        tab.stream.watch = None;
        tab.merged = Some(merged);
    }
    if let Some(context) = &loaded.context {
        tab.apply_file_context(context);
    }
    let annotated_lines = tab
        .comment_manager
        .get()
        .iter()
        .flat_map(|note| note.line_indices.iter().copied())
        .collect::<Vec<_>>();
    tab.mark_lines(&annotated_lines);
    tab.prepare_visible_controlled(cancel);
    if let Some(context) = &loaded.context {
        if let Some(handle) = &mut tab.filter.handle {
            handle.scroll_anchor = Some(context.scroll_offset);
        } else {
            tab.restore_scroll_to_line(Some(context.scroll_offset));
        }
        if !context.search_query.is_empty() {
            tab.begin_search(&context.search_query, true, false);
        }
    }
    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
        return Err("Operation canceled".into());
    }
    let widths = crate::gui::column_widths::fit_column_widths(&tab);
    Ok((tab, widths))
}

/// Applies a new `log_manager` (from a filter/group mutation) and re-runs
/// the tab's filters. Arranges for `log_selection` to be remapped across
/// the resulting `visible_indices` change the same way
/// `TabState::restore_scroll_to_line` re-anchors the scroll cursor —
/// otherwise a selection taken before the mutation (e.g. the row a "Filter
/// out before/after this" context-menu action was invoked on) keeps its
/// old visible *position*, which a filter-driven reshuffle of
/// `visible_indices` can repoint at a different line.
///
/// The rescan `force_recompute` starts is itself background work (see
/// `TabState::begin_filter_refresh`), so `visible_indices` is not yet
/// final when this function returns — remapping now would just read the
/// *pre*-mutation mapping. The selection's underlying file lines are
/// stashed in `pending_selection_anchor` and resolved once the scan
/// completes (immediately below, if it already has; otherwise by
/// `resolve_pending_selection` on a later poll).
fn apply_log_manager(state: &mut GuiState, tab_idx: usize, log_manager: LogManager) {
    let Some(tab) = state.tabs.get_mut(tab_idx) else {
        return;
    };
    let tab_id = tab.id;
    let anchor = state
        .log_selection
        .filter(|(id, _, _)| *id == tab_id)
        .map(|(_, row_a, row_b)| {
            (
                (row_a, row_b),
                tab.filter.visible_indices.get_opt(row_a),
                tab.filter.visible_indices.get_opt(row_b),
            )
        });
    tab.log_manager = log_manager;
    force_recompute(tab);
    if let Some((old_rows, line_a, line_b)) = anchor {
        state.pending_selection_anchor = Some((tab_id, old_rows, line_a, line_b));
        resolve_pending_selection(state);
    }
}

/// Remaps a pending selection anchor (see `apply_log_manager`) back into
/// `log_selection` once its tab's filter scan has finished. A no-op while
/// the scan is still running, or once resolved. Leaves `log_selection`
/// alone if it no longer matches the exact value the anchor was captured
/// from — the user selected something else in the meantime, so the stale
/// anchor must not clobber it.
pub fn resolve_pending_selection(state: &mut GuiState) {
    let Some((tab_id, old_rows, line_a, line_b)) = state.pending_selection_anchor else {
        return;
    };
    let Some(tab) = state.tabs.iter().find(|t| t.id == tab_id) else {
        state.pending_selection_anchor = None;
        return;
    };
    if tab.filter.handle.is_some() {
        return;
    }
    state.pending_selection_anchor = None;
    if state.log_selection != Some((tab_id, old_rows.0, old_rows.1)) {
        return;
    }
    state.log_selection = line_a.zip(line_b).and_then(|(a, b)| {
        tab.filter
            .visible_indices
            .nearest_position_of(a)
            .zip(tab.filter.visible_indices.nearest_position_of(b))
            .map(|(pa, pb)| (tab_id, pa, pb))
    });
}

/// Start a cancellable scan; the GUI polls streamed results on its frame timer.
fn force_recompute(tab: &mut TabState) {
    let handle = runtime::handle();
    let _guard = handle.enter();
    tab.begin_filter_refresh_background();
}

// Mode handlers already start background scans. Keep their handle alive.

/// Loads `path` into a fresh `FileReader` + `LogManager` pair. Framework-
/// neutral async work the glue layer runs under its tokio bridge in
/// response to `Effect::LoadFile`; `push_tab` turns the result into a
/// full `TabState` (format detection, continuation/year maps, etc.) via
/// the TUI's own `TabState::new`.
pub async fn load_file(path: PathBuf, db: Arc<Database>) -> Result<FileLoaded, String> {
    let (control, _rx) = crate::ingestion::archive_job::Control::new();
    load_file_controlled(path, db, control, None).await
}

/// Same bounded head preview used by the TUI; never waits for the full index.
pub(super) async fn load_file_preview(
    path: &std::path::Path,
    db: Arc<Database>,
    control: &crate::ingestion::archive_job::Control,
    preview_bytes: u64,
) -> Result<Option<FileLoaded>, String> {
    control.check().map_err(|e| e.to_string())?;
    let path = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
    if std::fs::metadata(&path).map_err(|e| e.to_string())?.len() <= preview_bytes {
        return Ok(None);
    }
    let source = path.to_string_lossy().into_owned();
    let reader = FileReader::from_file_head(&source, preview_bytes)
        .await
        .map_err(|e| e.to_string())?;
    if reader.line_count() == 0 {
        return Ok(None);
    }
    use crate::db::FileContextStore;
    let context = db
        .load_file_context(&source)
        .await
        .map_err(|e| e.to_string())?;
    let manager = LogManager::new(db, Some(source.clone())).await;
    let (_, rx) = tokio::sync::watch::channel(());
    Ok(Some(FileLoaded {
        path,
        context,
        reader,
        log_manager: manager,
        watch: crate::shared::watch_state_from_file(rx, source),
    }))
}

pub(super) async fn load_file_controlled(
    path: PathBuf,
    db: Arc<Database>,
    control: crate::ingestion::archive_job::Control,
    source: Option<String>,
) -> Result<FileLoaded, String> {
    control.check().map_err(|e| e.to_string())?;
    let path = std::fs::canonicalize(&path).map_err(|e| e.to_string())?;
    let path_str = path.to_string_lossy().into_owned();
    let mut handle = FileReader::load(path_str.clone(), None, false, control.cancel.clone(), false)
        .await
        .map_err(|e| e.to_string())?;
    let total_bytes = handle.total_bytes;
    control.phase(
        "Loading",
        source.clone().unwrap_or_else(|| path_str.clone()),
        Some(total_bytes),
    );
    let result = loop {
        tokio::select! {
            result = &mut handle.result_rx => break result.map_err(|_| "File load worker stopped".to_string())?.map_err(|e| e.to_string())?,
            _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                let fraction = *handle.progress_rx.borrow_and_update();
                control.bytes((fraction * total_bytes as f64) as u64);
            }
        }
    };
    control.check().map_err(|e| e.to_string())?;
    control.bytes(total_bytes);
    use crate::db::FileContextStore;
    let provenance = source.clone().unwrap_or_else(|| path_str.clone());
    let context = db
        .load_file_context(&provenance)
        .await
        .map_err(|e| e.to_string())?;
    let log_manager = LogManager::new(db, Some(provenance)).await;
    let watch_rx = if source.is_some() {
        tokio::sync::watch::channel(()).1
    } else {
        FileReader::spawn_file_watcher(path_str.clone(), total_bytes).await
    };
    let watch = crate::shared::watch_state_from_file(watch_rx, path_str);
    let path = source.map(PathBuf::from).unwrap_or(path);
    Ok(FileLoaded {
        path,
        context,
        reader: result.reader,
        log_manager,
        watch,
    })
}

/// A group's checkbox toggle in the sidebar: like the TUI's keyboard-driven
/// `GroupManagementMode::toggle_group`, this flips "any member enabled" for
/// a group with filters, or the group's own stored flag for a styled-but-
/// empty group — unlike the `:toggle-group` *command*, it never errors.
pub async fn toggle_group_checkbox(log_manager: &mut LogManager, name: &str) {
    let has_filters = log_manager
        .get_filters()
        .iter()
        .any(|f| f.group.as_deref() == Some(name));
    let currently_enabled = if has_filters {
        log_manager
            .get_filters()
            .iter()
            .any(|f| f.group.as_deref() == Some(name) && f.enabled)
    } else {
        group_enabled(log_manager.get_group_styles(), name)
    };
    let new_state = !currently_enabled;
    log_manager
        .set_filters_enabled_by_group(name, new_state)
        .await;
    log_manager.set_group_enabled(name, new_state).await;
}

/// Parses command-bar input the same way the TUI's `:` bar does — same
/// clap grammar (`CommandLine`), same shell-style tokenizing.
#[cfg(test)]
pub fn parse_command(input: &str) -> Result<Option<Commands>, String> {
    CommandLine::try_parse_from(shell_split(input))
        .map(|line| line.command)
        .map_err(|e| e.to_string())
}

/// Runs one `Commands` against `log_manager`, mutating and returning it —
/// mirrors the TUI's `App::run_command` dispatch for the subset of
/// commands this GUI supports. A command it doesn't implement yet
/// (structured-field filtering, auto colors, every other `:` command)
/// errors rather than silently doing nothing.
pub async fn execute_command(
    mut log_manager: LogManager,
    command: Commands,
    theme_bg: (u8, u8, u8),
    filter_context: Option<usize>,
) -> Result<LogManager, String> {
    match command {
        Commands::Filter {
            pattern,
            fg,
            bg,
            line_mode,
            field,
            regex,
            ignore_case,
            group,
            auto,
            fga,
        } => {
            let (fg, bg) =
                crate::shared::command_helpers::resolve_colors(auto, fga, fg, bg, theme_bg)?;
            let options = filter_options(fg, bg, line_mode, regex, ignore_case, group);
            let stored_pattern = stored_filter_pattern(&field, &pattern.join(" "))?;
            log_manager
                .add_filter_with_color(stored_pattern, FilterType::Include, options)
                .await;
        }
        Commands::Exclude {
            pattern,
            field,
            regex,
            ignore_case,
            group,
        } => {
            let options = filter_options(None, None, false, regex, ignore_case, group);
            let stored_pattern = stored_filter_pattern(&field, &pattern.join(" "))?;
            log_manager
                .add_filter_with_color(stored_pattern, FilterType::Exclude, options)
                .await;
        }
        Commands::Highlight {
            pattern,
            fg,
            bg,
            line_mode,
            field,
            regex,
            ignore_case,
            group,
            auto,
            fga,
        } => {
            let (fg, bg) =
                crate::shared::command_helpers::resolve_colors(auto, fga, fg, bg, theme_bg)?;
            let options = filter_options(fg, bg, line_mode, regex, ignore_case, group);
            let stored_pattern = stored_filter_pattern(&field, &pattern.join(" "))?;
            log_manager
                .add_filter_with_color(stored_pattern, FilterType::Highlight, options)
                .await;
        }
        Commands::SaveFilters { path } => {
            log_manager
                .save_filters(&crate::commands::auto_complete::expand_tilde(&path))
                .map_err(|e| e.to_string())?;
        }
        Commands::LoadFilters { path, append } => {
            log_manager
                .load_filters(&crate::commands::auto_complete::expand_tilde(&path), append)
                .await
                .map_err(|e| e.to_string())?;
        }
        Commands::ImportFilters { path, append } => {
            log_manager
                .import_npp_filters(&crate::commands::auto_complete::expand_tilde(&path), append)
                .await
                .map_err(|e| e.to_string())?;
        }
        Commands::ClearFilters => log_manager.clear_filters().await,
        Commands::DisableFilters => log_manager.disable_all_filters().await,
        Commands::EnableFilters => log_manager.enable_all_filters().await,
        Commands::ToggleGroup { name } => toggle_group_command(&mut log_manager, &name).await?,
        Commands::Group {
            name,
            fg,
            bg,
            line_mode,
            auto,
            clear,
        } => set_group_style(&mut log_manager, name, fg, bg, line_mode, auto, clear).await?,
        Commands::DateFilter {
            expr,
            fg,
            bg,
            line_mode,
        } => add_date_filter(&mut log_manager, expr, fg, bg, line_mode).await?,
        Commands::SetColor { fg, bg, line_mode } => {
            set_selected_filter_color(&mut log_manager, filter_context, fg, bg, line_mode).await;
        }
        other => return Err(format!("{other:?} is not yet supported in the GUI")),
    }
    Ok(log_manager)
}

/// The pattern stored on the new `FilterDef`: `pattern` verbatim when
/// there's no `--field` condition, or `pattern` AND'd with every `--field
/// key=value` via the same `@field:`-prefixed encoding the TUI's
/// `cmd_filter` uses — so a field filter added from the GUI (the facet
/// checkboxes) round-trips through `filter_id_for_field_equality` exactly
/// like one added from the TUI's command bar.
fn stored_filter_pattern(field: &[String], pattern: &str) -> Result<String, String> {
    if field.is_empty() {
        Ok(pattern.to_string())
    } else {
        crate::shared::command_helpers::build_field_filter_pattern(field, pattern)
    }
}

fn filter_options(
    fg: Option<String>,
    bg: Option<String>,
    line_mode: bool,
    regex: bool,
    ignore_case: bool,
    group: Option<String>,
) -> FilterOptions {
    let mut opts = FilterOptions::default();
    if line_mode {
        opts = opts.line_mode();
    }
    if regex {
        opts = opts.regex();
    }
    if ignore_case {
        opts = opts.ignore_case();
    }
    if let Some(c) = fg.as_deref() {
        opts = opts.fg(c);
    }
    if let Some(c) = bg.as_deref() {
        opts = opts.bg(c);
    }
    if let Some(g) = group.as_deref() {
        opts = opts.group(g);
    }
    opts
}

/// `:toggle-group` — if any member is currently enabled, disable the whole
/// group; otherwise enable it. Errors for a name with no member filters,
/// matching the TUI's `cmd_toggle_group`.
async fn toggle_group_command(log_manager: &mut LogManager, name: &str) -> Result<(), String> {
    let filters = log_manager.get_filters();
    if !filters.iter().any(|f| f.group.as_deref() == Some(name)) {
        return Err(format!("No such filter group: '{name}'"));
    }
    let any_enabled = filters
        .iter()
        .any(|f| f.group.as_deref() == Some(name) && f.enabled);
    log_manager
        .set_filters_enabled_by_group(name, !any_enabled)
        .await;
    Ok(())
}

/// `:group` — set/update a group's predefined style, or clear it with
/// `--clear` (mutually exclusive with every other flag).
async fn set_group_style(
    log_manager: &mut LogManager,
    name: String,
    fg: Option<String>,
    bg: Option<String>,
    line_mode: bool,
    auto: bool,
    clear: bool,
) -> Result<(), String> {
    if clear {
        if fg.is_some() || bg.is_some() || line_mode || auto {
            return Err("--clear cannot be combined with --fg/--bg/-l/--auto".to_string());
        }
        log_manager.clear_group_style(&name).await;
        return Ok(());
    }
    // Matches the TUI's `cmd_group`: `--auto` for a group style resolves
    // against a fixed black background, not the real theme background
    // (unlike `:filter --auto`/`--fga`, which use it) — not this GUI's
    // call to make differently.
    let (fg, bg) = crate::shared::command_helpers::resolve_colors(auto, false, fg, bg, (0, 0, 0))?;
    log_manager
        .set_group_style(&name, fg.as_deref(), bg.as_deref(), !line_mode)
        .await;
    Ok(())
}

/// `:date-filter` — validates the expression, then stores it as a
/// `DATE_PREFIX`-tagged Include filter, same encoding the TUI uses.
async fn add_date_filter(
    log_manager: &mut LogManager,
    expr: Vec<String>,
    fg: Option<String>,
    bg: Option<String>,
    line_mode: bool,
) -> Result<(), String> {
    let expression = expr.join(" ");
    parse_date_filter(&expression).map_err(|e| format!("Invalid date filter: {e}"))?;
    let pattern = format!("{DATE_PREFIX}{expression}");
    let options = filter_options(fg, bg, line_mode, false, false, None);
    log_manager
        .add_filter_with_color(pattern, FilterType::Include, options)
        .await;
    Ok(())
}

async fn set_selected_filter_color(
    log_manager: &mut LogManager,
    filter_context: Option<usize>,
    fg: Option<String>,
    bg: Option<String>,
    line_mode: bool,
) {
    let Some(selected) = filter_context else {
        return;
    };
    let Some(filter) = log_manager.get_filters().get(selected).cloned() else {
        return;
    };
    if !matches!(
        filter.filter_type,
        FilterType::Include | FilterType::Highlight
    ) {
        return;
    }
    let match_only = if line_mode {
        false
    } else {
        filter
            .color_config
            .as_ref()
            .map(|cc| cc.match_only)
            .unwrap_or(true)
    };
    log_manager
        .set_color_config(filter.id, fg.as_deref(), bg.as_deref(), match_only)
        .await;
}

#[cfg(test)]
fn handle_command_string(state: &mut GuiState, input: String) -> Effect {
    let command = match parse_command(&input) {
        Ok(Some(c)) => c,
        _ => return Effect::None,
    };
    let prior = state.status.clone();
    let effect = perform_gui(state, command, input.clone());
    if let Some(o) = &state.palette.outcome {
        state.status = match o.state {
            crate::gui::palette::ExecutionState::Error => {
                Some(StatusMessage::Error(o.output.clone()))
            }
            crate::gui::palette::ExecutionState::Success if input == "schema" => {
                Some(StatusMessage::Error(o.output.clone()))
            }
            crate::gui::palette::ExecutionState::Success if input == "path" => {
                Some(StatusMessage::Info(o.output.clone()))
            }
            _ => prior,
        };
    }
    effect
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::input::KeyModifiers;

    /// Arbitrary theme background for tests that don't care what `--auto`/
    /// `--fga` resolve against, only that a color comes out.
    const TEST_THEME_BG: (u8, u8, u8) = (30, 30, 30);

    async fn log_manager() -> LogManager {
        let db = Arc::new(Database::in_memory().await.unwrap());
        LogManager::new(db, None).await
    }

    fn command(input: &str) -> Commands {
        parse_command(input).unwrap().unwrap()
    }

    #[test]
    fn parse_command_parses_a_filter_command() {
        let parsed = parse_command("filter ERROR --fg red").unwrap().unwrap();
        assert!(matches!(parsed, Commands::Filter { .. }));
    }

    #[test]
    fn parse_command_rejects_invalid_input() {
        assert!(parse_command("not-a-real-command").is_err());
    }

    #[tokio::test]
    async fn execute_command_filter_adds_an_include_filter() {
        let lm = execute_command(
            log_manager().await,
            command("filter ERROR"),
            TEST_THEME_BG,
            None,
        )
        .await
        .unwrap();
        assert_eq!(lm.get_filters().len(), 1);
        assert_eq!(lm.get_filters()[0].filter_type, FilterType::Include);
        assert_eq!(lm.get_filters()[0].pattern, "ERROR");
    }

    #[tokio::test]
    async fn execute_command_exclude_adds_an_exclude_filter() {
        let lm = execute_command(
            log_manager().await,
            command("exclude DEBUG"),
            TEST_THEME_BG,
            None,
        )
        .await
        .unwrap();
        assert_eq!(lm.get_filters()[0].filter_type, FilterType::Exclude);
    }

    #[tokio::test]
    async fn execute_command_highlight_adds_a_highlight_filter() {
        let lm = execute_command(
            log_manager().await,
            command("highlight WARN"),
            TEST_THEME_BG,
            None,
        )
        .await
        .unwrap();
        assert_eq!(lm.get_filters()[0].filter_type, FilterType::Highlight);
    }

    #[tokio::test]
    async fn execute_command_accepts_field_filters() {
        let lm = execute_command(
            log_manager().await,
            command("filter --field level=error"),
            TEST_THEME_BG,
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            lm.get_filters()[0].pattern,
            format!("{FIELD_PREFIX}level:error")
        );
    }

    #[tokio::test]
    async fn execute_command_auto_assigns_a_readable_fg_bg_pair() {
        let lm = execute_command(
            log_manager().await,
            command("filter --auto ERROR"),
            TEST_THEME_BG,
            None,
        )
        .await
        .unwrap();
        let color_config = lm.get_filters()[0]
            .color_config
            .clone()
            .expect("--auto should assign a color_config");
        assert!(color_config.fg.is_some());
        assert!(color_config.bg.is_some());
    }

    #[tokio::test]
    async fn execute_command_fga_assigns_only_a_readable_fg() {
        let lm = execute_command(
            log_manager().await,
            command("filter --fga ERROR"),
            TEST_THEME_BG,
            None,
        )
        .await
        .unwrap();
        let color_config = lm.get_filters()[0]
            .color_config
            .clone()
            .expect("--fga should assign a color_config");
        assert!(color_config.fg.is_some());
        assert!(color_config.bg.is_none());
    }

    #[tokio::test]
    async fn execute_command_rejects_auto_combined_with_fga() {
        let err = execute_command(
            log_manager().await,
            command("filter --auto --fga ERROR"),
            TEST_THEME_BG,
            None,
        )
        .await
        .unwrap_err();
        assert!(err.contains("--auto"));
        assert!(err.contains("--fga"));
    }

    #[tokio::test]
    async fn execute_command_group_auto_assigns_a_readable_fg_bg_pair() {
        let lm = execute_command(
            log_manager().await,
            command("group net --auto"),
            TEST_THEME_BG,
            None,
        )
        .await
        .unwrap();
        let color_config = lm.get_group_styles()[0]
            .color_config
            .clone()
            .expect("--auto should assign a color_config");
        assert!(color_config.fg.is_some());
        assert!(color_config.bg.is_some());
    }

    #[tokio::test]
    async fn execute_command_clear_filters_removes_everything() {
        let lm = execute_command(
            log_manager().await,
            command("filter ERROR"),
            TEST_THEME_BG,
            None,
        )
        .await
        .unwrap();
        let lm = execute_command(lm, command("clear-filters"), TEST_THEME_BG, None)
            .await
            .unwrap();
        assert!(lm.get_filters().is_empty());
    }

    #[tokio::test]
    async fn execute_command_disable_then_enable_filters() {
        let lm = execute_command(
            log_manager().await,
            command("filter ERROR"),
            TEST_THEME_BG,
            None,
        )
        .await
        .unwrap();
        let lm = execute_command(lm, command("disable-filters"), TEST_THEME_BG, None)
            .await
            .unwrap();
        assert!(!lm.get_filters()[0].enabled);
        let lm = execute_command(lm, command("enable-filters"), TEST_THEME_BG, None)
            .await
            .unwrap();
        assert!(lm.get_filters()[0].enabled);
    }

    #[tokio::test]
    async fn execute_command_toggle_group_toggles_members_off_then_on() {
        let lm = execute_command(
            log_manager().await,
            command("filter -g net ERROR"),
            TEST_THEME_BG,
            None,
        )
        .await
        .unwrap();
        let lm = execute_command(lm, command("toggle-group net"), TEST_THEME_BG, None)
            .await
            .unwrap();
        assert!(!lm.get_filters()[0].enabled);
        let lm = execute_command(lm, command("toggle-group net"), TEST_THEME_BG, None)
            .await
            .unwrap();
        assert!(lm.get_filters()[0].enabled);
    }

    #[tokio::test]
    async fn execute_command_toggle_group_errors_for_an_unknown_group() {
        let err = execute_command(
            log_manager().await,
            command("toggle-group missing"),
            TEST_THEME_BG,
            None,
        )
        .await
        .unwrap_err();
        assert!(err.contains("missing"));
    }

    #[tokio::test]
    async fn execute_command_group_sets_a_predefined_style() {
        let lm = execute_command(
            log_manager().await,
            command("group net --fg red"),
            TEST_THEME_BG,
            None,
        )
        .await
        .unwrap();
        assert_eq!(lm.get_group_styles().len(), 1);
        assert_eq!(lm.get_group_styles()[0].name, "net");
    }

    #[tokio::test]
    async fn execute_command_group_clear_rejects_other_flags() {
        let err = execute_command(
            log_manager().await,
            command("group net --clear --fg red"),
            TEST_THEME_BG,
            None,
        )
        .await
        .unwrap_err();
        assert!(err.contains("--clear"));
    }

    #[tokio::test]
    async fn execute_command_date_filter_adds_a_tagged_include_filter() {
        let lm = execute_command(
            log_manager().await,
            command("date-filter > 2024-01-01"),
            TEST_THEME_BG,
            None,
        )
        .await
        .unwrap();
        assert!(lm.get_filters()[0].pattern.starts_with(DATE_PREFIX));
        assert_eq!(lm.get_filters()[0].filter_type, FilterType::Include);
    }

    #[tokio::test]
    async fn execute_command_date_filter_rejects_an_invalid_expression() {
        let err = execute_command(
            log_manager().await,
            command("date-filter not-a-date"),
            TEST_THEME_BG,
            None,
        )
        .await
        .unwrap_err();
        assert!(err.contains("Invalid date filter"));
    }

    #[tokio::test]
    async fn execute_command_rejects_an_unimplemented_command() {
        let err = execute_command(log_manager().await, command("wrap"), TEST_THEME_BG, None)
            .await
            .unwrap_err();
        assert!(err.contains("not yet supported"));
    }

    #[tokio::test]
    async fn set_color_recolors_the_filter_named_by_filter_context() {
        let mut lm = execute_command(
            log_manager().await,
            command("filter ERROR"),
            TEST_THEME_BG,
            None,
        )
        .await
        .unwrap();
        lm = execute_command(lm, command("filter WARN"), TEST_THEME_BG, None)
            .await
            .unwrap();
        // filter_context names a position in get_filters(), same as the
        // TUI's FilterManagementMode selection — recolor the second one.
        let lm = execute_command(
            lm,
            command("set-color --fg [255,0,0]"),
            TEST_THEME_BG,
            Some(1),
        )
        .await
        .unwrap();
        let recolored = &lm.get_filters()[1];
        assert_eq!(recolored.pattern, "WARN");
        assert_eq!(
            recolored.color_config.as_ref().and_then(|cc| cc.fg),
            Some(crate::style::Color::Rgb(255, 0, 0))
        );
        assert!(lm.get_filters()[0].color_config.is_none());
    }

    #[tokio::test]
    async fn set_color_is_a_no_op_without_a_filter_context() {
        let lm = execute_command(
            log_manager().await,
            command("filter ERROR"),
            TEST_THEME_BG,
            None,
        )
        .await
        .unwrap();
        let lm = execute_command(lm, command("set-color --fg [255,0,0]"), TEST_THEME_BG, None)
            .await
            .unwrap();
        assert!(lm.get_filters()[0].color_config.is_none());
    }

    #[tokio::test]
    async fn set_color_does_not_recolor_an_exclude_filter() {
        let lm = execute_command(
            log_manager().await,
            command("exclude DEBUG"),
            TEST_THEME_BG,
            None,
        )
        .await
        .unwrap();
        let lm = execute_command(
            lm,
            command("set-color --fg [255,0,0]"),
            TEST_THEME_BG,
            Some(0),
        )
        .await
        .unwrap();
        assert!(lm.get_filters()[0].color_config.is_none());
    }

    #[test]
    fn active_after_close_shifts_left_when_an_earlier_tab_closes() {
        assert_eq!(active_after_close(2, 0, 2), 1);
    }

    #[test]
    fn active_after_close_stays_put_when_a_later_tab_closes() {
        assert_eq!(active_after_close(0, 2, 2), 0);
    }

    #[test]
    fn active_after_close_clamps_to_the_last_remaining_tab() {
        assert_eq!(active_after_close(2, 2, 2), 1);
    }

    #[test]
    fn active_after_close_is_zero_when_no_tabs_remain() {
        assert_eq!(active_after_close(0, 0, 0), 0);
    }

    #[test]
    fn next_active_tab_wraps_forward() {
        assert_eq!(next_active_tab(2, 1, 3), 0);
    }

    #[test]
    fn next_active_tab_wraps_backward() {
        assert_eq!(next_active_tab(0, -1, 3), 2);
    }

    #[test]
    fn next_active_tab_is_zero_with_no_tabs() {
        assert_eq!(next_active_tab(0, 1, 0), 0);
    }

    async fn drain_gui_scan(tab: &mut TabState) {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while tab.filter.handle.is_some() || tab.filter.prepared.is_some() {
                tab.advance_filter_computation();
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("filter scan should finish");
    }

    #[tokio::test]
    async fn rapid_filter_toggles_are_immediate_cancel_stale_scans_and_persist() {
        let (mut state, _file) = state_with_one_tab().await;
        let mut lm = state.tabs[0].log_manager.clone();
        lm.add_filter_with_color("one".into(), FilterType::Include, FilterOptions::default())
            .await;
        lm.add_filter_with_color("two".into(), FilterType::Include, FilterOptions::default())
            .await;
        let id = lm.get_filters()[0].id;
        state.tabs[0].log_manager = lm;
        force_recompute(&mut state.tabs[0]);
        let cancel = state.tabs[0].filter.handle.as_ref().unwrap().cancel.clone();
        let first = update(&mut state, Message::FilterToggled(0, id));
        assert!(!state.tabs[0].log_manager.get_filters()[0].enabled);
        assert!(cancel.load(std::sync::atomic::Ordering::Relaxed));
        let second = update(&mut state, Message::FilterToggled(0, id));
        assert!(state.tabs[0].log_manager.get_filters()[0].enabled);
        for effect in [first, second] {
            let Effect::PersistFilterEdit(edit) = effect else {
                panic!("expected persistence");
            };
            edit.persist().await.unwrap();
        }
        drain_gui_scan(&mut state.tabs[0]).await;
        assert_eq!(state.tabs[0].filter.visible_indices.len(), 2);
        let reloaded = LogManager::new(state.db.clone(), Some(String::new())).await;
        assert!(reloaded.get_filters()[0].enabled);
    }

    #[tokio::test]
    async fn dragged_filters_reorder_immediately_and_survive_reload() {
        let (mut state, _file) = state_with_one_tab().await;
        for pattern in ["one", "two", "three"] {
            state.tabs[0]
                .log_manager
                .add_filter_with_color(
                    pattern.into(),
                    FilterType::Include,
                    FilterOptions::default(),
                )
                .await;
        }
        let ids: Vec<_> = state.tabs[0]
            .log_manager
            .get_filters()
            .iter()
            .map(|f| f.id)
            .collect();
        for (from, to, expected) in [
            (ids[0], ids[2], vec![ids[1], ids[2], ids[0]]),
            (ids[0], ids[1], vec![ids[0], ids[1], ids[2]]),
        ] {
            let effect = update(&mut state, Message::FilterReordered(0, from, to));
            assert_eq!(
                state.tabs[0]
                    .log_manager
                    .get_filters()
                    .iter()
                    .map(|f| f.id)
                    .collect::<Vec<_>>(),
                expected
            );
            let Effect::PersistFilterEdit(edit) = effect else {
                panic!("expected persistence");
            };
            edit.persist().await.unwrap();
            let reloaded = LogManager::new(state.db.clone(), Some(String::new())).await;
            assert_eq!(
                reloaded
                    .get_filters()
                    .iter()
                    .map(|f| f.id)
                    .collect::<Vec<_>>(),
                expected
            );
        }
        assert!(matches!(
            update(&mut state, Message::FilterReordered(0, ids[0], ids[0])),
            Effect::None
        ));
        drain_gui_scan(&mut state.tabs[0]).await;
    }

    #[tokio::test]
    async fn old_command_results_preserve_newer_toggles_and_drag_order() {
        let (mut state, _file) = state_with_one_tab().await;
        let mut lm = state.tabs[0].log_manager.clone();
        for pattern in ["one", "two", "three"] {
            lm.add_filter_with_color(
                pattern.into(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        }
        let mut old_result = lm.clone();
        let ids: Vec<_> = lm.get_filters().iter().map(|f| f.id).collect();
        state.tabs[0].log_manager = lm;
        update(&mut state, Message::FilterToggled(0, ids[0]));
        update(&mut state, Message::FilterReordered(0, ids[0], ids[2]));
        let changes = old_result.merge_newer_ui_edits(&state.tabs[0].log_manager);
        assert_eq!(
            old_result
                .get_filters()
                .iter()
                .map(|f| f.id)
                .collect::<Vec<_>>(),
            vec![ids[1], ids[2], ids[0]]
        );
        assert!(!old_result.get_filters()[2].enabled);
        let order = changes.order.unwrap();
        // Replaying absolute order is safe even if an earlier write already finished.
        for _ in 0..2 {
            crate::gui::effect::FilterEdit::Reorder {
                db: state.db.clone(),
                order: order.clone(),
            }
            .persist()
            .await
            .unwrap();
        }
        let reloaded = LogManager::new(state.db.clone(), Some(String::new())).await;
        assert_eq!(
            reloaded
                .get_filters()
                .iter()
                .map(|f| f.id)
                .collect::<Vec<_>>(),
            order
        );
        drain_gui_scan(&mut state.tabs[0]).await;
    }

    #[tokio::test]
    async fn group_toggle_updates_members_and_empty_group_immediately() {
        let (mut state, _file) = state_with_one_tab().await;
        let lm = execute_command(
            state.tabs[0].log_manager.clone(),
            command("filter --group workers one"),
            TEST_THEME_BG,
            None,
        )
        .await
        .unwrap();
        state.tabs[0].log_manager = lm;
        for name in ["workers", "empty"] {
            let effect = update(&mut state, Message::GroupToggled(0, name.into()));
            assert!(!crate::filters::group_enabled(
                state.tabs[0].log_manager.get_group_styles(),
                name
            ));
            let Effect::PersistFilterEdit(edit) = effect else {
                panic!("expected persistence");
            };
            edit.persist().await.unwrap();
        }
        assert!(!state.tabs[0].log_manager.get_filters()[0].enabled);
        let reloaded = LogManager::new(state.db.clone(), Some(String::new())).await;
        assert!(!reloaded.get_filters()[0].enabled);
        assert!(!crate::filters::group_enabled(
            reloaded.get_group_styles(),
            "empty"
        ));
    }

    async fn state_with_one_tab() -> (GuiState, tempfile::NamedTempFile) {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let mut state = GuiState::new(db);
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "line one\nline two\n").unwrap();
        let reader = FileReader::new(file.path().to_str().unwrap()).unwrap();
        let log_manager = LogManager::new(Arc::clone(&state.db), None).await;
        let mut tab = TabState::new(reader, log_manager, "test.log".to_string());
        force_recompute(&mut tab);
        state.tabs.push(tab);
        (state, file)
    }

    #[tokio::test]
    async fn clearing_gui_filters_defers_blank_row_scan() {
        let (mut state, _file) = state_with_one_tab().await;
        let tab = &mut state.tabs[0];
        tab.file_reader = FileReader::from_bytes(b"line one\n\nline two\n".to_vec());
        tab.display.format = Some(Arc::new(crate::parser::SyslogParser::default()));
        tab.filter.visible_indices = VisibleLines::Filtered(vec![2]);
        force_recompute(tab);
        assert_eq!(
            tab.filter.visible_indices.iter().collect::<Vec<_>>(),
            vec![2]
        );
        drain_gui_scan(tab).await;
        assert_eq!(
            tab.filter.visible_indices.iter().collect::<Vec<_>>(),
            vec![0, 2]
        );
    }

    #[tokio::test]
    async fn gui_filter_refresh_defers_compilation_and_keeps_latest_request() {
        let (mut state, _file) = state_with_one_tab().await;
        let tab = &mut state.tabs[0];
        tab.log_manager
            .add_filter_with_color(
                "line one".into(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        force_recompute(tab);
        assert_eq!(tab.filter.manager.filter_count(), 0);
        drain_gui_scan(tab).await;
        assert_eq!(tab.filter.manager.filter_count(), 1);
        assert_eq!(
            tab.filter.visible_indices.iter().collect::<Vec<_>>(),
            vec![0]
        );
        force_recompute(tab);
        tab.log_manager
            .add_filter_with_color(
                "line two".into(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        force_recompute(tab);
        drain_gui_scan(tab).await;
        assert_eq!(tab.filter.manager.filter_count(), 2);
        assert_eq!(
            tab.filter.visible_indices.iter().collect::<Vec<_>>(),
            vec![0, 1]
        );
        tab.filter.show_marks_only = true;
        tab.mark_line(1);
        force_recompute(tab);
        assert!(tab.filter.prepared.is_some());
        drain_gui_scan(tab).await;
        assert!(tab.filter.prepared.is_none());
        assert_eq!(
            tab.filter.visible_indices.iter().collect::<Vec<_>>(),
            vec![1]
        );
    }

    #[tokio::test]
    async fn apply_log_manager_remaps_log_selection_to_the_same_line_after_filtering() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let mut state = GuiState::new(db.clone());
        let log_manager = LogManager::new(Arc::clone(&db), None).await;
        let reader = FileReader::from_bytes(b"alpha\nbeta\ngamma\ndelta\n".to_vec());
        let mut tab = TabState::new(reader, log_manager, "test.log".to_string());
        force_recompute(&mut tab);
        drain_gui_scan(&mut tab).await;
        let tab_id = tab.id;
        state.tabs.push(tab);

        // "gamma" is selected at its current visible row (file line 2).
        state.log_selection = Some((tab_id, 2, 2));

        // A new filter hides "alpha"/"beta", shifting "gamma" to row 0 —
        // the selection must follow the line, not stay pinned to row 2
        // (which is now "delta" after the reshuffle).
        let mut manager = state.tabs[0].log_manager.clone();
        manager
            .add_filter_with_color(
                "gamma|delta".to_string(),
                FilterType::Include,
                FilterOptions {
                    use_regex: true,
                    ..Default::default()
                },
            )
            .await;
        apply_log_manager(&mut state, 0, manager);
        drain_gui_scan(&mut state.tabs[0]).await;
        resolve_pending_selection(&mut state);

        assert_eq!(state.tabs[0].filter.visible_indices.get_opt(0), Some(2));
        assert_eq!(state.log_selection, Some((tab_id, 0, 0)));
    }

    #[tokio::test]
    async fn pending_selection_anchor_does_not_clobber_a_newer_selection() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let mut state = GuiState::new(db.clone());
        let log_manager = LogManager::new(Arc::clone(&db), None).await;
        let reader = FileReader::from_bytes(b"alpha\nbeta\ngamma\ndelta\n".to_vec());
        let mut tab = TabState::new(reader, log_manager, "test.log".to_string());
        force_recompute(&mut tab);
        drain_gui_scan(&mut tab).await;
        let tab_id = tab.id;
        state.tabs.push(tab);
        state.log_selection = Some((tab_id, 2, 2));

        let mut manager = state.tabs[0].log_manager.clone();
        manager
            .add_filter_with_color(
                "gamma|delta".to_string(),
                FilterType::Include,
                FilterOptions {
                    use_regex: true,
                    ..Default::default()
                },
            )
            .await;
        apply_log_manager(&mut state, 0, manager);
        assert!(state.pending_selection_anchor.is_some());

        // The user clicks a different row before the scan finishes.
        state.log_selection = Some((tab_id, 1, 1));
        drain_gui_scan(&mut state.tabs[0]).await;
        resolve_pending_selection(&mut state);

        assert_eq!(state.log_selection, Some((tab_id, 1, 1)));
        assert!(state.pending_selection_anchor.is_none());
    }

    #[tokio::test]
    async fn native_search_edits_use_search_engine_without_changing_mode() {
        let (mut state, _file) = state_with_one_tab().await;
        update(&mut state, Message::SearchEdited("line".into()));
        assert!(state.tabs[0].search.handle.is_some());
        assert!(matches!(
            state.tabs[0].interaction.mode.render_state(),
            crate::mode::app_mode::ModeRenderState::Normal
        ));
        update(&mut state, Message::SearchEdited(String::new()));
        assert!(state.tabs[0].search.handle.is_none());
    }
    #[tokio::test]
    async fn columns_editor_cancel_and_apply_use_native_draft() {
        let (mut state, _file) = state_with_one_tab().await;
        state.tabs[0].display.format = Some(Arc::new(crate::parser::SyslogParser::default()));
        state.tabs[0].file_reader =
            FileReader::from_bytes(b"<134>Oct 11 22:14:15 host sshd[1234]: message\n".to_vec());
        state.tabs[0].collect_field_names();
        update(
            &mut state,
            Message::GuiAction(crate::gui::actions::Action::Columns),
        );
        update(&mut state, Message::EditorAll(false));
        update(&mut state, Message::EditorCancel);
        assert!(state.tabs[0].display.hidden_fields.is_empty());
        update(
            &mut state,
            Message::GuiAction(crate::gui::actions::Action::Columns),
        );
        update(&mut state, Message::EditorAll(false));
        update(&mut state, Message::EditorApply);
        assert!(!state.tabs[0].display.hidden_fields.is_empty());
    }

    fn fake_watch_state() -> crate::shared::FileWatchState {
        let (_tx, rx) = tokio::sync::watch::channel(());
        crate::shared::FileWatchState {
            snapshot_rx: rx,
            reader_path: PathBuf::from("test.log"),
            temp_file: None,
        }
    }

    #[tokio::test]
    async fn pause_command_pauses_a_live_tab() {
        let (mut state, _file) = state_with_one_tab().await;
        state.tabs[0].stream.watch = Some(fake_watch_state());
        assert!(is_tab_live(&state.tabs[0]));
        let effect = handle_command_string(&mut state, "pause".to_string());
        assert!(matches!(effect, Effect::None));
        assert!(state.tabs[0].stream.paused);
        assert!(!is_tab_live(&state.tabs[0]));
    }

    #[tokio::test]
    async fn resume_command_unpauses_a_tab() {
        let (mut state, _file) = state_with_one_tab().await;
        state.tabs[0].stream.watch = Some(fake_watch_state());
        state.tabs[0].stream.paused = true;
        let effect = handle_command_string(&mut state, "resume".to_string());
        assert!(matches!(effect, Effect::None));
        assert!(!state.tabs[0].stream.paused);
        assert!(is_tab_live(&state.tabs[0]));
    }

    #[tokio::test]
    async fn stop_command_clears_the_watcher() {
        let (mut state, _file) = state_with_one_tab().await;
        state.tabs[0].stream.watch = Some(fake_watch_state());
        let effect = handle_command_string(&mut state, "stop".to_string());
        assert!(matches!(effect, Effect::None));
        assert!(state.tabs[0].stream.watch.is_none());
        assert!(!is_tab_live(&state.tabs[0]));
    }

    #[tokio::test]
    async fn tail_command_toggles_tail_mode_and_jumps_to_the_last_line() {
        let (mut state, _file) = state_with_one_tab().await;
        assert!(!state.tabs[0].stream.tail_mode);
        state.tabs[0].scroll.scroll_offset = 0;
        let effect = handle_command_string(&mut state, "tail".to_string());
        assert!(matches!(effect, Effect::None));
        assert!(state.tabs[0].stream.tail_mode);
        let last = state.tabs[0].filter.visible_indices.len() - 1;
        assert_eq!(state.tabs[0].scroll.scroll_offset, last);
        handle_command_string(&mut state, "tail".to_string());
        assert!(!state.tabs[0].stream.tail_mode);
    }

    #[tokio::test]
    async fn clicking_a_log_row_moves_the_cursor_to_it() {
        // The status bar's "Ln N" reads `scroll.scroll_offset`, so a mouse
        // click on a row — not just keyboard navigation — must move it,
        // the same way it already updates `log_selection`.
        let (mut state, _file) = state_with_one_tab().await;
        assert_eq!(state.tabs[0].scroll.scroll_offset, 0);
        update(&mut state, Message::SelectLog(1, false));
        assert_eq!(state.tabs[0].scroll.scroll_offset, 1);
        assert_eq!(
            state.log_selection,
            Some((state.tabs[0].id, 1, 1)),
            "selection anchor/row must still be set alongside the cursor move"
        );
    }

    #[tokio::test]
    async fn is_tab_live_is_false_without_a_watcher() {
        let (state, _file) = state_with_one_tab().await;
        assert!(state.tabs[0].stream.watch.is_none());
        assert!(!is_tab_live(&state.tabs[0]));
    }

    #[tokio::test]
    async fn wrap_command_toggles_wrap_on_the_active_tab() {
        let (mut state, _file) = state_with_one_tab().await;
        let before = state.tabs[0].display.wrap;
        let effect = handle_command_string(&mut state, "wrap".to_string());
        assert!(matches!(effect, Effect::None));
        assert_eq!(state.tabs[0].display.wrap, !before);
    }

    #[tokio::test]
    async fn line_numbers_command_toggles_line_numbers() {
        let (mut state, _file) = state_with_one_tab().await;
        let before = state.tabs[0].display.show_line_numbers;
        handle_command_string(&mut state, "line-numbers".to_string());
        assert_eq!(state.tabs[0].display.show_line_numbers, !before);
    }

    #[tokio::test]
    async fn relative_line_numbers_command_toggles_relative_line_numbers() {
        let (mut state, _file) = state_with_one_tab().await;
        let before = state.tabs[0].display.relative_line_numbers;
        handle_command_string(&mut state, "relative-line-numbers".to_string());
        assert_eq!(state.tabs[0].display.relative_line_numbers, !before);
    }

    #[tokio::test]
    async fn show_keys_and_hide_keys_commands_set_show_keys() {
        let (mut state, _file) = state_with_one_tab().await;
        handle_command_string(&mut state, "show-keys".to_string());
        assert!(state.tabs[0].display.show_keys);
        handle_command_string(&mut state, "hide-keys".to_string());
        assert!(!state.tabs[0].display.show_keys);
    }

    #[tokio::test]
    async fn raw_command_toggles_raw_mode_and_recomputes_visibility() {
        let (mut state, _file) = state_with_one_tab().await;
        let before = state.tabs[0].display.raw_mode;
        let effect = handle_command_string(&mut state, "raw".to_string());
        assert!(matches!(effect, Effect::None));
        assert_eq!(state.tabs[0].display.raw_mode, !before);
    }

    #[tokio::test]
    async fn continuation_commands_defer_masking_and_preserve_latest_navigation() {
        let (mut state, _file) = state_with_one_tab().await;
        let tab = &mut state.tabs[0];
        tab.file_reader = FileReader::from_bytes(b"one\nchild\ntwo\nchild\n".to_vec());
        tab.display.format = Some(Arc::new(crate::parser::SyslogParser::default()));
        tab.continuation_map = Some(Arc::new(vec![0, 0, 2, 2]));
        tab.filter.visible_indices = VisibleLines::All(4);
        tab.display.collapse_continuations = false;
        handle_command_string(&mut state, "collapse".into());
        assert_eq!(state.tabs[0].filter.visible_indices.len(), 4);
        assert!(state.tabs[0].filter.handle.is_some());
        super::super::keyboard::apply(&mut state, KeyCode::End, KeyModifiers::NONE);
        drain_gui_scan(&mut state.tabs[0]).await;
        assert_eq!(
            state.tabs[0]
                .filter
                .visible_indices
                .iter()
                .collect::<Vec<_>>(),
            vec![0, 2]
        );
        assert_eq!(state.tabs[0].scroll.scroll_offset, 1);
        super::super::keyboard::apply(&mut state, KeyCode::Char('>'), KeyModifiers::NONE);
        assert_eq!(state.tabs[0].filter.visible_indices.len(), 2);
        drain_gui_scan(&mut state.tabs[0]).await;
        assert_eq!(
            state.tabs[0]
                .filter
                .visible_indices
                .iter()
                .collect::<Vec<_>>(),
            vec![0, 2, 3]
        );
        assert_eq!(state.tabs[0].scroll.scroll_offset, 1);
    }

    #[tokio::test]
    async fn end_during_pending_filtering_targets_the_complete_file() {
        let (mut state, _file) = state_with_one_tab().await;
        let tab = &mut state.tabs[0];
        tab.file_reader = FileReader::from_bytes(b"row\n".repeat(10_000));
        tab.display.format = Some(Arc::new(crate::parser::SyslogParser::default()));
        tab.filter.visible_indices = VisibleLines::Filtered(vec![0]);
        force_recompute(tab);
        super::super::keyboard::apply(&mut state, KeyCode::End, KeyModifiers::NONE);
        drain_gui_scan(&mut state.tabs[0]).await;
        assert_eq!(state.tabs[0].scroll.scroll_offset, 9_999);
    }

    #[tokio::test]
    async fn collapse_and_expand_commands_toggle_collapse_continuations() {
        let (mut state, _file) = state_with_one_tab().await;
        handle_command_string(&mut state, "collapse".to_string());
        assert!(state.tabs[0].display.collapse_continuations);
        handle_command_string(&mut state, "expand".to_string());
        assert!(!state.tabs[0].display.collapse_continuations);
    }

    #[tokio::test]
    async fn sidebar_position_command_moves_the_sidebar() {
        let (mut state, _file) = state_with_one_tab().await;
        assert_eq!(
            state.tabs[0].display.sidebar_side,
            crate::shared::SidebarSide::Right
        );
        let effect = handle_command_string(&mut state, "sidebar-position left".to_string());
        assert!(matches!(effect, Effect::None));
        assert_eq!(
            state.tabs[0].display.sidebar_side,
            crate::shared::SidebarSide::Left
        );
        handle_command_string(&mut state, "sidebar-position right".to_string());
        assert_eq!(
            state.tabs[0].display.sidebar_side,
            crate::shared::SidebarSide::Right
        );
    }

    #[tokio::test]
    async fn schema_command_with_no_name_reports_the_current_schema() {
        let (mut state, _file) = state_with_one_tab().await;
        assert!(state.tabs[0].display.format.is_none());
        let effect = handle_command_string(&mut state, "schema".to_string());
        assert!(matches!(effect, Effect::None));
        match state.status {
            Some(StatusMessage::Error(ref msg)) => assert!(msg.contains("none")),
            other => panic!("expected a status message, got {other:?}"),
        }
    }

    fn finish_schema(state: &mut GuiState, effect: Effect) {
        let Effect::PrepareSchema {
            request_id,
            tab_id,
            name,
        } = effect
        else {
            panic!("expected schema worker")
        };
        let reader = state
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .unwrap()
            .file_reader
            .clone();
        let prepared = super::super::schema_preparation::prepare(reader, &name);
        update(
            state,
            Message::SchemaPrepared(request_id, tab_id, name, Box::new(prepared)),
        );
    }

    #[tokio::test]
    async fn schema_changes_defer_full_file_metadata() {
        let (mut state, _file) = state_with_one_tab().await;
        let effect = handle_command_string(&mut state, "schema clf".into());
        assert!(state.tabs[0].display.format.is_none());
        assert!(state.palette.pending());
        let Effect::PrepareSchema {
            request_id,
            tab_id,
            name,
        } = effect
        else {
            panic!("expected schema worker")
        };
        let prepared =
            super::super::schema_preparation::prepare(state.tabs[0].file_reader.clone(), &name);
        state.tabs[0].file_reader =
            crate::ingestion::FileReader::from_bytes(b"replacement\n".to_vec());
        let retry = update(
            &mut state,
            Message::SchemaPrepared(request_id, tab_id, name, Box::new(prepared)),
        );
        assert!(state.tabs[0].display.format.is_none());
        finish_schema(&mut state, retry);
        assert_eq!(state.tabs[0].continuation_map.as_ref().unwrap().len(), 1);
        assert_eq!(state.tabs[0].display.format.as_ref().unwrap().name(), "clf");
    }

    #[tokio::test]
    async fn schema_command_applies_a_builtin_parser_by_name() {
        let (mut state, _file) = state_with_one_tab().await;
        let effect = handle_command_string(&mut state, "schema clf".to_string());
        finish_schema(&mut state, effect);
        assert_eq!(
            state.tabs[0].display.format.as_deref().map(|f| f.name()),
            Some("clf")
        );
    }

    #[tokio::test]
    async fn schema_command_none_clears_a_previously_set_format() {
        let (mut state, _file) = state_with_one_tab().await;
        let effect = handle_command_string(&mut state, "schema clf".to_string());
        finish_schema(&mut state, effect);
        assert!(state.tabs[0].display.format.is_some());
        let effect = handle_command_string(&mut state, "schema none".to_string());
        finish_schema(&mut state, effect);
        assert!(state.tabs[0].display.format.is_none());
    }

    #[tokio::test]
    async fn schema_command_reports_an_unknown_schema() {
        let (mut state, _file) = state_with_one_tab().await;
        let effect = handle_command_string(&mut state, "schema not-a-real-schema".to_string());
        finish_schema(&mut state, effect);
        assert!(state.tabs[0].display.format.is_none());
        assert!(matches!(state.status, Some(StatusMessage::Error(_))));
    }

    #[tokio::test]
    async fn path_command_reports_a_status_message() {
        // state_with_one_tab's log_manager isn't associated with its
        // FileReader's real path (no production code path leaves a tab in
        // that state — it's a test-fixture gap, not a real "no source"
        // tab), so display_source_path() falls back to the no-source-file
        // placeholder here. The point of this test is only that :path
        // reaches state.status as Info, not the exact message.
        let (mut state, _file) = state_with_one_tab().await;
        let effect = handle_command_string(&mut state, "path".to_string());
        assert!(matches!(effect, Effect::None));
        assert!(matches!(state.status, Some(StatusMessage::Info(_))));
    }

    #[tokio::test]
    async fn display_commands_never_reach_execute_command() {
        // These never touch LogManager, so handle_command_string must
        // intercept them the same way it already does for pause/resume/
        // stop, rather than returning an Effect::ExecutePalette that
        // execute_command would reject as unsupported.
        let (mut state, _file) = state_with_one_tab().await;
        for cmd in [
            "wrap",
            "line-numbers",
            "relative-line-numbers",
            "show-keys",
            "hide-keys",
            "raw",
            "collapse",
            "expand",
        ] {
            let effect = handle_command_string(&mut state, cmd.to_string());
            assert!(
                matches!(effect, Effect::None),
                "{cmd} should be intercepted, got {effect:?}"
            );
        }
    }

    #[tokio::test]
    async fn nav_page_selected_switches_pages_and_back() {
        let (mut state, _file) = state_with_one_tab().await;
        assert_eq!(state.nav_page, crate::gui::state::NavPage::Logs);
        update(
            &mut state,
            Message::NavPageSelected(crate::gui::state::NavPage::Bookmarks),
        );
        assert_eq!(state.nav_page, crate::gui::state::NavPage::Bookmarks);
        update(
            &mut state,
            Message::NavPageSelected(crate::gui::state::NavPage::Logs),
        );
        assert_eq!(state.nav_page, crate::gui::state::NavPage::Logs);
    }

    #[tokio::test]
    async fn time_range_toggled_opens_and_closes_the_dropdown() {
        let (mut state, _file) = state_with_one_tab().await;
        assert!(!state.time_range_open);
        update(&mut state, Message::TimeRangeToggled);
        assert!(state.time_range_open);
        update(&mut state, Message::TimeRangeToggled);
        assert!(!state.time_range_open);
    }

    #[tokio::test]
    async fn time_range_selected_closes_the_dropdown_and_records_the_preset() {
        use crate::gui::time_range::TimeRangePreset;
        let (mut state, _file) = state_with_one_tab().await;
        state.time_range_open = true;
        let effect = update(
            &mut state,
            Message::TimeRangeSelected(TimeRangePreset::LastHour),
        );
        assert!(!state.time_range_open);
        assert_eq!(state.time_range_preset, TimeRangePreset::LastHour);
        match effect {
            Effect::ExecutePalette { command, .. } => {
                assert!(matches!(command, Commands::DateFilter { .. }));
            }
            other => panic!("expected ExecuteCommand, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn all_time_clears_a_lone_date_filter() {
        use crate::gui::time_range::TimeRangePreset;
        let (mut state, _file) = state_with_one_tab().await;
        let lm = execute_command(
            state.tabs[0].log_manager.clone(),
            command("date-filter > 2024-01-01"),
            TEST_THEME_BG,
            None,
        )
        .await
        .unwrap();
        state.tabs[0].log_manager = lm;
        assert_eq!(state.tabs[0].log_manager.get_filters().len(), 1);

        let effect = update(
            &mut state,
            Message::TimeRangeSelected(TimeRangePreset::AllTime),
        );
        match effect {
            Effect::ClearGuiDates { .. } => {}
            other => panic!("expected date-filter removal, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn all_time_removes_only_the_date_filter() {
        use crate::gui::time_range::TimeRangePreset;
        let (mut state, _file) = state_with_one_tab().await;
        let lm = execute_command(
            state.tabs[0].log_manager.clone(),
            command("date-filter > 2024-01-01"),
            TEST_THEME_BG,
            None,
        )
        .await
        .unwrap();
        let lm = execute_command(lm, command("filter ERROR"), TEST_THEME_BG, None)
            .await
            .unwrap();
        state.tabs[0].log_manager = lm;
        assert_eq!(state.tabs[0].log_manager.get_filters().len(), 2);

        let effect = update(
            &mut state,
            Message::TimeRangeSelected(TimeRangePreset::AllTime),
        );
        let Effect::ClearGuiDates { log_manager, .. } = effect else {
            panic!("expected scoped date removal")
        };
        let manager = crate::gui::actions::clear_dates(log_manager).await.unwrap();
        assert_eq!(manager.get_filters().len(), 1);
        assert_eq!(manager.get_filters()[0].pattern, "ERROR");
    }

    #[tokio::test]
    async fn all_time_is_a_no_op_without_any_date_filter() {
        use crate::gui::time_range::TimeRangePreset;
        let (mut state, _file) = state_with_one_tab().await;
        let effect = update(
            &mut state,
            Message::TimeRangeSelected(TimeRangePreset::AllTime),
        );
        assert!(matches!(effect, Effect::None));
        assert!(state.status.is_none());
    }

    #[tokio::test]
    async fn stream_toggled_pauses_a_live_tab_and_resumes_it() {
        let (mut state, _file) = state_with_one_tab().await;
        state.tabs[0].stream.watch = Some(fake_watch_state());
        let effect = update(&mut state, Message::StreamToggled(0));
        assert!(matches!(effect, Effect::None));
        assert!(state.tabs[0].stream.paused);

        let effect = update(&mut state, Message::StreamToggled(0));
        assert!(matches!(effect, Effect::None));
        assert!(!state.tabs[0].stream.paused);
    }

    #[tokio::test]
    async fn sidebar_tab_selected_switches_tabs() {
        let (mut state, _file) = state_with_one_tab().await;
        assert_eq!(state.sidebar_tab, crate::gui::state::SidebarTab::Filters);
        update(
            &mut state,
            Message::SidebarTabSelected(crate::gui::state::SidebarTab::Groups),
        );
        assert_eq!(state.sidebar_tab, crate::gui::state::SidebarTab::Groups);
    }

    #[tokio::test]
    async fn clear_all_filters_dispatches_the_real_command() {
        let (mut state, _file) = state_with_one_tab().await;
        let effect = update(&mut state, Message::ClearAllFilters(0));
        match effect {
            Effect::ExecutePalette { command, .. } => {
                assert!(matches!(command, Commands::ClearFilters));
            }
            other => panic!("expected ExecuteCommand(ClearFilters), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn filter_removed_requests_the_remove_effect() {
        let (mut state, _file) = state_with_one_tab().await;
        let lm = execute_command(
            state.tabs[0].log_manager.clone(),
            command("filter ERROR"),
            TEST_THEME_BG,
            None,
        )
        .await
        .unwrap();
        state.tabs[0].log_manager = lm;
        let id = state.tabs[0].log_manager.get_filters()[0].id;

        let effect = update(&mut state, Message::FilterRemoved(0, id));
        match effect {
            Effect::RemoveFilter {
                tab_idx,
                id: removed_id,
                ..
            } => {
                assert_eq!(tab_idx, 0);
                assert_eq!(removed_id, id);
            }
            other => panic!("expected RemoveFilter, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn facet_toggled_flips_expand_state_starting_from_collapsed() {
        let (mut state, _file) = state_with_one_tab().await;
        assert!(!state.facet_expanded.contains_key("level"));
        update(&mut state, Message::FacetToggled("level".to_string()));
        assert_eq!(state.facet_expanded.get("level"), Some(&true));
        update(&mut state, Message::FacetToggled("level".to_string()));
        assert_eq!(state.facet_expanded.get("level"), Some(&false));
    }

    #[tokio::test]
    async fn column_resize_moved_applies_the_delta_from_the_previous_event() {
        use crate::gui::column_widths::{ColumnWidths, ResizableColumn};
        let (mut state, _file) = state_with_one_tab().await;
        assert_eq!(state.column_resize_last_x, None);
        update(
            &mut state,
            Message::ColumnResizeMoved(ResizableColumn::Time, 100.0),
        );
        // First event in a drag only records its position — nothing to
        // diff against yet, so the width is unchanged.
        assert_eq!(state.column_widths.time, ColumnWidths::default().time);
        assert_eq!(state.column_resize_last_x, Some(100.0));
        update(
            &mut state,
            Message::ColumnResizeMoved(ResizableColumn::Time, 130.0),
        );
        assert_eq!(
            state.column_widths.time,
            ColumnWidths::default().time + 30.0
        );
        assert_eq!(state.column_resize_last_x, Some(130.0));
    }

    #[tokio::test]
    async fn column_resize_ended_clears_the_last_x_so_the_next_drag_starts_fresh() {
        use crate::gui::column_widths::ResizableColumn;
        let (mut state, _file) = state_with_one_tab().await;
        update(
            &mut state,
            Message::ColumnResizeMoved(ResizableColumn::Level, 50.0),
        );
        assert_eq!(state.column_resize_last_x, Some(50.0));
        update(&mut state, Message::ColumnResizeEnded);
        assert_eq!(state.column_resize_last_x, None);
    }

    #[tokio::test]
    async fn facet_value_toggled_adds_a_field_filter_when_none_exists() {
        let (mut state, _file) = state_with_one_tab().await;
        let effect = update(
            &mut state,
            Message::FacetValueToggled(0, "level".to_string(), "error".to_string()),
        );
        match effect {
            Effect::ExecutePalette { command, .. } => {
                assert!(matches!(command, Commands::Filter { .. }));
            }
            other => panic!("expected ExecuteCommand(Filter), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn field_filter_refresh_applies_structured_conditions() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let mut state = GuiState::new(db);
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            file.path(),
            concat!(
                "{\"level\":\"INFO\",\"message\":\"ready\"}\n",
                "{\"level\":\"ERROR\",\"message\":\"failed\"}\n",
            ),
        )
        .unwrap();
        let reader = FileReader::new(file.path().to_str().unwrap()).unwrap();
        let lm = LogManager::new(Arc::clone(&state.db), None).await;
        state
            .tabs
            .push(TabState::new(reader, lm, "test.jsonl".into()));
        assert!(state.tabs[0].display.format.is_some());
        let lm = execute_command(
            state.tabs[0].log_manager.clone(),
            command("filter --field level=ERROR"),
            TEST_THEME_BG,
            None,
        )
        .await
        .unwrap();
        update(&mut state, Message::CommandExecuted(0, Ok(lm)));
        drain_gui_scan(&mut state.tabs[0]).await;
        assert_eq!(
            state.tabs[0].filter.visible_indices,
            VisibleLines::Filtered(vec![1])
        );
    }

    #[tokio::test]
    async fn facet_values_preserve_whitespace_and_quotes() {
        let (mut state, _file) = state_with_one_tab().await;
        let value = "worker \"one\" \\ path";
        let effect = update(
            &mut state,
            Message::FacetValueToggled(0, "target".into(), value.into()),
        );
        let Effect::ExecutePalette {
            command: Commands::Filter { field, pattern, .. },
            ..
        } = effect
        else {
            panic!("expected field filter command");
        };
        assert_eq!(field, vec![format!("target={value}")]);
        assert!(pattern.is_empty());
    }

    #[tokio::test]
    async fn facet_value_toggled_removes_the_existing_field_filter() {
        let (mut state, _file) = state_with_one_tab().await;
        let log_manager = execute_command(
            state.tabs[0].log_manager.clone(),
            command("filter --field level=error"),
            TEST_THEME_BG,
            None,
        )
        .await
        .expect("field filter should be accepted");
        let id = log_manager.get_filters()[0].id;
        state.tabs[0].log_manager = log_manager;

        let effect = update(
            &mut state,
            Message::FacetValueToggled(0, "level".to_string(), "error".to_string()),
        );
        match effect {
            Effect::RemoveFilter { id: removed_id, .. } => assert_eq!(removed_id, id),
            other => panic!("expected RemoveFilter, got {other:?}"),
        }
    }

    #[test]
    fn filter_id_for_field_equality_finds_the_matching_filter() {
        let def = FilterDef {
            id: 7,
            pattern: format!("{FIELD_PREFIX}level:error"),
            filter_type: FilterType::Include,
            enabled: true,
            color_config: None,
            use_regex: false,
            ignore_case: false,
            group: None,
        };
        assert_eq!(
            filter_id_for_field_equality(&[def], "level", "error"),
            Some(7)
        );
    }

    #[test]
    fn filter_id_for_field_equality_ignores_non_field_filters() {
        let def = FilterDef {
            id: 1,
            pattern: "ERROR".to_string(),
            filter_type: FilterType::Include,
            enabled: true,
            color_config: None,
            use_regex: false,
            ignore_case: false,
            group: None,
        };
        assert_eq!(filter_id_for_field_equality(&[def], "level", "error"), None);
    }

    #[tokio::test]
    async fn settings_back_restores_draft_and_refreshes_editor_widgets() {
        let (mut state, _file) = state_with_one_tab().await;
        update(
            &mut state,
            Message::GuiAction(crate::gui::actions::Action::Settings),
        );
        update(&mut state, Message::EditorText(2, "12345".into()));
        let epoch = state.editor.as_ref().unwrap().epoch;
        update(
            &mut state,
            Message::EditorNavigate(crate::gui::actions::Action::Keybindings),
        );
        update(&mut state, Message::EditorBack);
        assert_eq!(
            state.editor.as_ref().unwrap().action,
            crate::gui::actions::Action::Settings
        );
        assert_eq!(state.editor.as_ref().unwrap().values[2], "12345");
        assert_ne!(state.editor.as_ref().unwrap().epoch, epoch);
    }

    #[tokio::test]
    async fn sidebar_position_toggle_opens_hidden_sidebar_and_can_be_reversed() {
        let (mut state, _file) = state_with_one_tab().await;
        state.tabs[0].display.show_sidebar = false;
        state.tabs[0].display.sidebar_side = crate::shared::SidebarSide::Right;
        crate::gui::actions::open(&mut state, crate::gui::actions::Action::View, None, None);
        update(&mut state, Message::SidebarSideToggled);
        assert!(state.tabs[0].display.sidebar_side.is_left());
        assert!(state.tabs[0].display.show_sidebar);
        update(&mut state, Message::SidebarSideToggled);
        assert!(!state.tabs[0].display.sidebar_side.is_left());
    }

    #[tokio::test]
    async fn open_file_dialog_requests_the_dialog_effect() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let mut state = GuiState::new(db);
        let effect = update(&mut state, Message::OpenFileDialog);
        assert!(matches!(effect, Effect::OpenFileDialog));
    }

    #[tokio::test]
    async fn file_dialog_result_with_a_path_requests_loading_it() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let mut state = GuiState::new(db);
        let effect = update(
            &mut state,
            Message::FileDialogResult(Some(PathBuf::from("/tmp/x.log"))),
        );
        assert!(matches!(effect, Effect::LoadFile(p) if p == std::path::Path::new("/tmp/x.log")));
    }

    #[tokio::test]
    async fn opening_a_file_does_not_block_the_next_file_dialog() {
        let (mut state, _file) = state_with_one_tab().await;
        for path in ["/tmp/first.log", "/tmp/second.log"] {
            assert!(matches!(
                update(&mut state, Message::OpenFileDialog),
                Effect::OpenFileDialog
            ));
            assert!(matches!(
                update(
                    &mut state,
                    Message::FileDialogResult(Some(PathBuf::from(path)))
                ),
                Effect::LoadFile(_)
            ));
            assert!(!state.palette.pending());
        }
        assert!(matches!(
            update(&mut state, Message::OpenFileDialog),
            Effect::OpenFileDialog
        ));
    }

    #[tokio::test]
    async fn file_loaded_err_sets_a_status_message() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let mut state = GuiState::new(db);
        update(&mut state, Message::FileLoaded(Err("boom".to_string())));
        assert_eq!(state.status, Some(StatusMessage::Error("boom".to_string())));
    }

    #[tokio::test]
    async fn tab_selected_switches_the_active_tab() {
        let (mut state, _file) = state_with_one_tab().await;
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "x\n").unwrap();
        let reader = FileReader::new(file.path().to_str().unwrap()).unwrap();
        let lm = LogManager::new(Arc::clone(&state.db), None).await;
        state
            .tabs
            .push(TabState::new(reader, lm, "x.log".to_string()));
        update(&mut state, Message::TabSelected(1));
        assert_eq!(state.active_tab, 1);
    }

    #[tokio::test]
    async fn tab_closed_removes_the_tab_and_adjusts_active() {
        let (mut state, _file) = state_with_one_tab().await;
        update(&mut state, Message::TabClosed(0));
        assert!(state.tabs.is_empty());
        assert_eq!(state.active_tab, 0);
    }

    #[tokio::test]
    async fn set_theme_command_applies_a_known_bundled_theme() {
        let (mut state, _file) = state_with_one_tab().await;
        let before = state.theme.clone();
        let entries = crate::theme::Theme::list_available_themes();
        let other = entries
            .iter()
            .find(|name| {
                crate::theme::Theme::from_file(format!("{}.json", name.to_lowercase()))
                    .is_ok_and(|t| t != before)
            })
            .expect("at least one bundled theme should differ from the default")
            .clone();
        let effect = handle_command_string(&mut state, format!("set-theme {other}"));
        assert!(matches!(effect, Effect::SaveGuiTheme { .. }));
        assert_ne!(state.theme, before);
        assert!(state.status.is_none());
    }

    #[tokio::test]
    async fn set_theme_command_reports_an_unknown_theme() {
        let (mut state, _file) = state_with_one_tab().await;
        let before = state.theme.clone();
        handle_command_string(&mut state, "set-theme not-a-real-theme".to_string());
        assert_eq!(state.theme, before);
        assert!(matches!(state.status, Some(StatusMessage::Error(_))));
    }

    /// Reproduces the real panic this guarded against: a `#[tokio::test]`
    /// already runs inside its own ambient tokio reactor, which would mask
    /// a missing `.enter()` guard in `dispatch_key` — gpui's real render
    /// thread has no such ambient reactor. So setup runs inside a scratch
    /// runtime's `block_on` (which exits its context when it returns), and
    /// the actual `update()` dispatch calls run as a plain, non-async
    /// `#[test]` with no tokio context of their own — exactly like gpui's
    /// render thread — to prove `dispatch_key` supplies one itself.

    #[tokio::test]
    async fn command_executed_ok_applies_the_log_manager_and_recomputes() {
        let (mut state, _file) = state_with_one_tab().await;
        let lm = execute_command(
            state.tabs[0].log_manager.clone(),
            command("filter line"),
            TEST_THEME_BG,
            None,
        )
        .await
        .unwrap();
        update(&mut state, Message::CommandExecuted(0, Ok(lm)));
        assert_eq!(state.tabs[0].log_manager.get_filters().len(), 1);
    }

    #[tokio::test]
    async fn toggle_group_checkbox_enables_a_disabled_group() {
        let mut lm = execute_command(
            log_manager().await,
            command("filter -g net ERROR"),
            TEST_THEME_BG,
            None,
        )
        .await
        .unwrap();
        lm = execute_command(lm, command("toggle-group net"), TEST_THEME_BG, None)
            .await
            .unwrap();
        assert!(!lm.get_filters()[0].enabled);
        toggle_group_checkbox(&mut lm, "net").await;
        assert!(lm.get_filters()[0].enabled);
    }

    #[tokio::test]
    async fn load_file_reads_a_real_file_into_a_log_manager() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "hello\nworld\n").unwrap();
        let loaded = load_file(file.path().to_path_buf(), db).await.unwrap();
        assert_eq!(loaded.path, file.path());
        assert!(loaded.log_manager.get_filters().is_empty());
        assert_eq!(
            loaded.watch.reader_path.to_str(),
            file.path().to_str(),
            "load_file should spawn a real watcher for the opened file"
        );
    }

    #[tokio::test]
    async fn pushing_a_loaded_tab_makes_it_live() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let mut state = GuiState::new(Arc::clone(&db));
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "hello\nworld\n").unwrap();
        let loaded = load_file(file.path().to_path_buf(), db).await.unwrap();
        update(&mut state, Message::FileLoaded(Ok(loaded)));
        assert!(is_tab_live(&state.tabs[0]));
    }

    #[tokio::test]
    async fn pushing_a_loaded_tab_auto_fits_column_widths() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let mut state = GuiState::new(Arc::clone(&db));
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "hello\nworld\n").unwrap();
        let loaded = load_file(file.path().to_path_buf(), db).await.unwrap();
        update(&mut state, Message::FileLoaded(Ok(loaded)));
        assert_eq!(
            state.column_widths,
            crate::gui::column_widths::fit_column_widths(&state.tabs[0]),
            "push_tab should auto-fit column_widths to the newly loaded file"
        );
    }
}
