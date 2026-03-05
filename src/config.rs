//! Configuration loaded from `~/.config/logana/config.json`.
//!
//! Keybinding string format:
//!   `"j"`, `"Ctrl+d"`, `"Shift+Tab"`, `"Tab"`, `"PageDown"`, `"Space"`, `"Esc"`, …
//!
//! Example config file:
//! ```json
//! {
//!   "theme": "dracula",
//!   "keybindings": {
//!     "normal": { "scroll_down": ["j", "Down"], "half_page_down": "Ctrl+d" },
//!     "global": { "quit": "q" }
//!   }
//! }
//! ```

use crossterm::event::{KeyCode, KeyModifiers};
use serde::{Deserialize, Serialize, de};
use std::fmt;

// ---------------------------------------------------------------------------
// KeyBinding
// ---------------------------------------------------------------------------

/// A single keybinding: `(KeyCode, KeyModifiers)` serialised/deserialised
/// as a human-readable string such as `"Ctrl+d"` or `"Shift+Tab"`.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyBinding(pub KeyCode, pub KeyModifiers);

impl KeyBinding {
    /// Returns `true` when this binding matches the given key event.
    ///
    /// Modifier semantics:
    /// - No `CONTROL`/`ALT` in binding → accepts both `NONE` and `SHIFT`
    ///   (terminals may report uppercase letters either way).
    /// - `CONTROL`/`ALT` present in binding → those bits must be set in
    ///   `modifiers`.
    pub fn matches(&self, key: KeyCode, modifiers: KeyModifiers) -> bool {
        if self.0 != key {
            return false;
        }
        let has_ctrl = self.1.contains(KeyModifiers::CONTROL);
        let has_alt = self.1.contains(KeyModifiers::ALT);
        let has_shift = self.1.contains(KeyModifiers::SHIFT);

        if !has_ctrl && !has_alt {
            return match self.0 {
                // For character keys, terminals may report the SHIFT modifier either
                // way for uppercase letters (e.g. 'G' arrives as both NONE and SHIFT
                // depending on the terminal). Accept both, reject CTRL/ALT.
                KeyCode::Char(_) => {
                    !modifiers.contains(KeyModifiers::CONTROL)
                        && !modifiers.contains(KeyModifiers::ALT)
                }
                // For non-character keys (Enter, Tab, F-keys …) SHIFT changes the
                // key meaning, so require an exact SHIFT match.
                _ => {
                    let shift_ok = has_shift == modifiers.contains(KeyModifiers::SHIFT);
                    shift_ok
                        && !modifiers.contains(KeyModifiers::CONTROL)
                        && !modifiers.contains(KeyModifiers::ALT)
                }
            };
        }
        let ctrl_ok = !has_ctrl || modifiers.contains(KeyModifiers::CONTROL);
        let alt_ok = !has_alt || modifiers.contains(KeyModifiers::ALT);
        ctrl_ok && alt_ok
    }

    fn parse(s: &str) -> Result<Self, String> {
        // "Shift+Tab" is a special alias for BackTab (no modifier stored).
        if s.eq_ignore_ascii_case("shift+tab") {
            return Ok(KeyBinding(KeyCode::BackTab, KeyModifiers::NONE));
        }

        let mut mods = KeyModifiers::NONE;
        let mut rest = s;
        loop {
            if let Some(r) = rest
                .strip_prefix("Ctrl+")
                .or_else(|| rest.strip_prefix("ctrl+"))
            {
                mods |= KeyModifiers::CONTROL;
                rest = r;
            } else if let Some(r) = rest
                .strip_prefix("Alt+")
                .or_else(|| rest.strip_prefix("alt+"))
            {
                mods |= KeyModifiers::ALT;
                rest = r;
            } else if let Some(r) = rest
                .strip_prefix("Shift+")
                .or_else(|| rest.strip_prefix("shift+"))
            {
                mods |= KeyModifiers::SHIFT;
                rest = r;
            } else {
                break;
            }
        }

        let key = match rest {
            "Tab" | "tab" => KeyCode::Tab,
            "PageDown" | "pagedown" => KeyCode::PageDown,
            "PageUp" | "pageup" => KeyCode::PageUp,
            "Space" | "space" => KeyCode::Char(' '),
            "Esc" | "esc" => KeyCode::Esc,
            "Up" | "up" => KeyCode::Up,
            "Down" | "down" => KeyCode::Down,
            "Left" | "left" => KeyCode::Left,
            "Right" | "right" => KeyCode::Right,
            "Enter" | "enter" => KeyCode::Enter,
            "Backspace" | "backspace" => KeyCode::Backspace,
            "Delete" | "delete" => KeyCode::Delete,
            "Home" | "home" => KeyCode::Home,
            "End" | "end" => KeyCode::End,
            "Insert" | "insert" => KeyCode::Insert,
            s if s.chars().count() == 1 => KeyCode::Char(s.chars().next().unwrap()),
            // F-keys: "F1".."F12"
            s if s.starts_with('F') || s.starts_with('f') => {
                let n: u8 = s[1..]
                    .parse()
                    .map_err(|_| format!("Unknown key: {:?}", s))?;
                KeyCode::F(n)
            }
            other => return Err(format!("Unknown key: {:?}", other)),
        };

        Ok(KeyBinding(key, mods))
    }
}

impl KeyBinding {
    /// Human-readable string for display (e.g. `"Ctrl+d"`, `"Shift+Tab"`).
    pub fn display(&self) -> String {
        if self.0 == KeyCode::BackTab {
            return "Shift+Tab".to_string();
        }
        let mut s = String::new();
        if self.1.contains(KeyModifiers::CONTROL) {
            s.push_str("Ctrl+");
        }
        if self.1.contains(KeyModifiers::ALT) {
            s.push_str("Alt+");
        }
        if self.1.contains(KeyModifiers::SHIFT) {
            s.push_str("Shift+");
        }
        match self.0 {
            KeyCode::Tab => s.push_str("Tab"),
            KeyCode::PageDown => s.push_str("PageDown"),
            KeyCode::PageUp => s.push_str("PageUp"),
            KeyCode::Char(' ') => s.push_str("Space"),
            KeyCode::Esc => s.push_str("Esc"),
            KeyCode::Up => s.push_str("Up"),
            KeyCode::Down => s.push_str("Down"),
            KeyCode::Left => s.push_str("Left"),
            KeyCode::Right => s.push_str("Right"),
            KeyCode::Enter => s.push_str("Enter"),
            KeyCode::Backspace => s.push_str("Backspace"),
            KeyCode::Delete => s.push_str("Delete"),
            KeyCode::Home => s.push_str("Home"),
            KeyCode::End => s.push_str("End"),
            KeyCode::Insert => s.push_str("Insert"),
            KeyCode::Char(c) => s.push(c),
            KeyCode::F(n) => s.push_str(&format!("F{}", n)),
            _ => s.push('?'),
        }
        s
    }
}

impl Serialize for KeyBinding {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.display())
    }
}

impl<'de> Deserialize<'de> for KeyBinding {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        KeyBinding::parse(&s).map_err(de::Error::custom)
    }
}

// ---------------------------------------------------------------------------
// KeyBindings  (Vec<KeyBinding> with single-string OR array JSON support)
// ---------------------------------------------------------------------------

/// One or more alternative keybindings for a single action.
///
/// In JSON either a single string or an array is accepted:
/// ```json
/// "scroll_down": "j"
/// "scroll_down": ["j", "Down"]
/// ```
#[derive(Debug, Clone, PartialEq, Default)]
pub struct KeyBindings(pub Vec<KeyBinding>);

impl KeyBindings {
    pub fn matches(&self, key: KeyCode, mods: KeyModifiers) -> bool {
        self.0.iter().any(|b| b.matches(key, mods))
    }

    /// Human-readable string joining all alternatives with `/` (e.g. `"j/Down"`).
    pub fn display(&self) -> String {
        self.0
            .iter()
            .map(|b| b.display())
            .collect::<Vec<_>>()
            .join("/")
    }
}

impl Serialize for KeyBindings {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0.as_slice() {
            [single] => single.serialize(serializer),
            many => many.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for KeyBindings {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl<'de> de::Visitor<'de> for Visitor {
            type Value = KeyBindings;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a key string or array of key strings")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<KeyBindings, E> {
                KeyBinding::parse(v)
                    .map(|b| KeyBindings(vec![b]))
                    .map_err(E::custom)
            }

            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<KeyBindings, A::Error> {
                let mut bindings = Vec::new();
                while let Some(s) = seq.next_element::<String>()? {
                    let b = KeyBinding::parse(&s).map_err(de::Error::custom)?;
                    bindings.push(b);
                }
                Ok(KeyBindings(bindings))
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

// ---------------------------------------------------------------------------
// NormalKeybindings — defaults
// ---------------------------------------------------------------------------

fn default_scroll_down() -> KeyBindings {
    KeyBindings(vec![
        KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE),
        KeyBinding(KeyCode::Down, KeyModifiers::NONE),
    ])
}
fn default_scroll_up() -> KeyBindings {
    KeyBindings(vec![
        KeyBinding(KeyCode::Char('k'), KeyModifiers::NONE),
        KeyBinding(KeyCode::Up, KeyModifiers::NONE),
    ])
}
fn default_scroll_left() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('h'), KeyModifiers::NONE)])
}
fn default_scroll_right() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('l'), KeyModifiers::NONE)])
}
fn default_half_page_down() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('d'), KeyModifiers::CONTROL)])
}
fn default_half_page_up() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('u'), KeyModifiers::CONTROL)])
}
fn default_page_down() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::PageDown, KeyModifiers::NONE)])
}
fn default_page_up() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::PageUp, KeyModifiers::NONE)])
}
fn default_command_mode() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char(':'), KeyModifiers::NONE)])
}
fn default_filter_mode_key() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('f'), KeyModifiers::NONE)])
}
fn default_toggle_filtering() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('F'), KeyModifiers::NONE)])
}
fn default_toggle_sidebar() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('s'), KeyModifiers::NONE)])
}
fn default_toggle_mode_bar() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('b'), KeyModifiers::NONE)])
}
fn default_toggle_borders() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('B'), KeyModifiers::NONE)])
}
fn default_go_to_top_chord() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('g'), KeyModifiers::NONE)])
}
fn default_go_to_bottom() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('G'), KeyModifiers::NONE)])
}
fn default_mark_line() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('m'), KeyModifiers::NONE)])
}
fn default_search_forward() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('/'), KeyModifiers::NONE)])
}
fn default_search_backward() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('?'), KeyModifiers::NONE)])
}
fn default_next_match() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('n'), KeyModifiers::NONE)])
}
fn default_prev_match() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('N'), KeyModifiers::NONE)])
}
fn default_toggle_wrap() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('w'), KeyModifiers::NONE)])
}
fn default_visual_mode() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('V'), KeyModifiers::NONE)])
}
fn default_toggle_marks_only() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('M'), KeyModifiers::NONE)])
}
fn default_yank_line() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('y'), KeyModifiers::NONE)])
}
fn default_yank_marked() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('Y'), KeyModifiers::NONE)])
}
fn default_show_keybindings() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::F(1), KeyModifiers::NONE)])
}
fn default_clear_all() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('C'), KeyModifiers::NONE)])
}
fn default_edit_comment() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('c'), KeyModifiers::NONE)])
}
fn default_normal_filter_include() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('i'), KeyModifiers::NONE)])
}
fn default_normal_filter_exclude() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('o'), KeyModifiers::NONE)])
}
fn default_enter_ui_mode() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('u'), KeyModifiers::NONE)])
}
fn default_ui_exit() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Esc, KeyModifiers::NONE)])
}
fn default_clear_search() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Esc, KeyModifiers::NONE)])
}

// ---------------------------------------------------------------------------
// NavigationKeybindings — shared across all modes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NavigationKeybindings {
    #[serde(default = "default_scroll_down")]
    pub scroll_down: KeyBindings,
    #[serde(default = "default_scroll_up")]
    pub scroll_up: KeyBindings,
    #[serde(default = "default_half_page_down")]
    pub half_page_down: KeyBindings,
    #[serde(default = "default_half_page_up")]
    pub half_page_up: KeyBindings,
    #[serde(default = "default_page_down")]
    pub page_down: KeyBindings,
    #[serde(default = "default_page_up")]
    pub page_up: KeyBindings,
}

impl Default for NavigationKeybindings {
    fn default() -> Self {
        Self {
            scroll_down: default_scroll_down(),
            scroll_up: default_scroll_up(),
            half_page_down: default_half_page_down(),
            half_page_up: default_half_page_up(),
            page_down: default_page_down(),
            page_up: default_page_up(),
        }
    }
}

// ---------------------------------------------------------------------------
// NormalKeybindings
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalKeybindings {
    #[serde(default = "default_scroll_left")]
    pub scroll_left: KeyBindings,
    #[serde(default = "default_scroll_right")]
    pub scroll_right: KeyBindings,
    #[serde(default = "default_command_mode")]
    pub command_mode: KeyBindings,
    #[serde(default = "default_filter_mode_key")]
    pub filter_mode: KeyBindings,
    #[serde(default = "default_toggle_filtering")]
    pub toggle_filtering: KeyBindings,
    #[serde(default = "default_go_to_top_chord")]
    pub go_to_top_chord: KeyBindings,
    #[serde(default = "default_go_to_bottom")]
    pub go_to_bottom: KeyBindings,
    #[serde(default = "default_mark_line")]
    pub mark_line: KeyBindings,
    #[serde(default = "default_search_forward")]
    pub search_forward: KeyBindings,
    #[serde(default = "default_search_backward")]
    pub search_backward: KeyBindings,
    #[serde(default = "default_next_match")]
    pub next_match: KeyBindings,
    #[serde(default = "default_prev_match")]
    pub prev_match: KeyBindings,
    #[serde(default = "default_visual_mode")]
    pub visual_mode: KeyBindings,
    #[serde(default = "default_toggle_marks_only")]
    pub toggle_marks_only: KeyBindings,
    #[serde(default = "default_yank_line")]
    pub yank_line: KeyBindings,
    #[serde(default = "default_yank_marked")]
    pub yank_marked: KeyBindings,
    #[serde(default = "default_show_keybindings")]
    pub show_keybindings: KeyBindings,
    #[serde(default = "default_clear_all")]
    pub clear_all: KeyBindings,
    #[serde(default = "default_edit_comment")]
    pub edit_comment: KeyBindings,
    #[serde(default = "default_normal_filter_include")]
    pub filter_include: KeyBindings,
    #[serde(default = "default_normal_filter_exclude")]
    pub filter_exclude: KeyBindings,
    #[serde(default = "default_enter_ui_mode")]
    pub enter_ui_mode: KeyBindings,
    #[serde(default = "default_clear_search")]
    pub clear_search: KeyBindings,
}

impl Default for NormalKeybindings {
    fn default() -> Self {
        Self {
            scroll_left: default_scroll_left(),
            scroll_right: default_scroll_right(),
            command_mode: default_command_mode(),
            filter_mode: default_filter_mode_key(),
            toggle_filtering: default_toggle_filtering(),
            go_to_top_chord: default_go_to_top_chord(),
            go_to_bottom: default_go_to_bottom(),
            mark_line: default_mark_line(),
            search_forward: default_search_forward(),
            search_backward: default_search_backward(),
            next_match: default_next_match(),
            prev_match: default_prev_match(),
            visual_mode: default_visual_mode(),
            toggle_marks_only: default_toggle_marks_only(),
            yank_line: default_yank_line(),
            yank_marked: default_yank_marked(),
            show_keybindings: default_show_keybindings(),
            clear_all: default_clear_all(),
            edit_comment: default_edit_comment(),
            filter_include: default_normal_filter_include(),
            filter_exclude: default_normal_filter_exclude(),
            enter_ui_mode: default_enter_ui_mode(),
            clear_search: default_clear_search(),
        }
    }
}

// ---------------------------------------------------------------------------
// FilterKeybindings — defaults
// ---------------------------------------------------------------------------

fn default_filter_toggle() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char(' '), KeyModifiers::NONE)])
}
fn default_filter_delete() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('d'), KeyModifiers::NONE)])
}
fn default_filter_move_up() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('K'), KeyModifiers::NONE)])
}
fn default_filter_move_down() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('J'), KeyModifiers::NONE)])
}
fn default_filter_edit() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('e'), KeyModifiers::NONE)])
}
fn default_filter_set_color() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('c'), KeyModifiers::NONE)])
}
fn default_filter_toggle_all() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('A'), KeyModifiers::NONE)])
}
fn default_filter_clear_all() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('C'), KeyModifiers::NONE)])
}
fn default_filter_add_date() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('t'), KeyModifiers::NONE)])
}
fn default_filter_exit() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Esc, KeyModifiers::NONE)])
}

// ---------------------------------------------------------------------------
// FilterKeybindings
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterKeybindings {
    #[serde(default = "default_filter_toggle")]
    pub toggle_filter: KeyBindings,
    #[serde(default = "default_filter_delete")]
    pub delete_filter: KeyBindings,
    #[serde(default = "default_filter_move_up")]
    pub move_filter_up: KeyBindings,
    #[serde(default = "default_filter_move_down")]
    pub move_filter_down: KeyBindings,
    #[serde(default = "default_filter_edit")]
    pub edit_filter: KeyBindings,
    #[serde(default = "default_filter_set_color")]
    pub set_color: KeyBindings,
    #[serde(default = "default_filter_toggle_all")]
    pub toggle_all_filters: KeyBindings,
    #[serde(default = "default_filter_clear_all")]
    pub clear_all_filters: KeyBindings,
    #[serde(default = "default_filter_add_date")]
    pub add_date_filter: KeyBindings,
    #[serde(default = "default_filter_exit")]
    pub exit_mode: KeyBindings,
}

impl Default for FilterKeybindings {
    fn default() -> Self {
        Self {
            toggle_filter: default_filter_toggle(),
            delete_filter: default_filter_delete(),
            move_filter_up: default_filter_move_up(),
            move_filter_down: default_filter_move_down(),
            edit_filter: default_filter_edit(),
            set_color: default_filter_set_color(),
            toggle_all_filters: default_filter_toggle_all(),
            clear_all_filters: default_filter_clear_all(),
            add_date_filter: default_filter_add_date(),
            exit_mode: default_filter_exit(),
        }
    }
}

// ---------------------------------------------------------------------------
// GlobalKeybindings — defaults
// ---------------------------------------------------------------------------

fn default_quit() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('q'), KeyModifiers::NONE)])
}
fn default_next_tab() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Tab, KeyModifiers::NONE)])
}
fn default_prev_tab() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::BackTab, KeyModifiers::NONE)])
}
fn default_close_tab() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('w'), KeyModifiers::CONTROL)])
}
fn default_new_tab() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('t'), KeyModifiers::CONTROL)])
}

// ---------------------------------------------------------------------------
// GlobalKeybindings
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalKeybindings {
    #[serde(default = "default_quit")]
    pub quit: KeyBindings,
    #[serde(default = "default_next_tab")]
    pub next_tab: KeyBindings,
    #[serde(default = "default_prev_tab")]
    pub prev_tab: KeyBindings,
    #[serde(default = "default_close_tab")]
    pub close_tab: KeyBindings,
    #[serde(default = "default_new_tab")]
    pub new_tab: KeyBindings,
}

impl Default for GlobalKeybindings {
    fn default() -> Self {
        Self {
            quit: default_quit(),
            next_tab: default_next_tab(),
            prev_tab: default_prev_tab(),
            close_tab: default_close_tab(),
            new_tab: default_new_tab(),
        }
    }
}

// ---------------------------------------------------------------------------
// CommentKeybindings — defaults
// ---------------------------------------------------------------------------

fn default_comment_save() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('s'), KeyModifiers::CONTROL)])
}
fn default_comment_cancel() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Esc, KeyModifiers::NONE)])
}
fn default_comment_delete() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('d'), KeyModifiers::CONTROL)])
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommentKeybindings {
    /// Key to save the comment and return to Normal mode.
    #[serde(default = "default_comment_save")]
    pub save: KeyBindings,
    /// Key to cancel the comment and return to Normal mode.
    #[serde(default = "default_comment_cancel")]
    pub cancel: KeyBindings,
    /// Key to delete the comment being edited (only in edit mode).
    #[serde(default = "default_comment_delete")]
    pub delete: KeyBindings,
}

impl Default for CommentKeybindings {
    fn default() -> Self {
        Self {
            save: default_comment_save(),
            cancel: default_comment_cancel(),
            delete: default_comment_delete(),
        }
    }
}

// ---------------------------------------------------------------------------
// VisualLineKeybindings — defaults
// ---------------------------------------------------------------------------

fn default_visual_comment() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('c'), KeyModifiers::NONE)])
}
fn default_visual_yank() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('y'), KeyModifiers::NONE)])
}
fn default_visual_mark() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('m'), KeyModifiers::NONE)])
}
fn default_visual_exit() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Esc, KeyModifiers::NONE)])
}

// ---------------------------------------------------------------------------
// VisualLineKeybindings
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualLineKeybindings {
    #[serde(default = "default_visual_comment")]
    pub comment: KeyBindings,
    #[serde(default = "default_visual_yank")]
    pub yank: KeyBindings,
    #[serde(default = "default_visual_mark")]
    pub mark: KeyBindings,
    #[serde(default = "default_visual_exit")]
    pub exit: KeyBindings,
}

impl Default for VisualLineKeybindings {
    fn default() -> Self {
        Self {
            comment: default_visual_comment(),
            yank: default_visual_yank(),
            mark: default_visual_mark(),
            exit: default_visual_exit(),
        }
    }
}

// ---------------------------------------------------------------------------
// SearchKeybindings — defaults
// ---------------------------------------------------------------------------

fn default_search_cancel() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Esc, KeyModifiers::NONE)])
}
fn default_search_confirm() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Enter, KeyModifiers::NONE)])
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchKeybindings {
    #[serde(default = "default_search_cancel")]
    pub cancel: KeyBindings,
    #[serde(default = "default_search_confirm")]
    pub confirm: KeyBindings,
}

impl Default for SearchKeybindings {
    fn default() -> Self {
        Self {
            cancel: default_search_cancel(),
            confirm: default_search_confirm(),
        }
    }
}

// ---------------------------------------------------------------------------
// FilterEditKeybindings — defaults
// ---------------------------------------------------------------------------

fn default_filter_edit_cancel() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Esc, KeyModifiers::NONE)])
}
fn default_filter_edit_confirm() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Enter, KeyModifiers::NONE)])
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterEditKeybindings {
    #[serde(default = "default_filter_edit_cancel")]
    pub cancel: KeyBindings,
    #[serde(default = "default_filter_edit_confirm")]
    pub confirm: KeyBindings,
}

impl Default for FilterEditKeybindings {
    fn default() -> Self {
        Self {
            cancel: default_filter_edit_cancel(),
            confirm: default_filter_edit_confirm(),
        }
    }
}

// ---------------------------------------------------------------------------
// CommandModeKeybindings — defaults
// ---------------------------------------------------------------------------

fn default_command_cancel() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Esc, KeyModifiers::NONE)])
}
fn default_command_confirm() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Enter, KeyModifiers::NONE)])
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandModeKeybindings {
    #[serde(default = "default_command_cancel")]
    pub cancel: KeyBindings,
    #[serde(default = "default_command_confirm")]
    pub confirm: KeyBindings,
}

impl Default for CommandModeKeybindings {
    fn default() -> Self {
        Self {
            cancel: default_command_cancel(),
            confirm: default_command_confirm(),
        }
    }
}

// ---------------------------------------------------------------------------
// DockerSelectKeybindings — defaults
// ---------------------------------------------------------------------------

fn default_docker_confirm() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Enter, KeyModifiers::NONE)])
}
fn default_docker_cancel() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Esc, KeyModifiers::NONE)])
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerSelectKeybindings {
    #[serde(default = "default_docker_confirm")]
    pub confirm: KeyBindings,
    #[serde(default = "default_docker_cancel")]
    pub cancel: KeyBindings,
}

impl Default for DockerSelectKeybindings {
    fn default() -> Self {
        Self {
            confirm: default_docker_confirm(),
            cancel: default_docker_cancel(),
        }
    }
}

// ---------------------------------------------------------------------------
// ValueColorsKeybindings — defaults
// ---------------------------------------------------------------------------

fn default_vc_toggle() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char(' '), KeyModifiers::NONE)])
}
fn default_vc_all() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('a'), KeyModifiers::NONE)])
}
fn default_vc_none() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('n'), KeyModifiers::NONE)])
}
fn default_vc_apply() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Enter, KeyModifiers::NONE)])
}
fn default_vc_cancel() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Esc, KeyModifiers::NONE)])
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValueColorsKeybindings {
    #[serde(default = "default_vc_toggle")]
    pub toggle: KeyBindings,
    #[serde(default = "default_vc_all")]
    pub all: KeyBindings,
    #[serde(default = "default_vc_none")]
    pub none: KeyBindings,
    #[serde(default = "default_vc_apply")]
    pub apply: KeyBindings,
    #[serde(default = "default_vc_cancel")]
    pub cancel: KeyBindings,
}

impl Default for ValueColorsKeybindings {
    fn default() -> Self {
        Self {
            toggle: default_vc_toggle(),
            all: default_vc_all(),
            none: default_vc_none(),
            apply: default_vc_apply(),
            cancel: default_vc_cancel(),
        }
    }
}

// ---------------------------------------------------------------------------
// SelectFieldsKeybindings — defaults
// ---------------------------------------------------------------------------

fn default_sf_toggle() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char(' '), KeyModifiers::NONE)])
}
fn default_sf_move_up() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('K'), KeyModifiers::NONE)])
}
fn default_sf_move_down() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('J'), KeyModifiers::NONE)])
}
fn default_sf_all() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('a'), KeyModifiers::NONE)])
}
fn default_sf_none() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('n'), KeyModifiers::NONE)])
}
fn default_sf_apply() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Enter, KeyModifiers::NONE)])
}
fn default_sf_cancel() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Esc, KeyModifiers::NONE)])
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectFieldsKeybindings {
    #[serde(default = "default_sf_toggle")]
    pub toggle: KeyBindings,
    #[serde(default = "default_sf_move_up")]
    pub move_up: KeyBindings,
    #[serde(default = "default_sf_move_down")]
    pub move_down: KeyBindings,
    #[serde(default = "default_sf_all")]
    pub all: KeyBindings,
    #[serde(default = "default_sf_none")]
    pub none: KeyBindings,
    #[serde(default = "default_sf_apply")]
    pub apply: KeyBindings,
    #[serde(default = "default_sf_cancel")]
    pub cancel: KeyBindings,
}

impl Default for SelectFieldsKeybindings {
    fn default() -> Self {
        Self {
            toggle: default_sf_toggle(),
            move_up: default_sf_move_up(),
            move_down: default_sf_move_down(),
            all: default_sf_all(),
            none: default_sf_none(),
            apply: default_sf_apply(),
            cancel: default_sf_cancel(),
        }
    }
}

// ---------------------------------------------------------------------------
// HelpKeybindings — defaults
// ---------------------------------------------------------------------------

fn default_help_close() -> KeyBindings {
    KeyBindings(vec![
        KeyBinding(KeyCode::Char('q'), KeyModifiers::NONE),
        KeyBinding(KeyCode::Esc, KeyModifiers::NONE),
    ])
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelpKeybindings {
    #[serde(default = "default_help_close")]
    pub close: KeyBindings,
}

impl Default for HelpKeybindings {
    fn default() -> Self {
        Self {
            close: default_help_close(),
        }
    }
}

// ---------------------------------------------------------------------------
// ConfirmKeybindings — defaults
// ---------------------------------------------------------------------------

fn default_confirm_yes() -> KeyBindings {
    KeyBindings(vec![
        KeyBinding(KeyCode::Char('y'), KeyModifiers::NONE),
        KeyBinding(KeyCode::Enter, KeyModifiers::NONE),
    ])
}
fn default_confirm_no() -> KeyBindings {
    KeyBindings(vec![
        KeyBinding(KeyCode::Char('n'), KeyModifiers::NONE),
        KeyBinding(KeyCode::Esc, KeyModifiers::NONE),
    ])
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfirmKeybindings {
    #[serde(default = "default_confirm_yes")]
    pub yes: KeyBindings,
    #[serde(default = "default_confirm_no")]
    pub no: KeyBindings,
}

impl Default for ConfirmKeybindings {
    fn default() -> Self {
        Self {
            yes: default_confirm_yes(),
            no: default_confirm_no(),
        }
    }
}

// ---------------------------------------------------------------------------
// UiKeybindings — UI display toggles accessible from UiMode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiKeybindings {
    #[serde(default = "default_toggle_sidebar")]
    pub toggle_sidebar: KeyBindings,
    #[serde(default = "default_toggle_mode_bar")]
    pub toggle_mode_bar: KeyBindings,
    #[serde(default = "default_toggle_borders")]
    pub toggle_borders: KeyBindings,
    #[serde(default = "default_toggle_wrap")]
    pub toggle_wrap: KeyBindings,
    #[serde(default = "default_ui_exit")]
    pub exit: KeyBindings,
}

impl Default for UiKeybindings {
    fn default() -> Self {
        Self {
            toggle_sidebar: default_toggle_sidebar(),
            toggle_mode_bar: default_toggle_mode_bar(),
            toggle_borders: default_toggle_borders(),
            toggle_wrap: default_toggle_wrap(),
            exit: default_ui_exit(),
        }
    }
}

// ---------------------------------------------------------------------------
// Keybindings
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Keybindings {
    #[serde(default)]
    pub navigation: NavigationKeybindings,
    #[serde(default)]
    pub normal: NormalKeybindings,
    #[serde(default)]
    pub filter: FilterKeybindings,
    #[serde(default)]
    pub global: GlobalKeybindings,
    #[serde(default)]
    pub comment: CommentKeybindings,
    #[serde(default)]
    pub visual_line: VisualLineKeybindings,
    #[serde(default)]
    pub search: SearchKeybindings,
    #[serde(default)]
    pub filter_edit: FilterEditKeybindings,
    #[serde(default)]
    pub command: CommandModeKeybindings,
    #[serde(default)]
    pub docker_select: DockerSelectKeybindings,
    #[serde(default)]
    pub value_colors: ValueColorsKeybindings,
    #[serde(default)]
    pub select_fields: SelectFieldsKeybindings,
    #[serde(default)]
    pub help: HelpKeybindings,
    #[serde(default)]
    pub confirm: ConfirmKeybindings,
    #[serde(default)]
    pub ui: UiKeybindings,
}

impl KeyBindings {
    /// Returns true if this set and `other` share at least one identical binding.
    pub fn has_overlap(&self, other: &KeyBindings) -> bool {
        for a in &self.0 {
            for b in &other.0 {
                if a.0 == b.0 && a.1 == b.1 {
                    return true;
                }
            }
        }
        false
    }
}

impl Keybindings {
    /// Check all modes for duplicate keybindings and return a list of
    /// human-readable conflict descriptions.
    ///
    /// Two bindings conflict when they share the same key+modifier within the
    /// same mode scope (normal mode or filter mode). Global bindings are
    /// checked against themselves and against both mode scopes.
    pub fn validate(&self) -> Vec<String> {
        let mut conflicts = Vec::new();

        // Build (name, &KeyBindings) slices for each scope.
        // Navigation keys are shared; include them in every scope that uses them.
        let nav = &self.navigation;

        let normal_actions: &[(&str, &KeyBindings)] = &[
            ("navigation.scroll_down", &nav.scroll_down),
            ("navigation.scroll_up", &nav.scroll_up),
            ("navigation.half_page_down", &nav.half_page_down),
            ("navigation.half_page_up", &nav.half_page_up),
            ("navigation.page_down", &nav.page_down),
            ("navigation.page_up", &nav.page_up),
            ("normal.scroll_left", &self.normal.scroll_left),
            ("normal.scroll_right", &self.normal.scroll_right),
            ("normal.command_mode", &self.normal.command_mode),
            ("normal.filter_mode", &self.normal.filter_mode),
            ("normal.toggle_filtering", &self.normal.toggle_filtering),
            ("normal.enter_ui_mode", &self.normal.enter_ui_mode),
            ("normal.filter_include", &self.normal.filter_include),
            ("normal.filter_exclude", &self.normal.filter_exclude),
            ("normal.go_to_top_chord", &self.normal.go_to_top_chord),
            ("normal.go_to_bottom", &self.normal.go_to_bottom),
            ("normal.mark_line", &self.normal.mark_line),
            ("normal.toggle_marks_only", &self.normal.toggle_marks_only),
            ("normal.yank_line", &self.normal.yank_line),
            ("normal.yank_marked", &self.normal.yank_marked),
            ("normal.visual_mode", &self.normal.visual_mode),
            ("normal.search_forward", &self.normal.search_forward),
            ("normal.search_backward", &self.normal.search_backward),
            ("normal.next_match", &self.normal.next_match),
            ("normal.prev_match", &self.normal.prev_match),
            ("normal.show_keybindings", &self.normal.show_keybindings),
            ("normal.clear_all", &self.normal.clear_all),
            ("normal.edit_comment", &self.normal.edit_comment),
            ("global.quit", &self.global.quit),
            ("global.next_tab", &self.global.next_tab),
            ("global.prev_tab", &self.global.prev_tab),
            ("global.close_tab", &self.global.close_tab),
            ("global.new_tab", &self.global.new_tab),
        ];

        let filter_actions: &[(&str, &KeyBindings)] = &[
            ("navigation.scroll_down", &nav.scroll_down),
            ("navigation.scroll_up", &nav.scroll_up),
            ("filter.toggle_filter", &self.filter.toggle_filter),
            ("filter.delete_filter", &self.filter.delete_filter),
            ("filter.move_filter_up", &self.filter.move_filter_up),
            ("filter.move_filter_down", &self.filter.move_filter_down),
            ("filter.edit_filter", &self.filter.edit_filter),
            ("filter.set_color", &self.filter.set_color),
            ("filter.toggle_all_filters", &self.filter.toggle_all_filters),
            ("filter.clear_all_filters", &self.filter.clear_all_filters),
            ("filter.add_date_filter", &self.filter.add_date_filter),
            ("filter.exit_mode", &self.filter.exit_mode),
            ("global.quit", &self.global.quit),
            ("global.next_tab", &self.global.next_tab),
            ("global.prev_tab", &self.global.prev_tab),
        ];

        let visual_line_actions: &[(&str, &KeyBindings)] = &[
            ("navigation.scroll_down", &nav.scroll_down),
            ("navigation.scroll_up", &nav.scroll_up),
            ("visual_line.comment", &self.visual_line.comment),
            ("visual_line.yank", &self.visual_line.yank),
            ("visual_line.mark", &self.visual_line.mark),
            ("visual_line.exit", &self.visual_line.exit),
        ];

        let docker_select_actions: &[(&str, &KeyBindings)] = &[
            ("navigation.scroll_down", &nav.scroll_down),
            ("navigation.scroll_up", &nav.scroll_up),
            ("docker_select.confirm", &self.docker_select.confirm),
            ("docker_select.cancel", &self.docker_select.cancel),
        ];

        let value_colors_actions: &[(&str, &KeyBindings)] = &[
            ("navigation.scroll_down", &nav.scroll_down),
            ("navigation.scroll_up", &nav.scroll_up),
            ("value_colors.toggle", &self.value_colors.toggle),
            ("value_colors.all", &self.value_colors.all),
            ("value_colors.none", &self.value_colors.none),
            ("value_colors.apply", &self.value_colors.apply),
            ("value_colors.cancel", &self.value_colors.cancel),
        ];

        let select_fields_actions: &[(&str, &KeyBindings)] = &[
            ("navigation.scroll_down", &nav.scroll_down),
            ("navigation.scroll_up", &nav.scroll_up),
            ("select_fields.toggle", &self.select_fields.toggle),
            ("select_fields.move_up", &self.select_fields.move_up),
            ("select_fields.move_down", &self.select_fields.move_down),
            ("select_fields.all", &self.select_fields.all),
            ("select_fields.none", &self.select_fields.none),
            ("select_fields.apply", &self.select_fields.apply),
            ("select_fields.cancel", &self.select_fields.cancel),
        ];

        let help_actions: &[(&str, &KeyBindings)] = &[
            ("navigation.scroll_down", &nav.scroll_down),
            ("navigation.scroll_up", &nav.scroll_up),
            ("navigation.half_page_down", &nav.half_page_down),
            ("navigation.half_page_up", &nav.half_page_up),
            ("help.close", &self.help.close),
        ];

        let ui_actions: &[(&str, &KeyBindings)] = &[
            ("navigation.scroll_down", &nav.scroll_down),
            ("navigation.scroll_up", &nav.scroll_up),
            ("ui.toggle_sidebar", &self.ui.toggle_sidebar),
            ("ui.toggle_mode_bar", &self.ui.toggle_mode_bar),
            ("ui.toggle_borders", &self.ui.toggle_borders),
            ("ui.toggle_wrap", &self.ui.toggle_wrap),
            ("ui.exit", &self.ui.exit),
            ("global.quit", &self.global.quit),
            ("global.next_tab", &self.global.next_tab),
            ("global.prev_tab", &self.global.prev_tab),
        ];

        check_conflicts(normal_actions, &mut conflicts);
        check_conflicts(filter_actions, &mut conflicts);
        check_conflicts(visual_line_actions, &mut conflicts);
        check_conflicts(docker_select_actions, &mut conflicts);
        check_conflicts(value_colors_actions, &mut conflicts);
        check_conflicts(select_fields_actions, &mut conflicts);
        check_conflicts(help_actions, &mut conflicts);
        check_conflicts(ui_actions, &mut conflicts);

        conflicts
    }
}

/// Check all pairs in `actions` for overlapping key bindings and append
/// human-readable descriptions to `out`.
fn check_conflicts(actions: &[(&str, &KeyBindings)], out: &mut Vec<String>) {
    for i in 0..actions.len() {
        for j in (i + 1)..actions.len() {
            let (name_a, kb_a) = actions[i];
            let (name_b, kb_b) = actions[j];
            if kb_a.has_overlap(kb_b) {
                // Find the overlapping key string for the message.
                let key_str = kb_a
                    .0
                    .iter()
                    .find(|a| kb_b.0.iter().any(|b| a.0 == b.0 && a.1 == b.1))
                    .map(|b| b.display())
                    .unwrap_or_default();
                out.push(format!(
                    "keybinding conflict: '{}' and '{}' both use '{}'",
                    name_a, name_b, key_str
                ));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    /// Theme name (without `.json` extension) to load on startup.
    pub theme: Option<String>,
    #[serde(default)]
    pub keybindings: Keybindings,
    /// Whether to show the mode bar at the bottom (default: true).
    #[serde(default = "default_true")]
    pub show_mode_bar: bool,
    /// Whether to show panel borders (default: true).
    #[serde(default = "default_true")]
    pub show_borders: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: None,
            keybindings: Keybindings::default(),
            show_mode_bar: true,
            show_borders: true,
        }
    }
}

impl Config {
    /// Load configuration from `~/.config/logana/config.json`.
    ///
    /// This function is infallible — any I/O or parse error falls back to
    /// `Config::default()` so a bad config never prevents startup.
    pub fn load() -> Self {
        let Some(config_path) = dirs::config_dir().map(|d| d.join("logana").join("config.json"))
        else {
            return Config::default();
        };

        let Ok(contents) = std::fs::read_to_string(&config_path) else {
            return Config::default();
        };

        serde_json::from_str(&contents).unwrap_or_else(|e| {
            tracing::warn!("Failed to parse config file {:?}: {}", config_path, e);
            Config::default()
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};

    // ── KeyBinding::parse — basic keys ──────────────────────────────────

    #[test]
    fn test_parse_single_char() {
        let kb = KeyBinding::parse("j").unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE));
    }

    #[test]
    fn test_parse_uppercase_char() {
        let kb = KeyBinding::parse("G").unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::Char('G'), KeyModifiers::NONE));
    }

    #[test]
    fn test_parse_tab() {
        let kb = KeyBinding::parse("Tab").unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::Tab, KeyModifiers::NONE));
    }

    #[test]
    fn test_parse_page_down() {
        let kb = KeyBinding::parse("PageDown").unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::PageDown, KeyModifiers::NONE));
    }

    #[test]
    fn test_parse_page_up() {
        let kb = KeyBinding::parse("PageUp").unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::PageUp, KeyModifiers::NONE));
    }

    #[test]
    fn test_parse_space() {
        let kb = KeyBinding::parse("Space").unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::Char(' '), KeyModifiers::NONE));
    }

    #[test]
    fn test_parse_esc() {
        let kb = KeyBinding::parse("Esc").unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::Esc, KeyModifiers::NONE));
    }

    #[test]
    fn test_parse_arrow_keys() {
        assert_eq!(
            KeyBinding::parse("Up").unwrap(),
            KeyBinding(KeyCode::Up, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("Down").unwrap(),
            KeyBinding(KeyCode::Down, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("Left").unwrap(),
            KeyBinding(KeyCode::Left, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("Right").unwrap(),
            KeyBinding(KeyCode::Right, KeyModifiers::NONE)
        );
    }

    #[test]
    fn test_parse_enter_backspace_delete() {
        assert_eq!(
            KeyBinding::parse("Enter").unwrap(),
            KeyBinding(KeyCode::Enter, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("Backspace").unwrap(),
            KeyBinding(KeyCode::Backspace, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("Delete").unwrap(),
            KeyBinding(KeyCode::Delete, KeyModifiers::NONE)
        );
    }

    #[test]
    fn test_parse_home_end_insert() {
        assert_eq!(
            KeyBinding::parse("Home").unwrap(),
            KeyBinding(KeyCode::Home, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("End").unwrap(),
            KeyBinding(KeyCode::End, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("Insert").unwrap(),
            KeyBinding(KeyCode::Insert, KeyModifiers::NONE)
        );
    }

    #[test]
    fn test_parse_f_keys() {
        assert_eq!(
            KeyBinding::parse("F1").unwrap(),
            KeyBinding(KeyCode::F(1), KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("F12").unwrap(),
            KeyBinding(KeyCode::F(12), KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("f5").unwrap(),
            KeyBinding(KeyCode::F(5), KeyModifiers::NONE)
        );
    }

    #[test]
    fn test_parse_invalid_f_key() {
        assert!(KeyBinding::parse("Fxx").is_err());
    }

    // ── KeyBinding::parse — lowercase key names ─────────────────────────

    #[test]
    fn test_parse_lowercase_key_names() {
        assert_eq!(
            KeyBinding::parse("tab").unwrap(),
            KeyBinding(KeyCode::Tab, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("pagedown").unwrap(),
            KeyBinding(KeyCode::PageDown, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("pageup").unwrap(),
            KeyBinding(KeyCode::PageUp, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("space").unwrap(),
            KeyBinding(KeyCode::Char(' '), KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("esc").unwrap(),
            KeyBinding(KeyCode::Esc, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("up").unwrap(),
            KeyBinding(KeyCode::Up, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("down").unwrap(),
            KeyBinding(KeyCode::Down, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("left").unwrap(),
            KeyBinding(KeyCode::Left, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("right").unwrap(),
            KeyBinding(KeyCode::Right, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("enter").unwrap(),
            KeyBinding(KeyCode::Enter, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("backspace").unwrap(),
            KeyBinding(KeyCode::Backspace, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("delete").unwrap(),
            KeyBinding(KeyCode::Delete, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("home").unwrap(),
            KeyBinding(KeyCode::Home, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("end").unwrap(),
            KeyBinding(KeyCode::End, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("insert").unwrap(),
            KeyBinding(KeyCode::Insert, KeyModifiers::NONE)
        );
    }

    // ── KeyBinding::parse — modifiers ───────────────────────────────────

    #[test]
    fn test_parse_ctrl_prefix() {
        let kb = KeyBinding::parse("Ctrl+d").unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::Char('d'), KeyModifiers::CONTROL));
    }

    #[test]
    fn test_parse_ctrl_lowercase_prefix() {
        let kb = KeyBinding::parse("ctrl+d").unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::Char('d'), KeyModifiers::CONTROL));
    }

    #[test]
    fn test_parse_alt_prefix() {
        let kb = KeyBinding::parse("Alt+x").unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::Char('x'), KeyModifiers::ALT));
    }

    #[test]
    fn test_parse_alt_lowercase_prefix() {
        let kb = KeyBinding::parse("alt+x").unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::Char('x'), KeyModifiers::ALT));
    }

    #[test]
    fn test_parse_shift_prefix() {
        let kb = KeyBinding::parse("Shift+Enter").unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::Enter, KeyModifiers::SHIFT));
    }

    #[test]
    fn test_parse_shift_lowercase_prefix() {
        let kb = KeyBinding::parse("shift+Enter").unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::Enter, KeyModifiers::SHIFT));
    }

    #[test]
    fn test_parse_shift_tab_special_alias() {
        let kb = KeyBinding::parse("Shift+Tab").unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::BackTab, KeyModifiers::NONE));
    }

    #[test]
    fn test_parse_shift_tab_case_insensitive() {
        let kb = KeyBinding::parse("shift+tab").unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::BackTab, KeyModifiers::NONE));
    }

    #[test]
    fn test_parse_combined_ctrl_alt() {
        let kb = KeyBinding::parse("Ctrl+Alt+x").unwrap();
        assert_eq!(
            kb,
            KeyBinding(
                KeyCode::Char('x'),
                KeyModifiers::CONTROL | KeyModifiers::ALT
            )
        );
    }

    #[test]
    fn test_parse_ctrl_shift() {
        let kb = KeyBinding::parse("Ctrl+Shift+Enter").unwrap();
        assert_eq!(
            kb,
            KeyBinding(KeyCode::Enter, KeyModifiers::CONTROL | KeyModifiers::SHIFT)
        );
    }

    #[test]
    fn test_parse_invalid_returns_err() {
        assert!(KeyBinding::parse("NotAKey").is_err());
    }

    // ── KeyBinding::display ─────────────────────────────────────────────

    #[test]
    fn test_display_backtab() {
        let kb = KeyBinding(KeyCode::BackTab, KeyModifiers::NONE);
        assert_eq!(kb.display(), "Shift+Tab");
    }

    #[test]
    fn test_display_ctrl_modifier() {
        let kb = KeyBinding(KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert_eq!(kb.display(), "Ctrl+d");
    }

    #[test]
    fn test_display_alt_modifier() {
        let kb = KeyBinding(KeyCode::Char('x'), KeyModifiers::ALT);
        assert_eq!(kb.display(), "Alt+x");
    }

    #[test]
    fn test_display_shift_modifier() {
        let kb = KeyBinding(KeyCode::Enter, KeyModifiers::SHIFT);
        assert_eq!(kb.display(), "Shift+Enter");
    }

    #[test]
    fn test_display_combined_modifiers() {
        let kb = KeyBinding(
            KeyCode::Char('x'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        );
        assert_eq!(kb.display(), "Ctrl+Alt+x");
    }

    #[test]
    fn test_display_named_keys() {
        assert_eq!(
            KeyBinding(KeyCode::Tab, KeyModifiers::NONE).display(),
            "Tab"
        );
        assert_eq!(
            KeyBinding(KeyCode::PageDown, KeyModifiers::NONE).display(),
            "PageDown"
        );
        assert_eq!(
            KeyBinding(KeyCode::PageUp, KeyModifiers::NONE).display(),
            "PageUp"
        );
        assert_eq!(
            KeyBinding(KeyCode::Char(' '), KeyModifiers::NONE).display(),
            "Space"
        );
        assert_eq!(
            KeyBinding(KeyCode::Esc, KeyModifiers::NONE).display(),
            "Esc"
        );
        assert_eq!(KeyBinding(KeyCode::Up, KeyModifiers::NONE).display(), "Up");
        assert_eq!(
            KeyBinding(KeyCode::Down, KeyModifiers::NONE).display(),
            "Down"
        );
        assert_eq!(
            KeyBinding(KeyCode::Left, KeyModifiers::NONE).display(),
            "Left"
        );
        assert_eq!(
            KeyBinding(KeyCode::Right, KeyModifiers::NONE).display(),
            "Right"
        );
        assert_eq!(
            KeyBinding(KeyCode::Enter, KeyModifiers::NONE).display(),
            "Enter"
        );
        assert_eq!(
            KeyBinding(KeyCode::Backspace, KeyModifiers::NONE).display(),
            "Backspace"
        );
        assert_eq!(
            KeyBinding(KeyCode::Delete, KeyModifiers::NONE).display(),
            "Delete"
        );
        assert_eq!(
            KeyBinding(KeyCode::Home, KeyModifiers::NONE).display(),
            "Home"
        );
        assert_eq!(
            KeyBinding(KeyCode::End, KeyModifiers::NONE).display(),
            "End"
        );
        assert_eq!(
            KeyBinding(KeyCode::Insert, KeyModifiers::NONE).display(),
            "Insert"
        );
    }

    #[test]
    fn test_display_f_key() {
        assert_eq!(
            KeyBinding(KeyCode::F(1), KeyModifiers::NONE).display(),
            "F1"
        );
        assert_eq!(
            KeyBinding(KeyCode::F(12), KeyModifiers::NONE).display(),
            "F12"
        );
    }

    #[test]
    fn test_display_char() {
        assert_eq!(
            KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE).display(),
            "j"
        );
        assert_eq!(
            KeyBinding(KeyCode::Char('G'), KeyModifiers::NONE).display(),
            "G"
        );
    }

    #[test]
    fn test_display_roundtrip() {
        let cases = vec![
            "j",
            "G",
            "Tab",
            "PageDown",
            "PageUp",
            "Space",
            "Esc",
            "Up",
            "Down",
            "Left",
            "Right",
            "Enter",
            "Backspace",
            "Delete",
            "Home",
            "End",
            "Insert",
            "F1",
            "Ctrl+d",
            "Alt+x",
            "Shift+Enter",
            "Shift+Tab",
        ];
        for s in cases {
            let kb = KeyBinding::parse(s).unwrap();
            let displayed = kb.display();
            let reparsed = KeyBinding::parse(&displayed).unwrap();
            assert_eq!(kb, reparsed, "Roundtrip failed for {:?}", s);
        }
    }

    // ── KeyBinding::matches ─────────────────────────────────────────────

    #[test]
    fn test_matches_exact() {
        let kb = KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE);
        assert!(kb.matches(KeyCode::Char('j'), KeyModifiers::NONE));
    }

    #[test]
    fn test_matches_accepts_shift_when_no_ctrl_alt() {
        let kb = KeyBinding(KeyCode::Char('G'), KeyModifiers::NONE);
        assert!(kb.matches(KeyCode::Char('G'), KeyModifiers::SHIFT));
    }

    #[test]
    fn test_matches_rejects_ctrl_when_not_in_binding() {
        let kb = KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE);
        assert!(!kb.matches(KeyCode::Char('j'), KeyModifiers::CONTROL));
    }

    #[test]
    fn test_matches_rejects_alt_when_not_in_binding() {
        let kb = KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE);
        assert!(!kb.matches(KeyCode::Char('j'), KeyModifiers::ALT));
    }

    #[test]
    fn test_matches_ctrl_binding_requires_ctrl() {
        let kb = KeyBinding(KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert!(kb.matches(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert!(!kb.matches(KeyCode::Char('d'), KeyModifiers::NONE));
    }

    #[test]
    fn test_matches_alt_binding_requires_alt() {
        let kb = KeyBinding(KeyCode::Char('x'), KeyModifiers::ALT);
        assert!(kb.matches(KeyCode::Char('x'), KeyModifiers::ALT));
        assert!(!kb.matches(KeyCode::Char('x'), KeyModifiers::NONE));
    }

    #[test]
    fn test_matches_wrong_key() {
        let kb = KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE);
        assert!(!kb.matches(KeyCode::Char('k'), KeyModifiers::NONE));
    }

    #[test]
    fn test_matches_non_char_shift_exact() {
        // Shift+Enter binding must match only Shift+Enter
        let kb = KeyBinding(KeyCode::Enter, KeyModifiers::SHIFT);
        assert!(kb.matches(KeyCode::Enter, KeyModifiers::SHIFT));
        assert!(!kb.matches(KeyCode::Enter, KeyModifiers::NONE));
    }

    #[test]
    fn test_matches_non_char_no_shift_rejects_shift() {
        // Plain Enter binding must NOT match Shift+Enter
        let kb = KeyBinding(KeyCode::Enter, KeyModifiers::NONE);
        assert!(kb.matches(KeyCode::Enter, KeyModifiers::NONE));
        assert!(!kb.matches(KeyCode::Enter, KeyModifiers::SHIFT));
    }

    #[test]
    fn test_matches_non_char_rejects_ctrl() {
        let kb = KeyBinding(KeyCode::Enter, KeyModifiers::NONE);
        assert!(!kb.matches(KeyCode::Enter, KeyModifiers::CONTROL));
    }

    #[test]
    fn test_matches_ctrl_alt_combined() {
        let kb = KeyBinding(
            KeyCode::Char('x'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        );
        assert!(kb.matches(
            KeyCode::Char('x'),
            KeyModifiers::CONTROL | KeyModifiers::ALT
        ));
        assert!(!kb.matches(KeyCode::Char('x'), KeyModifiers::CONTROL));
        assert!(!kb.matches(KeyCode::Char('x'), KeyModifiers::ALT));
        assert!(!kb.matches(KeyCode::Char('x'), KeyModifiers::NONE));
    }

    // ── KeyBinding serde ────────────────────────────────────────────────

    #[test]
    fn test_keybinding_serialize() {
        let kb = KeyBinding(KeyCode::Char('d'), KeyModifiers::CONTROL);
        let json = serde_json::to_string(&kb).unwrap();
        assert_eq!(json, r#""Ctrl+d""#);
    }

    #[test]
    fn test_keybinding_deserialize() {
        let kb: KeyBinding = serde_json::from_str(r#""Alt+x""#).unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::Char('x'), KeyModifiers::ALT));
    }

    #[test]
    fn test_keybinding_deserialize_invalid() {
        let result: Result<KeyBinding, _> = serde_json::from_str(r#""NotAKey""#);
        assert!(result.is_err());
    }

    // ── KeyBindings ─────────────────────────────────────────────────────

    #[test]
    fn test_keybindings_matches_any() {
        let kbs = KeyBindings(vec![
            KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE),
            KeyBinding(KeyCode::Down, KeyModifiers::NONE),
        ]);
        assert!(kbs.matches(KeyCode::Char('j'), KeyModifiers::NONE));
        assert!(kbs.matches(KeyCode::Down, KeyModifiers::NONE));
        assert!(!kbs.matches(KeyCode::Char('k'), KeyModifiers::NONE));
    }

    #[test]
    fn test_keybindings_matches_empty() {
        let kbs = KeyBindings(vec![]);
        assert!(!kbs.matches(KeyCode::Char('j'), KeyModifiers::NONE));
    }

    #[test]
    fn test_keybindings_display_single() {
        let kbs = KeyBindings(vec![KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE)]);
        assert_eq!(kbs.display(), "j");
    }

    #[test]
    fn test_keybindings_display_multi() {
        let kbs = KeyBindings(vec![
            KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE),
            KeyBinding(KeyCode::Down, KeyModifiers::NONE),
        ]);
        assert_eq!(kbs.display(), "j/Down");
    }

    #[test]
    fn test_keybindings_display_empty() {
        let kbs = KeyBindings(vec![]);
        assert_eq!(kbs.display(), "");
    }

    // ── KeyBindings serde ───────────────────────────────────────────────

    #[test]
    fn test_keybindings_deserialize_string() {
        let kbs: KeyBindings = serde_json::from_str(r#""j""#).unwrap();
        assert_eq!(kbs.0.len(), 1);
        assert!(kbs.matches(KeyCode::Char('j'), KeyModifiers::NONE));
    }

    #[test]
    fn test_keybindings_deserialize_array() {
        let kbs: KeyBindings = serde_json::from_str(r#"["j", "Down"]"#).unwrap();
        assert_eq!(kbs.0.len(), 2);
        assert!(kbs.matches(KeyCode::Char('j'), KeyModifiers::NONE));
        assert!(kbs.matches(KeyCode::Down, KeyModifiers::NONE));
    }

    #[test]
    fn test_keybindings_serialize_single_as_string() {
        let kbs = KeyBindings(vec![KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE)]);
        let json = serde_json::to_string(&kbs).unwrap();
        assert_eq!(json, r#""j""#);
    }

    #[test]
    fn test_keybindings_serialize_multi_as_array() {
        let kbs = KeyBindings(vec![
            KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE),
            KeyBinding(KeyCode::Down, KeyModifiers::NONE),
        ]);
        let json = serde_json::to_string(&kbs).unwrap();
        assert_eq!(json, r#"["j","Down"]"#);
    }

    #[test]
    fn test_keybindings_serde_roundtrip() {
        let original = KeyBindings(vec![
            KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE),
            KeyBinding(KeyCode::Down, KeyModifiers::NONE),
        ]);
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: KeyBindings = serde_json::from_str(&json).unwrap();
        assert_eq!(original, deserialized);
    }

    // ── KeyBindings::has_overlap ────────────────────────────────────────

    #[test]
    fn test_has_overlap_true() {
        let a = KeyBindings(vec![
            KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE),
            KeyBinding(KeyCode::Down, KeyModifiers::NONE),
        ]);
        let b = KeyBindings(vec![KeyBinding(KeyCode::Down, KeyModifiers::NONE)]);
        assert!(a.has_overlap(&b));
    }

    #[test]
    fn test_has_overlap_false() {
        let a = KeyBindings(vec![KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE)]);
        let b = KeyBindings(vec![KeyBinding(KeyCode::Char('k'), KeyModifiers::NONE)]);
        assert!(!a.has_overlap(&b));
    }

    #[test]
    fn test_has_overlap_same_key_different_modifiers() {
        let a = KeyBindings(vec![KeyBinding(KeyCode::Char('d'), KeyModifiers::NONE)]);
        let b = KeyBindings(vec![KeyBinding(KeyCode::Char('d'), KeyModifiers::CONTROL)]);
        assert!(!a.has_overlap(&b));
    }

    #[test]
    fn test_has_overlap_empty() {
        let a = KeyBindings(vec![]);
        let b = KeyBindings(vec![KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE)]);
        assert!(!a.has_overlap(&b));
    }

    // ── Keybindings::validate ───────────────────────────────────────────

    #[test]
    fn test_validate_default_no_conflicts() {
        let kb = Keybindings::default();
        let conflicts = kb.validate();
        assert!(
            conflicts.is_empty(),
            "Default keybindings should have no conflicts, got: {:?}",
            conflicts
        );
    }

    #[test]
    fn test_validate_detects_normal_conflict() {
        let mut kb = Keybindings::default();
        // Make navigation.scroll_down overlap with navigation.scroll_up by assigning 'k'
        kb.navigation.scroll_down =
            KeyBindings(vec![KeyBinding(KeyCode::Char('k'), KeyModifiers::NONE)]);
        let conflicts = kb.validate();
        assert!(
            !conflicts.is_empty(),
            "Should detect conflict between scroll_down and scroll_up"
        );
        assert!(conflicts[0].contains("scroll_down"));
        assert!(conflicts[0].contains("scroll_up"));
    }

    #[test]
    fn test_validate_detects_normal_global_conflict() {
        let mut kb = Keybindings::default();
        // Make navigation.scroll_down overlap with quit by assigning 'q'
        kb.navigation.scroll_down =
            KeyBindings(vec![KeyBinding(KeyCode::Char('q'), KeyModifiers::NONE)]);
        let conflicts = kb.validate();
        assert!(
            !conflicts.is_empty(),
            "Should detect conflict between scroll_down and global quit"
        );
        let joined = conflicts.join(" ");
        assert!(joined.contains("scroll_down"));
        assert!(joined.contains("quit"));
    }

    #[test]
    fn test_validate_detects_filter_conflict() {
        let mut kb = Keybindings::default();
        // Make navigation.scroll_up overlap with toggle_filter by assigning Space
        kb.navigation.scroll_up =
            KeyBindings(vec![KeyBinding(KeyCode::Char(' '), KeyModifiers::NONE)]);
        let conflicts = kb.validate();
        assert!(
            !conflicts.is_empty(),
            "Should detect conflict between navigation.scroll_up and toggle_filter"
        );
    }

    // ── Default keybindings ─────────────────────────────────────────────

    #[test]
    fn test_navigation_keybindings_default() {
        let kb = NavigationKeybindings::default();
        assert!(
            kb.scroll_down
                .matches(KeyCode::Char('j'), KeyModifiers::NONE)
        );
        assert!(kb.scroll_down.matches(KeyCode::Down, KeyModifiers::NONE));
        assert!(kb.scroll_up.matches(KeyCode::Char('k'), KeyModifiers::NONE));
        assert!(kb.scroll_up.matches(KeyCode::Up, KeyModifiers::NONE));
        assert!(
            kb.half_page_down
                .matches(KeyCode::Char('d'), KeyModifiers::CONTROL)
        );
        assert!(
            kb.half_page_up
                .matches(KeyCode::Char('u'), KeyModifiers::CONTROL)
        );
        assert!(kb.page_down.matches(KeyCode::PageDown, KeyModifiers::NONE));
        assert!(kb.page_up.matches(KeyCode::PageUp, KeyModifiers::NONE));
    }

    #[test]
    fn test_normal_keybindings_default() {
        let kb = NormalKeybindings::default();
        assert!(
            kb.scroll_left
                .matches(KeyCode::Char('h'), KeyModifiers::NONE)
        );
        assert!(
            kb.scroll_right
                .matches(KeyCode::Char('l'), KeyModifiers::NONE)
        );
        assert!(
            kb.command_mode
                .matches(KeyCode::Char(':'), KeyModifiers::NONE)
        );
        assert!(
            kb.filter_mode
                .matches(KeyCode::Char('f'), KeyModifiers::NONE)
        );
        assert!(
            kb.toggle_filtering
                .matches(KeyCode::Char('F'), KeyModifiers::NONE)
        );
        assert!(
            kb.filter_include
                .matches(KeyCode::Char('i'), KeyModifiers::NONE)
        );
        assert!(
            kb.filter_exclude
                .matches(KeyCode::Char('o'), KeyModifiers::NONE)
        );
        assert!(
            kb.enter_ui_mode
                .matches(KeyCode::Char('u'), KeyModifiers::NONE)
        );
        assert!(
            kb.go_to_top_chord
                .matches(KeyCode::Char('g'), KeyModifiers::NONE)
        );
        assert!(
            kb.go_to_bottom
                .matches(KeyCode::Char('G'), KeyModifiers::NONE)
        );
        assert!(kb.mark_line.matches(KeyCode::Char('m'), KeyModifiers::NONE));
        assert!(
            kb.search_forward
                .matches(KeyCode::Char('/'), KeyModifiers::NONE)
        );
        assert!(
            kb.search_backward
                .matches(KeyCode::Char('?'), KeyModifiers::NONE)
        );
        assert!(
            kb.next_match
                .matches(KeyCode::Char('n'), KeyModifiers::NONE)
        );
        assert!(
            kb.prev_match
                .matches(KeyCode::Char('N'), KeyModifiers::NONE)
        );
        assert!(
            kb.visual_mode
                .matches(KeyCode::Char('V'), KeyModifiers::NONE)
        );
        assert!(
            kb.toggle_marks_only
                .matches(KeyCode::Char('M'), KeyModifiers::NONE)
        );
        assert!(kb.yank_line.matches(KeyCode::Char('y'), KeyModifiers::NONE));
        assert!(
            kb.yank_marked
                .matches(KeyCode::Char('Y'), KeyModifiers::NONE)
        );
        assert!(
            kb.show_keybindings
                .matches(KeyCode::F(1), KeyModifiers::NONE)
        );
        assert!(kb.clear_all.matches(KeyCode::Char('C'), KeyModifiers::NONE));
        assert!(
            kb.edit_comment
                .matches(KeyCode::Char('c'), KeyModifiers::NONE)
        );
    }

    #[test]
    fn test_filter_keybindings_default() {
        let kb = FilterKeybindings::default();
        assert!(
            kb.toggle_filter
                .matches(KeyCode::Char(' '), KeyModifiers::NONE)
        );
        assert!(
            kb.delete_filter
                .matches(KeyCode::Char('d'), KeyModifiers::NONE)
        );
        assert!(
            kb.move_filter_up
                .matches(KeyCode::Char('K'), KeyModifiers::NONE)
        );
        assert!(
            kb.move_filter_down
                .matches(KeyCode::Char('J'), KeyModifiers::NONE)
        );
        assert!(
            kb.edit_filter
                .matches(KeyCode::Char('e'), KeyModifiers::NONE)
        );
        assert!(kb.set_color.matches(KeyCode::Char('c'), KeyModifiers::NONE));
        assert!(
            kb.toggle_all_filters
                .matches(KeyCode::Char('A'), KeyModifiers::NONE)
        );
        assert!(
            kb.clear_all_filters
                .matches(KeyCode::Char('C'), KeyModifiers::NONE)
        );
        assert!(kb.exit_mode.matches(KeyCode::Esc, KeyModifiers::NONE));
    }

    #[test]
    fn test_global_keybindings_default() {
        let kb = GlobalKeybindings::default();
        assert!(kb.quit.matches(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(kb.next_tab.matches(KeyCode::Tab, KeyModifiers::NONE));
        assert!(kb.prev_tab.matches(KeyCode::BackTab, KeyModifiers::NONE));
        assert!(
            kb.close_tab
                .matches(KeyCode::Char('w'), KeyModifiers::CONTROL)
        );
        assert!(
            kb.new_tab
                .matches(KeyCode::Char('t'), KeyModifiers::CONTROL)
        );
    }

    #[test]
    fn test_comment_keybindings_default() {
        let kb = CommentKeybindings::default();
        assert!(kb.save.matches(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(kb.cancel.matches(KeyCode::Esc, KeyModifiers::NONE));
        assert!(kb.delete.matches(KeyCode::Char('d'), KeyModifiers::CONTROL));
    }

    #[test]
    fn test_search_keybindings_default() {
        let kb = SearchKeybindings::default();
        assert!(kb.cancel.matches(KeyCode::Esc, KeyModifiers::NONE));
        assert!(kb.confirm.matches(KeyCode::Enter, KeyModifiers::NONE));
    }

    #[test]
    fn test_filter_edit_keybindings_default() {
        let kb = FilterEditKeybindings::default();
        assert!(kb.cancel.matches(KeyCode::Esc, KeyModifiers::NONE));
        assert!(kb.confirm.matches(KeyCode::Enter, KeyModifiers::NONE));
    }

    #[test]
    fn test_command_keybindings_default() {
        let kb = CommandModeKeybindings::default();
        assert!(kb.cancel.matches(KeyCode::Esc, KeyModifiers::NONE));
        assert!(kb.confirm.matches(KeyCode::Enter, KeyModifiers::NONE));
    }

    #[test]
    fn test_docker_select_keybindings_default() {
        let kb = DockerSelectKeybindings::default();
        assert!(kb.confirm.matches(KeyCode::Enter, KeyModifiers::NONE));
        assert!(kb.cancel.matches(KeyCode::Esc, KeyModifiers::NONE));
    }

    #[test]
    fn test_value_colors_keybindings_default() {
        let kb = ValueColorsKeybindings::default();
        assert!(kb.toggle.matches(KeyCode::Char(' '), KeyModifiers::NONE));
        assert!(kb.all.matches(KeyCode::Char('a'), KeyModifiers::NONE));
        assert!(kb.none.matches(KeyCode::Char('n'), KeyModifiers::NONE));
        assert!(kb.apply.matches(KeyCode::Enter, KeyModifiers::NONE));
        assert!(kb.cancel.matches(KeyCode::Esc, KeyModifiers::NONE));
    }

    #[test]
    fn test_select_fields_keybindings_default() {
        let kb = SelectFieldsKeybindings::default();
        assert!(kb.toggle.matches(KeyCode::Char(' '), KeyModifiers::NONE));
        assert!(kb.move_up.matches(KeyCode::Char('K'), KeyModifiers::NONE));
        assert!(kb.move_down.matches(KeyCode::Char('J'), KeyModifiers::NONE));
        assert!(kb.all.matches(KeyCode::Char('a'), KeyModifiers::NONE));
        assert!(kb.none.matches(KeyCode::Char('n'), KeyModifiers::NONE));
        assert!(kb.apply.matches(KeyCode::Enter, KeyModifiers::NONE));
        assert!(kb.cancel.matches(KeyCode::Esc, KeyModifiers::NONE));
    }

    #[test]
    fn test_help_keybindings_default() {
        let kb = HelpKeybindings::default();
        assert!(kb.close.matches(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(kb.close.matches(KeyCode::Esc, KeyModifiers::NONE));
    }

    #[test]
    fn test_confirm_keybindings_default() {
        let kb = ConfirmKeybindings::default();
        assert!(kb.yes.matches(KeyCode::Char('y'), KeyModifiers::NONE));
        assert!(kb.yes.matches(KeyCode::Enter, KeyModifiers::NONE));
        assert!(kb.no.matches(KeyCode::Char('n'), KeyModifiers::NONE));
        assert!(kb.no.matches(KeyCode::Esc, KeyModifiers::NONE));
    }

    // ── Config ──────────────────────────────────────────────────────────

    #[test]
    fn test_config_load_fallback_on_missing_file() {
        let config = Config::default();
        assert!(config.theme.is_none());
        assert!(
            config
                .keybindings
                .global
                .quit
                .matches(KeyCode::Char('q'), KeyModifiers::NONE)
        );
    }

    #[test]
    fn test_config_deserialize_theme_and_keybinding() {
        let json = r#"{"theme":"dracula","keybindings":{"navigation":{"scroll_down":"e"}}}"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.theme.as_deref(), Some("dracula"));
        assert!(
            cfg.keybindings
                .navigation
                .scroll_down
                .matches(KeyCode::Char('e'), KeyModifiers::NONE)
        );
        // Default bindings still intact
        assert!(
            cfg.keybindings
                .global
                .quit
                .matches(KeyCode::Char('q'), KeyModifiers::NONE)
        );
    }

    #[test]
    fn test_config_deserialize_empty_object() {
        let cfg: Config = serde_json::from_str("{}").unwrap();
        assert!(cfg.theme.is_none());
        // All defaults should be intact
        assert!(
            cfg.keybindings
                .navigation
                .scroll_down
                .matches(KeyCode::Char('j'), KeyModifiers::NONE)
        );
    }

    #[test]
    fn test_config_deserialize_comment_keybindings() {
        let json = r#"{"keybindings":{"comment":{"save":"Shift+Enter"}}}"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert!(
            cfg.keybindings
                .comment
                .save
                .matches(KeyCode::Enter, KeyModifiers::SHIFT)
        );
    }

    #[test]
    fn test_config_deserialize_filter_keybindings() {
        let json = r#"{"keybindings":{"filter":{"delete_filter":"x"}}}"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert!(
            cfg.keybindings
                .filter
                .delete_filter
                .matches(KeyCode::Char('x'), KeyModifiers::NONE)
        );
        // Other filter defaults still intact
        assert!(
            cfg.keybindings
                .filter
                .toggle_filter
                .matches(KeyCode::Char(' '), KeyModifiers::NONE)
        );
    }

    #[test]
    fn test_config_serialize_roundtrip() {
        let original = Config::default();
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(original.theme, deserialized.theme);
        // Verify a few bindings survived roundtrip
        assert!(
            deserialized
                .keybindings
                .navigation
                .scroll_down
                .matches(KeyCode::Char('j'), KeyModifiers::NONE)
        );
        assert!(
            deserialized
                .keybindings
                .global
                .quit
                .matches(KeyCode::Char('q'), KeyModifiers::NONE)
        );
    }

    // ── check_conflicts ─────────────────────────────────────────────────

    #[test]
    fn test_check_conflicts_no_overlap() {
        let a = KeyBindings(vec![KeyBinding(KeyCode::Char('a'), KeyModifiers::NONE)]);
        let b = KeyBindings(vec![KeyBinding(KeyCode::Char('b'), KeyModifiers::NONE)]);
        let actions: &[(&str, &KeyBindings)] = &[("action_a", &a), ("action_b", &b)];
        let mut conflicts = Vec::new();
        check_conflicts(actions, &mut conflicts);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn test_check_conflicts_overlap_reports_key() {
        let a = KeyBindings(vec![KeyBinding(KeyCode::Char('x'), KeyModifiers::NONE)]);
        let b = KeyBindings(vec![KeyBinding(KeyCode::Char('x'), KeyModifiers::NONE)]);
        let actions: &[(&str, &KeyBindings)] = &[("alpha", &a), ("beta", &b)];
        let mut conflicts = Vec::new();
        check_conflicts(actions, &mut conflicts);
        assert_eq!(conflicts.len(), 1);
        assert!(conflicts[0].contains("alpha"));
        assert!(conflicts[0].contains("beta"));
        assert!(conflicts[0].contains("x"));
    }

    // ── Config show_mode_bar / show_borders ────────────────────────────

    #[test]
    fn test_config_show_mode_bar_defaults_true() {
        let cfg: Config = serde_json::from_str("{}").unwrap();
        assert!(cfg.show_mode_bar);
    }

    #[test]
    fn test_config_show_borders_defaults_true() {
        let cfg: Config = serde_json::from_str("{}").unwrap();
        assert!(cfg.show_borders);
    }

    #[test]
    fn test_config_show_mode_bar_false() {
        let cfg: Config = serde_json::from_str(r#"{"show_mode_bar": false}"#).unwrap();
        assert!(!cfg.show_mode_bar);
    }

    #[test]
    fn test_config_show_borders_false() {
        let cfg: Config = serde_json::from_str(r#"{"show_borders": false}"#).unwrap();
        assert!(!cfg.show_borders);
    }

    #[test]
    fn test_config_default_show_mode_bar_true() {
        let cfg = Config::default();
        assert!(cfg.show_mode_bar);
    }

    #[test]
    fn test_config_default_show_borders_true() {
        let cfg = Config::default();
        assert!(cfg.show_borders);
    }

    #[test]
    fn test_toggle_mode_bar_keybinding_default() {
        let kb = UiKeybindings::default();
        assert!(
            kb.toggle_mode_bar
                .matches(KeyCode::Char('b'), KeyModifiers::NONE)
        );
    }

    #[test]
    fn test_toggle_borders_keybinding_default() {
        let kb = UiKeybindings::default();
        assert!(
            kb.toggle_borders
                .matches(KeyCode::Char('B'), KeyModifiers::NONE)
        );
    }

    #[test]
    fn test_filter_include_keybinding_default() {
        let kb = NormalKeybindings::default();
        assert!(
            kb.filter_include
                .matches(KeyCode::Char('i'), KeyModifiers::NONE)
        );
    }

    #[test]
    fn test_filter_exclude_keybinding_default() {
        let kb = NormalKeybindings::default();
        assert!(
            kb.filter_exclude
                .matches(KeyCode::Char('o'), KeyModifiers::NONE)
        );
    }

    #[test]
    fn test_enter_ui_mode_keybinding_default() {
        let kb = NormalKeybindings::default();
        assert!(
            kb.enter_ui_mode
                .matches(KeyCode::Char('u'), KeyModifiers::NONE)
        );
    }

    #[test]
    fn test_ui_keybindings_defaults() {
        let kb = UiKeybindings::default();
        assert!(
            kb.toggle_sidebar
                .matches(KeyCode::Char('s'), KeyModifiers::NONE)
        );
        assert!(
            kb.toggle_wrap
                .matches(KeyCode::Char('w'), KeyModifiers::NONE)
        );
        assert!(kb.exit.matches(KeyCode::Esc, KeyModifiers::NONE));
    }
}
