use gpui_kit::gpui::Keystroke;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuiKey {
    Character(String),
    Named(NamedKey),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamedKey {
    ArrowUp,
    ArrowDown,
    PageUp,
    PageDown,
    Tab,
    Escape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GuiModifiers {
    pub control: bool,
    pub shift: bool,
}

/// Converts gpui's raw keystroke into the small neutral vocabulary
/// `update.rs`'s key mapping understands, so that module never needs to
/// depend on gpui types directly.
pub fn from_gpui_keystroke(keystroke: &Keystroke) -> (GuiKey, GuiModifiers) {
    let key = match keystroke.key.as_str() {
        "up" => GuiKey::Named(NamedKey::ArrowUp),
        "down" => GuiKey::Named(NamedKey::ArrowDown),
        "pageup" => GuiKey::Named(NamedKey::PageUp),
        "pagedown" => GuiKey::Named(NamedKey::PageDown),
        "tab" => GuiKey::Named(NamedKey::Tab),
        "escape" => GuiKey::Named(NamedKey::Escape),
        other => GuiKey::Character(other.to_string()),
    };
    let modifiers = GuiModifiers {
        control: keystroke.modifiers.control,
        shift: keystroke.modifiers.shift,
    };
    (key, modifiers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::gpui::Modifiers;

    fn keystroke(key: &str, control: bool, shift: bool) -> Keystroke {
        Keystroke {
            modifiers: Modifiers {
                control,
                shift,
                ..Modifiers::default()
            },
            key: key.to_string(),
            key_char: None,
        }
    }

    #[test]
    fn named_keys_map_to_their_variant() {
        let cases = [
            ("up", NamedKey::ArrowUp),
            ("down", NamedKey::ArrowDown),
            ("pageup", NamedKey::PageUp),
            ("pagedown", NamedKey::PageDown),
            ("tab", NamedKey::Tab),
            ("escape", NamedKey::Escape),
        ];
        for (raw, expected) in cases {
            let (key, _) = from_gpui_keystroke(&keystroke(raw, false, false));
            assert_eq!(key, GuiKey::Named(expected), "{raw}");
        }
    }

    #[test]
    fn unrecognized_keys_pass_through_as_characters() {
        let (key, _) = from_gpui_keystroke(&keystroke("g", false, false));
        assert_eq!(key, GuiKey::Character("g".to_string()));
    }

    #[test]
    fn modifiers_carry_control_and_shift_only() {
        let (_, modifiers) = from_gpui_keystroke(&keystroke("d", true, false));
        assert_eq!(
            modifiers,
            GuiModifiers {
                control: true,
                shift: false
            }
        );
    }
}
