use crate::gui::{actions::Action, effect::Effect, message::Message, state::GuiState};
use crate::input::{KeyCode, KeyModifiers};
pub(super) fn bindings(state: &GuiState) -> std::sync::Arc<crate::config::Keybindings> {
    state
        .active_tab()
        .map(|tab| tab.interaction.keybindings.clone())
        .unwrap_or_else(|| {
            std::sync::Arc::new(
                crate::config::Config::load()
                    .unwrap_or_default()
                    .keybindings,
            )
        })
}
fn action_bindings(
    kb: &crate::config::Keybindings,
) -> Vec<(&crate::config::keybindings::KeyBindings, Action)> {
    vec![
        (&kb.normal.show_keybindings, Action::Help),
        (&kb.normal.filter_include, Action::Filter),
        (&kb.normal.filter_include_auto, Action::Filter),
        (&kb.normal.filter_exclude, Action::Filter),
        (&kb.normal.toggle_filtering, Action::ToggleFiltering),
        (&kb.normal.toggle_marks_only, Action::MarksOnly),
        (&kb.normal.mark_line, Action::Bookmark),
        (&kb.normal.comment_line, Action::Annotation),
        (&kb.normal.enter_ui_mode, Action::View),
        (&kb.global.new_tab, Action::Open),
        (&kb.global.file_switcher, Action::SwitchFile),
    ]
}
pub fn handled(state: &GuiState, key: KeyCode, modifiers: KeyModifiers) -> bool {
    let kb = bindings(state);
    let nav_mods = if matches!(
        key,
        KeyCode::Up | KeyCode::Down | KeyCode::Home | KeyCode::End
    ) && modifiers == KeyModifiers::SHIFT
    {
        KeyModifiers::NONE
    } else {
        modifiers
    };
    sidebar_handled(state, key, modifiers)
        || action_bindings(&kb)
            .iter()
            .any(|(binding, _)| binding.matches(key, modifiers))
        || kb
            .custom
            .iter()
            .any(|binding| binding.key.matches(key, modifiers))
        || [
            &kb.global.quit,
            &kb.normal.search_forward,
            &kb.normal.search_backward,
            &kb.normal.command_mode,
            &kb.normal.visual_mode,
            &kb.normal.edit_comment,
            &kb.normal.delete_comment,
            &kb.normal.clear_all,
            &kb.normal.scroll_left,
            &kb.normal.scroll_right,
            &kb.normal.start_of_line,
            &kb.normal.end_of_line,
            &kb.normal.next_error,
            &kb.normal.prev_error,
            &kb.normal.next_warning,
            &kb.normal.prev_warning,
            &kb.normal.clear_search,
            &kb.normal.expand_continuation,
            &kb.normal.collapse_continuation,
            &kb.normal.toggle_highlight_mode,
            &kb.normal.yank_line,
            &kb.normal.yank_marked,
        ]
        .iter()
        .any(|binding| binding.matches(key, modifiers))
        || matches!(
            key,
            KeyCode::Home | KeyCode::End | KeyCode::F(1) | KeyCode::Char('?')
        )
        || [
            &kb.navigation.scroll_down,
            &kb.navigation.scroll_up,
            &kb.navigation.page_down,
            &kb.navigation.page_up,
            &kb.navigation.half_page_down,
            &kb.navigation.half_page_up,
            &kb.normal.go_to_top_chord,
            &kb.normal.go_to_bottom,
            &kb.normal.mark_line,
            &kb.normal.next_match,
            &kb.normal.prev_match,
            &kb.normal.filter_mode,
            &kb.normal.group_mode,
            &kb.global.next_tab,
            &kb.global.prev_tab,
            &kb.global.close_tab,
            &kb.global.new_tab,
            &kb.global.file_switcher,
        ]
        .iter()
        .any(|binding| binding.matches(key, nav_mods))
        || (modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key, KeyCode::Char('o') | KeyCode::Char('b')))
        || (modifiers.contains(KeyModifiers::CONTROL) && key == KeyCode::Tab)
}
pub fn apply(state: &mut GuiState, key: KeyCode, modifiers: KeyModifiers) -> Effect {
    if state.editor.is_some() || state.palette.visible() {
        return Effect::None;
    }
    let kb = bindings(state);
    if state.keyboard_sidebar.is_some() && sidebar_handled(state, key, modifiers) {
        return apply_sidebar(state, key, modifiers);
    }
    if kb.normal.filter_mode.matches(key, modifiers) || kb.normal.group_mode.matches(key, modifiers)
    {
        if state.active_tab().is_none() {
            return Effect::None;
        }
        let group = kb.normal.group_mode.matches(key, modifiers);
        state.sidebar_tab = if group {
            crate::gui::state::SidebarTab::Groups
        } else {
            crate::gui::state::SidebarTab::Filters
        };
        state.keyboard_sidebar = Some(state.sidebar_tab);
        state.sidebar_cursor = 0;
        if let Some(tab) = state.active_tab_mut() {
            tab.display.show_sidebar = true;
        }
        return Effect::None;
    }
    if let Some(custom) = kb
        .custom
        .iter()
        .find(|binding| binding.key.matches(key, modifiers))
    {
        use clap::Parser;
        if let Ok(parsed) = crate::commands::CommandLine::try_parse_from(
            crate::commands::auto_complete::shell_split(&custom.command),
        ) && let Some(command) = parsed.command
        {
            return crate::gui::update::perform_gui(state, command, custom.command.clone());
        }
        return Effect::None;
    }
    if state.keyboard_sidebar.is_none()
        && state.active_tab().is_some()
        && kb.global.quit.matches(key, modifiers)
    {
        return Effect::Quit;
    }
    if kb.normal.visual_mode.matches(key, modifiers) {
        state.visual_lines = !state.visual_lines;
        if let Some(tab) = state.active_tab() {
            state.log_selection =
                Some((tab.id, tab.scroll.scroll_offset, tab.scroll.scroll_offset));
        }
        return Effect::None;
    }
    if kb.normal.command_mode.matches(key, modifiers) {
        return crate::gui::update::update(state, Message::OpenCommands);
    }
    if kb.normal.search_forward.matches(key, modifiers)
        || kb.normal.search_backward.matches(key, modifiers)
    {
        let forward = kb.normal.search_forward.matches(key, modifiers);
        if let Some(tab) = state.active_tab_mut() {
            tab.search.query.set_forward(forward);
        }
        return crate::gui::update::update(state, Message::SearchPopup(true));
    }
    if kb.normal.edit_comment.matches(key, modifiers)
        || kb.normal.delete_comment.matches(key, modifiers)
    {
        let note = state.active_tab().and_then(|tab| {
            tab.filter
                .visible_indices
                .get_opt(tab.scroll.scroll_offset)
                .and_then(|line| {
                    tab.comment_manager
                        .get()
                        .iter()
                        .position(|note| note.line_indices.contains(&line))
                })
        });
        if let Some(index) = note {
            if kb.normal.edit_comment.matches(key, modifiers) {
                return crate::gui::update::update(state, Message::EditAnnotation(index));
            }
            let tab = state.active_tab_mut().unwrap();
            tab.comment_manager.remove(index);
            tab.invalidate_parse_cache();
        }
        return Effect::None;
    }
    if kb.normal.clear_all.matches(key, modifiers) {
        if let Some(tab) = state.active_tab_mut() {
            tab.mark_manager.clear();
            tab.comment_manager.clear();
            tab.begin_filter_refresh_background();
        }
        return Effect::None;
    }
    if let Some(tab) = state.active_tab_mut() {
        if kb.normal.scroll_left.matches(key, modifiers)
            || kb.normal.scroll_right.matches(key, modifiers)
            || kb.normal.start_of_line.matches(key, modifiers)
            || kb.normal.end_of_line.matches(key, modifiers)
        {
            tab.scroll.horizontal_scroll = if kb.normal.start_of_line.matches(key, modifiers) {
                0
            } else if kb.normal.scroll_left.matches(key, modifiers) {
                tab.scroll.horizontal_scroll.saturating_sub(1)
            } else if kb.normal.scroll_right.matches(key, modifiers) {
                tab.scroll.horizontal_scroll.saturating_add(1)
            } else {
                tab.filter
                    .visible_indices
                    .get_opt(tab.scroll.scroll_offset)
                    .map(|line| tab.get_display_text(line).chars().count())
                    .unwrap_or(0)
            };
            return Effect::ScrollHorizontal(tab.scroll.horizontal_scroll);
        }
        let position = tab.scroll.scroll_offset;
        let target = if kb.normal.next_error.matches(key, modifiers) {
            tab.next_error_position(position)
        } else if kb.normal.prev_error.matches(key, modifiers) {
            tab.prev_error_position(position)
        } else if kb.normal.next_warning.matches(key, modifiers) {
            tab.next_warning_position(position)
        } else if kb.normal.prev_warning.matches(key, modifiers) {
            tab.prev_warning_position(position)
        } else {
            None
        };
        if let Some(target) = target {
            tab.scroll.scroll_offset = target;
            return Effect::None;
        }
    }
    if kb.normal.clear_search.matches(key, modifiers) {
        return crate::gui::update::update(state, Message::SearchEdited(String::new()));
    }
    if kb.normal.expand_continuation.matches(key, modifiers)
        || kb.normal.collapse_continuation.matches(key, modifiers)
    {
        if let Some(tab) = state.active_tab_mut()
            && let Some(line) = tab.filter.visible_indices.get_opt(tab.scroll.scroll_offset)
            && let Some(parent) = tab
                .active_continuation_map()
                .and_then(|map| map.get(line))
                .copied()
        {
            let collapsed = kb.normal.collapse_continuation.matches(key, modifiers);
            if tab.display.collapse_continuations != collapsed {
                tab.filter.overridden_groups.insert(parent);
            } else {
                tab.filter.overridden_groups.remove(&parent);
            }
            tab.begin_filter_refresh_background();
            if let Some(handle) = &mut tab.filter.handle {
                handle.scroll_anchor = Some(parent);
            }
        }
        return Effect::None;
    }
    if kb.normal.toggle_highlight_mode.matches(key, modifiers) {
        if let Some(tab) = state.active_tab_mut() {
            tab.filter.highlight_mode = !tab.filter.highlight_mode;
            tab.begin_filter_refresh_background();
        }
        return Effect::None;
    }
    if (kb.normal.yank_line.matches(key, modifiers)
        || kb.normal.yank_marked.matches(key, modifiers))
        && let Some(tab) = state.active_tab()
    {
        let lines = if kb.normal.yank_marked.matches(key, modifiers) {
            tab.mark_manager.get_indices()
        } else {
            crate::gui::view::annotations::selected_lines(state)
        };
        let text = lines
            .iter()
            .map(|line| crate::gui::view::log_pane::raw_log_message(&tab.file_reader, *line))
            .collect::<Vec<_>>()
            .join("\n");
        return Effect::CopyText(text);
    }
    if let Some((_, action)) = action_bindings(&kb)
        .iter()
        .find(|(binding, _)| binding.matches(key, modifiers))
    {
        let effect = crate::gui::actions::open(state, *action, None, None);
        if *action == Action::Filter
            && let Some(editor) = &mut state.editor
        {
            editor.kind = if kb.normal.filter_exclude.matches(key, modifiers) {
                crate::filters::FilterType::Exclude
            } else {
                crate::filters::FilterType::Include
            };
            // `App::dispatch` fills `values[0]` from a manual text
            // selection (if any) once this returns; it never comes
            // from the cursor line alone.
            if kb.normal.filter_include_auto.matches(key, modifiers) {
                editor.regenerate_auto_style();
            }
        }
        return effect;
    }
    let action = if matches!(key, KeyCode::F(1) | KeyCode::Char('?')) {
        Some(Action::Help)
    } else if kb.global.file_switcher.matches(key, modifiers) {
        Some(Action::SwitchFile)
    } else if kb.global.new_tab.matches(key, modifiers)
        || (key == KeyCode::Char('o') && modifiers.contains(KeyModifiers::CONTROL))
    {
        Some(Action::Open)
    } else if kb.normal.filter_mode.matches(key, modifiers) {
        Some(Action::Filter)
    } else if kb.normal.group_mode.matches(key, modifiers) {
        Some(Action::Group)
    } else if kb.normal.mark_line.matches(key, modifiers)
        || (key == KeyCode::Char('b') && modifiers.contains(KeyModifiers::CONTROL))
    {
        Some(Action::Bookmark)
    } else {
        None
    };
    if let Some(action) = action {
        return crate::gui::actions::open(state, action, None, None);
    }
    let ctrl_tab = key == KeyCode::Tab && modifiers.contains(KeyModifiers::CONTROL);
    if !state.tabs.is_empty() {
        if kb.global.next_tab.matches(key, modifiers)
            || ctrl_tab && !modifiers.contains(KeyModifiers::SHIFT)
        {
            return crate::gui::update::update(
                state,
                Message::TabSelected((state.active_tab + 1) % state.tabs.len()),
            );
        }
        if kb.global.prev_tab.matches(key, modifiers)
            || ctrl_tab && modifiers.contains(KeyModifiers::SHIFT)
        {
            return crate::gui::update::update(
                state,
                Message::TabSelected((state.active_tab + state.tabs.len() - 1) % state.tabs.len()),
            );
        }
        if kb.global.close_tab.matches(key, modifiers) {
            return crate::gui::update::update(state, Message::TabClosed(state.active_tab));
        }
    }
    if kb.normal.next_match.matches(key, modifiers) {
        return crate::gui::update::update(state, Message::SearchNext(true));
    }
    if kb.normal.prev_match.matches(key, modifiers) {
        return crate::gui::update::update(state, Message::SearchNext(false));
    }
    let selection = state.log_selection;
    let visual_lines = state.visual_lines;
    let Some(tab) = state.active_tab_mut() else {
        return Effect::None;
    };
    let extend = visual_lines
        || modifiers == KeyModifiers::SHIFT
            && matches!(
                key,
                KeyCode::Up | KeyCode::Down | KeyCode::Home | KeyCode::End
            );
    let nav_mods = if extend && modifiers == KeyModifiers::SHIFT {
        KeyModifiers::NONE
    } else {
        modifiers
    };
    let old = selection
        .filter(|(id, _, _)| *id == tab.id)
        .map(|(_, _, row)| row)
        .unwrap_or(tab.scroll.scroll_offset);
    let page = tab.scroll.visible_height.max(1);
    let row = if kb.navigation.scroll_down.matches(key, nav_mods) {
        old.saturating_add(1)
    } else if kb.navigation.scroll_up.matches(key, nav_mods) {
        old.saturating_sub(1)
    } else if kb.navigation.page_down.matches(key, nav_mods) {
        old.saturating_add(page)
    } else if kb.navigation.page_up.matches(key, nav_mods) {
        old.saturating_sub(page)
    } else if kb.navigation.half_page_down.matches(key, nav_mods) {
        old.saturating_add((page / 2).max(1))
    } else if kb.navigation.half_page_up.matches(key, nav_mods) {
        old.saturating_sub((page / 2).max(1))
    } else if key == KeyCode::End || kb.normal.go_to_bottom.matches(key, nav_mods) {
        usize::MAX
    } else if key == KeyCode::Home {
        0
    } else if kb.normal.go_to_top_chord.matches(key, nav_mods) {
        if !tab.interaction.g_key_pressed {
            tab.interaction.g_key_pressed = true;
            return Effect::None;
        }
        0
    } else {
        return Effect::None;
    };
    tab.interaction.g_key_pressed = false;
    let to_end = row == usize::MAX;
    let row = row.min(tab.filter.visible_indices.len().saturating_sub(1));
    tab.scroll.scroll_offset = row;
    if let Some(handle) = &mut tab.filter.handle {
        handle.scroll_anchor = if to_end {
            tab.file_reader.line_count().checked_sub(1)
        } else {
            tab.filter.visible_indices.get_opt(row)
        };
    }
    tab.stream.tail_mode = false;
    let id = tab.id;
    let anchor = if extend {
        state
            .log_selection
            .filter(|(source, _, _)| *source == id)
            .map(|(_, anchor, _)| anchor)
            .unwrap_or(old)
    } else {
        row
    };
    state.log_selection = Some((id, anchor, row));
    Effect::None
}
pub fn shortcuts(state: &GuiState) -> Vec<(&'static str, String, String)> {
    let kb = bindings(state);
    vec![
        (
            "Navigate logs",
            "Move through logs".into(),
            format!(
                "{} / {}",
                kb.navigation.scroll_up.display(),
                kb.navigation.scroll_down.display()
            ),
        ),
        (
            "Navigate logs",
            "Page through logs".into(),
            format!(
                "{} / {}",
                kb.navigation.page_up.display(),
                kb.navigation.page_down.display()
            ),
        ),
        (
            "Navigate logs",
            "First / last row".into(),
            "Home / End, gg / G".into(),
        ),
        (
            "Navigate logs",
            "Extend row selection".into(),
            "Shift+Up / Shift+Down".into(),
        ),
        (
            "Files",
            "Next / previous tab".into(),
            format!(
                "{} / {}, Ctrl+Tab / Ctrl+Shift+Tab",
                kb.global.next_tab.display(),
                kb.global.prev_tab.display()
            ),
        ),
        (
            "Files",
            "Switch open file".into(),
            kb.global.file_switcher.display(),
        ),
        (
            "Files",
            "Open / close file".into(),
            format!("Ctrl+O / {}", kb.global.close_tab.display()),
        ),
        (
            "Investigate",
            "Bookmark selected rows".into(),
            format!("Ctrl+B / {}", kb.normal.mark_line.display()),
        ),
        (
            "Investigate",
            "Add annotation".into(),
            "Ctrl+Shift+A".into(),
        ),
        (
            "Investigate",
            "Next / previous search match".into(),
            format!(
                "{} / {}",
                kb.normal.next_match.display(),
                kb.normal.prev_match.display()
            ),
        ),
        (
            "Investigate",
            "Next / previous selected filter match (filter sidebar)".into(),
            format!(
                "{} / {}",
                kb.filter.next_match.display(),
                kb.filter.prev_match.display()
            ),
        ),
        (
            "General",
            "Available actions".into(),
            "Ctrl+Shift+P / :".into(),
        ),
        (
            "General",
            "Copy selected text".into(),
            "Ctrl+C / Cmd+C".into(),
        ),
        ("General", "Close a dialog".into(), "Escape".into()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn navigation_extends_ranges_clamps_and_leaves_follow_mode() {
        let db = std::sync::Arc::new(crate::db::Database::in_memory().await.unwrap());
        let manager = crate::db::LogManager::new(db.clone(), None).await;
        let mut state = GuiState::new(db);
        state.tabs.push(crate::shared::TabState::new(
            crate::ingestion::FileReader::from_bytes(b"a\nb\nc\n".to_vec()),
            manager,
            "test".into(),
        ));
        state.tabs[0].stream.tail_mode = true;
        apply(&mut state, KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(state.tabs[0].scroll.scroll_offset, 1);
        assert!(!state.tabs[0].stream.tail_mode);
        apply(&mut state, KeyCode::Down, KeyModifiers::SHIFT);
        assert_eq!(state.log_selection.unwrap().1, 1);
        assert_eq!(state.log_selection.unwrap().2, 2);
        apply(&mut state, KeyCode::End, KeyModifiers::NONE);
        assert_eq!(state.tabs[0].scroll.scroll_offset, 2);
        apply(&mut state, KeyCode::Home, KeyModifiers::NONE);
        assert_eq!(state.tabs[0].scroll.scroll_offset, 0);
        assert!(handled(&state, KeyCode::Char('p'), KeyModifiers::CONTROL));
        apply(&mut state, KeyCode::Char('p'), KeyModifiers::CONTROL);
        assert_eq!(state.editor.as_ref().unwrap().action, Action::SwitchFile);
    }
}

/// Keep the editor driven by the serialized shared keybinding schema.
pub fn editable_bindings(kb: &crate::config::Keybindings) -> Vec<(String, String)> {
    let value = serde_json::to_value(kb).expect("keybindings serialize");
    let mut entries = Vec::new();
    for (group, bindings) in value.as_object().unwrap() {
        if let Some(bindings) = bindings.as_object() {
            for (name, keys) in bindings {
                let keys = match keys {
                    serde_json::Value::String(key) => key.clone(),
                    serde_json::Value::Array(keys) => keys
                        .iter()
                        .filter_map(|key| key.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                    _ => continue,
                };
                entries.push((format!("{group}.{name}"), keys));
            }
        }
    }
    entries
}
pub fn edited_bindings(
    kb: &crate::config::Keybindings,
    fields: &[(String, bool)],
    values: &[String],
) -> Result<crate::config::Keybindings, String> {
    let mut value = serde_json::to_value(kb).map_err(|e| e.to_string())?;
    for ((name, _), keys) in fields.iter().zip(values) {
        let (group, name) = name.split_once('.').ok_or("Invalid binding name")?;
        value[group][name] = serde_json::json!(
            keys.split(", ")
                .filter(|key| !key.is_empty())
                .collect::<Vec<_>>()
        );
    }
    serde_json::from_value(value).map_err(|e| e.to_string())
}
pub fn save_bindings(
    path: &std::path::Path,
    kb: &crate::config::Keybindings,
) -> Result<(), String> {
    let mut config: serde_json::Value = match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| e.to_string())?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(error) => return Err(error.to_string()),
    };
    config["keybindings"] = serde_json::to_value(kb).map_err(|e| e.to_string())?;
    let parent = path.parent().ok_or("Missing settings directory")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    use std::io::Write;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    file.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod settings_tests {
    #[test]
    fn captured_bindings_round_trip_and_preserve_other_configuration() {
        let kb = crate::config::Keybindings::default();
        let entries = super::editable_bindings(&kb);
        let fields: Vec<_> = entries
            .iter()
            .map(|(name, _)| (name.clone(), true))
            .collect();
        let mut values: Vec<_> = entries.into_iter().map(|(_, keys)| keys).collect();
        let index = fields
            .iter()
            .position(|(name, _)| name == "normal.filter_include_auto")
            .unwrap();
        values[index] = "Ctrl+F2".into();
        let edited = super::edited_bindings(&kb, &fields, &values).unwrap();
        assert!(edited.normal.filter_include_auto.matches(
            crate::input::KeyCode::F(2),
            crate::input::KeyModifiers::CONTROL
        ));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, br#"{"theme":"dark","extra":42}"#).unwrap();
        super::save_bindings(&path, &edited).unwrap();
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(saved["extra"], 42);
        assert_eq!(saved["theme"], "dark");
        assert_eq!(
            saved["keybindings"]["normal"]["filter_include_auto"],
            "Ctrl+F2"
        );
    }
}

/// The same configured shortcut labels are used throughout the GUI.
pub fn shortcut_for_action(kb: &crate::config::Keybindings, action: Action) -> String {
    match action {
        Action::Undo => "Ctrl+Z".into(),
        Action::Redo => "Ctrl+Shift+Z".into(),
        Action::CloseFile => kb.global.close_tab.display(),
        Action::Group => kb.normal.group_mode.display(),
        _ => action_bindings(kb)
            .iter()
            .filter(|(_, a)| *a == action)
            .map(|(keys, _)| keys.display())
            .collect::<Vec<_>>()
            .join(" / "),
    }
}

pub fn mutates_tab(state: &GuiState, key: KeyCode, modifiers: KeyModifiers) -> bool {
    let kb = bindings(state);
    [
        &kb.normal.clear_all,
        &kb.normal.delete_comment,
        &kb.normal.mark_line,
        &kb.normal.toggle_filtering,
        &kb.normal.toggle_marks_only,
        &kb.normal.toggle_highlight_mode,
        &kb.normal.expand_continuation,
        &kb.normal.collapse_continuation,
    ]
    .iter()
    .any(|binding| binding.matches(key, modifiers))
        || kb
            .custom
            .iter()
            .any(|binding| binding.key.matches(key, modifiers))
}

pub fn sidebar_handled(state: &GuiState, key: KeyCode, modifiers: KeyModifiers) -> bool {
    if state.keyboard_sidebar.is_none() {
        return false;
    }
    let kb = bindings(state);
    if state.keyboard_sidebar == Some(crate::gui::state::SidebarTab::Filters)
        && (kb.filter.next_match.matches(key, modifiers)
            || kb.filter.prev_match.matches(key, modifiers))
    {
        return true;
    }
    [
        &kb.navigation.scroll_down,
        &kb.navigation.scroll_up,
        &kb.navigation.page_down,
        &kb.navigation.page_up,
        &kb.filter.toggle_filter,
        &kb.filter.delete_filter,
        &kb.filter.edit_filter,
        &kb.filter.set_color,
        &kb.filter.move_filter_up,
        &kb.filter.move_filter_down,
        &kb.filter.toggle_all_filters,
        &kb.filter.clear_all_filters,
        &kb.filter.add_include_filter,
        &kb.filter.add_include_filter_auto,
        &kb.filter.add_exclude_filter,
        &kb.filter.add_highlight_filter,
        &kb.filter.add_date_filter,
        &kb.filter.exit_mode,
        &kb.group.add_group,
    ]
    .iter()
    .any(|binding| binding.matches(key, modifiers))
}
fn apply_sidebar(state: &mut GuiState, key: KeyCode, modifiers: KeyModifiers) -> Effect {
    use crate::gui::state::SidebarTab;
    let kb = bindings(state);
    let group = state.keyboard_sidebar == Some(SidebarTab::Groups);
    if kb.filter.exit_mode.matches(key, modifiers) {
        if let Some(tab) = state.active_tab_mut() {
            tab.filter.navigation = None;
        }
        state.keyboard_sidebar = None;
        return Effect::None;
    }
    let Some(tab) = state.active_tab() else {
        return Effect::None;
    };
    let names = tab.log_manager.group_names();
    let count = if group {
        names.len()
    } else {
        tab.log_manager.get_filters().len()
    };
    state.sidebar_cursor = state.sidebar_cursor.min(count.saturating_sub(1));
    if !group
        && (kb.filter.next_match.matches(key, modifiers)
            || kb.filter.prev_match.matches(key, modifiers))
    {
        let selected = state.sidebar_cursor;
        state.tabs[state.active_tab].begin_selected_filter_match(
            selected,
            kb.filter.next_match.matches(key, modifiers),
            1,
        );
        return Effect::None;
    }
    if kb.navigation.scroll_down.matches(key, modifiers)
        || kb.navigation.page_down.matches(key, modifiers)
    {
        state.tabs[state.active_tab].filter.navigation = None;
        state.sidebar_cursor = (state.sidebar_cursor
            + if kb.navigation.page_down.matches(key, modifiers) {
                10
            } else {
                1
            })
        .min(count.saturating_sub(1));
        return Effect::None;
    }
    if kb.navigation.scroll_up.matches(key, modifiers)
        || kb.navigation.page_up.matches(key, modifiers)
    {
        state.tabs[state.active_tab].filter.navigation = None;
        state.sidebar_cursor =
            state
                .sidebar_cursor
                .saturating_sub(if kb.navigation.page_up.matches(key, modifiers) {
                    10
                } else {
                    1
                });
        return Effect::None;
    }
    let index = state.active_tab;
    if group {
        if kb.group.add_group.matches(key, modifiers) {
            return crate::gui::actions::open(state, Action::Group, None, None);
        }
        if let Some(name) = names.get(state.sidebar_cursor) {
            let message = if kb.filter.toggle_filter.matches(key, modifiers) {
                Some(Message::GroupToggled(index, name.clone()))
            } else if kb.filter.delete_filter.matches(key, modifiers) {
                Some(Message::GroupDeleted(index, name.clone()))
            } else if kb.filter.edit_filter.matches(key, modifiers)
                || kb.filter.set_color.matches(key, modifiers)
            {
                Some(Message::EditGroup(name.clone()))
            } else {
                None
            };
            if let Some(message) = message {
                return crate::gui::update::update(state, message);
            }
        }
        return Effect::None;
    }
    if kb.filter.add_include_filter.matches(key, modifiers)
        || kb.filter.add_include_filter_auto.matches(key, modifiers)
        || kb.filter.add_exclude_filter.matches(key, modifiers)
        || kb.filter.add_highlight_filter.matches(key, modifiers)
        || kb.filter.add_date_filter.matches(key, modifiers)
    {
        let effect = crate::gui::actions::open(state, Action::Filter, None, None);
        if let Some(editor) = &mut state.editor {
            editor.kind = if kb.filter.add_exclude_filter.matches(key, modifiers) {
                crate::filters::FilterType::Exclude
            } else if kb.filter.add_highlight_filter.matches(key, modifiers) {
                crate::filters::FilterType::Highlight
            } else {
                crate::filters::FilterType::Include
            };
            if kb.filter.add_date_filter.matches(key, modifiers) {
                editor.values[0] = crate::filters::DATE_PREFIX.into();
            }
            if kb.filter.add_include_filter_auto.matches(key, modifiers) {
                editor.regenerate_auto_style();
            }
        }
        return effect;
    }
    if kb.filter.clear_all_filters.matches(key, modifiers) {
        return crate::gui::update::update(state, Message::ClearAllFilters(index));
    }
    if kb.filter.toggle_all_filters.matches(key, modifiers) {
        let action = if state.tabs[index]
            .log_manager
            .get_filters()
            .iter()
            .all(|f| f.enabled)
        {
            Action::DisableFilters
        } else {
            Action::EnableFilters
        };
        return crate::gui::actions::open(state, action, None, None);
    }
    if let Some(filter) = state.tabs[index]
        .log_manager
        .get_filters()
        .get(state.sidebar_cursor)
    {
        let id = filter.id;
        if kb.filter.edit_filter.matches(key, modifiers)
            || kb.filter.set_color.matches(key, modifiers)
        {
            return crate::gui::update::update(state, Message::EditFilter(id));
        }
        if kb.filter.toggle_filter.matches(key, modifiers) {
            return crate::gui::update::update(state, Message::FilterToggled(index, id));
        }
        if kb.filter.delete_filter.matches(key, modifiers) {
            return crate::gui::update::update(state, Message::FilterRemoved(index, id));
        }
        let target = if kb.filter.move_filter_up.matches(key, modifiers) {
            state.sidebar_cursor.saturating_sub(1)
        } else {
            (state.sidebar_cursor + 1).min(count.saturating_sub(1))
        };
        if let Some(other) = state.tabs[index].log_manager.get_filters().get(target) {
            let other = other.id;
            state.sidebar_cursor = target;
            return crate::gui::update::update(state, Message::FilterReordered(index, id, other));
        }
    }
    Effect::None
}

#[cfg(test)]
mod sidebar_tests {
    use super::*;
    #[tokio::test]
    async fn selected_filter_navigation_respects_custom_keys_and_sidebar_scope() {
        let db = std::sync::Arc::new(crate::db::Database::in_memory().await.unwrap());
        let mut manager = crate::db::LogManager::new(db.clone(), None).await;
        manager
            .add_filter_with_color(
                "hit".into(),
                crate::filters::FilterType::Highlight,
                crate::filters::FilterOptions::default(),
            )
            .await;
        let mut state = GuiState::new(db);
        let mut tab = crate::shared::TabState::new(
            crate::ingestion::FileReader::from_bytes(b"start\nhit\n".to_vec()),
            manager,
            "test".into(),
        );
        let mut keys = crate::config::Keybindings::default();
        keys.filter.next_match = serde_json::from_str("\"F6\"").unwrap();
        tab.interaction.keybindings = std::sync::Arc::new(keys);
        state.tabs.push(tab);
        assert!(!sidebar_handled(&state, KeyCode::F(6), KeyModifiers::NONE));
        state.keyboard_sidebar = Some(crate::gui::state::SidebarTab::Groups);
        assert!(!sidebar_handled(&state, KeyCode::F(6), KeyModifiers::NONE));
        state.keyboard_sidebar = Some(crate::gui::state::SidebarTab::Filters);
        assert!(sidebar_handled(&state, KeyCode::F(6), KeyModifiers::NONE));
        assert!(!sidebar_handled(
            &state,
            KeyCode::Char('n'),
            KeyModifiers::NONE
        ));
        apply(&mut state, KeyCode::F(6), KeyModifiers::NONE);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if state.tabs[0].advance_selected_filter_match().is_some() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(state.tabs[0].scroll.scroll_offset, 1);
        assert!(state.tabs[0].search.query.get_pattern().is_none());
    }

    #[tokio::test]
    async fn filter_keys_are_scoped_and_auto_filter_opens_a_prefilled_draft() {
        let db = std::sync::Arc::new(crate::db::Database::in_memory().await.unwrap());
        let mut manager = crate::db::LogManager::new(db.clone(), None).await;
        for pattern in ["alpha", "beta"] {
            manager
                .add_filter_with_color(
                    pattern.into(),
                    crate::filters::FilterType::Highlight,
                    crate::filters::FilterOptions::default(),
                )
                .await;
        }
        let mut state = GuiState::new(db);
        state.tabs.push(crate::shared::TabState::new(
            crate::ingestion::FileReader::from_bytes(b"alpha\nbeta\n".to_vec()),
            manager,
            "test".into(),
        ));
        apply(&mut state, KeyCode::Char('f'), KeyModifiers::NONE);
        assert_eq!(
            state.keyboard_sidebar,
            Some(crate::gui::state::SidebarTab::Filters)
        );
        assert!(state.editor.is_none());
        apply(&mut state, KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(state.sidebar_cursor, 1);
        apply(&mut state, KeyCode::Char(' '), KeyModifiers::NONE);
        assert!(!state.tabs[0].log_manager.get_filters()[1].enabled);
        assert!(state.tabs[0].log_manager.get_filters()[0].enabled);
        apply(&mut state, KeyCode::Esc, KeyModifiers::NONE);
        assert!(state.keyboard_sidebar.is_none());
        apply(&mut state, KeyCode::Char('a'), KeyModifiers::NONE);
        let editor = state.editor.as_ref().unwrap();
        assert_eq!(editor.action, Action::Filter);
        assert!(editor.auto_style);
        // `keyboard::apply` alone never prefills from the cursor line —
        // `App::dispatch` fills `values[0]` from a manual text selection
        // (if any) afterward; see `auto_filter_prefills_from_a_manual_text_selection_only`.
        assert_eq!(editor.values[0], "");
    }
}
