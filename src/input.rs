use std::ops::{BitOr, BitOrAssign};

/// A keyboard key, independent of which input backend produced it —
/// crossterm for the TUI, gpui for the GUI. Covers exactly the variants
/// any `Mode::handle_key` implementation or `Keybindings` entry matches
/// on or can be bound to; see `src/config/keybindings.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyCode {
    Backspace,
    BackTab,
    Char(char),
    Delete,
    Down,
    End,
    Enter,
    Esc,
    F(u8),
    Home,
    Insert,
    Left,
    /// No key — used by tests that dispatch a `KeyResult` without a real
    /// originating keypress.
    Null,
    PageDown,
    PageUp,
    Right,
    Tab,
    Up,
}

/// Modifier keys held during a keystroke, independent of input backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyModifiers(u8);

impl KeyModifiers {
    pub const NONE: Self = Self(0);
    pub const SHIFT: Self = Self(0b001);
    pub const CONTROL: Self = Self(0b010);
    pub const ALT: Self = Self(0b100);

    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl Default for KeyModifiers {
    fn default() -> Self {
        Self::NONE
    }
}

impl BitOr for KeyModifiers {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for KeyModifiers {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_has_no_modifiers() {
        assert!(KeyModifiers::NONE.is_empty());
        assert!(!KeyModifiers::NONE.contains(KeyModifiers::CONTROL));
    }

    #[test]
    fn modifiers_compose_with_bitor() {
        let combined = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
        assert!(combined.contains(KeyModifiers::CONTROL));
        assert!(combined.contains(KeyModifiers::SHIFT));
        assert!(!combined.contains(KeyModifiers::ALT));
        assert!(!combined.is_empty());
    }

    #[test]
    fn bitor_assign_adds_a_modifier() {
        let mut modifiers = KeyModifiers::CONTROL;
        modifiers |= KeyModifiers::ALT;
        assert!(modifiers.contains(KeyModifiers::CONTROL));
        assert!(modifiers.contains(KeyModifiers::ALT));
    }

    #[test]
    fn contains_requires_every_bit_in_other() {
        let combined = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
        assert!(!KeyModifiers::CONTROL.contains(combined));
    }
}
