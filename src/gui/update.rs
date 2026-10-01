use crate::commands::auto_complete::shell_split;
use crate::commands::{CommandLine, Commands};
use crate::db::LogManager;
use crate::filters::{DATE_PREFIX, FilterOptions, FilterType, group_enabled, parse_date_filter};
use crate::gui::effect::Effect;
use crate::gui::key::{GuiKey, GuiModifiers, NamedKey};
use crate::gui::message::{FileLoaded, Message};
use crate::gui::state::{GuiState, InteractionMode, StatusMessage, TabState};
use crate::ingestion::FileReader;
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
            if idx < state.tabs.len() {
                state.tabs.remove(idx);
                state.active_tab = active_after_close(state.active_tab, idx, state.tabs.len());
            }
            Effect::None
        }
        Message::KeyPressed(key, modifiers) => handle_key_press(state, key, modifiers),
        Message::CommandInputChanged(new_input) => {
            if let InteractionMode::Command { input } = &mut state.mode {
                *input = new_input;
            }
            Effect::None
        }
        Message::CommandSubmitted => handle_command_submitted(state),
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
    }
}

fn handle_key_press(state: &mut GuiState, key: GuiKey, modifiers: GuiModifiers) -> Effect {
    match &state.mode {
        InteractionMode::Normal => handle_normal_mode_key(state, key, modifiers),
        InteractionMode::Command { .. } => handle_command_mode_key(state, key, modifiers),
    }
}

fn handle_normal_mode_key(state: &mut GuiState, key: GuiKey, modifiers: GuiModifiers) -> Effect {
    let (action, g_pending) = normal_action(&key, &modifiers, state.g_pending);
    state.g_pending = g_pending;
    match action {
        NormalAction::Scroll(target) => match resolve_scroll(state, target) {
            Some(offset) => Effect::Scroll(offset),
            None => Effect::None,
        },
        NormalAction::NextTab => {
            advance_tab(state, 1);
            Effect::None
        }
        NormalAction::PrevTab => {
            advance_tab(state, -1);
            Effect::None
        }
        NormalAction::EnterCommandMode => {
            state.mode = InteractionMode::Command {
                input: String::new(),
            };
            Effect::FocusCommandBar
        }
        NormalAction::Quit => Effect::Quit,
        NormalAction::None => Effect::None,
    }
}

/// Command mode has no focused text-editing widget of its own to fall
/// keys through to (see `should_capture`), so every key that isn't a
/// control character is handled here directly: typed characters append to
/// the buffer, Backspace removes the last one, Enter submits, Escape
/// cancels back to Normal mode.
fn handle_command_mode_key(state: &mut GuiState, key: GuiKey, modifiers: GuiModifiers) -> Effect {
    match key {
        GuiKey::Named(NamedKey::Escape) => {
            state.mode = InteractionMode::Normal;
            Effect::None
        }
        GuiKey::Named(NamedKey::Enter) => handle_command_submitted(state),
        GuiKey::Named(NamedKey::Backspace) => {
            if let InteractionMode::Command { input } = &mut state.mode {
                input.pop();
            }
            Effect::None
        }
        GuiKey::Character(c) if !modifiers.control => {
            if let InteractionMode::Command { input } = &mut state.mode {
                input.push_str(&c);
            }
            Effect::None
        }
        _ => Effect::None,
    }
}

/// Closes the command bar unconditionally (matching the TUI, which always
/// returns to Normal mode on submit) and, for a non-empty valid command,
/// asks the glue layer to run it asynchronously against the active tab's
/// `LogManager`. A parse error is shown as a status message instead.
fn handle_command_submitted(state: &mut GuiState) -> Effect {
    let input = match &state.mode {
        InteractionMode::Command { input } => input.clone(),
        InteractionMode::Normal => return Effect::None,
    };
    state.mode = InteractionMode::Normal;
    let tab_idx = state.active_tab;
    let Some(tab) = state.tabs.get(tab_idx) else {
        return Effect::None;
    };
    match parse_command(&input) {
        Ok(Some(command)) => Effect::ExecuteCommand {
            tab_idx,
            log_manager: tab.log_manager.clone(),
            command,
        },
        Ok(None) => Effect::None,
        Err(err) => {
            state.status = Some(StatusMessage::Error(err));
            Effect::None
        }
    }
}

fn advance_tab(state: &mut GuiState, delta: isize) {
    if !state.tabs.is_empty() {
        state.active_tab = next_active_tab(state.active_tab, delta, state.tabs.len());
    }
}

/// Applies `target` to the active tab's `scroll_offset`, clamped to its
/// visible-line count, and returns the resulting absolute offset — or
/// `None` if there's no active tab (nothing to scroll).
fn resolve_scroll(state: &mut GuiState, target: ScrollTarget) -> Option<usize> {
    let tab = state.active_tab_mut()?;
    let total = tab.visible_lines.len();
    let new_offset = match target {
        ScrollTarget::By(delta) => (tab.scroll_offset as i64 + delta as i64).max(0) as usize,
        ScrollTarget::Top => 0,
        ScrollTarget::Bottom => total.saturating_sub(1),
    };
    tab.scroll_offset = clamp_scroll_offset(new_offset, total);
    Some(tab.scroll_offset)
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
    let mut tab = TabState::new(loaded.path, loaded.reader, loaded.log_manager);
    recompute_tab(&mut tab);
    state.tabs.push(tab);
    state.active_tab = state.tabs.len() - 1;
}

fn apply_log_manager(state: &mut GuiState, tab_idx: usize, log_manager: LogManager) {
    if let Some(tab) = state.tabs.get_mut(tab_idx) {
        tab.log_manager = log_manager;
        recompute_tab(tab);
    }
}

/// Rebuilds `visible_lines`/`filter_defs`/`group_defs` from the tab's
/// `LogManager` — called after every mutation, mirroring the TUI's
/// `begin_filter_refresh`.
pub fn recompute_tab(tab: &mut TabState) {
    let (filter_manager, _, _, _) = tab.log_manager.build_filter_manager();
    tab.visible_lines = filter_manager.compute_visible(&tab.reader);
    tab.filter_defs = tab.log_manager.get_filters().to_vec();
    tab.group_defs = tab.log_manager.get_group_styles().to_vec();
    tab.scroll_offset = clamp_scroll_offset(tab.scroll_offset, tab.visible_lines.len());
}

/// Clamps a scroll offset to the last valid row of a `total`-line list
/// (`0` when there are no lines at all).
fn clamp_scroll_offset(offset: usize, total: usize) -> usize {
    offset.min(total.saturating_sub(1))
}

/// Loads `path` into a fresh `FileReader` + `LogManager` pair. Framework-
/// neutral async work the glue layer runs under its tokio bridge in
/// response to `Effect::LoadFile`.
pub async fn load_file(path: PathBuf, db: Arc<crate::db::Database>) -> Result<FileLoaded, String> {
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
    let result = handle
        .result_rx
        .await
        .map_err(|_| "file load was cancelled".to_string())?
        .map_err(|e| e.to_string())?;
    let log_manager = LogManager::new(db, Some(path_str)).await;
    Ok(FileLoaded {
        path,
        reader: result.reader,
        log_manager,
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

/// What a Normal-mode key press should do, independent of how the key
/// arrived (gpui keystroke vs. the TUI's crossterm event) — mirrors the
/// TUI NormalMode's `j/k`, `Ctrl+d/u`, `PageUp/Down`, `gg`/`G`,
/// `Tab`/`Shift+Tab`, `:`, `q` bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormalAction {
    Scroll(ScrollTarget),
    NextTab,
    PrevTab,
    EnterCommandMode,
    Quit,
    None,
}

/// A relative or absolute scroll request from a Normal-mode key —
/// resolved against the active tab's line count into an absolute
/// `Effect::Scroll` offset by `handle_normal_mode_key`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollTarget {
    By(i32),
    Top,
    Bottom,
}

/// Default half/full "page" size in lines, used until a later step wires
/// the log pane's real viewport height through — matches a plausible
/// terminal/window height, same ballpark as the TUI's own page scrolling.
const HALF_PAGE_LINES: i32 = 15;
const FULL_PAGE_LINES: i32 = 30;

/// Maps a Normal-mode key press to the action it performs, and the new
/// value `g_pending` should take afterward. `g_pending` tracks a leading
/// `g` so `gg` (jump to top) can be recognized as a two-key chord, exactly
/// like the TUI's `g_key_pressed` flag.
pub fn normal_action(
    key: &GuiKey,
    modifiers: &GuiModifiers,
    g_pending: bool,
) -> (NormalAction, bool) {
    if g_pending {
        let action = if is_character(key, "g") {
            NormalAction::Scroll(ScrollTarget::Top)
        } else {
            NormalAction::None
        };
        return (action, false);
    }

    let action = match key {
        GuiKey::Character(c) if c == "g" => {
            return (NormalAction::None, true);
        }
        GuiKey::Character(c) if c == "G" => NormalAction::Scroll(ScrollTarget::Bottom),
        GuiKey::Character(c) if c == "j" => NormalAction::Scroll(ScrollTarget::By(1)),
        GuiKey::Character(c) if c == "k" => NormalAction::Scroll(ScrollTarget::By(-1)),
        GuiKey::Character(c) if c == "d" && modifiers.control => {
            NormalAction::Scroll(ScrollTarget::By(HALF_PAGE_LINES))
        }
        GuiKey::Character(c) if c == "u" && modifiers.control => {
            NormalAction::Scroll(ScrollTarget::By(-HALF_PAGE_LINES))
        }
        GuiKey::Character(c) if c == ":" => NormalAction::EnterCommandMode,
        GuiKey::Character(c) if c == "q" => NormalAction::Quit,
        GuiKey::Named(NamedKey::PageDown) => {
            NormalAction::Scroll(ScrollTarget::By(FULL_PAGE_LINES))
        }
        GuiKey::Named(NamedKey::PageUp) => NormalAction::Scroll(ScrollTarget::By(-FULL_PAGE_LINES)),
        GuiKey::Named(NamedKey::Tab) if !modifiers.shift => NormalAction::NextTab,
        GuiKey::Named(NamedKey::Tab) if modifiers.shift => NormalAction::PrevTab,
        _ => NormalAction::None,
    };
    (action, false)
}

fn is_character(key: &GuiKey, expected: &str) -> bool {
    matches!(key, GuiKey::Character(c) if c == expected)
}

/// Whether `key` should be handled by the capture-phase handler rather
/// than left to bubble to a focused widget. Both modes capture everything
/// today — Command mode has no separate text-editing widget of its own
/// (see `handle_command_mode_key`), so there's nothing to fall through
/// to. Kept as an explicit per-mode decision rather than an unconditional
/// `true`, since a future phase may adopt a real focusable input widget
/// for Command mode, which would need to receive some keys directly.
pub fn should_capture(mode: &InteractionMode, _key: &GuiKey) -> bool {
    match mode {
        InteractionMode::Normal => true,
        InteractionMode::Command { .. } => true,
    }
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
/// commands Phase 1 supports. A command this GUI doesn't implement yet
/// (structured-field filtering, auto colors, every other `:` command)
/// errors rather than silently doing nothing.
pub async fn execute_command(
    mut log_manager: LogManager,
    command: Commands,
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
            reject_unsupported_filter_args(&field, auto, fga)?;
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
            reject_unsupported_filter_args(&field, false, false)?;
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
            reject_unsupported_filter_args(&field, auto, fga)?;
            let options = filter_options(fg, bg, line_mode, regex, ignore_case, group);
            log_manager
                .add_filter_with_color(pattern.join(" "), FilterType::Highlight, options)
                .await;
        }
        Commands::ClearFilters => log_manager.clear_filters().await,
        Commands::DisableFilters => log_manager.disable_all_filters().await,
        Commands::EnableFilters => log_manager.enable_all_filters().await,
        Commands::ToggleGroup { name } => toggle_group(&mut log_manager, &name).await?,
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
/// doesn't have yet; `--auto`/`--fga` need the TUI's private color-resolver
/// (kept out of scope for this migration, see project notes). Both are
/// rejected rather than silently ignored.
fn reject_unsupported_filter_args(field: &[String], auto: bool, fga: bool) -> Result<(), String> {
    if !field.is_empty() {
        return Err(
            "--field filtering requires structured field display, not yet supported in the GUI"
                .to_string(),
        );
    }
    if auto || fga {
        return Err("--auto/--fga are not yet supported in the GUI".to_string());
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
async fn toggle_group(log_manager: &mut LogManager, name: &str) -> Result<(), String> {
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
    if auto {
        return Err("--auto is not yet supported in the GUI".to_string());
    }
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

    fn char_key(c: &str) -> GuiKey {
        GuiKey::Character(c.to_string())
    }

    fn mods(control: bool, shift: bool) -> GuiModifiers {
        GuiModifiers { control, shift }
    }

    #[test]
    fn j_scrolls_down_one_line() {
        let (action, g) = normal_action(&char_key("j"), &mods(false, false), false);
        assert_eq!(action, NormalAction::Scroll(ScrollTarget::By(1)));
        assert!(!g);
    }

    #[test]
    fn k_scrolls_up_one_line() {
        let (action, _) = normal_action(&char_key("k"), &mods(false, false), false);
        assert_eq!(action, NormalAction::Scroll(ScrollTarget::By(-1)));
    }

    #[test]
    fn ctrl_d_scrolls_down_half_a_page() {
        let (action, _) = normal_action(&char_key("d"), &mods(true, false), false);
        assert_eq!(
            action,
            NormalAction::Scroll(ScrollTarget::By(HALF_PAGE_LINES))
        );
    }

    #[test]
    fn ctrl_u_scrolls_up_half_a_page() {
        let (action, _) = normal_action(&char_key("u"), &mods(true, false), false);
        assert_eq!(
            action,
            NormalAction::Scroll(ScrollTarget::By(-HALF_PAGE_LINES))
        );
    }

    #[test]
    fn plain_d_and_u_do_nothing() {
        assert_eq!(
            normal_action(&char_key("d"), &mods(false, false), false).0,
            NormalAction::None
        );
        assert_eq!(
            normal_action(&char_key("u"), &mods(false, false), false).0,
            NormalAction::None
        );
    }

    #[test]
    fn page_down_scrolls_a_full_page() {
        let (action, _) = normal_action(
            &GuiKey::Named(NamedKey::PageDown),
            &mods(false, false),
            false,
        );
        assert_eq!(
            action,
            NormalAction::Scroll(ScrollTarget::By(FULL_PAGE_LINES))
        );
    }

    #[test]
    fn page_up_scrolls_a_full_page_up() {
        let (action, _) =
            normal_action(&GuiKey::Named(NamedKey::PageUp), &mods(false, false), false);
        assert_eq!(
            action,
            NormalAction::Scroll(ScrollTarget::By(-FULL_PAGE_LINES))
        );
    }

    #[test]
    fn single_g_sets_g_pending_without_acting() {
        let (action, g) = normal_action(&char_key("g"), &mods(false, false), false);
        assert_eq!(action, NormalAction::None);
        assert!(g);
    }

    #[test]
    fn gg_jumps_to_top_and_clears_g_pending() {
        let (action, g) = normal_action(&char_key("g"), &mods(false, false), true);
        assert_eq!(action, NormalAction::Scroll(ScrollTarget::Top));
        assert!(!g);
    }

    #[test]
    fn g_then_unrelated_key_clears_g_pending_without_acting() {
        let (action, g) = normal_action(&char_key("j"), &mods(false, false), true);
        assert_eq!(action, NormalAction::None);
        assert!(!g);
    }

    #[test]
    fn shift_g_jumps_to_bottom() {
        let (action, _) = normal_action(&char_key("G"), &mods(false, true), false);
        assert_eq!(action, NormalAction::Scroll(ScrollTarget::Bottom));
    }

    #[test]
    fn tab_moves_to_next_tab() {
        let (action, _) = normal_action(&GuiKey::Named(NamedKey::Tab), &mods(false, false), false);
        assert_eq!(action, NormalAction::NextTab);
    }

    #[test]
    fn shift_tab_moves_to_previous_tab() {
        let (action, _) = normal_action(&GuiKey::Named(NamedKey::Tab), &mods(false, true), false);
        assert_eq!(action, NormalAction::PrevTab);
    }

    #[test]
    fn colon_enters_command_mode() {
        let (action, _) = normal_action(&char_key(":"), &mods(false, false), false);
        assert_eq!(action, NormalAction::EnterCommandMode);
    }

    #[test]
    fn q_quits() {
        let (action, _) = normal_action(&char_key("q"), &mods(false, false), false);
        assert_eq!(action, NormalAction::Quit);
    }

    #[test]
    fn unrecognized_key_does_nothing() {
        let (action, _) = normal_action(&char_key("z"), &mods(false, false), false);
        assert_eq!(action, NormalAction::None);
    }

    #[test]
    fn normal_mode_captures_every_key() {
        assert!(should_capture(&InteractionMode::Normal, &char_key("j")));
        assert!(should_capture(
            &InteractionMode::Normal,
            &GuiKey::Named(NamedKey::Escape)
        ));
    }

    #[test]
    fn command_mode_also_captures_every_key() {
        let mode = InteractionMode::Command {
            input: String::new(),
        };
        assert!(should_capture(&mode, &GuiKey::Named(NamedKey::Escape)));
        assert!(should_capture(&mode, &char_key("j")));
        assert!(should_capture(&mode, &char_key("a")));
    }

    use crate::db::Database;
    use std::sync::Arc;

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
        let lm = execute_command(log_manager().await, command("filter ERROR"))
            .await
            .unwrap();
        assert_eq!(lm.get_filters().len(), 1);
        assert_eq!(lm.get_filters()[0].filter_type, FilterType::Include);
        assert_eq!(lm.get_filters()[0].pattern, "ERROR");
    }

    #[tokio::test]
    async fn execute_command_exclude_adds_an_exclude_filter() {
        let lm = execute_command(log_manager().await, command("exclude DEBUG"))
            .await
            .unwrap();
        assert_eq!(lm.get_filters()[0].filter_type, FilterType::Exclude);
    }

    #[tokio::test]
    async fn execute_command_highlight_adds_a_highlight_filter() {
        let lm = execute_command(log_manager().await, command("highlight WARN"))
            .await
            .unwrap();
        assert_eq!(lm.get_filters()[0].filter_type, FilterType::Highlight);
    }

    #[tokio::test]
    async fn execute_command_rejects_field_filters() {
        let err = execute_command(log_manager().await, command("filter --field level=error"))
            .await
            .unwrap_err();
        assert!(err.contains("structured field display"));
    }

    #[tokio::test]
    async fn execute_command_rejects_auto_color_flags() {
        let err = execute_command(log_manager().await, command("filter --auto ERROR"))
            .await
            .unwrap_err();
        assert!(err.contains("--auto"));
    }

    #[tokio::test]
    async fn execute_command_clear_filters_removes_everything() {
        let lm = execute_command(log_manager().await, command("filter ERROR"))
            .await
            .unwrap();
        let lm = execute_command(lm, command("clear-filters")).await.unwrap();
        assert!(lm.get_filters().is_empty());
    }

    #[tokio::test]
    async fn execute_command_disable_then_enable_filters() {
        let lm = execute_command(log_manager().await, command("filter ERROR"))
            .await
            .unwrap();
        let lm = execute_command(lm, command("disable-filters"))
            .await
            .unwrap();
        assert!(!lm.get_filters()[0].enabled);
        let lm = execute_command(lm, command("enable-filters"))
            .await
            .unwrap();
        assert!(lm.get_filters()[0].enabled);
    }

    #[tokio::test]
    async fn execute_command_toggle_group_toggles_members_off_then_on() {
        let lm = execute_command(log_manager().await, command("filter -g net ERROR"))
            .await
            .unwrap();
        let lm = execute_command(lm, command("toggle-group net"))
            .await
            .unwrap();
        assert!(!lm.get_filters()[0].enabled);
        let lm = execute_command(lm, command("toggle-group net"))
            .await
            .unwrap();
        assert!(lm.get_filters()[0].enabled);
    }

    #[tokio::test]
    async fn execute_command_toggle_group_errors_for_an_unknown_group() {
        let err = execute_command(log_manager().await, command("toggle-group missing"))
            .await
            .unwrap_err();
        assert!(err.contains("missing"));
    }

    #[tokio::test]
    async fn execute_command_group_sets_a_predefined_style() {
        let lm = execute_command(log_manager().await, command("group net --fg red"))
            .await
            .unwrap();
        assert_eq!(lm.get_group_styles().len(), 1);
        assert_eq!(lm.get_group_styles()[0].name, "net");
    }

    #[tokio::test]
    async fn execute_command_group_clear_rejects_other_flags() {
        let err = execute_command(log_manager().await, command("group net --clear --fg red"))
            .await
            .unwrap_err();
        assert!(err.contains("--clear"));
    }

    #[tokio::test]
    async fn execute_command_date_filter_adds_a_tagged_include_filter() {
        let lm = execute_command(log_manager().await, command("date-filter > 2024-01-01"))
            .await
            .unwrap();
        assert!(lm.get_filters()[0].pattern.starts_with(DATE_PREFIX));
        assert_eq!(lm.get_filters()[0].filter_type, FilterType::Include);
    }

    #[tokio::test]
    async fn execute_command_date_filter_rejects_an_invalid_expression() {
        let err = execute_command(log_manager().await, command("date-filter not-a-date"))
            .await
            .unwrap_err();
        assert!(err.contains("Invalid date filter"));
    }

    #[tokio::test]
    async fn execute_command_rejects_an_unimplemented_command() {
        let err = execute_command(log_manager().await, command("wrap"))
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
        let mut tab = TabState::new(file.path().to_path_buf(), reader, log_manager);
        recompute_tab(&mut tab);
        state.tabs.push(tab);
        (state, file)
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
        state.tabs.push({
            let file = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(file.path(), "x\n").unwrap();
            let reader = FileReader::new(file.path().to_str().unwrap()).unwrap();
            let lm = LogManager::new(Arc::clone(&state.db), None).await;
            TabState::new(file.path().to_path_buf(), reader, lm)
        });
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
    async fn command_input_changed_updates_the_command_mode_buffer() {
        let (mut state, _file) = state_with_one_tab().await;
        state.mode = InteractionMode::Command {
            input: String::new(),
        };
        update(
            &mut state,
            Message::CommandInputChanged("filter ERROR".to_string()),
        );
        assert_eq!(
            state.mode,
            InteractionMode::Command {
                input: "filter ERROR".to_string()
            }
        );
    }

    #[tokio::test]
    async fn enter_command_mode_switches_mode_and_requests_focus() {
        let (mut state, _file) = state_with_one_tab().await;
        let effect = update(
            &mut state,
            Message::KeyPressed(GuiKey::Character(":".to_string()), GuiModifiers::default()),
        );
        assert_eq!(
            state.mode,
            InteractionMode::Command {
                input: String::new()
            }
        );
        assert!(matches!(effect, Effect::FocusCommandBar));
    }

    #[tokio::test]
    async fn typing_a_character_in_command_mode_appends_to_the_buffer() {
        let (mut state, _file) = state_with_one_tab().await;
        state.mode = InteractionMode::Command {
            input: "filter".to_string(),
        };
        update(
            &mut state,
            Message::KeyPressed(char_key(" "), GuiModifiers::default()),
        );
        assert_eq!(
            state.mode,
            InteractionMode::Command {
                input: "filter ".to_string()
            }
        );
    }

    #[tokio::test]
    async fn backspace_in_command_mode_removes_the_last_character() {
        let (mut state, _file) = state_with_one_tab().await;
        state.mode = InteractionMode::Command {
            input: "filter".to_string(),
        };
        update(
            &mut state,
            Message::KeyPressed(GuiKey::Named(NamedKey::Backspace), GuiModifiers::default()),
        );
        assert_eq!(
            state.mode,
            InteractionMode::Command {
                input: "filte".to_string()
            }
        );
    }

    #[tokio::test]
    async fn ctrl_held_characters_are_not_inserted_in_command_mode() {
        let (mut state, _file) = state_with_one_tab().await;
        state.mode = InteractionMode::Command {
            input: "filter".to_string(),
        };
        update(
            &mut state,
            Message::KeyPressed(
                char_key("c"),
                GuiModifiers {
                    control: true,
                    shift: false,
                },
            ),
        );
        assert_eq!(
            state.mode,
            InteractionMode::Command {
                input: "filter".to_string()
            }
        );
    }

    #[tokio::test]
    async fn escape_in_command_mode_returns_to_normal() {
        let (mut state, _file) = state_with_one_tab().await;
        state.mode = InteractionMode::Command {
            input: "filter".to_string(),
        };
        update(
            &mut state,
            Message::KeyPressed(GuiKey::Named(NamedKey::Escape), GuiModifiers::default()),
        );
        assert_eq!(state.mode, InteractionMode::Normal);
    }

    #[tokio::test]
    async fn enter_in_command_mode_submits_the_command() {
        let (mut state, _file) = state_with_one_tab().await;
        state.mode = InteractionMode::Command {
            input: "filter ERROR".to_string(),
        };
        let effect = update(
            &mut state,
            Message::KeyPressed(GuiKey::Named(NamedKey::Enter), GuiModifiers::default()),
        );
        assert!(matches!(effect, Effect::ExecuteCommand { tab_idx: 0, .. }));
        assert_eq!(state.mode, InteractionMode::Normal);
    }

    #[tokio::test]
    async fn j_in_normal_mode_scrolls_down_one_line() {
        let (mut state, _file) = state_with_one_tab().await;
        let effect = update(
            &mut state,
            Message::KeyPressed(char_key("j"), GuiModifiers::default()),
        );
        assert!(matches!(effect, Effect::Scroll(1)));
        assert_eq!(state.tabs[0].scroll_offset, 1);
    }

    #[tokio::test]
    async fn k_at_the_top_does_not_scroll_past_zero() {
        let (mut state, _file) = state_with_one_tab().await;
        let effect = update(
            &mut state,
            Message::KeyPressed(char_key("k"), GuiModifiers::default()),
        );
        assert!(matches!(effect, Effect::Scroll(0)));
        assert_eq!(state.tabs[0].scroll_offset, 0);
    }

    #[tokio::test]
    async fn shift_g_scrolls_to_the_last_line() {
        let (mut state, _file) = state_with_one_tab().await;
        let effect = update(
            &mut state,
            Message::KeyPressed(char_key("G"), GuiModifiers::default()),
        );
        assert!(matches!(effect, Effect::Scroll(1)));
        assert_eq!(state.tabs[0].scroll_offset, 1);
    }

    #[tokio::test]
    async fn scroll_with_no_tabs_open_does_nothing() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let mut state = GuiState::new(db);
        let effect = update(
            &mut state,
            Message::KeyPressed(char_key("j"), GuiModifiers::default()),
        );
        assert!(matches!(effect, Effect::None));
    }

    #[tokio::test]
    async fn command_submitted_with_no_tabs_does_nothing() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let mut state = GuiState::new(db);
        state.mode = InteractionMode::Command {
            input: "filter ERROR".to_string(),
        };
        let effect = update(&mut state, Message::CommandSubmitted);
        assert!(matches!(effect, Effect::None));
        assert_eq!(state.mode, InteractionMode::Normal);
    }

    #[tokio::test]
    async fn command_submitted_with_a_valid_command_requests_execution() {
        let (mut state, _file) = state_with_one_tab().await;
        state.mode = InteractionMode::Command {
            input: "filter ERROR".to_string(),
        };
        let effect = update(&mut state, Message::CommandSubmitted);
        assert!(matches!(effect, Effect::ExecuteCommand { tab_idx: 0, .. }));
        assert_eq!(state.mode, InteractionMode::Normal);
    }

    #[tokio::test]
    async fn command_submitted_with_an_invalid_command_sets_status() {
        let (mut state, _file) = state_with_one_tab().await;
        state.mode = InteractionMode::Command {
            input: "not-a-real-command".to_string(),
        };
        update(&mut state, Message::CommandSubmitted);
        assert!(matches!(state.status, Some(StatusMessage::Error(_))));
    }

    #[tokio::test]
    async fn command_executed_ok_applies_the_log_manager_and_recomputes() {
        let (mut state, _file) = state_with_one_tab().await;
        let lm = execute_command(state.tabs[0].log_manager.clone(), command("filter line"))
            .await
            .unwrap();
        update(&mut state, Message::CommandExecuted(0, Ok(lm)));
        assert_eq!(state.tabs[0].filter_defs.len(), 1);
    }

    #[tokio::test]
    async fn toggle_group_checkbox_enables_a_disabled_group() {
        let mut lm = execute_command(log_manager().await, command("filter -g net ERROR"))
            .await
            .unwrap();
        lm = execute_command(lm, command("toggle-group net"))
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
    }
}
