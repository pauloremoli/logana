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
        (&kb.normal.group_mode, Action::Group),
        (&kb.normal.filter_include, Action::Filter),
        (&kb.normal.filter_include_auto, Action::Filter),
        (&kb.normal.filter_exclude, Action::Filter),
        (&kb.normal.filter_mode, Action::Filter),
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
    action_bindings(&kb)
        .iter()
        .any(|(binding, _)| binding.matches(key, modifiers))
        || kb
            .custom
            .iter()
            .any(|binding| binding.key.matches(key, modifiers))
        || [
            &kb.normal.search_forward,
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
    if let Some(custom) = kb
        .custom
        .iter()
        .find(|binding| binding.key.matches(key, modifiers))
    {
        use clap::Parser;
        if let Ok(parsed) = crate::commands::CommandLine::try_parse_from(
            crate::commands::auto_complete::shell_split(&custom.command),
        ) {
            if let Some(command) = parsed.command {
                return crate::gui::update::perform_gui(state, command, custom.command.clone());
            }
        }
        return Effect::None;
    }
    if kb.normal.search_forward.matches(key, modifiers) {
        return crate::gui::update::update(state, Message::SearchPopup(true));
    }
    if kb.normal.clear_search.matches(key, modifiers) {
        return crate::gui::update::update(state, Message::SearchEdited(String::new()));
    }
    if kb.normal.expand_continuation.matches(key, modifiers)
        || kb.normal.collapse_continuation.matches(key, modifiers)
    {
        if let Some(tab) = state.active_tab_mut() {
            if let Some(line) = tab.filter.visible_indices.get_opt(tab.scroll.scroll_offset) {
                if let Some(parent) = tab
                    .active_continuation_map()
                    .and_then(|map| map.get(line))
                    .copied()
                {
                    tab.set_continuation_collapsed(
                        parent,
                        kb.normal.collapse_continuation.matches(key, modifiers),
                    );
                }
            }
        }
        return Effect::None;
    }
    if kb.normal.toggle_highlight_mode.matches(key, modifiers) {
        if let Some(tab) = state.active_tab_mut() {
            tab.filter.highlight_mode = !tab.filter.highlight_mode;
            tab.begin_filter_refresh();
        }
        return Effect::None;
    }
    if kb.normal.yank_line.matches(key, modifiers) || kb.normal.yank_marked.matches(key, modifiers)
    {
        if let Some(tab) = state.active_tab() {
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
    }
    if let Some((_, action)) = action_bindings(&kb)
        .iter()
        .find(|(binding, _)| binding.matches(key, modifiers))
    {
        let effect = crate::gui::actions::open(state, *action, None, None);
        if *action == Action::Filter {
            let selected = state
                .active_tab()
                .and_then(|tab| {
                    tab.filter
                        .visible_indices
                        .get_opt(tab.scroll.scroll_offset)
                        .map(|line| {
                            crate::gui::view::log_pane::raw_log_message(&tab.file_reader, line)
                        })
                })
                .unwrap_or_default();
            if let Some(editor) = &mut state.editor {
                editor.kind = if kb.normal.filter_exclude.matches(key, modifiers) {
                    crate::filters::FilterType::Exclude
                } else {
                    crate::filters::FilterType::Include
                };
                editor.auto_style = kb.normal.filter_include_auto.matches(key, modifiers);
                if editor.auto_style {
                    editor.values[0] = selected;
                }
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
    let Some(tab) = state.active_tab_mut() else {
        return Effect::None;
    };
    let extend = modifiers == KeyModifiers::SHIFT
        && matches!(
            key,
            KeyCode::Up | KeyCode::Down | KeyCode::Home | KeyCode::End
        );
    let nav_mods = if extend {
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
    let row = row.min(tab.filter.visible_indices.len().saturating_sub(1));
    tab.scroll.scroll_offset = row;
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
pub fn shortcuts(state: &GuiState) -> Vec<(String, String)> {
    let kb = bindings(state);
    vec![
        (
            "Move through logs".into(),
            format!(
                "{} / {}",
                kb.navigation.scroll_up.display(),
                kb.navigation.scroll_down.display()
            ),
        ),
        (
            "Page through logs".into(),
            format!(
                "{} / {}",
                kb.navigation.page_up.display(),
                kb.navigation.page_down.display()
            ),
        ),
        ("First / last row".into(), "Home / End, gg / G".into()),
        (
            "Extend row selection".into(),
            "Shift+Up / Shift+Down".into(),
        ),
        (
            "Next / previous tab".into(),
            format!(
                "{} / {}, Ctrl+Tab / Ctrl+Shift+Tab",
                kb.global.next_tab.display(),
                kb.global.prev_tab.display()
            ),
        ),
        ("Switch open file".into(), kb.global.file_switcher.display()),
        (
            "Open / close file".into(),
            format!("Ctrl+O / {}", kb.global.close_tab.display()),
        ),
        (
            "Bookmark selected rows".into(),
            format!("Ctrl+B / {}", kb.normal.mark_line.display()),
        ),
        ("Add annotation".into(), "Ctrl+Shift+A".into()),
        (
            "Next / previous search match".into(),
            format!(
                "{} / {}",
                kb.normal.next_match.display(),
                kb.normal.prev_match.display()
            ),
        ),
        ("Available actions".into(), "Ctrl+Shift+P / :".into()),
        ("Copy selected text".into(), "Ctrl+C / Cmd+C".into()),
        ("Close a dialog".into(), "Escape".into()),
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
