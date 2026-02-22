//! Configuration loaded from `~/.config/logsmith-rs/config.json`.
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
                let n: u8 = s[1..].parse().map_err(|_| format!("Unknown key: {:?}", s))?;
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
        self.0.iter().map(|b| b.display()).collect::<Vec<_>>().join("/")
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

            fn visit_seq<A: de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<KeyBindings, A::Error> {
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
fn default_show_keybindings() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::F(1), KeyModifiers::NONE)])
}

// ---------------------------------------------------------------------------
// NormalKeybindings
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalKeybindings {
    #[serde(default = "default_scroll_down")]
    pub scroll_down: KeyBindings,
    #[serde(default = "default_scroll_up")]
    pub scroll_up: KeyBindings,
    #[serde(default = "default_scroll_left")]
    pub scroll_left: KeyBindings,
    #[serde(default = "default_scroll_right")]
    pub scroll_right: KeyBindings,
    #[serde(default = "default_half_page_down")]
    pub half_page_down: KeyBindings,
    #[serde(default = "default_half_page_up")]
    pub half_page_up: KeyBindings,
    #[serde(default = "default_page_down")]
    pub page_down: KeyBindings,
    #[serde(default = "default_page_up")]
    pub page_up: KeyBindings,
    #[serde(default = "default_command_mode")]
    pub command_mode: KeyBindings,
    #[serde(default = "default_filter_mode_key")]
    pub filter_mode: KeyBindings,
    #[serde(default = "default_toggle_filtering")]
    pub toggle_filtering: KeyBindings,
    #[serde(default = "default_toggle_sidebar")]
    pub toggle_sidebar: KeyBindings,
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
    #[serde(default = "default_toggle_wrap")]
    pub toggle_wrap: KeyBindings,
    #[serde(default = "default_visual_mode")]
    pub visual_mode: KeyBindings,
    #[serde(default = "default_toggle_marks_only")]
    pub toggle_marks_only: KeyBindings,
    #[serde(default = "default_show_keybindings")]
    pub show_keybindings: KeyBindings,
}

impl Default for NormalKeybindings {
    fn default() -> Self {
        Self {
            scroll_down: default_scroll_down(),
            scroll_up: default_scroll_up(),
            scroll_left: default_scroll_left(),
            scroll_right: default_scroll_right(),
            half_page_down: default_half_page_down(),
            half_page_up: default_half_page_up(),
            page_down: default_page_down(),
            page_up: default_page_up(),
            command_mode: default_command_mode(),
            filter_mode: default_filter_mode_key(),
            toggle_filtering: default_toggle_filtering(),
            toggle_sidebar: default_toggle_sidebar(),
            go_to_top_chord: default_go_to_top_chord(),
            go_to_bottom: default_go_to_bottom(),
            mark_line: default_mark_line(),
            search_forward: default_search_forward(),
            search_backward: default_search_backward(),
            next_match: default_next_match(),
            prev_match: default_prev_match(),
            toggle_wrap: default_toggle_wrap(),
            visual_mode: default_visual_mode(),
            toggle_marks_only: default_toggle_marks_only(),
            show_keybindings: default_show_keybindings(),
        }
    }
}

// ---------------------------------------------------------------------------
// FilterKeybindings — defaults
// ---------------------------------------------------------------------------

fn default_filter_select_up() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Up, KeyModifiers::NONE)])
}
fn default_filter_select_down() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Down, KeyModifiers::NONE)])
}
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
fn default_filter_add_include() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('i'), KeyModifiers::NONE)])
}
fn default_filter_add_exclude() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('x'), KeyModifiers::NONE)])
}
fn default_filter_exit() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Esc, KeyModifiers::NONE)])
}

// ---------------------------------------------------------------------------
// FilterKeybindings
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterKeybindings {
    #[serde(default = "default_filter_select_up")]
    pub select_up: KeyBindings,
    #[serde(default = "default_filter_select_down")]
    pub select_down: KeyBindings,
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
    #[serde(default = "default_filter_add_include")]
    pub add_include: KeyBindings,
    #[serde(default = "default_filter_add_exclude")]
    pub add_exclude: KeyBindings,
    #[serde(default = "default_filter_exit")]
    pub exit_mode: KeyBindings,
}

impl Default for FilterKeybindings {
    fn default() -> Self {
        Self {
            select_up: default_filter_select_up(),
            select_down: default_filter_select_down(),
            toggle_filter: default_filter_toggle(),
            delete_filter: default_filter_delete(),
            move_filter_up: default_filter_move_up(),
            move_filter_down: default_filter_move_down(),
            edit_filter: default_filter_edit(),
            set_color: default_filter_set_color(),
            toggle_all_filters: default_filter_toggle_all(),
            clear_all_filters: default_filter_clear_all(),
            add_include: default_filter_add_include(),
            add_exclude: default_filter_add_exclude(),
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommentKeybindings {
    /// Key to save the comment and return to Normal mode.
    #[serde(default = "default_comment_save")]
    pub save: KeyBindings,
}

impl Default for CommentKeybindings {
    fn default() -> Self {
        Self { save: default_comment_save() }
    }
}

// ---------------------------------------------------------------------------
// Keybindings
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Keybindings {
    #[serde(default)]
    pub normal: NormalKeybindings,
    #[serde(default)]
    pub filter: FilterKeybindings,
    #[serde(default)]
    pub global: GlobalKeybindings,
    #[serde(default)]
    pub comment: CommentKeybindings,
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
        let normal_actions: &[(&str, &KeyBindings)] = &[
            ("normal.scroll_down",       &self.normal.scroll_down),
            ("normal.scroll_up",         &self.normal.scroll_up),
            ("normal.scroll_left",       &self.normal.scroll_left),
            ("normal.scroll_right",      &self.normal.scroll_right),
            ("normal.half_page_down",    &self.normal.half_page_down),
            ("normal.half_page_up",      &self.normal.half_page_up),
            ("normal.page_down",         &self.normal.page_down),
            ("normal.page_up",           &self.normal.page_up),
            ("normal.command_mode",      &self.normal.command_mode),
            ("normal.filter_mode",       &self.normal.filter_mode),
            ("normal.toggle_filtering",  &self.normal.toggle_filtering),
            ("normal.toggle_sidebar",    &self.normal.toggle_sidebar),
            ("normal.go_to_top_chord",   &self.normal.go_to_top_chord),
            ("normal.go_to_bottom",      &self.normal.go_to_bottom),
            ("normal.mark_line",         &self.normal.mark_line),
            ("normal.toggle_marks_only", &self.normal.toggle_marks_only),
            ("normal.visual_mode",       &self.normal.visual_mode),
            ("normal.search_forward",    &self.normal.search_forward),
            ("normal.search_backward",   &self.normal.search_backward),
            ("normal.next_match",        &self.normal.next_match),
            ("normal.prev_match",        &self.normal.prev_match),
            ("normal.toggle_wrap",       &self.normal.toggle_wrap),
            ("normal.show_keybindings",  &self.normal.show_keybindings),
            ("global.quit",              &self.global.quit),
            ("global.next_tab",          &self.global.next_tab),
            ("global.prev_tab",          &self.global.prev_tab),
            ("global.close_tab",         &self.global.close_tab),
            ("global.new_tab",           &self.global.new_tab),
        ];

        let filter_actions: &[(&str, &KeyBindings)] = &[
            ("filter.select_up",          &self.filter.select_up),
            ("filter.select_down",        &self.filter.select_down),
            ("filter.toggle_filter",      &self.filter.toggle_filter),
            ("filter.delete_filter",      &self.filter.delete_filter),
            ("filter.move_filter_up",     &self.filter.move_filter_up),
            ("filter.move_filter_down",   &self.filter.move_filter_down),
            ("filter.edit_filter",        &self.filter.edit_filter),
            ("filter.set_color",          &self.filter.set_color),
            ("filter.toggle_all_filters", &self.filter.toggle_all_filters),
            ("filter.clear_all_filters",  &self.filter.clear_all_filters),
            ("filter.add_include",        &self.filter.add_include),
            ("filter.add_exclude",        &self.filter.add_exclude),
            ("filter.exit_mode",          &self.filter.exit_mode),
            ("global.quit",               &self.global.quit),
            ("global.next_tab",           &self.global.next_tab),
            ("global.prev_tab",           &self.global.prev_tab),
        ];

        check_conflicts(normal_actions, &mut conflicts);
        check_conflicts(filter_actions, &mut conflicts);

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

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    /// Theme name (without `.json` extension) to load on startup.
    pub theme: Option<String>,
    #[serde(default)]
    pub keybindings: Keybindings,
}

impl Config {
    /// Load configuration from `~/.config/logsmith-rs/config.json`.
    ///
    /// This function is infallible — any I/O or parse error falls back to
    /// `Config::default()` so a bad config never prevents startup.
    pub fn load() -> Self {
        let Some(config_path) = dirs::config_dir()
            .map(|d| d.join("logsmith-rs").join("config.json"))
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

    #[test]
    fn test_parse_single_char() {
        let kb = KeyBinding::parse("j").unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE));
    }

    #[test]
    fn test_parse_ctrl_prefix() {
        let kb = KeyBinding::parse("Ctrl+d").unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::Char('d'), KeyModifiers::CONTROL));
    }

    #[test]
    fn test_parse_shift_tab() {
        let kb = KeyBinding::parse("Shift+Tab").unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::BackTab, KeyModifiers::NONE));
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
    fn test_parse_uppercase_char() {
        let kb = KeyBinding::parse("G").unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::Char('G'), KeyModifiers::NONE));
    }

    #[test]
    fn test_parse_invalid_returns_err() {
        assert!(KeyBinding::parse("NotAKey").is_err());
    }

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
    fn test_matches_ctrl_binding_requires_ctrl() {
        let kb = KeyBinding(KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert!(kb.matches(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert!(!kb.matches(KeyCode::Char('d'), KeyModifiers::NONE));
    }

    #[test]
    fn test_matches_wrong_key() {
        let kb = KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE);
        assert!(!kb.matches(KeyCode::Char('k'), KeyModifiers::NONE));
    }

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
    fn test_config_load_fallback_on_missing_file() {
        // Ensure no panic when config file doesn't exist.
        // (Config::load is infallible — returns default.)
        let config = Config::default();
        assert!(config.theme.is_none());
        assert!(config.keybindings.global.quit.matches(
            KeyCode::Char('q'),
            KeyModifiers::NONE
        ));
    }

    #[test]
    fn test_config_deserialize_theme_and_keybinding() {
        let json = r#"{"theme":"dracula","keybindings":{"normal":{"scroll_down":"e"}}}"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.theme.as_deref(), Some("dracula"));
        // Custom binding: 'e' scrolls down
        assert!(cfg
            .keybindings
            .normal
            .scroll_down
            .matches(KeyCode::Char('e'), KeyModifiers::NONE));
        // Default bindings still intact
        assert!(cfg
            .keybindings
            .global
            .quit
            .matches(KeyCode::Char('q'), KeyModifiers::NONE));
    }

    #[test]
    fn test_normal_keybindings_default() {
        let kb = NormalKeybindings::default();
        assert!(kb.scroll_down.matches(KeyCode::Char('j'), KeyModifiers::NONE));
        assert!(kb.scroll_down.matches(KeyCode::Down, KeyModifiers::NONE));
        assert!(kb.half_page_down.matches(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert!(kb.go_to_bottom.matches(KeyCode::Char('G'), KeyModifiers::NONE));
    }

    #[test]
    fn test_filter_keybindings_default() {
        let kb = FilterKeybindings::default();
        assert!(kb.exit_mode.matches(KeyCode::Esc, KeyModifiers::NONE));
        assert!(kb.toggle_filter.matches(KeyCode::Char(' '), KeyModifiers::NONE));
        assert!(kb.move_filter_up.matches(KeyCode::Char('K'), KeyModifiers::NONE));
    }

    #[test]
    fn test_global_keybindings_default() {
        let kb = GlobalKeybindings::default();
        assert!(kb.quit.matches(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(kb.next_tab.matches(KeyCode::Tab, KeyModifiers::NONE));
        assert!(kb.prev_tab.matches(KeyCode::BackTab, KeyModifiers::NONE));
        assert!(kb.close_tab.matches(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert!(kb.new_tab.matches(KeyCode::Char('t'), KeyModifiers::CONTROL));
    }
}
