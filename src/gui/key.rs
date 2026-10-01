use crate::input::{KeyCode, KeyModifiers};
use gpui_kit::gpui::Keystroke;

/// Converts gpui's raw keystroke into the framework-neutral type
/// `Mode::handle_key`/`Keybindings` operate on — the GUI's only place
/// that touches gpui's key types, mirroring the TUI's crossterm boundary
/// (`ui::input::from_crossterm`). Keys with no neutral equivalent return
/// `None`. A character key prefers `key_char` (what was actually typed,
/// e.g. `:` for shift+`;`) over `key` (the unshifted key name), so typed
/// text reflects the real character. Shift+Tab reports as gpui key `"tab"`
/// with the shift modifier held, not a separate key name — translated to
/// `KeyCode::BackTab` with shift cleared, matching crossterm's convention
/// (and the TUI's default bindings, e.g. `prev_tab` is bound to `BackTab`).
pub fn from_gpui_keystroke(keystroke: &Keystroke) -> Option<(KeyCode, KeyModifiers)> {
    let mut modifiers = KeyModifiers::NONE;
    if keystroke.modifiers.shift {
        modifiers |= KeyModifiers::SHIFT;
    }
    if keystroke.modifiers.control {
        modifiers |= KeyModifiers::CONTROL;
    }
    if keystroke.modifiers.alt {
        modifiers |= KeyModifiers::ALT;
    }

    if keystroke.key == "tab" && modifiers.contains(KeyModifiers::SHIFT) {
        let mut mods = KeyModifiers::NONE;
        if keystroke.modifiers.control {
            mods |= KeyModifiers::CONTROL;
        }
        if keystroke.modifiers.alt {
            mods |= KeyModifiers::ALT;
        }
        return Some((KeyCode::BackTab, mods));
    }

    let key = match keystroke.key.as_str() {
        "backspace" => KeyCode::Backspace,
        "delete" => KeyCode::Delete,
        "down" => KeyCode::Down,
        "end" => KeyCode::End,
        "enter" => KeyCode::Enter,
        "escape" => KeyCode::Esc,
        "home" => KeyCode::Home,
        "insert" => KeyCode::Insert,
        "left" => KeyCode::Left,
        "pagedown" => KeyCode::PageDown,
        "pageup" => KeyCode::PageUp,
        "right" => KeyCode::Right,
        "tab" => KeyCode::Tab,
        "up" => KeyCode::Up,
        other => {
            if let Some(n) = other.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
                KeyCode::F(n)
            } else if let Some(c) = keystroke.key_char.as_deref().and_then(|s| s.chars().next()) {
                KeyCode::Char(c)
            } else {
                other.chars().next().map(KeyCode::Char)?
            }
        }
    };
    Some((key, modifiers))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::gpui::Modifiers;

    fn keystroke(key: &str, control: bool, shift: bool, alt: bool) -> Keystroke {
        Keystroke {
            modifiers: Modifiers {
                control,
                shift,
                alt,
                ..Modifiers::default()
            },
            key: key.to_string(),
            key_char: None,
        }
    }

    #[test]
    fn named_keys_map_to_their_variant() {
        let cases = [
            ("up", KeyCode::Up),
            ("down", KeyCode::Down),
            ("left", KeyCode::Left),
            ("right", KeyCode::Right),
            ("pageup", KeyCode::PageUp),
            ("pagedown", KeyCode::PageDown),
            ("home", KeyCode::Home),
            ("end", KeyCode::End),
            ("insert", KeyCode::Insert),
            ("delete", KeyCode::Delete),
            ("tab", KeyCode::Tab),
            ("escape", KeyCode::Esc),
            ("enter", KeyCode::Enter),
            ("backspace", KeyCode::Backspace),
        ];
        for (raw, expected) in cases {
            let (key, _) = from_gpui_keystroke(&keystroke(raw, false, false, false)).unwrap();
            assert_eq!(key, expected, "{raw}");
        }
    }

    #[test]
    fn f_keys_parse_their_number() {
        let (key, _) = from_gpui_keystroke(&keystroke("f1", false, false, false)).unwrap();
        assert_eq!(key, KeyCode::F(1));
        let (key, _) = from_gpui_keystroke(&keystroke("f12", false, false, false)).unwrap();
        assert_eq!(key, KeyCode::F(12));
    }

    #[test]
    fn shift_tab_becomes_backtab_with_shift_cleared() {
        let (key, modifiers) = from_gpui_keystroke(&keystroke("tab", false, true, false)).unwrap();
        assert_eq!(key, KeyCode::BackTab);
        assert!(!modifiers.contains(KeyModifiers::SHIFT));
    }

    #[test]
    fn ctrl_shift_tab_keeps_control_on_backtab() {
        let (key, modifiers) = from_gpui_keystroke(&keystroke("tab", true, true, false)).unwrap();
        assert_eq!(key, KeyCode::BackTab);
        assert!(modifiers.contains(KeyModifiers::CONTROL));
        assert!(!modifiers.contains(KeyModifiers::SHIFT));
    }

    #[test]
    fn unrecognized_keys_with_no_key_char_pass_through_as_a_character() {
        let (key, _) = from_gpui_keystroke(&keystroke("g", false, false, false)).unwrap();
        assert_eq!(key, KeyCode::Char('g'));
    }

    #[test]
    fn a_key_char_is_preferred_over_the_key_name() {
        let mut stroke = keystroke(";", false, true, false);
        stroke.key_char = Some(":".to_string());
        let (key, _) = from_gpui_keystroke(&stroke).unwrap();
        assert_eq!(key, KeyCode::Char(':'));
    }

    #[test]
    fn modifiers_carry_control_shift_and_alt() {
        let (_, modifiers) = from_gpui_keystroke(&keystroke("d", true, false, true)).unwrap();
        assert!(modifiers.contains(KeyModifiers::CONTROL));
        assert!(modifiers.contains(KeyModifiers::ALT));
        assert!(!modifiers.contains(KeyModifiers::SHIFT));
    }
}
