//! GUI actions and drafts. No terminal modes or command-line grammar.
use crate::commands::Commands;
use crate::filters::{FIELD_PREFIX, FilterType};
use crate::gui::{effect::Effect, state::GuiState};
use crate::shared::TabId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Open,
    OpenDirectory,
    Annotation,
    Bookmark,
    ShowBookmarks,
    ShowLogs,
    ToggleBookmarks,
    Help,
    SwitchFile,
    Merge,
    EnableMcp,
    DisableMcp,
    DefaultFilters,
    Docker,
    Dlt,
    Otel,
    Run,
    Export,
    SaveLogs,
    SaveMarked,
    SaveFilters,
    LoadFilters,
    AppendFilters,
    ImportFilters,
    Filter,
    Group,
    Columns,
    Appearance,
    Settings,
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
        } else if matches!(self.action, Action::Bookmark | Action::Annotation)
            && crate::gui::view::annotations::selected_lines(state).is_empty()
        {
            Some("Select a log line first")
        } else {
            None
        }
    }
}
pub const CATALOG: &[Descriptor] = &[
    Descriptor {
        action: Action::OpenDirectory,
        label: "Open directory…",
        description: "Choose a directory and browse its log files",
        aliases: "open-directory open-folder",
        tab: false,
    },
    Descriptor {
        action: Action::Settings,
        label: "Settings",
        description: "Restore options, defaults for new files, and application preferences",
        aliases: "settings preferences restore-session restore-file-context",
        tab: false,
    },
    Descriptor {
        action: Action::Merge,
        label: "Merge files",
        description: "Combine open logs in timestamp order",
        aliases: "merge combine",
        tab: false,
    },
    Descriptor {
        action: Action::Help,
        label: "Keyboard shortcuts",
        description: "Show GUI keyboard controls",
        aliases: "help shortcuts",
        tab: false,
    },
    Descriptor {
        action: Action::SwitchFile,
        label: "Switch file",
        description: "Choose an open tab",
        aliases: "file-switcher tabs",
        tab: false,
    },
    Descriptor {
        action: Action::EnableMcp,
        label: "Enable MCP server",
        description: "Expose bookmarks and annotations through MCP",
        aliases: "enable-mcp",
        tab: false,
    },
    Descriptor {
        action: Action::DisableMcp,
        label: "Disable MCP server",
        description: "Stop the MCP server",
        aliases: "disable-mcp",
        tab: false,
    },
    Descriptor {
        action: Action::DefaultFilters,
        label: "Default filters by schema",
        description: "Choose a filter file to load automatically for a schema",
        aliases: "default-filters",
        tab: false,
    },
    Descriptor {
        action: Action::Docker,
        label: "Open Docker logs",
        description: "Select a running container and follow its logs",
        aliases: "docker container",
        tab: false,
    },
    Descriptor {
        action: Action::Dlt,
        label: "Connect DLT device",
        description: "Select a configured device or enter its host and port",
        aliases: "dlt",
        tab: false,
    },
    Descriptor {
        action: Action::Otel,
        label: "Receive OpenTelemetry logs",
        description: "Start an OTLP HTTP or gRPC receiver",
        aliases: "otel otlp",
        tab: false,
    },
    Descriptor {
        action: Action::Run,
        label: "Run command",
        description: "Stream a program's output into a log tab",
        aliases: "run subprocess",
        tab: false,
    },
    Descriptor {
        action: Action::Export,
        label: "Export analysis",
        description: "Export annotations and bookmarks using a template",
        aliases: "export template",
        tab: true,
    },
    Descriptor {
        action: Action::SaveLogs,
        label: "Save logs",
        description: "Save visible source log entries",
        aliases: "save-logs",
        tab: true,
    },
    Descriptor {
        action: Action::SaveMarked,
        label: "Save bookmarked logs",
        description: "Save bookmarked source log entries",
        aliases: "export-marked",
        tab: true,
    },
    Descriptor {
        action: Action::SaveFilters,
        label: "Save filters",
        description: "Save filters and groups to JSON",
        aliases: "save-filters",
        tab: true,
    },
    Descriptor {
        action: Action::LoadFilters,
        label: "Load filters",
        description: "Replace filters and groups from JSON",
        aliases: "load-filters",
        tab: true,
    },
    Descriptor {
        action: Action::AppendFilters,
        label: "Append filters",
        description: "Merge a JSON filter file into this tab",
        aliases: "append-filters",
        tab: true,
    },
    Descriptor {
        action: Action::Bookmark,
        label: "Toggle bookmark",
        description: "Bookmark or unbookmark selected log lines · Ctrl+B",
        aliases: "bookmark mark",
        tab: true,
    },
    Descriptor {
        action: Action::ShowBookmarks,
        label: "Show bookmarks",
        description: "View bookmarked log lines",
        aliases: "bookmarks",
        tab: false,
    },
    Descriptor {
        action: Action::ShowLogs,
        label: "Show logs",
        description: "Return to the log table",
        aliases: "logs",
        tab: false,
    },
    Descriptor {
        action: Action::ToggleBookmarks,
        label: "Switch Logs / Bookmarks",
        description: "Switch between logs and bookmarks · Ctrl+Shift+B",
        aliases: "toggle-bookmarks",
        tab: false,
    },
    Descriptor {
        action: Action::ImportFilters,
        label: "Import Notepad++ filters",
        description: "Import AnalyseDoc or User Defined Language XML",
        aliases: "import-filters npp",
        tab: true,
    },
    Descriptor {
        action: Action::Annotation,
        label: "Add annotation",
        description: "Attach a note to selected log rows · Ctrl+Shift+A",
        aliases: "annotation annotate comment note",
        tab: true,
    },
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
    let query = query.trim().to_lowercase();
    let rank = |text: &str| {
        let text = text.to_lowercase();
        if !crate::commands::auto_complete::fuzzy_match(&query, &text) {
            return None;
        }
        let kind = if text == query {
            0
        } else if text.starts_with(&query) {
            1
        } else if text.contains(&query) {
            2
        } else {
            3
        };
        Some((
            kind,
            text.chars().count().saturating_sub(query.chars().count()),
        ))
    };
    let mut matches: Vec<_> = CATALOG
        .iter()
        .filter_map(|descriptor| {
            let score = std::iter::once(descriptor.label)
                .chain(descriptor.aliases.split_whitespace())
                .filter_map(rank)
                .min()?;
            Some((score, descriptor))
        })
        .collect();
    if !query.is_empty() {
        matches.sort_by_key(|(score, _)| *score);
    }
    matches
        .into_iter()
        .map(|(_, descriptor)| descriptor)
        .collect()
}
pub fn select_best(state: &mut GuiState) {
    state.palette.selected = matching(&state.palette.input)
        .iter()
        .position(|d| d.unavailable(state).is_none())
        .unwrap_or(0);
}
#[derive(Clone, Debug)]
pub struct Editor {
    pub action: Action,
    pub source: Option<TabId>,
    pub epoch: u64,
    pub condition_edit: Option<usize>,
    pub values: Vec<String>,
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
    pub merge_sources: Vec<(TabId, bool)>,
}
pub fn open(
    state: &mut GuiState,
    action: Action,
    edit_id: Option<usize>,
    group: Option<String>,
) -> Effect {
    if state.editor.is_some()
        || (state.palette.pending()
            && !matches!(
                action,
                Action::ShowLogs
                    | Action::ShowBookmarks
                    | Action::ToggleBookmarks
                    | Action::Bookmark
            ))
    {
        return Effect::None;
    }
    let descriptor = CATALOG.iter().find(|d| d.action == action).unwrap();
    if descriptor.unavailable(state).is_some()
        && !(action == Action::Annotation && edit_id.is_some())
    {
        return Effect::None;
    }
    state.palette.open = false;
    match action {
        Action::ShowLogs => {
            state.nav_page = crate::gui::state::NavPage::Logs;
            return Effect::None;
        }
        Action::ShowBookmarks => {
            state.nav_page = crate::gui::state::NavPage::Bookmarks;
            return Effect::None;
        }
        Action::ToggleBookmarks => {
            state.nav_page = if state.nav_page == crate::gui::state::NavPage::Bookmarks {
                crate::gui::state::NavPage::Logs
            } else {
                crate::gui::state::NavPage::Bookmarks
            };
            return Effect::None;
        }
        Action::Bookmark => {
            let lines = crate::gui::view::annotations::selected_lines(state);
            let tab = state.active_tab_mut().unwrap();
            let mark = lines.iter().any(|line| !tab.mark_manager.is_marked(*line));
            for line in &lines {
                if tab.mark_manager.is_marked(*line) != mark {
                    tab.toggle_mark(*line);
                }
            }
            state.notify(crate::gui::state::StatusMessage::Info(format!(
                "{} {} line(s).",
                if mark { "Bookmarked" } else { "Unbookmarked" },
                lines.len()
            )));
            return Effect::None;
        }
        _ => {}
    }
    if action == Action::DisableMcp {
        return Effect::ConfigureMcp(None);
    }
    if action == Action::Open {
        return Effect::OpenFileDialog;
    }
    if action == Action::OpenDirectory {
        return Effect::OpenDirectoryDialog;
    }
    if matches!(
        action,
        Action::SaveFilters | Action::LoadFilters | Action::AppendFilters | Action::ImportFilters
    ) {
        return Effect::FilterFileDialog {
            action,
            tab_id: state.active_tab().unwrap().id,
        };
    }
    if matches!(action, Action::SaveLogs | Action::SaveMarked) {
        return Effect::SaveLogsDialog {
            tab_id: state.active_tab().unwrap().id,
            marked: action == Action::SaveMarked,
        };
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
        condition_edit: None,
        values: vec![String::new(); 6],
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
        merge_sources: Vec::new(),
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
                if let Some((field, value)) = e.conditions.first() {
                    e.values[1] = field.clone();
                    e.values[2] = value.clone();
                    e.condition_edit = Some(0);
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
    if action == Action::EnableMcp {
        e.values[0] = state.preferences.mcp_port.to_string();
    }
    if action == Action::Settings {
        let prefs = &state.preferences;
        e.values[0] = prefs.restore_session.to_string();
        e.values[1] = prefs.restore_file.to_string();
        e.values[2] = prefs.preview_bytes.to_string();
        e.values[3] = prefs.mcp_port.to_string();
        e.fields = [
            "Wrap lines",
            "Line numbers",
            "Relative line numbers",
            "Show sidebar",
            "Sidebar on left",
            "Collapse continuations",
        ]
        .into_iter()
        .zip(prefs.display)
        .map(|(name, on)| (name.into(), on))
        .collect();
    }
    if action == Action::DefaultFilters {
        e.values[0] = state
            .active_tab()
            .and_then(|t| t.display.format.as_deref())
            .map(|f| f.name().to_string())
            .unwrap_or_else(|| "syslog".into());
    }
    if action == Action::Dlt {
        e.values[0] = "localhost".into();
        e.values[1] = crate::config::DEFAULT_DLT_PORT.to_string();
        for (index, device) in crate::config::Config::load()
            .unwrap_or_default()
            .dlt_devices
            .into_iter()
            .enumerate()
        {
            e.members.push((index, device.name, true));
            e.conditions.push((
                device.host,
                device
                    .port
                    .unwrap_or(crate::config::DEFAULT_DLT_PORT)
                    .to_string(),
            ));
        }
    }
    if action == Action::Merge {
        e.merge_sources = state.tabs.iter().map(|tab| (tab.id, false)).collect();
    }
    if action == Action::Otel {
        e.values[0] = "4318".into();
        e.values[1] = "http".into();
    }
    if action == Action::Export {
        configure_export(&mut e, "markdown");
    }
    if action == Action::Annotation {
        let tab = state.active_tab().unwrap();
        if let Some(comment) = edit_id.and_then(|i| tab.comment_manager.get().get(i)) {
            e.values[0] = comment.text.clone();
            e.members = comment
                .line_indices
                .iter()
                .map(|&line| (line, String::new(), true))
                .collect();
        } else {
            e.members = crate::gui::view::annotations::selected_lines(state)
                .into_iter()
                .map(|line| (line, String::new(), true))
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
    if action == Action::DefaultFilters {
        return Effect::LoadDefaultMappings {
            epoch: state.editor.as_ref().unwrap().epoch,
        };
    }
    if action == Action::Docker {
        return Effect::ListDocker {
            epoch: state.editor.as_ref().unwrap().epoch,
        };
    }
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
    let Some(mut e) = state.editor.clone() else {
        return Effect::None;
    };
    if e.action == Action::Settings {
        let mut prefs = state.preferences.clone();
        let parsed = (|| {
            use crate::db::SettingsKey;
            if !prefs.locked.contains(&SettingsKey::RestoreSession) {
                prefs.restore_session = e.values[0]
                    .parse()
                    .map_err(|_| "Choose a session restore policy.")?;
            }
            if !prefs.locked.contains(&SettingsKey::RestoreFileContext) {
                prefs.restore_file = e.values[1]
                    .parse()
                    .map_err(|_| "Choose a file restore policy.")?;
            }
            if !prefs.locked.contains(&SettingsKey::PreviewBytes) {
                prefs.preview_bytes = e.values[2]
                    .parse::<u64>()
                    .ok()
                    .filter(|v| (1..=128 * 1024 * 1024).contains(v))
                    .ok_or("Preview size must be between 1 byte and 128 MiB.")?;
            }
            if !prefs.locked.contains(&SettingsKey::McpPort) {
                prefs.mcp_port = e.values[3]
                    .parse::<u16>()
                    .ok()
                    .filter(|p| *p > 0)
                    .ok_or("MCP port must be between 1 and 65535.")?;
            }
            for (i, (_, on)) in e.fields.iter().enumerate() {
                if !prefs
                    .locked
                    .contains(&crate::gui::persistence::DISPLAY_KEYS[i])
                {
                    prefs.display[i] = *on;
                }
            }
            Ok::<_, &str>(())
        })();
        if let Err(error) = parsed {
            state.editor.as_mut().unwrap().error = Some(error.into());
            return Effect::None;
        }
        close(state, true);
        return Effect::SavePreferences(prefs);
    }
    if state.palette.pending() {
        return Effect::None;
    }
    if e.action == Action::Merge {
        let ids: Vec<_> = e
            .merge_sources
            .iter()
            .filter(|(_, on)| *on)
            .map(|(id, _)| *id)
            .collect();
        if ids.len() < 2 {
            state.editor.as_mut().unwrap().error = Some("Select at least two open files.".into());
            return Effect::None;
        }
        if ids
            .iter()
            .any(|id| !state.tabs.iter().any(|tab| tab.id == *id))
        {
            state.editor.as_mut().unwrap().error =
                Some("A selected source was closed. Reopen the merge picker.".into());
            return Effect::None;
        }
        close(state, true);
        return Effect::MergeTabs(ids);
    }
    if let Some(source) = e.source {
        let Some(idx) = state.tabs.iter().position(|t| t.id == source) else {
            state.editor.as_mut().unwrap().error = Some("The source tab was closed.".into());
            return Effect::None;
        };
        state.active_tab = idx;
    }
    if e.action == Action::Filter && !e.values[1].trim().is_empty() {
        let condition = (e.values[1].clone(), e.values[2].clone());
        if let Some(index) = e
            .condition_edit
            .and_then(|index| e.conditions.get_mut(index))
        {
            *index = condition;
        } else {
            e.conditions.push(condition);
        }
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
    if e.action == Action::EnableMcp {
        let Some(port) = e.values[0].parse::<u16>().ok().filter(|p| *p > 0) else {
            state.editor.as_mut().unwrap().error = Some("Enter a port between 1 and 65535.".into());
            return Effect::None;
        };
        close(state, true);
        return Effect::ConfigureMcp(Some(port));
    }
    if e.action == Action::DefaultFilters {
        if e.values[0].trim().is_empty() {
            state.editor.as_mut().unwrap().error = Some("Choose a schema.".into());
            return Effect::None;
        }
        let effect = Effect::SaveDefaultMapping {
            format: e.values[0].clone(),
            path: e.values[1].clone(),
        };
        close(state, true);
        return effect;
    }
    if e.action == Action::Docker {
        if e.values[0].trim().is_empty() {
            state.editor.as_mut().unwrap().error =
                Some("Select a container or enter its name/ID.".into());
            return Effect::None;
        }
        let source = crate::gui::sources::Source::Docker {
            container: e.values[0].clone(),
        };
        close(state, true);
        return Effect::ConnectSource(source);
    }
    if e.action == Action::Dlt {
        let Some(port) = e.values[1].parse::<u16>().ok().filter(|port| *port > 0) else {
            state.editor.as_mut().unwrap().error = Some("Enter a port between 1 and 65535.".into());
            return Effect::None;
        };
        if e.values[0].trim().is_empty() {
            state.editor.as_mut().unwrap().error = Some("Enter the device host.".into());
            return Effect::None;
        }
        let source = crate::gui::sources::Source::Dlt {
            host: e.values[0].clone(),
            port,
        };
        close(state, true);
        return Effect::ConnectSource(source);
    }
    if e.action == Action::Otel {
        let Some(port) = e.values[0].parse::<u16>().ok().filter(|port| *port > 0) else {
            state.editor.as_mut().unwrap().error = Some("Enter a port between 1 and 65535.".into());
            return Effect::None;
        };
        let source = crate::gui::sources::Source::Otel {
            port,
            grpc: e.values[1] == "grpc",
        };
        close(state, true);
        return Effect::ConnectSource(source);
    }
    if e.action == Action::Run {
        if e.values[0].trim().is_empty() {
            state.editor.as_mut().unwrap().error = Some("Choose an executable.".into());
            return Effect::None;
        }
        let source = crate::gui::sources::Source::Run {
            program: e.values[0].clone(),
            args: e.values[1].lines().map(str::to_string).collect(),
        };
        close(state, true);
        return Effect::ConnectSource(source);
    }
    if e.action == Action::Export {
        if let Err(error) = crate::commands::load_template(&e.values[0]) {
            state.editor.as_mut().unwrap().error = Some(error.to_string());
            return Effect::None;
        }
        let effect = Effect::ExportDialog {
            tab_id: e.source.unwrap(),
            template: e.values[0].clone(),
            fields: e
                .fields
                .iter()
                .enumerate()
                .map(|(i, (key, _))| (key.clone(), e.values[i + 6].clone()))
                .collect(),
        };
        close(state, true);
        return effect;
    }
    if e.action == Action::Annotation {
        if e.values[0].trim().is_empty() || e.members.is_empty() {
            state.editor.as_mut().unwrap().error =
                Some("Enter a note and select at least one log row.".into());
            return Effect::None;
        }
        let tab = state.active_tab_mut().unwrap();
        if let Some(index) = e.edit_id {
            let mut comments = tab.comment_manager.get().to_vec();
            if let Some(comment) = comments.get_mut(index) {
                comment.text = e.values[0].clone();
            }
            tab.comment_manager.set(comments);
        } else {
            tab.comment_manager.add(
                e.values[0].clone(),
                e.members.iter().map(|(line, _, _)| *line).collect(),
            );
        }
        let annotated_lines = tab
            .comment_manager
            .get()
            .iter()
            .flat_map(|note| note.line_indices.iter().copied())
            .collect::<Vec<_>>();
        tab.mark_lines(&annotated_lines);
        close(state, true);
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
        close(state, true);
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
        close(state, true);
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
    #[tokio::test]
    async fn bookmark_annotation_and_navigation_actions_follow_source_selection() {
        let mut state = state().await;
        let source = state.tabs[0].id;
        update(&mut state, Message::SelectLog(0, false));
        update(&mut state, Message::SelectLog(1, true));
        open(&mut state, Action::Bookmark, None, None);
        assert_eq!(state.tabs[0].mark_manager.get_indices(), vec![0, 1]);
        open(&mut state, Action::Bookmark, None, None);
        assert!(state.tabs[0].mark_manager.get_indices().is_empty());
        update(
            &mut state,
            Message::LogRowAction(source, 1, Action::Annotation),
        );
        assert_eq!(
            state
                .editor
                .as_ref()
                .unwrap()
                .members
                .iter()
                .map(|m| m.0)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
        close(&mut state, false);
        state.log_selection = None;
        update(
            &mut state,
            Message::LogRowAction(source, 1, Action::Bookmark),
        );
        assert_eq!(state.tabs[0].mark_manager.get_indices(), vec![1]);
        for (action, expected) in [
            (Action::ShowBookmarks, crate::gui::state::NavPage::Bookmarks),
            (Action::ToggleBookmarks, crate::gui::state::NavPage::Logs),
            (Action::ShowLogs, crate::gui::state::NavPage::Logs),
        ] {
            open(&mut state, action, None, None);
            assert_eq!(state.nav_page, expected);
        }
        state.tabs.clear();
        assert!(matches!(
            update(
                &mut state,
                Message::LogRowAction(source, 1, Action::Bookmark)
            ),
            Effect::None
        ));
    }

    #[tokio::test]
    async fn best_matching_action_is_selected_and_enter_executes_it() {
        let mut state = state().await;
        for (query, action) in [
            ("bookmark", Action::Bookmark),
            ("bookmarks", Action::ShowBookmarks),
            ("logs", Action::ShowLogs),
            ("annotate", Action::Annotation),
            ("th", Action::Appearance),
        ] {
            assert_eq!(matching(query)[0].action, action);
        }
        update(&mut state, Message::OpenCommands);
        update(&mut state, Message::PaletteEdited("bookmarks".into()));
        assert_eq!(state.palette.selected, 0);
        update(
            &mut state,
            Message::PaletteKey(crate::input::KeyCode::Enter, Default::default()),
        );
        assert_eq!(state.nav_page, crate::gui::state::NavPage::Bookmarks);
        assert!(!state.palette.open);
        state.tabs.clear();
        update(&mut state, Message::OpenCommands);
        update(&mut state, Message::PaletteEdited("annotate".into()));
        update(
            &mut state,
            Message::PaletteKey(crate::input::KeyCode::Enter, Default::default()),
        );
        assert!(state.editor.is_none());
        assert!(state.palette.open);
    }

    #[tokio::test]
    async fn editing_filter_prefills_content_and_updates_one_condition() {
        let mut state = state().await;
        let pattern = crate::filters::encode_field_filter(
            &[
                ("service".into(), "payments".into()),
                ("level".into(), "ERROR".into()),
            ],
            Some("timeout"),
        );
        state.tabs[0].log_manager = save_filter(
            state.tabs[0].log_manager.clone(),
            None,
            pattern.clone(),
            FilterType::Highlight,
            crate::filters::FilterOptions {
                fg: Some("#123456".into()),
                bg: Some("#654321".into()),
                match_only: false,
                use_regex: true,
                ignore_case: true,
                group: Some("alerts".into()),
            },
        )
        .await
        .unwrap();
        let id = state.tabs[0].log_manager.get_filters()[0].id;
        update(&mut state, Message::EditFilter(id));
        let editor = state.editor.as_ref().unwrap();
        assert_eq!(
            editor.values,
            [
                "timeout", "service", "payments", "alerts", "#123456", "#654321"
            ]
            .map(str::to_string)
        );
        assert_eq!(editor.kind, FilterType::Highlight);
        assert!(editor.regex && editor.ignore_case && editor.whole_line);
        assert_eq!(editor.condition_edit, Some(0));
        update(&mut state, Message::EditorText(2, "auth".into()));
        let Effect::SaveGuiFilter {
            pattern,
            options,
            id: saved_id,
            ..
        } = apply(&mut state)
        else {
            panic!("expected filter save")
        };
        assert_eq!(saved_id, Some(id));
        let (conditions, text) =
            crate::filters::parse_field_filter_expr(pattern.strip_prefix(FIELD_PREFIX).unwrap())
                .unwrap();
        assert_eq!(
            conditions,
            vec![
                ("service".into(), "auth".into()),
                ("level".into(), "ERROR".into())
            ]
        );
        assert_eq!(text.as_deref(), Some("timeout"));
        assert_eq!(options.fg.as_deref(), Some("#123456"));
        assert_eq!(options.bg.as_deref(), Some("#654321"));
    }
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
    async fn validation_then_apply_closes_editor_and_preserves_stable_tab_results() {
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
        assert!(state.editor.is_none());
        assert!(!state.palette.open);
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
        assert!(state.editor.is_none());
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
    async fn stale_failures_clear_and_empty_output_keep_submitted_editor_closed() {
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
        assert!(state.editor.is_none());
        assert!(!state.palette.open);
        assert_eq!(
            state.status,
            Some(crate::gui::state::StatusMessage::Error(
                "disk failure\nretry".into()
            ))
        );
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

pub fn configure_export(editor: &mut Editor, name: &str) {
    match crate::commands::load_template(name) {
        Ok(template) => {
            editor.values[0] = name.into();
            editor.fields = crate::commands::extract_user_fields(&template)
                .into_iter()
                .map(|key| (key, true))
                .collect();
            editor.values.resize(6 + editor.fields.len(), String::new());
            editor.values[6..].fill(String::new());
            editor.epoch = editor.epoch.wrapping_add(1);
            editor.error = None;
        }
        Err(error) => editor.error = Some(error.to_string()),
    }
}
