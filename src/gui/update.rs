use crate::commands::auto_complete::shell_split;
use crate::commands::{CommandLine, Commands};
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
use crate::input::{KeyCode, KeyModifiers};
use crate::mode::command_mode::CommandMode;
use crate::mode::normal_mode::NormalMode;
use crate::ui::{KeyResult, TabState, VisibleLines};
use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// The central reducer: every user intent (`Message`) flows through here,
/// mutating `state` directly and returning an `Effect` describing any
/// async work or imperative widget action the gpui glue layer (`App` in
/// `app.rs`) still needs to perform. Never touches a gpui type itself.
pub fn update(state: &mut GuiState, message: Message) -> Effect {
    match message {
        Message::OpenFileDialog => Effect::OpenFileDialog,
        Message::FileDialogResult(Some(path)) => Effect::LoadFile(path),
        Message::FileDialogResult(None) => Effect::None,
        Message::FileLoaded(Ok(loaded)) => {
            push_tab(state, loaded);
            Effect::None
        }
        Message::FileLoaded(Err(err)) => {
            state.status = Some(StatusMessage::Error(err));
            Effect::None
        }
        Message::TabSelected(idx) => {
            if idx < state.tabs.len() {
                state.active_tab = idx;
            }
            Effect::None
        }
        Message::TabClosed(idx) => {
            close_tab_at(state, idx);
            Effect::None
        }
        Message::KeyPressed(code, modifiers) => dispatch_key(state, code, modifiers),
        Message::CommandExecuted(tab_idx, result) => {
            match result {
                Ok(log_manager) => apply_log_manager(state, tab_idx, log_manager),
                Err(err) => state.status = Some(StatusMessage::Error(err)),
            }
            Effect::None
        }
        Message::FilterToggled(tab_idx, id) => match state.tabs.get(tab_idx) {
            Some(tab) => Effect::ToggleFilter {
                tab_idx,
                log_manager: tab.log_manager.clone(),
                id,
            },
            None => Effect::None,
        },
        Message::FiltersMutated(tab_idx, log_manager) => {
            apply_log_manager(state, tab_idx, log_manager);
            Effect::None
        }
        Message::GroupToggled(tab_idx, name) => match state.tabs.get(tab_idx) {
            Some(tab) => Effect::ToggleGroup {
                tab_idx,
                log_manager: tab.log_manager.clone(),
                name,
            },
            None => Effect::None,
        },
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
            state.nav_page = page;
            Effect::None
        }
        Message::TimeRangeToggled => {
            state.time_range_open = !state.time_range_open;
            Effect::None
        }
        Message::TimeRangeSelected(preset) => {
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
            handle_command_string(state, cmd.to_string())
        }
        Message::SidebarTabSelected(tab) => {
            state.sidebar_tab = tab;
            Effect::None
        }
        Message::SidebarToggled => {
            toggle_display(state, |d| d.show_sidebar = !d.show_sidebar);
            Effect::None
        }
        Message::ClearAllFilters(tab_idx) => {
            state.active_tab = tab_idx;
            handle_command_string(state, "clear-filters".to_string())
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
                None => handle_command_string(state, format!("filter --field {field}={value}")),
            }
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
    let tab_idx = state.active_tab;
    let Some(expr) =
        crate::gui::time_range::date_filter_expression(preset, time::OffsetDateTime::now_utc())
    else {
        let Some(tab) = state.tabs.get(tab_idx) else {
            return Effect::None;
        };
        let filters = tab.log_manager.get_filters();
        return match filters {
            [only] if only.pattern.starts_with(DATE_PREFIX) => {
                handle_command_string(state, "clear-filters".to_string())
            }
            _ if filters.iter().any(|f| f.pattern.starts_with(DATE_PREFIX)) => {
                state.status = Some(StatusMessage::Error(
                    "Remove the date filter from the Filters panel".to_string(),
                ));
                Effect::None
            }
            _ => Effect::None,
        };
    };
    handle_command_string(state, format!("date-filter {expr}"))
}

/// Drives the active tab's real `Mode` object with a keystroke, exactly
/// like the TUI's event loop (`App::run` in `src/ui/app.rs`) does.
///
/// `Mode::handle_key` isn't guaranteed to resolve on its very first poll —
/// several implementations call real `LogManager`/DB methods directly
/// (e.g. `FilterManagementMode::delete_filter`), and a real DB write
/// essentially never completes synchronously. This used to be driven with
/// `futures::FutureExt::now_or_never()`, which polls a future exactly
/// once and discards it if that poll doesn't finish — silently abandoning
/// it mid-flight the moment it returned `Pending`: whatever ran before
/// that point (a synchronous `Vec::retain`, say) stayed applied, but
/// anything after it (like `TabState::begin_filter_refresh`, which
/// recomputes the visible lines) never ran, leaving the log pane stale —
/// e.g. deleting every filter never brought the hidden lines back.
///
/// `futures::executor::block_on` instead actually drives the future to
/// completion on this thread. It's used in place of `tokio::runtime::
/// Handle::block_on` specifically because this function also runs inside
/// unit tests' `#[tokio::test]` bodies (via `update()`), which already
/// have their own ambient tokio runtime on the current thread — `Handle::
/// block_on` panics ("Cannot start a runtime from within a runtime") in
/// that case, where `futures::executor::block_on` (a plain, runtime-
/// agnostic poll loop) does not. The `.enter()` guard still ensures
/// `tokio::task::spawn_blocking`/sqlx calls inside the future always have
/// a live `Handle` to dispatch onto, same as before — on gpui's own
/// thread, which has no ambient runtime at all, this is the only source
/// of one. Blocking the calling thread is fine either way: every `Mode::
/// handle_key` implementation only does small, local, bounded work
/// (in-memory mutation, a local SQLite write) — nothing in this codebase
/// keeps one pending indefinitely.
fn dispatch_key(state: &mut GuiState, code: KeyCode, modifiers: KeyModifiers) -> Effect {
    let Some(tab) = state.active_tab_mut() else {
        return Effect::None;
    };
    let mode = std::mem::replace(&mut tab.interaction.mode, Box::new(NormalMode::default()));
    let _guard = runtime::handle().enter();
    let (next_mode, result) = futures::executor::block_on(mode.handle_key(tab, code, modifiers));
    tab.interaction.mode = next_mode;
    sync_filter_recompute(tab);
    apply_key_result(state, result, code, modifiers)
}

/// Handles whatever `Mode::handle_key` decided beyond mutating the tab
/// directly: a command to run (`ExecuteCommand`), a global keybinding the
/// mode itself doesn't own (`Ignored` — quit, tab switching), or one of
/// `UiMode`'s bulk visibility toggles. Variants needing substantial new
/// GUI surface this phase doesn't have yet (Docker/DLT streaming, the
/// archive picker, merge view, export, theme picker, session restore,
/// default-filters mapping, the file switcher) fall through as a no-op —
/// a deliberate, incremental scoping choice, not a silent gap.
fn apply_key_result(
    state: &mut GuiState,
    result: KeyResult,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> Effect {
    match result {
        KeyResult::ExecuteCommand(cmd) => return handle_command_string(state, cmd),
        KeyResult::Ignored => return handle_global_key(state, code, modifiers),
        KeyResult::ToggleSidebar => toggle_display(state, |d| d.show_sidebar = !d.show_sidebar),
        KeyResult::ToggleModeBar => toggle_display(state, |d| d.show_mode_bar = !d.show_mode_bar),
        KeyResult::ToggleBorders => toggle_display(state, |d| d.show_borders = !d.show_borders),
        KeyResult::ToggleWrap => toggle_display(state, |d| d.wrap = !d.wrap),
        KeyResult::ToggleLineNumbers => {
            toggle_display(state, |d| d.show_line_numbers = !d.show_line_numbers)
        }
        KeyResult::ToggleRelativeLineNumbers => toggle_display(state, |d| {
            d.relative_line_numbers = !d.relative_line_numbers
        }),
        KeyResult::ToggleGroupsPanel => {
            toggle_display(state, |d| d.show_groups_panel = !d.show_groups_panel)
        }
        KeyResult::ResizeSidebar(width) => {
            if let Some(tab) = state.active_tab_mut() {
                tab.display.sidebar_width = width;
            }
        }
        KeyResult::ApplyValueColors(disabled) => state.theme.value_colors.disabled = disabled,
        KeyResult::ApplyLevelColors(disabled) => {
            toggle_display(state, |d| d.level_colors_disabled = disabled)
        }
        KeyResult::SwitchToTab(id) => {
            if let Some(idx) = state.tabs.iter().position(|t| t.id == id) {
                state.active_tab = idx;
            }
        }
        _ => {}
    }
    Effect::None
}

fn toggle_display(state: &mut GuiState, f: impl FnOnce(&mut crate::ui::DisplayConfig)) {
    if let Some(tab) = state.active_tab_mut() {
        f(&mut tab.display);
    }
}

/// Mirrors the TUI's `App::handle_global_key` (`src/ui/input.rs`) for the
/// subset this GUI supports — quit, tab switching, closing the active
/// tab, and opening a new one via the command bar. Only reached when the
/// active mode didn't consume the key itself (`KeyResult::Ignored`).
fn handle_global_key(state: &mut GuiState, code: KeyCode, modifiers: KeyModifiers) -> Effect {
    let Some(tab) = state.active_tab() else {
        return Effect::None;
    };
    let kb = tab.interaction.keybindings.clone();
    if kb.global.quit.matches(code, modifiers) {
        return Effect::Quit;
    }
    if kb.global.next_tab.matches(code, modifiers) {
        advance_tab(state, 1);
    } else if kb.global.prev_tab.matches(code, modifiers) {
        advance_tab(state, -1);
    } else if kb.global.close_tab.matches(code, modifiers) {
        close_tab_at(state, state.active_tab);
    } else if kb.global.new_tab.matches(code, modifiers)
        && let Some(tab) = state.active_tab_mut()
    {
        let history = tab.interaction.command_history.clone();
        tab.interaction.mode = Box::new(CommandMode::with_history("open ".to_string(), 5, history));
    }
    Effect::None
}

/// Parses and runs a command string the exact same way `KeyResult::
/// ExecuteCommand` asks the TUI to (mirroring `App::run_command`'s grammar,
/// via the same clap-derived `CommandLine`), against the active tab's
/// `LogManager`. Unsupported commands are rejected with a status message
/// rather than silently ignored — see `execute_command`.
fn handle_command_string(state: &mut GuiState, cmd: String) -> Effect {
    let tab_idx = state.active_tab;
    let command = match parse_command(&cmd) {
        Ok(Some(command)) => command,
        Ok(None) => return Effect::None,
        Err(err) => {
            state.status = Some(StatusMessage::Error(err));
            return Effect::None;
        }
    };
    if let Some(tab) = state.tabs.get_mut(tab_idx)
        && apply_stream_command(tab, &command)
    {
        return Effect::None;
    }
    let Some(tab) = state.tabs.get(tab_idx) else {
        return Effect::None;
    };
    Effect::ExecuteCommand {
        tab_idx,
        log_manager: tab.log_manager.clone(),
        command,
        theme_bg: crate::theme::color_to_rgb(state.theme.root_bg),
    }
}

/// Handles `Commands::Pause`/`Resume`/`Stop` directly on `tab.stream`,
/// mirroring the TUI's `cmd_pause`/`cmd_resume`/`cmd_stop` (`src/ui/
/// commands/stream.rs`) — these never touch `LogManager`, so they skip the
/// async `Effect::ExecuteCommand` round trip entirely. Returns `true` when
/// `command` was one of these and has been applied.
fn apply_stream_command(tab: &mut TabState, command: &Commands) -> bool {
    match command {
        Commands::Pause => tab.stream.paused = true,
        Commands::Resume => tab.stream.paused = false,
        Commands::Stop => tab.stream.watch = None,
        _ => return false,
    }
    true
}

/// Whether `tab` is actively tailing its source: a watcher is running and
/// hasn't been paused. Feeds the search bar's "Live" dot/toggle.
pub fn is_tab_live(tab: &TabState) -> bool {
    tab.stream.watch.is_some() && !tab.stream.paused
}

fn advance_tab(state: &mut GuiState, delta: isize) {
    if !state.tabs.is_empty() {
        state.active_tab = next_active_tab(state.active_tab, delta, state.tabs.len());
    }
}

fn close_tab_at(state: &mut GuiState, idx: usize) {
    if idx < state.tabs.len() {
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
fn next_active_tab(active: usize, delta: isize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let len = len as isize;
    (((active as isize + delta) % len) + len) as usize % len as usize
}

fn push_tab(state: &mut GuiState, loaded: FileLoaded) {
    let title = loaded
        .path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| loaded.path.display().to_string());
    let mut tab = TabState::new(loaded.reader, loaded.log_manager, title);
    tab.stream.watch = Some(loaded.watch);
    force_recompute(&mut tab);
    state.tabs.push(tab);
    state.active_tab = state.tabs.len() - 1;
}

fn apply_log_manager(state: &mut GuiState, tab_idx: usize, log_manager: LogManager) {
    if let Some(tab) = state.tabs.get_mut(tab_idx) {
        tab.log_manager = log_manager;
        force_recompute(tab);
    }
}

/// Recomputes `visible_indices` from scratch against the tab's current
/// `LogManager`/filter-enabled state. Simpler than the TUI's
/// `TabState::begin_filter_refresh` (which streams results from a
/// cancellable background scan for large files) — a deliberate
/// simplification for this phase; see `sync_filter_recompute`.
fn force_recompute(tab: &mut TabState) {
    let has_active_filters = tab.log_manager.get_filters().iter().any(|f| f.enabled);
    if !tab.filter.enabled || !has_active_filters {
        tab.filter.visible_indices = VisibleLines::All(tab.file_reader.line_count());
        return;
    }
    let (filter_manager, ..) = tab.log_manager.build_filter_manager();
    let visible = filter_manager.compute_visible(&tab.file_reader);
    tab.filter.visible_indices = VisibleLines::Filtered(visible);
}

/// A `Mode::handle_key` implementation that wants a filter/visibility
/// refresh calls the TUI's real `TabState::begin_filter_refresh`, which
/// (for the "has active filters" case) kicks off a cancellable background
/// scan and leaves `tab.filter.handle` set rather than updating
/// `visible_indices` synchronously. This GUI doesn't yet drive that
/// background-scan/polling machinery (see `force_recompute`'s doc comment),
/// so it discards the handle and recomputes synchronously instead —
/// correct, just without the background scan's large-file performance win.
fn sync_filter_recompute(tab: &mut TabState) {
    if tab.filter.handle.take().is_some() {
        force_recompute(tab);
    }
}

/// Loads `path` into a fresh `FileReader` + `LogManager` pair. Framework-
/// neutral async work the glue layer runs under its tokio bridge in
/// response to `Effect::LoadFile`; `push_tab` turns the result into a
/// full `TabState` (format detection, continuation/year maps, etc.) via
/// the TUI's own `TabState::new`.
pub async fn load_file(path: PathBuf, db: Arc<Database>) -> Result<FileLoaded, String> {
    let path_str = path.to_string_lossy().into_owned();
    let handle = FileReader::load(
        path_str.clone(),
        None,
        false,
        Arc::new(AtomicBool::new(false)),
        false,
    )
    .await
    .map_err(|e| e.to_string())?;
    let total_bytes = handle.total_bytes;
    let result = handle
        .result_rx
        .await
        .map_err(|_| "file load was cancelled".to_string())?
        .map_err(|e| e.to_string())?;
    let log_manager = LogManager::new(db, Some(path_str.clone())).await;
    // Same watcher the TUI spawns on every real file open (`src/ingestion/
    // loading.rs`) — makes `tab.stream.watch`/`is_tab_live` real instead of
    // always `None`. Consuming its ticks to actually refresh the view on
    // growth is separate, still-unimplemented follow-up work.
    let watch_rx = FileReader::spawn_file_watcher(path_str.clone(), total_bytes).await;
    let watch = crate::ui::watch_state_from_file(watch_rx, path_str);
    Ok(FileLoaded {
        path,
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
            reject_unsupported_filter_args(&field)?;
            let (fg, bg) = crate::ui::resolve_colors(auto, fga, fg, bg, theme_bg)?;
            let options = filter_options(fg, bg, line_mode, regex, ignore_case, group);
            log_manager
                .add_filter_with_color(pattern.join(" "), FilterType::Include, options)
                .await;
        }
        Commands::Exclude {
            pattern,
            field,
            regex,
            ignore_case,
            group,
        } => {
            reject_unsupported_filter_args(&field)?;
            let options = filter_options(None, None, false, regex, ignore_case, group);
            log_manager
                .add_filter_with_color(pattern.join(" "), FilterType::Exclude, options)
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
            reject_unsupported_filter_args(&field)?;
            let (fg, bg) = crate::ui::resolve_colors(auto, fga, fg, bg, theme_bg)?;
            let options = filter_options(fg, bg, line_mode, regex, ignore_case, group);
            log_manager
                .add_filter_with_color(pattern.join(" "), FilterType::Highlight, options)
                .await;
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
        other => return Err(format!("{other:?} is not yet supported in the GUI")),
    }
    Ok(log_manager)
}

/// `--field` filtering needs the structured/JSON field display the GUI
/// doesn't have yet, so it's rejected rather than silently ignored.
fn reject_unsupported_filter_args(field: &[String]) -> Result<(), String> {
    if !field.is_empty() {
        return Err(
            "--field filtering requires structured field display, not yet supported in the GUI"
                .to_string(),
        );
    }
    Ok(())
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
    let (fg, bg) = crate::ui::resolve_colors(auto, false, fg, bg, (0, 0, 0))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

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
        let lm = execute_command(log_manager().await, command("filter ERROR"), TEST_THEME_BG)
            .await
            .unwrap();
        assert_eq!(lm.get_filters().len(), 1);
        assert_eq!(lm.get_filters()[0].filter_type, FilterType::Include);
        assert_eq!(lm.get_filters()[0].pattern, "ERROR");
    }

    #[tokio::test]
    async fn execute_command_exclude_adds_an_exclude_filter() {
        let lm = execute_command(log_manager().await, command("exclude DEBUG"), TEST_THEME_BG)
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
        )
        .await
        .unwrap();
        assert_eq!(lm.get_filters()[0].filter_type, FilterType::Highlight);
    }

    #[tokio::test]
    async fn execute_command_rejects_field_filters() {
        let err = execute_command(
            log_manager().await,
            command("filter --field level=error"),
            TEST_THEME_BG,
        )
        .await
        .unwrap_err();
        assert!(err.contains("structured field display"));
    }

    #[tokio::test]
    async fn execute_command_auto_assigns_a_readable_fg_bg_pair() {
        let lm = execute_command(
            log_manager().await,
            command("filter --auto ERROR"),
            TEST_THEME_BG,
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
        let lm = execute_command(log_manager().await, command("filter ERROR"), TEST_THEME_BG)
            .await
            .unwrap();
        let lm = execute_command(lm, command("clear-filters"), TEST_THEME_BG)
            .await
            .unwrap();
        assert!(lm.get_filters().is_empty());
    }

    #[tokio::test]
    async fn execute_command_disable_then_enable_filters() {
        let lm = execute_command(log_manager().await, command("filter ERROR"), TEST_THEME_BG)
            .await
            .unwrap();
        let lm = execute_command(lm, command("disable-filters"), TEST_THEME_BG)
            .await
            .unwrap();
        assert!(!lm.get_filters()[0].enabled);
        let lm = execute_command(lm, command("enable-filters"), TEST_THEME_BG)
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
        )
        .await
        .unwrap();
        let lm = execute_command(lm, command("toggle-group net"), TEST_THEME_BG)
            .await
            .unwrap();
        assert!(!lm.get_filters()[0].enabled);
        let lm = execute_command(lm, command("toggle-group net"), TEST_THEME_BG)
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
        )
        .await
        .unwrap_err();
        assert!(err.contains("Invalid date filter"));
    }

    #[tokio::test]
    async fn execute_command_rejects_an_unimplemented_command() {
        let err = execute_command(log_manager().await, command("wrap"), TEST_THEME_BG)
            .await
            .unwrap_err();
        assert!(err.contains("not yet supported"));
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

    fn fake_watch_state() -> crate::ui::FileWatchState {
        let (_tx, rx) = tokio::sync::watch::channel(());
        crate::ui::FileWatchState {
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
    async fn is_tab_live_is_false_without_a_watcher() {
        let (state, _file) = state_with_one_tab().await;
        assert!(state.tabs[0].stream.watch.is_none());
        assert!(!is_tab_live(&state.tabs[0]));
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
            Effect::ExecuteCommand { command, .. } => {
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
            Effect::ExecuteCommand { command, .. } => {
                assert!(matches!(command, Commands::ClearFilters));
            }
            other => panic!("expected ExecuteCommand(ClearFilters), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn all_time_leaves_other_filters_alone_and_shows_a_status_message() {
        use crate::gui::time_range::TimeRangePreset;
        let (mut state, _file) = state_with_one_tab().await;
        let lm = execute_command(
            state.tabs[0].log_manager.clone(),
            command("date-filter > 2024-01-01"),
            TEST_THEME_BG,
        )
        .await
        .unwrap();
        let lm = execute_command(lm, command("filter ERROR"), TEST_THEME_BG)
            .await
            .unwrap();
        state.tabs[0].log_manager = lm;
        assert_eq!(state.tabs[0].log_manager.get_filters().len(), 2);

        let effect = update(
            &mut state,
            Message::TimeRangeSelected(TimeRangePreset::AllTime),
        );
        assert!(matches!(effect, Effect::None));
        assert_eq!(state.tabs[0].log_manager.get_filters().len(), 2);
        assert!(matches!(state.status, Some(StatusMessage::Error(_))));
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
            Effect::ExecuteCommand { command, .. } => {
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
    async fn facet_value_toggled_adds_a_field_filter_when_none_exists() {
        let (mut state, _file) = state_with_one_tab().await;
        let effect = update(
            &mut state,
            Message::FacetValueToggled(0, "level".to_string(), "error".to_string()),
        );
        match effect {
            Effect::ExecuteCommand { command, .. } => {
                assert!(matches!(command, Commands::Filter { .. }));
            }
            other => panic!("expected ExecuteCommand(Filter), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn facet_value_toggled_removes_the_existing_field_filter() {
        let (mut state, _file) = state_with_one_tab().await;
        let lm = execute_command(
            state.tabs[0].log_manager.clone(),
            command("filter --field level=error"),
            TEST_THEME_BG,
        )
        .await;
        // --field filters are currently rejected by the GUI (see
        // reject_unsupported_filter_args) - confirm that, then exercise
        // filter_id_for_field_equality directly against a hand-inserted
        // field filter, since execute_command can't produce one today.
        assert!(lm.is_err());

        state.tabs[0]
            .log_manager
            .add_filter_with_color(
                format!("{FIELD_PREFIX}level:error"),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        let id = state.tabs[0].log_manager.get_filters()[0].id;

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
    async fn j_in_normal_mode_scrolls_down_one_line() {
        let (mut state, _file) = state_with_one_tab().await;
        update(
            &mut state,
            Message::KeyPressed(KeyCode::Char('j'), KeyModifiers::NONE),
        );
        assert_eq!(state.tabs[0].scroll.scroll_offset, 1);
    }

    #[tokio::test]
    async fn colon_enters_command_mode() {
        let (mut state, _file) = state_with_one_tab().await;
        update(
            &mut state,
            Message::KeyPressed(KeyCode::Char(':'), KeyModifiers::NONE),
        );
        assert!(matches!(
            state.tabs[0].interaction.mode.render_state(),
            crate::mode::app_mode::ModeRenderState::Command { .. }
        ));
    }

    #[tokio::test]
    async fn typing_a_command_and_pressing_enter_runs_it() {
        let (mut state, _file) = state_with_one_tab().await;
        update(
            &mut state,
            Message::KeyPressed(KeyCode::Char(':'), KeyModifiers::NONE),
        );
        for c in "filter ERROR".chars() {
            update(
                &mut state,
                Message::KeyPressed(KeyCode::Char(c), KeyModifiers::NONE),
            );
        }
        let effect = update(
            &mut state,
            Message::KeyPressed(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert!(matches!(effect, Effect::ExecuteCommand { tab_idx: 0, .. }));
    }

    #[tokio::test]
    async fn q_quits() {
        let (mut state, _file) = state_with_one_tab().await;
        let effect = update(
            &mut state,
            Message::KeyPressed(KeyCode::Char('q'), KeyModifiers::NONE),
        );
        assert!(matches!(effect, Effect::Quit));
    }

    #[tokio::test]
    async fn ctrl_g_enters_group_management_mode() {
        let (mut state, _file) = state_with_one_tab().await;
        update(
            &mut state,
            Message::KeyPressed(KeyCode::Char('g'), KeyModifiers::CONTROL),
        );
        assert!(matches!(
            state.tabs[0].interaction.mode.render_state(),
            crate::mode::app_mode::ModeRenderState::GroupManagement { .. }
        ));
    }

    #[tokio::test]
    async fn f_enters_filter_management_mode() {
        let (mut state, _file) = state_with_one_tab().await;
        update(
            &mut state,
            Message::KeyPressed(KeyCode::Char('f'), KeyModifiers::NONE),
        );
        assert!(matches!(
            state.tabs[0].interaction.mode.render_state(),
            crate::mode::app_mode::ModeRenderState::FilterManagement { .. }
        ));
    }

    /// Reproduces the real panic this guarded against: a `#[tokio::test]`
    /// already runs inside its own ambient tokio reactor, which would mask
    /// a missing `.enter()` guard in `dispatch_key` — gpui's real render
    /// thread has no such ambient reactor. So setup runs inside a scratch
    /// runtime's `block_on` (which exits its context when it returns), and
    /// the actual `update()` dispatch calls run as a plain, non-async
    /// `#[test]` with no tokio context of their own — exactly like gpui's
    /// render thread — to prove `dispatch_key` supplies one itself.
    #[test]
    fn toggling_a_filter_while_others_stay_active_does_not_panic() {
        let setup_rt = tokio::runtime::Runtime::new().unwrap();
        let (mut state, _file) = setup_rt.block_on(state_with_one_tab());
        let lm = setup_rt.block_on(execute_command(
            state.tabs[0].log_manager.clone(),
            command("filter one"),
            TEST_THEME_BG,
        ));
        let lm = setup_rt.block_on(execute_command(
            lm.unwrap(),
            command("filter two"),
            TEST_THEME_BG,
        ));
        state.tabs[0].log_manager = lm.unwrap();
        force_recompute(&mut state.tabs[0]);
        drop(setup_rt);

        // No tokio context entered on this thread from here on — two
        // enabled filters means toggling one off still leaves the tab
        // with active filters, hitting the branch of `TabState::
        // begin_filter_refresh` that spawns a background scan via
        // `tokio::task::spawn_blocking`.
        update(
            &mut state,
            Message::KeyPressed(KeyCode::Char('f'), KeyModifiers::NONE),
        );
        update(
            &mut state,
            Message::KeyPressed(KeyCode::Char(' '), KeyModifiers::NONE),
        );

        assert!(!state.tabs[0].log_manager.get_filters()[0].enabled);
    }

    #[tokio::test]
    async fn slash_enters_search_mode() {
        let (mut state, _file) = state_with_one_tab().await;
        update(
            &mut state,
            Message::KeyPressed(KeyCode::Char('/'), KeyModifiers::NONE),
        );
        assert!(matches!(
            state.tabs[0].interaction.mode.render_state(),
            crate::mode::app_mode::ModeRenderState::Search { .. }
        ));
    }

    #[tokio::test]
    async fn command_executed_ok_applies_the_log_manager_and_recomputes() {
        let (mut state, _file) = state_with_one_tab().await;
        let lm = execute_command(
            state.tabs[0].log_manager.clone(),
            command("filter line"),
            TEST_THEME_BG,
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
        )
        .await
        .unwrap();
        lm = execute_command(lm, command("toggle-group net"), TEST_THEME_BG)
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
}
