//! GUI actions and drafts. No terminal modes or command-line grammar.
use crate::commands::Commands;
use crate::filters::{FIELD_PREFIX, FilterType};
use crate::gui::{effect::Effect, state::GuiState};
use crate::shared::TabId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Open,
    Filter,
    Group,
    Columns,
    Appearance,
    ValueColors,
    LevelColors,
    Schema,
    FileInfo,
    View,
    ClearFilters,
    EnableFilters,
    DisableFilters,
    Stop,
    Pause,
    Tail,
}
#[derive(Clone, Copy)]
pub struct Descriptor {
    pub action: Action,
    pub label: &'static str,
    pub description: &'static str,
    pub aliases: &'static str,
    pub tab: bool,
}
impl Descriptor {
    pub fn unavailable(&self, state: &GuiState) -> Option<&'static str> {
        if self.tab && state.active_tab().is_none() {
            Some("Open a log file first")
        } else {
            None
        }
    }
}
pub const CATALOG: &[Descriptor] = &[
    Descriptor {
        action: Action::Open,
        label: "Open file",
        description: "Choose a log file",
        aliases: "open",
        tab: false,
    },
    Descriptor {
        action: Action::Filter,
        label: "Add filter",
        description: "Include, exclude, or highlight matching logs",
        aliases: "filter include exclude highlight",
        tab: true,
    },
    Descriptor {
        action: Action::Group,
        label: "Add group",
        description: "Create a named filter group and style",
        aliases: "group",
        tab: true,
    },
    Descriptor {
        action: Action::Columns,
        label: "Choose columns",
        description: "Show, hide, and reorder log fields",
        aliases: "columns select-fields hide-field show-field show-all-fields",
        tab: true,
    },
    Descriptor {
        action: Action::Appearance,
        label: "Change theme",
        description: "Preview and apply an application theme",
        aliases: "theme set-theme appearance",
        tab: false,
    },
    Descriptor {
        action: Action::ValueColors,
        label: "Value colors",
        description: "Enable or disable value coloring categories",
        aliases: "value-colors",
        tab: false,
    },
    Descriptor {
        action: Action::LevelColors,
        label: "Level colors",
        description: "Configure coloring for log levels",
        aliases: "level-colors",
        tab: true,
    },
    Descriptor {
        action: Action::Schema,
        label: "Choose schema",
        description: "Select a built-in or custom parser",
        aliases: "schema",
        tab: true,
    },
    Descriptor {
        action: Action::FileInfo,
        label: "File information",
        description: "Inspect and copy the source path",
        aliases: "file-info path",
        tab: true,
    },
    Descriptor {
        action: Action::View,
        label: "Display options",
        description: "Wrapping, line numbers, field keys, sidebar, and raw display",
        aliases: "view display wrap line-numbers relative-line-numbers show-keys hide-keys raw collapse expand sidebar-position",
        tab: true,
    },
    Descriptor {
        action: Action::ClearFilters,
        label: "Clear filters",
        description: "Remove all filters from this tab",
        aliases: "clear-filters",
        tab: true,
    },
    Descriptor {
        action: Action::EnableFilters,
        label: "Enable all filters",
        description: "Enable every filter in this tab",
        aliases: "enable-filters",
        tab: true,
    },
    Descriptor {
        action: Action::DisableFilters,
        label: "Disable all filters",
        description: "Disable every filter in this tab",
        aliases: "disable-filters",
        tab: true,
    },
    Descriptor {
        action: Action::Stop,
        label: "Stop file watching",
        description: "Stop watching this file for new logs",
        aliases: "stop",
        tab: true,
    },
    Descriptor {
        action: Action::Pause,
        label: "Pause or resume stream",
        description: "Toggle live file watching",
        aliases: "pause resume",
        tab: true,
    },
    Descriptor {
        action: Action::Tail,
        label: "Follow latest logs",
        description: "Toggle automatic scrolling to the newest logs",
        aliases: "tail",
        tab: true,
    },
];
pub fn matching(query: &str) -> Vec<&'static Descriptor> {
    let query = query.trim();
    CATALOG
        .iter()
        .filter(|d| {
            crate::commands::auto_complete::fuzzy_match(query, d.label)
                || d.aliases
                    .split_whitespace()
                    .any(|alias| crate::commands::auto_complete::fuzzy_match(query, alias))
        })
        .collect()
}
#[derive(Clone, Debug)]
pub struct Editor {
    pub action: Action,
    pub source: Option<TabId>,
    pub epoch: u64,
    pub values: [String; 6],
    pub kind: FilterType,
    pub regex: bool,
    pub ignore_case: bool,
    pub whole_line: bool,
    pub fields: Vec<(String, bool)>,
    pub defaults: Vec<(String, bool)>,
    pub conditions: Vec<(String, String)>,
    pub members: Vec<(usize, String, bool)>,
    pub discovery: crate::commands::auto_complete::FieldIndex,
    pub original_theme: crate::theme::Theme,
    pub error: Option<String>,
    pub edit_id: Option<usize>,
    pub group_name_locked: bool,
}
pub fn open(
    state: &mut GuiState,
    action: Action,
    edit_id: Option<usize>,
    group: Option<String>,
) -> Effect {
    if state.editor.is_some() || state.palette.pending() {
        return Effect::None;
    }
    let descriptor = CATALOG.iter().find(|d| d.action == action).unwrap();
    if descriptor.unavailable(state).is_some() {
        return Effect::None;
    }
    state.palette.open = false;
    if action == Action::Open {
        return Effect::OpenFileDialog;
    }
    let cmd = match action {
        Action::ClearFilters => Some(Commands::ClearFilters),
        Action::EnableFilters => Some(Commands::EnableFilters),
        Action::DisableFilters => Some(Commands::DisableFilters),
        Action::Stop => Some(Commands::Stop),
        Action::Pause => Some(if state.active_tab().unwrap().stream.paused {
            Commands::Resume
        } else {
            Commands::Pause
        }),
        Action::Tail => Some(Commands::Tail),
        _ => None,
    };
    if let Some(cmd) = cmd {
        return crate::gui::update::perform_gui(state, cmd, descriptor.label.into());
    }
    let discovery = if action == Action::Filter {
        let tab = state.active_tab().unwrap();
        let key = (tab.id, tab.cache.parse_gen, tab.file_reader.line_count());
        if let Some((id, generation, count, fields)) = &state.field_discovery {
            if (*id, *generation, *count) == key {
                fields.clone()
            } else {
                let fields = tab.build_field_index();
                state.field_discovery = Some((key.0, key.1, key.2, fields.clone()));
                fields
            }
        } else {
            let fields = tab.build_field_index();
            state.field_discovery = Some((key.0, key.1, key.2, fields.clone()));
            fields
        }
    } else {
        Default::default()
    };
    let mut e = Editor {
        action,
        source: if descriptor.tab {
            state.active_tab().map(|t| t.id)
        } else {
            None
        },
        epoch: state.palette.focus_epoch.wrapping_add(1),
        values: Default::default(),
        kind: FilterType::Include,
        regex: false,
        ignore_case: false,
        whole_line: false,
        fields: Vec::new(),
        defaults: Vec::new(),
        conditions: Vec::new(),
        members: Vec::new(),
        discovery,
        original_theme: state.theme.clone(),
        error: None,
        edit_id,
        group_name_locked: false,
    };
    if let Some(tab) = state.active_tab_mut() {
        if action == Action::Columns {
            let mut names = tab.collect_field_names();
            if tab.display.format.is_none() && !names.iter().any(|n| n == "message") {
                names.push("message".into());
            }
            e.defaults = names.iter().map(|name| (name.clone(), true)).collect();
            let mut order = tab
                .display
                .field_layout
                .columns
                .clone()
                .unwrap_or_else(|| names.clone());
            for n in names {
                if !order.contains(&n) {
                    order.push(n);
                }
            }
            e.fields = order
                .into_iter()
                .map(|n| {
                    let enabled = !tab.display.hidden_fields.contains(&n);
                    (n, enabled)
                })
                .collect();
        }
        if action == Action::Filter {
            if let Some(f) =
                edit_id.and_then(|id| tab.log_manager.get_filters().iter().find(|f| f.id == id))
            {
                e.values[0] = f.pattern.clone();
                e.values[3] = f.group.clone().unwrap_or_default();
                e.kind = f.filter_type.clone();
                e.regex = f.use_regex;
                e.ignore_case = f.ignore_case;
                if let Some(expr) = f
                    .pattern
                    .strip_prefix(FIELD_PREFIX)
                    .and_then(|x| crate::filters::parse_field_filter_expr(x).ok())
                {
                    e.conditions = expr.0;
                    e.values[0] = expr.1.unwrap_or_default();
                }
                if let Some(c) = &f.color_config {
                    e.values[4] = c.fg.map(|c| c.to_string()).unwrap_or_default();
                    e.values[5] = c.bg.map(|c| c.to_string()).unwrap_or_default();
                    e.whole_line = !c.match_only;
                }
            }
        }
        if action == Action::Group {
            if let Some(name) = group {
                e.group_name_locked = true;
                e.values[0] = name.clone();
                if let Some(g) = tab
                    .log_manager
                    .get_group_styles()
                    .iter()
                    .find(|g| g.name == name)
                {
                    if let Some(c) = &g.color_config {
                        e.values[4] = c.fg.map(|c| c.to_string()).unwrap_or_default();
                        e.values[5] = c.bg.map(|c| c.to_string()).unwrap_or_default();
                        e.whole_line = !c.match_only;
                    }
                }
            }
        }
        if action == Action::Group {
            e.members = tab
                .log_manager
                .get_filters()
                .iter()
                .map(|f| {
                    (
                        f.id,
                        f.pattern.clone(),
                        f.group.as_deref() == Some(e.values[0].as_str()) && !e.values[0].is_empty(),
                    )
                })
                .collect();
        }
        if action == Action::Schema {
            e.values[0] = tab
                .display
                .format
                .as_ref()
                .map(|f| f.name().to_string())
                .unwrap_or_else(|| "none".into());
        }
        if action == Action::LevelColors {
            e.fields = [
                "trace", "debug", "info", "notice", "warning", "error", "fatal",
            ]
            .iter()
            .map(|k| {
                (
                    k.to_string(),
                    !tab.display.level_colors_disabled.contains(*k),
                )
            })
            .collect();
        }
    }
    if action == Action::ValueColors {
        e.fields = state
            .theme
            .value_colors
            .grouped_categories(state.theme.process_colors.first().copied())
            .into_iter()
            .flat_map(|g| {
                g.children.into_iter().map(|(k, _, _)| {
                    (
                        k.to_string(),
                        !state.theme.value_colors.disabled.contains(k),
                    )
                })
            })
            .collect();
    }
    if e.action != Action::Columns {
        e.defaults = e.fields.clone();
    }
    state.palette.open = false;
    state.time_range_open = false;
    state.editor = Some(e);
    if action == Action::FileInfo {
        let output = state
            .active_tab()
            .and_then(|t| t.display_source_path())
            .unwrap_or("This tab has no source file.")
            .to_string();
        crate::gui::update::gui_outcome(
            state,
            "File information".into(),
            crate::gui::palette::ExecutionState::Information,
            output,
        );
    }
    Effect::None
}
pub fn close(state: &mut GuiState, applied: bool) {
    if let Some(e) = state.editor.take() {
        if !applied && !state.palette.pending() && e.action == Action::Appearance {
            state.theme = e.original_theme;
        }
        state.palette.open = false;
        state.palette.focus_epoch = state.palette.focus_epoch.wrapping_add(1);
        if !applied && state.palette.outcome.is_none() {
            crate::gui::update::gui_outcome(
                state,
                "Canceled".into(),
                crate::gui::palette::ExecutionState::Information,
                "Changes canceled.".into(),
            );
        }
    }
}
pub fn apply(state: &mut GuiState) -> Effect {
    let Some(e) = state.editor.clone() else {
        return Effect::None;
    };
    if state.palette.pending() {
        return Effect::None;
    }
    if let Some(source) = e.source {
        let Some(idx) = state.tabs.iter().position(|t| t.id == source) else {
            state.editor.as_mut().unwrap().error = Some("The source tab was closed.".into());
            return Effect::None;
        };
        state.active_tab = idx;
    }
    let nonempty = |s: &String| {
        if s.trim().is_empty() {
            None
        } else {
            Some(s.clone())
        }
    };
    let invalid = if matches!(e.action, Action::Filter | Action::Group) {
        if e.action == Action::Group && e.values[0].trim().is_empty() {
            Some("Enter a group name.".into())
        } else if e.action == Action::Filter && e.values[0].is_empty() && e.conditions.is_empty() {
            Some("Enter text or add a field condition.".into())
        } else if e.regex && regex::Regex::new(&e.values[0]).is_err() {
            Some("The regular expression is invalid.".into())
        } else {
            e.values[4..6]
                .iter()
                .find(|v| !v.is_empty() && crate::theme::parse_color(v).is_none())
                .map(|v| format!("Invalid color: {v}"))
        }
    } else {
        None
    };
    if let Some(error) = invalid {
        state.editor.as_mut().unwrap().error = Some(error);
        return Effect::None;
    }
    let command = match e.action {
        Action::Filter => {
            let field = e
                .conditions
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            let pattern = if e.values[0].is_empty() {
                vec![]
            } else {
                vec![e.values[0].clone()]
            };
            match e.kind {
                FilterType::Exclude => Commands::Exclude {
                    pattern,
                    field,
                    regex: e.regex,
                    ignore_case: e.ignore_case,
                    group: nonempty(&e.values[3]),
                },
                FilterType::Highlight => Commands::Highlight {
                    pattern,
                    field,
                    regex: e.regex,
                    ignore_case: e.ignore_case,
                    group: nonempty(&e.values[3]),
                    fg: nonempty(&e.values[4]),
                    bg: nonempty(&e.values[5]),
                    line_mode: e.whole_line,
                    auto: false,
                    fga: false,
                },
                _ => Commands::Filter {
                    pattern,
                    field,
                    regex: e.regex,
                    ignore_case: e.ignore_case,
                    group: nonempty(&e.values[3]),
                    fg: nonempty(&e.values[4]),
                    bg: nonempty(&e.values[5]),
                    line_mode: e.whole_line,
                    auto: false,
                    fga: false,
                },
            }
        }
        Action::Group => Commands::Group {
            name: e.values[0].clone(),
            fg: nonempty(&e.values[4]),
            bg: nonempty(&e.values[5]),
            line_mode: e.whole_line,
            auto: false,
            clear: false,
        },
        Action::Schema => Commands::Schema {
            name: Some(e.values[0].clone()),
        },
        Action::Appearance => Commands::SetTheme {
            theme_name: e.values[0].clone(),
        },
        Action::Columns => {
            let tab = state.active_tab_mut().unwrap();
            tab.display.field_layout.columns =
                Some(e.fields.iter().map(|(n, _)| n.clone()).collect());
            tab.display.hidden_fields = e
                .fields
                .iter()
                .filter(|(_, on)| !*on)
                .map(|(n, _)| n.clone())
                .collect();
            tab.invalidate_parse_cache();
            close(state, true);
            crate::gui::update::gui_outcome(
                state,
                "Choose columns".into(),
                crate::gui::palette::ExecutionState::Success,
                "Column visibility and order updated.".into(),
            );
            return Effect::None;
        }
        Action::ValueColors | Action::LevelColors => {
            let disabled = e
                .fields
                .iter()
                .filter(|(_, on)| !*on)
                .map(|(n, _)| n.clone())
                .collect();
            if e.action == Action::ValueColors {
                state.theme.value_colors.disabled = disabled;
            } else {
                state
                    .active_tab_mut()
                    .unwrap()
                    .display
                    .level_colors_disabled = disabled;
            }
            close(state, true);
            crate::gui::update::gui_outcome(
                state,
                "Color categories".into(),
                crate::gui::palette::ExecutionState::Success,
                "Color categories updated.".into(),
            );
            return Effect::None;
        }
        _ => {
            close(state, true);
            return Effect::None;
        }
    };
    if e.action == Action::Filter {
        let pattern = if e.conditions.is_empty() {
            e.values[0].clone()
        } else {
            crate::filters::encode_field_filter(&e.conditions, nonempty(&e.values[0]).as_deref())
        };
        let options = crate::filters::FilterOptions {
            fg: nonempty(&e.values[4]),
            bg: nonempty(&e.values[5]),
            match_only: !e.whole_line,
            use_regex: e.regex,
            ignore_case: e.ignore_case,
            group: nonempty(&e.values[3]),
        };
        crate::gui::update::gui_outcome(
            state,
            if e.edit_id.is_some() {
                "Edit filter"
            } else {
                "Add filter"
            }
            .into(),
            crate::gui::palette::ExecutionState::Running,
            "Saving filter…".into(),
        );
        return Effect::SaveGuiFilter {
            request_id: state.palette.next_request,
            tab_id: e.source.unwrap(),
            log_manager: state.active_tab().unwrap().log_manager.clone(),
            id: e.edit_id,
            pattern,
            kind: e.kind,
            options,
        };
    }
    if e.action == Action::Group {
        crate::gui::update::gui_outcome(
            state,
            "Save group".into(),
            crate::gui::palette::ExecutionState::Running,
            "Saving group…".into(),
        );
        return Effect::SaveGuiGroup {
            request_id: state.palette.next_request,
            tab_id: e.source.unwrap(),
            log_manager: state.active_tab().unwrap().log_manager.clone(),
            name: e.values[0].clone(),
            fg: nonempty(&e.values[4]),
            bg: nonempty(&e.values[5]),
            whole_line: e.whole_line,
            members: e.members.into_iter().map(|(id, _, on)| (id, on)).collect(),
        };
    }
    crate::gui::update::perform_gui(
        state,
        command,
        CATALOG
            .iter()
            .find(|d| d.action == e.action)
            .unwrap()
            .label
            .into(),
    )
}

/// Persist native filter drafts with explicit storage errors before refreshing the snapshot.
pub async fn save_filter(
    mut manager: crate::db::LogManager,
    id: Option<usize>,
    pattern: String,
    kind: FilterType,
    options: crate::filters::FilterOptions,
) -> Result<crate::db::LogManager, String> {
    use crate::db::FilterStore;
    let db = manager.db.clone();
    let source = manager.source_file().map(str::to_owned);
    let color = if kind == FilterType::Exclude {
        None
    } else {
        crate::filters::ColorConfig::try_from(&options).ok()
    };
    let id = id.or_else(|| {
        manager
            .get_filters()
            .iter()
            .find(|f| f.pattern == pattern && f.filter_type == kind)
            .map(|f| f.id)
    });
    if let Some(id) = id {
        db.update_filter(
            id as i64,
            &pattern,
            &kind,
            color.as_ref(),
            options.use_regex,
            options.ignore_case,
            options.group.as_deref(),
        )
        .await
        .map_err(|e| e.to_string())?;
    } else {
        let mut insert = crate::filters::FilterInsertOptions::new();
        if let Some(c) = color {
            insert = insert.color(c);
        }
        if let Some(s) = &source {
            insert = insert.source(s.clone());
        }
        insert.use_regex = options.use_regex;
        insert.ignore_case = options.ignore_case;
        insert.group = options.group;
        db.insert_filter(&pattern, &kind, insert)
            .await
            .map_err(|e| e.to_string())?;
    }
    manager.reload_checked().await.map_err(|e| e.to_string())?;
    Ok(manager)
}
pub async fn save_group(
    mut manager: crate::db::LogManager,
    name: String,
    fg: Option<String>,
    bg: Option<String>,
    whole_line: bool,
    members: Vec<(usize, bool)>,
) -> Result<crate::db::LogManager, String> {
    use crate::db::{FilterStore, GroupStore};
    let db = manager.db.clone();
    let source = manager.source_file().map(str::to_owned);
    let color = crate::filters::ColorConfig {
        fg: fg.as_deref().and_then(crate::theme::parse_color),
        bg: bg.as_deref().and_then(crate::theme::parse_color),
        match_only: !whole_line,
    };
    db.upsert_group_style(source.as_deref().unwrap_or(""), &name, &color)
        .await
        .map_err(|e| e.to_string())?;
    for (id, on) in members {
        let Some(filter) = manager.get_filters().iter().find(|f| f.id == id) else {
            continue;
        };
        if on {
            db.update_filter_group(id as i64, Some(&name))
                .await
                .map_err(|e| e.to_string())?;
        } else if filter.group.as_deref() == Some(name.as_str()) {
            db.update_filter_group(id as i64, None)
                .await
                .map_err(|e| e.to_string())?;
        }
    }
    manager.reload_checked().await.map_err(|e| e.to_string())?;
    Ok(manager)
}

pub async fn clear_dates(
    mut manager: crate::db::LogManager,
) -> Result<crate::db::LogManager, String> {
    use crate::db::FilterStore;
    let ids: Vec<_> = manager
        .get_filters()
        .iter()
        .filter(|f| f.pattern.starts_with(crate::filters::DATE_PREFIX))
        .map(|f| f.id)
        .collect();
    for id in ids {
        manager
            .db
            .delete_filter(id as i64)
            .await
            .map_err(|e| e.to_string())?;
    }
    manager.reload_checked().await.map_err(|e| e.to_string())?;
    Ok(manager)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::{
        message::Message,
        palette::ExecutionState,
        update::{gui_outcome, update},
    };
    async fn state() -> GuiState {
        let db = std::sync::Arc::new(crate::db::Database::in_memory().await.unwrap());
        let manager = crate::db::LogManager::new(db.clone(), None).await;
        let mut state = GuiState::new(db);
        state.tabs.push(crate::shared::TabState::new(
            crate::ingestion::FileReader::from_bytes(b"hello\nworld\n".to_vec()),
            manager,
            "source.log".into(),
        ));
        state
    }
    #[test]
    fn fuzzy_labels_aliases_and_empty_catalog() {
        assert_eq!(matching("chth")[0].action, Action::Appearance);
        assert_eq!(matching("select-fields")[0].action, Action::Columns);
        for (query, action) in [
            ("columns", Action::Columns),
            ("view", Action::View),
            ("schema", Action::Schema),
            ("file-info", Action::FileInfo),
        ] {
            assert!(matching(query).iter().any(|entry| entry.action == action));
        }
        assert!(matching("東京").is_empty());
        assert!(matching("").iter().all(|d| !d.label.contains("recent")));
    }
    #[tokio::test]
    async fn idempotent_launcher_and_editor_shortcut_isolation() {
        let mut state = state().await;
        update(&mut state, Message::OpenCommands);
        update(&mut state, Message::PaletteEdited("theme".into()));
        update(&mut state, Message::OpenCommands);
        assert_eq!(state.palette.input, "theme");
        update(&mut state, Message::PaletteInsert(0));
        assert!(!state.palette.open);
        update(&mut state, Message::OpenCommands);
        assert!(!state.palette.open);
        close(&mut state, false);
        assert!(!state.palette.open);
    }
    #[tokio::test]
    async fn selected_actions_and_finished_editors_return_to_main_ui() {
        let mut state = state().await;
        for applied in [false, true] {
            update(&mut state, Message::OpenCommands);
            update(&mut state, Message::PaletteEdited("select-fields".into()));
            update(&mut state, Message::PaletteInsert(0));
            assert!(!state.palette.open);
            assert!(state.editor.is_some());
            if applied {
                apply(&mut state);
            } else {
                close(&mut state, false);
            }
            assert!(!state.palette.open);
            assert!(state.editor.is_none());
        }
        update(&mut state, Message::OpenCommands);
        update(&mut state, Message::PaletteEdited("tail".into()));
        update(&mut state, Message::PaletteInsert(0));
        assert!(!state.palette.open);
        assert!(state.editor.is_none());
        assert!(!state.palette.pending());
        update(&mut state, Message::OpenCommands);
        update(&mut state, Message::PaletteEdited("clear-filters".into()));
        let effect = update(&mut state, Message::PaletteInsert(0));
        assert!(!state.palette.open);
        assert!(state.editor.is_none());
        assert!(matches!(effect, Effect::ExecutePalette { .. }));
        assert!(state.palette.pending());
    }

    #[tokio::test]
    async fn no_tab_context_and_global_appearance() {
        let mut state = state().await;
        state.tabs.clear();
        assert!(
            CATALOG
                .iter()
                .find(|d| d.action == Action::Columns)
                .unwrap()
                .unavailable(&state)
                .is_some()
        );
        open(&mut state, Action::Columns, None, None);
        assert!(state.editor.is_none());
        open(&mut state, Action::Appearance, None, None);
        assert!(state.editor.as_ref().unwrap().source.is_none());
    }
    #[tokio::test]
    async fn preview_cancel_and_apply_restore_committed_theme() {
        let mut state = state().await;
        let original = state.theme.clone();
        open(&mut state, Action::Appearance, None, None);
        update(&mut state, Message::EditorPreview("github-light".into()));
        assert_ne!(state.theme, original);
        close(&mut state, false);
        assert_eq!(state.theme, original);
        open(&mut state, Action::Appearance, None, None);
        update(&mut state, Message::EditorPreview("github-light".into()));
        let Effect::SaveGuiTheme { request_id, .. } = apply(&mut state) else {
            panic!("expected save")
        };
        update(
            &mut state,
            Message::GuiFinished(request_id, Ok("Applied".into())),
        );
        let committed = state.theme.clone();
        close(&mut state, false);
        assert_eq!(state.theme, committed);
        assert_eq!(
            state.palette.outcome.as_ref().unwrap().state,
            ExecutionState::Success
        );
    }
    #[tokio::test]
    async fn validation_pending_draft_and_stable_tab_results() {
        let mut state = state().await;
        let source = state.tabs[0].id;
        open(&mut state, Action::Filter, None, None);
        apply(&mut state);
        assert!(state.editor.as_ref().unwrap().error.is_some());
        update(&mut state, Message::EditorText(0, "hello 東京".into()));
        let Effect::SaveGuiFilter {
            request_id,
            tab_id,
            log_manager,
            id,
            pattern,
            kind,
            options,
        } = apply(&mut state)
        else {
            panic!()
        };
        assert_eq!(tab_id, source);
        assert!(matches!(apply(&mut state), Effect::None));
        update(&mut state, Message::EditorText(0, "next draft".into()));
        let manager = save_filter(log_manager, id, pattern, kind, options)
            .await
            .unwrap();
        let other = crate::db::LogManager::new(state.db.clone(), None).await;
        state.tabs.push(crate::shared::TabState::new(
            crate::ingestion::FileReader::from_bytes(b"other\n".to_vec()),
            other,
            "other.log".into(),
        ));
        state.active_tab = 1;
        update(
            &mut state,
            Message::PaletteExecuted(request_id, source, Ok(manager)),
        );
        assert_eq!(state.active_tab, 1);
        assert_eq!(state.editor.as_ref().unwrap().values[0], "next draft");
        assert_eq!(
            state.tabs[0].log_manager.get_filters()[0].pattern,
            "hello 東京"
        );
    }
    #[tokio::test]
    async fn closed_editor_keeps_running_and_closed_tab_is_not_reused() {
        let mut state = state().await;
        open(&mut state, Action::Filter, None, None);
        update(&mut state, Message::EditorText(0, "hello".into()));
        let Effect::SaveGuiFilter {
            request_id,
            tab_id,
            log_manager,
            ..
        } = apply(&mut state)
        else {
            panic!()
        };
        close(&mut state, false);
        assert!(state.palette.pending());
        state.tabs.clear();
        let other = crate::db::LogManager::new(state.db.clone(), None).await;
        state.tabs.push(crate::shared::TabState::new(
            crate::ingestion::FileReader::from_bytes(b"new\n".to_vec()),
            other,
            "new.log".into(),
        ));
        update(
            &mut state,
            Message::PaletteExecuted(request_id, tab_id, Ok(log_manager)),
        );
        assert!(state.tabs[0].log_manager.get_filters().is_empty());
        assert_eq!(
            state.palette.outcome.as_ref().unwrap().state,
            ExecutionState::Information
        );
    }
    #[tokio::test]
    async fn stale_failures_clear_and_empty_output_preserve_drafts() {
        let mut state = state().await;
        let source = state.tabs[0].id;
        open(&mut state, Action::Filter, None, None);
        update(&mut state, Message::EditorText(0, "hello".into()));
        let Effect::SaveGuiFilter { request_id, .. } = apply(&mut state) else {
            panic!()
        };
        update(&mut state, Message::PaletteClear);
        assert!(state.palette.pending());
        update(
            &mut state,
            Message::PaletteExecuted(request_id + 1, source, Err("stale".into())),
        );
        assert!(state.palette.pending());
        update(
            &mut state,
            Message::PaletteExecuted(request_id, source, Err("disk failure\nretry".into())),
        );
        assert_eq!(state.editor.as_ref().unwrap().values[0], "hello");
        assert!(state.editor.as_ref().unwrap().error.is_some());
        update(&mut state, Message::PaletteClear);
        assert!(state.palette.outcome.is_none());
        gui_outcome(
            &mut state,
            "Empty".into(),
            ExecutionState::Information,
            String::new(),
        );
        assert!(state.palette.outcome.as_ref().unwrap().output.is_empty());
    }
    #[tokio::test]
    async fn group_membership_and_filter_edit_persist() {
        let state = state().await;
        let manager = save_filter(
            state.tabs[0].log_manager.clone(),
            None,
            "hello".into(),
            FilterType::Include,
            Default::default(),
        )
        .await
        .unwrap();
        let id = manager.get_filters()[0].id;
        let manager = save_group(
            manager,
            "payments".into(),
            Some("cyan".into()),
            None,
            false,
            vec![(id, true)],
        )
        .await
        .unwrap();
        assert_eq!(manager.get_filters()[0].group.as_deref(), Some("payments"));
        assert_eq!(manager.get_group_styles()[0].name, "payments");
        let manager = save_filter(
            manager,
            Some(id),
            "world".into(),
            FilterType::Exclude,
            Default::default(),
        )
        .await
        .unwrap();
        assert_eq!(manager.get_filters().len(), 1);
        assert_eq!(manager.get_filters()[0].id, id);
        assert_eq!(manager.get_filters()[0].pattern, "world");
        assert_eq!(manager.get_filters()[0].filter_type, FilterType::Exclude);
    }
}
