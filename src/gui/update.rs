use crate::gui::effect::ScrollTarget;
use crate::gui::key::{GuiKey, GuiModifiers, NamedKey};
use crate::gui::state::InteractionMode;

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
}
