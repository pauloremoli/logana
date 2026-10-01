use crate::commands::auto_complete::shell_split;
use crate::commands::{CommandLine, Commands};
use crate::db::LogManager;
use crate::filters::{DATE_PREFIX, FilterOptions, FilterType, parse_date_filter};
use crate::gui::effect::ScrollTarget;
use crate::gui::key::{GuiKey, GuiModifiers, NamedKey};
use crate::gui::state::InteractionMode;
use clap::Parser;

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

/// Whether `key` should be let through to a focused widget (e.g. the
/// command-bar text input) instead of being handled/consumed here —
/// Normal mode consumes everything; Command mode only intercepts Escape.
pub fn should_capture(mode: &InteractionMode, key: &GuiKey) -> bool {
    match mode {
        InteractionMode::Normal => true,
        InteractionMode::Command { .. } => matches!(key, GuiKey::Named(NamedKey::Escape)),
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
    fn command_mode_only_captures_escape() {
        let mode = InteractionMode::Command {
            input: String::new(),
        };
        assert!(should_capture(&mode, &GuiKey::Named(NamedKey::Escape)));
        assert!(!should_capture(&mode, &char_key("j")));
        assert!(!should_capture(&mode, &char_key("a")));
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
}
