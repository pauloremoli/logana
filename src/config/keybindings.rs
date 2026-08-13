use crossterm::event::{KeyCode, KeyModifiers};
use serde::{Deserialize, Serialize, de};
use std::fmt;

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
                // BackTab already encodes Shift; terminals may or may not set the
                // SHIFT modifier bit alongside it, so ignore SHIFT here too.
                KeyCode::BackTab => {
                    !modifiers.contains(KeyModifiers::CONTROL)
                        && !modifiers.contains(KeyModifiers::ALT)
                }
                // For other non-character keys (Enter, Tab, F-keys …) SHIFT changes
                // the key meaning, so require an exact SHIFT match.
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

    pub(crate) fn parse(s: &str) -> Result<Self, String> {
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

impl schemars::JsonSchema for KeyBinding {
    fn schema_name() -> String {
        "KeyBinding".to_string()
    }
    fn json_schema(_g: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        schemars::schema::SchemaObject {
            instance_type: Some(schemars::schema::InstanceType::String.into()),
            metadata: Some(Box::new(schemars::schema::Metadata {
                description: Some(
                    "A key combination string, e.g. \"Ctrl+d\", \"Shift+Tab\", \"F1\", \"j\""
                        .into(),
                ),
                examples: vec![
                    serde_json::json!("j"),
                    serde_json::json!("Ctrl+d"),
                    serde_json::json!("Shift+Tab"),
                    serde_json::json!("F1"),
                ],
                ..Default::default()
            })),
            ..Default::default()
        }
        .into()
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct KeyBindings(pub Vec<KeyBinding>);

impl KeyBindings {
    pub fn matches(&self, key: KeyCode, mods: KeyModifiers) -> bool {
        self.0.iter().any(|b| b.matches(key, mods))
    }

    pub fn display(&self) -> String {
        self.0
            .iter()
            .map(|b| b.display())
            .collect::<Vec<_>>()
            .join("/")
    }

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

impl schemars::JsonSchema for KeyBindings {
    fn schema_name() -> String {
        "KeyBindings".to_string()
    }
    fn json_schema(g: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        let str_schema = g.subschema_for::<String>();
        let arr_schema = g.subschema_for::<Vec<String>>();
        schemars::schema::SchemaObject {
            subschemas: Some(Box::new(schemars::schema::SubschemaValidation {
                one_of: Some(vec![str_schema, arr_schema]),
                ..Default::default()
            })),
            metadata: Some(Box::new(schemars::schema::Metadata {
                description: Some(
                    "One key binding string or an array of alternatives, e.g. \"j\" or [\"j\", \"Down\"]".into(),
                ),
                ..Default::default()
            })),
            ..Default::default()
        }
        .into()
    }
}

#[inline(always)]
fn default_scroll_down() -> KeyBindings {
    KeyBindings(vec![
        KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE),
        KeyBinding(KeyCode::Down, KeyModifiers::NONE),
    ])
}
#[inline(always)]
fn default_scroll_up() -> KeyBindings {
    KeyBindings(vec![
        KeyBinding(KeyCode::Char('k'), KeyModifiers::NONE),
        KeyBinding(KeyCode::Up, KeyModifiers::NONE),
    ])
}
#[inline(always)]
fn default_scroll_left() -> KeyBindings {
    KeyBindings(vec![
        KeyBinding(KeyCode::Char('h'), KeyModifiers::NONE),
        KeyBinding(KeyCode::Left, KeyModifiers::NONE),
    ])
}
#[inline(always)]
fn default_scroll_right() -> KeyBindings {
    KeyBindings(vec![
        KeyBinding(KeyCode::Char('l'), KeyModifiers::NONE),
        KeyBinding(KeyCode::Right, KeyModifiers::NONE),
    ])
}
#[inline(always)]
fn default_start_of_line() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('0'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_end_of_line() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('$'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_half_page_down() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('d'), KeyModifiers::CONTROL)])
}
#[inline(always)]
fn default_half_page_up() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('u'), KeyModifiers::CONTROL)])
}
#[inline(always)]
fn default_page_down() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::PageDown, KeyModifiers::NONE)])
}
#[inline(always)]
fn default_page_up() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::PageUp, KeyModifiers::NONE)])
}
#[inline(always)]
fn default_command_mode() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char(':'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_filter_mode_key() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('f'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_group_mode_key() -> KeyBindings {
    // Plain 'g' is already the first key of the `gg` (go to top) chord
    // (`go_to_top_chord`) — Ctrl+g keeps the mnemonic without colliding.
    KeyBindings(vec![KeyBinding(KeyCode::Char('g'), KeyModifiers::CONTROL)])
}
#[inline(always)]
fn default_toggle_filtering() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('F'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_toggle_highlight_mode() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('H'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_toggle_sidebar() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('s'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_toggle_mode_bar() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('b'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_toggle_borders() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('B'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_go_to_top_chord() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('g'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_go_to_bottom() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('G'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_expand_continuation() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('>'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_collapse_continuation() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('<'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_mark_line() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('m'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_search_forward() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('/'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_search_backward() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('?'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_next_match() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('n'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_prev_match() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('N'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_toggle_wrap() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('w'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_toggle_relative_line_numbers() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('r'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_toggle_groups_panel() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('g'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_visual_mode() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('V'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_visual_char() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('v'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_toggle_marks_only() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('M'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_yank_line() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('y'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_yank_marked() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('Y'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_show_keybindings() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::F(1), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_clear_all() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('C'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_edit_comment() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('r'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_delete_comment() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('d'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_comment_line() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('c'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_next_error() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('e'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_prev_error() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('E'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_next_warning() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('w'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_prev_warning() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('W'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_normal_filter_include() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('i'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_normal_filter_include_auto() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('a'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_normal_filter_exclude() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('o'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_enter_ui_mode() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('u'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_ui_exit() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Esc, KeyModifiers::NONE)])
}
#[inline(always)]
fn default_clear_search() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Esc, KeyModifiers::NONE)])
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
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

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct NormalKeybindings {
    #[serde(default = "default_scroll_left")]
    pub scroll_left: KeyBindings,
    #[serde(default = "default_scroll_right")]
    pub scroll_right: KeyBindings,
    #[serde(default = "default_start_of_line")]
    pub start_of_line: KeyBindings,
    #[serde(default = "default_end_of_line")]
    pub end_of_line: KeyBindings,
    #[serde(default = "default_command_mode")]
    pub command_mode: KeyBindings,
    #[serde(default = "default_filter_mode_key")]
    pub filter_mode: KeyBindings,
    #[serde(default = "default_group_mode_key")]
    pub group_mode: KeyBindings,
    #[serde(default = "default_toggle_filtering")]
    pub toggle_filtering: KeyBindings,
    #[serde(default = "default_toggle_highlight_mode")]
    pub toggle_highlight_mode: KeyBindings,
    #[serde(default = "default_go_to_top_chord")]
    pub go_to_top_chord: KeyBindings,
    #[serde(default = "default_go_to_bottom")]
    pub go_to_bottom: KeyBindings,
    #[serde(default = "default_mark_line")]
    pub mark_line: KeyBindings,
    #[serde(default = "default_expand_continuation")]
    pub expand_continuation: KeyBindings,
    #[serde(default = "default_collapse_continuation")]
    pub collapse_continuation: KeyBindings,
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
    #[serde(default = "default_visual_char")]
    pub visual_char: KeyBindings,
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
    #[serde(default = "default_delete_comment")]
    pub delete_comment: KeyBindings,
    #[serde(default = "default_comment_line")]
    pub comment_line: KeyBindings,
    #[serde(default = "default_next_error")]
    pub next_error: KeyBindings,
    #[serde(default = "default_prev_error")]
    pub prev_error: KeyBindings,
    #[serde(default = "default_next_warning")]
    pub next_warning: KeyBindings,
    #[serde(default = "default_prev_warning")]
    pub prev_warning: KeyBindings,
    #[serde(default = "default_normal_filter_include")]
    pub filter_include: KeyBindings,
    #[serde(default = "default_normal_filter_include_auto")]
    pub filter_include_auto: KeyBindings,
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
            start_of_line: default_start_of_line(),
            end_of_line: default_end_of_line(),
            command_mode: default_command_mode(),
            filter_mode: default_filter_mode_key(),
            group_mode: default_group_mode_key(),
            toggle_filtering: default_toggle_filtering(),
            toggle_highlight_mode: default_toggle_highlight_mode(),
            go_to_top_chord: default_go_to_top_chord(),
            go_to_bottom: default_go_to_bottom(),
            mark_line: default_mark_line(),
            expand_continuation: default_expand_continuation(),
            collapse_continuation: default_collapse_continuation(),
            search_forward: default_search_forward(),
            search_backward: default_search_backward(),
            next_match: default_next_match(),
            prev_match: default_prev_match(),
            visual_mode: default_visual_mode(),
            visual_char: default_visual_char(),
            toggle_marks_only: default_toggle_marks_only(),
            yank_line: default_yank_line(),
            yank_marked: default_yank_marked(),
            show_keybindings: default_show_keybindings(),
            clear_all: default_clear_all(),
            edit_comment: default_edit_comment(),
            delete_comment: default_delete_comment(),
            comment_line: default_comment_line(),
            next_error: default_next_error(),
            prev_error: default_prev_error(),
            next_warning: default_next_warning(),
            prev_warning: default_prev_warning(),
            filter_include: default_normal_filter_include(),
            filter_include_auto: default_normal_filter_include_auto(),
            filter_exclude: default_normal_filter_exclude(),
            enter_ui_mode: default_enter_ui_mode(),
            clear_search: default_clear_search(),
        }
    }
}

#[inline(always)]
fn default_filter_toggle() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char(' '), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_filter_delete() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('d'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_filter_move_up() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('K'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_filter_move_down() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('J'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_filter_edit() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('e'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_filter_set_color() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('c'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_filter_toggle_all() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('A'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_filter_clear_all() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('C'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_filter_add_include() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('i'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_filter_add_include_auto() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('a'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_filter_add_exclude() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('o'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_filter_add_date() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('t'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_filter_add_highlight() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('h'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_filter_exit() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Esc, KeyModifiers::NONE)])
}
#[inline(always)]
fn default_filter_sidebar_grow() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('>'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_filter_sidebar_shrink() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('<'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_filter_search() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('/'), KeyModifiers::NONE)])
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
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
    #[serde(default = "default_filter_add_include")]
    pub add_include_filter: KeyBindings,
    #[serde(default = "default_filter_add_include_auto")]
    pub add_include_filter_auto: KeyBindings,
    #[serde(default = "default_filter_add_exclude")]
    pub add_exclude_filter: KeyBindings,
    #[serde(default = "default_filter_add_date")]
    pub add_date_filter: KeyBindings,
    #[serde(default = "default_filter_add_highlight")]
    pub add_highlight_filter: KeyBindings,
    #[serde(default = "default_filter_exit")]
    pub exit_mode: KeyBindings,
    #[serde(default = "default_filter_sidebar_grow")]
    pub sidebar_grow: KeyBindings,
    #[serde(default = "default_filter_sidebar_shrink")]
    pub sidebar_shrink: KeyBindings,
    #[serde(default = "default_filter_search")]
    pub search: KeyBindings,
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
            add_include_filter: default_filter_add_include(),
            add_include_filter_auto: default_filter_add_include_auto(),
            add_exclude_filter: default_filter_add_exclude(),
            add_date_filter: default_filter_add_date(),
            add_highlight_filter: default_filter_add_highlight(),
            exit_mode: default_filter_exit(),
            sidebar_grow: default_filter_sidebar_grow(),
            sidebar_shrink: default_filter_sidebar_shrink(),
            search: default_filter_search(),
        }
    }
}

#[inline(always)]
fn default_quit() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('q'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_next_tab() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Tab, KeyModifiers::NONE)])
}
#[inline(always)]
fn default_prev_tab() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::BackTab, KeyModifiers::NONE)])
}
#[inline(always)]
fn default_close_tab() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('w'), KeyModifiers::CONTROL)])
}
#[inline(always)]
fn default_new_tab() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('t'), KeyModifiers::CONTROL)])
}
#[inline(always)]
fn default_file_switcher() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('p'), KeyModifiers::CONTROL)])
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
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
    #[serde(default = "default_file_switcher")]
    pub file_switcher: KeyBindings,
}

impl Default for GlobalKeybindings {
    fn default() -> Self {
        Self {
            quit: default_quit(),
            next_tab: default_next_tab(),
            prev_tab: default_prev_tab(),
            close_tab: default_close_tab(),
            new_tab: default_new_tab(),
            file_switcher: default_file_switcher(),
        }
    }
}

#[inline(always)]
fn default_comment_save() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('s'), KeyModifiers::CONTROL)])
}
#[inline(always)]
fn default_comment_newline() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Enter, KeyModifiers::NONE)])
}
#[inline(always)]
fn default_comment_cancel() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Esc, KeyModifiers::NONE)])
}
#[inline(always)]
fn default_comment_delete() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('d'), KeyModifiers::CONTROL)])
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CommentKeybindings {
    /// Key to save the comment and return to Normal mode (default: Enter).
    #[serde(default = "default_comment_save")]
    pub save: KeyBindings,
    /// Key to insert a newline inside the comment (default: Shift+Enter).
    #[serde(default = "default_comment_newline")]
    pub newline: KeyBindings,
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
            newline: default_comment_newline(),
            cancel: default_comment_cancel(),
            delete: default_comment_delete(),
        }
    }
}

#[inline(always)]
fn default_visual_comment() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('c'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_visual_yank() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('y'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_visual_mark() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('m'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_visual_exit() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Esc, KeyModifiers::NONE)])
}
#[inline(always)]
fn default_visual_line_search() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('/'), KeyModifiers::NONE)])
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct VisualLineKeybindings {
    #[serde(default = "default_visual_comment")]
    pub comment: KeyBindings,
    #[serde(default = "default_visual_yank")]
    pub yank: KeyBindings,
    #[serde(default = "default_visual_mark")]
    pub mark: KeyBindings,
    #[serde(default = "default_visual_line_search")]
    pub search: KeyBindings,
    #[serde(default = "default_visual_exit")]
    pub exit: KeyBindings,
}

impl Default for VisualLineKeybindings {
    fn default() -> Self {
        Self {
            comment: default_visual_comment(),
            yank: default_visual_yank(),
            mark: default_visual_mark(),
            search: default_visual_line_search(),
            exit: default_visual_exit(),
        }
    }
}

#[inline(always)]
fn default_vc_move_left() -> KeyBindings {
    KeyBindings(vec![
        KeyBinding(KeyCode::Char('h'), KeyModifiers::NONE),
        KeyBinding(KeyCode::Left, KeyModifiers::NONE),
    ])
}
#[inline(always)]
fn default_vc_move_right() -> KeyBindings {
    KeyBindings(vec![
        KeyBinding(KeyCode::Char('l'), KeyModifiers::NONE),
        KeyBinding(KeyCode::Right, KeyModifiers::NONE),
    ])
}
#[inline(always)]
fn default_vc_word_forward() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('w'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_vc_word_backward() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('b'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_vc_word_end() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('e'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_vc_word_forward_big() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('W'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_vc_word_backward_big() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('B'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_vc_word_end_big() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('E'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_vc_start_of_line() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('0'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_vc_first_nonblank() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('^'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_vc_end_of_line() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('$'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_vc_find_forward() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('f'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_vc_find_backward() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('F'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_vc_till_forward() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('t'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_vc_till_backward() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('T'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_vc_repeat_motion() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char(';'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_vc_repeat_motion_rev() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char(','), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_vc_filter_include() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('i'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_vc_filter_exclude() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('o'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_vc_search() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('/'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_vc_yank() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('y'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_vc_start_selection() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('v'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_vc_exit() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Esc, KeyModifiers::NONE)])
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct VisualKeybindings {
    #[serde(default = "default_vc_move_left")]
    pub move_left: KeyBindings,
    #[serde(default = "default_vc_move_right")]
    pub move_right: KeyBindings,
    #[serde(default = "default_vc_word_forward")]
    pub word_forward: KeyBindings,
    #[serde(default = "default_vc_word_backward")]
    pub word_backward: KeyBindings,
    #[serde(default = "default_vc_word_end")]
    pub word_end: KeyBindings,
    #[serde(default = "default_vc_word_forward_big")]
    pub word_forward_big: KeyBindings,
    #[serde(default = "default_vc_word_backward_big")]
    pub word_backward_big: KeyBindings,
    #[serde(default = "default_vc_word_end_big")]
    pub word_end_big: KeyBindings,
    #[serde(default = "default_vc_start_of_line")]
    pub start_of_line: KeyBindings,
    #[serde(default = "default_vc_first_nonblank")]
    pub first_nonblank: KeyBindings,
    #[serde(default = "default_vc_end_of_line")]
    pub end_of_line: KeyBindings,
    #[serde(default = "default_vc_find_forward")]
    pub find_forward: KeyBindings,
    #[serde(default = "default_vc_find_backward")]
    pub find_backward: KeyBindings,
    #[serde(default = "default_vc_till_forward")]
    pub till_forward: KeyBindings,
    #[serde(default = "default_vc_till_backward")]
    pub till_backward: KeyBindings,
    #[serde(default = "default_vc_repeat_motion")]
    pub repeat_motion: KeyBindings,
    #[serde(default = "default_vc_repeat_motion_rev")]
    pub repeat_motion_rev: KeyBindings,
    #[serde(default = "default_vc_filter_include")]
    pub filter_include: KeyBindings,
    #[serde(default = "default_vc_filter_exclude")]
    pub filter_exclude: KeyBindings,
    #[serde(default = "default_vc_search")]
    pub search: KeyBindings,
    #[serde(default = "default_vc_start_selection")]
    pub start_selection: KeyBindings,
    #[serde(default = "default_vc_yank")]
    pub yank: KeyBindings,
    #[serde(default = "default_vc_exit")]
    pub exit: KeyBindings,
}

impl Default for VisualKeybindings {
    fn default() -> Self {
        Self {
            move_left: default_vc_move_left(),
            move_right: default_vc_move_right(),
            word_forward: default_vc_word_forward(),
            word_backward: default_vc_word_backward(),
            word_end: default_vc_word_end(),
            word_forward_big: default_vc_word_forward_big(),
            word_backward_big: default_vc_word_backward_big(),
            word_end_big: default_vc_word_end_big(),
            start_of_line: default_vc_start_of_line(),
            first_nonblank: default_vc_first_nonblank(),
            end_of_line: default_vc_end_of_line(),
            find_forward: default_vc_find_forward(),
            find_backward: default_vc_find_backward(),
            till_forward: default_vc_till_forward(),
            till_backward: default_vc_till_backward(),
            repeat_motion: default_vc_repeat_motion(),
            repeat_motion_rev: default_vc_repeat_motion_rev(),
            filter_include: default_vc_filter_include(),
            filter_exclude: default_vc_filter_exclude(),
            search: default_vc_search(),
            start_selection: default_vc_start_selection(),
            yank: default_vc_yank(),
            exit: default_vc_exit(),
        }
    }
}

#[inline(always)]
fn default_search_cancel() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Esc, KeyModifiers::NONE)])
}
#[inline(always)]
fn default_search_confirm() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Enter, KeyModifiers::NONE)])
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
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

#[inline(always)]
fn default_filter_edit_cancel() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Esc, KeyModifiers::NONE)])
}
#[inline(always)]
fn default_filter_edit_confirm() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Enter, KeyModifiers::NONE)])
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
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

#[inline(always)]
fn default_command_cancel() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Esc, KeyModifiers::NONE)])
}
#[inline(always)]
fn default_command_confirm() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Enter, KeyModifiers::NONE)])
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
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

#[inline(always)]
fn default_docker_confirm() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Enter, KeyModifiers::NONE)])
}
#[inline(always)]
fn default_docker_cancel() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Esc, KeyModifiers::NONE)])
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
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

#[inline(always)]
fn default_dlt_confirm() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Enter, KeyModifiers::NONE)])
}
#[inline(always)]
fn default_dlt_cancel() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Esc, KeyModifiers::NONE)])
}
#[inline(always)]
fn default_dlt_delete() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('d'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_dlt_tab() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Tab, KeyModifiers::NONE)])
}
#[inline(always)]
fn default_dlt_backtab() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::BackTab, KeyModifiers::NONE)])
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DltSelectKeybindings {
    #[serde(default = "default_dlt_confirm")]
    pub confirm: KeyBindings,
    #[serde(default = "default_dlt_cancel")]
    pub cancel: KeyBindings,
    #[serde(default = "default_dlt_delete")]
    pub delete: KeyBindings,
    #[serde(default = "default_dlt_tab")]
    pub next_field: KeyBindings,
    #[serde(default = "default_dlt_backtab")]
    pub prev_field: KeyBindings,
}

impl Default for DltSelectKeybindings {
    fn default() -> Self {
        Self {
            confirm: default_dlt_confirm(),
            cancel: default_dlt_cancel(),
            delete: default_dlt_delete(),
            next_field: default_dlt_tab(),
            prev_field: default_dlt_backtab(),
        }
    }
}

#[inline(always)]
fn default_vc_toggle() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char(' '), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_vc_all() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('a'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_vc_none() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('n'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_vc_apply() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Enter, KeyModifiers::NONE)])
}
#[inline(always)]
fn default_vc_cancel() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Esc, KeyModifiers::NONE)])
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
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

#[inline(always)]
fn default_sf_toggle() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char(' '), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_sf_move_up() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('K'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_sf_move_down() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('J'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_sf_all() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('a'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_sf_none() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('n'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_sf_reset() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('r'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_sf_apply() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Enter, KeyModifiers::NONE)])
}
#[inline(always)]
fn default_sf_cancel() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Esc, KeyModifiers::NONE)])
}
#[inline(always)]
fn default_sf_search() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('/'), KeyModifiers::NONE)])
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
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
    /// Resets the popup's staged fields to the default order with everything
    /// visible — clears both any reorder and any hidden fields. Still
    /// requires `apply` to commit, same as `all`/`none`.
    #[serde(default = "default_sf_reset")]
    pub reset: KeyBindings,
    #[serde(default = "default_sf_apply")]
    pub apply: KeyBindings,
    #[serde(default = "default_sf_cancel")]
    pub cancel: KeyBindings,
    /// Enters typeahead search — used by the archive file picker to narrow
    /// its (potentially deep) file tree. Not read by `select_fields_mode.rs`
    /// or `merge_select_mode.rs`, which also share this keybindings struct.
    #[serde(default = "default_sf_search")]
    pub search: KeyBindings,
}

impl Default for SelectFieldsKeybindings {
    fn default() -> Self {
        Self {
            toggle: default_sf_toggle(),
            move_up: default_sf_move_up(),
            move_down: default_sf_move_down(),
            all: default_sf_all(),
            none: default_sf_none(),
            reset: default_sf_reset(),
            apply: default_sf_apply(),
            cancel: default_sf_cancel(),
            search: default_sf_search(),
        }
    }
}

#[inline(always)]
fn default_ap_toggle() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char(' '), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_ap_merge_toggle() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('m'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_ap_expand() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Right, KeyModifiers::NONE)])
}
#[inline(always)]
fn default_ap_collapse() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Left, KeyModifiers::NONE)])
}
#[inline(always)]
fn default_ap_all() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('a'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_ap_none() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('n'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_ap_apply() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Enter, KeyModifiers::NONE)])
}
#[inline(always)]
fn default_ap_cancel() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Esc, KeyModifiers::NONE)])
}
#[inline(always)]
fn default_ap_search() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('/'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_ap_search_toggle() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('e'), KeyModifiers::CONTROL)])
}
#[inline(always)]
fn default_ap_search_merge_toggle() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('m'), KeyModifiers::ALT)])
}
#[inline(always)]
fn default_ap_search_select_all() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('a'), KeyModifiers::CONTROL)])
}
#[inline(always)]
fn default_ap_search_merge_all() -> KeyBindings {
    KeyBindings(vec![KeyBinding(
        KeyCode::Char('m'),
        KeyModifiers::CONTROL | KeyModifiers::ALT,
    )])
}

/// Keybindings for the archive picker popup (file tree shown when opening
/// an archive). Deliberately its own struct rather than sharing
/// `SelectFieldsKeybindings` (which the popup used to borrow wholesale) —
/// that struct is also used by the unrelated `:select-fields` column
/// picker and `:merge`'s tab-picker, and an archive-only action like
/// `merge_toggle` would otherwise leak into their keybinding help/schema.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ArchivePickerKeybindings {
    /// Toggles a file (or a container's whole subtree) for extraction —
    /// each toggled file opens as its own tab on `apply`.
    #[serde(default = "default_ap_toggle")]
    pub toggle: KeyBindings,
    /// Toggles a file (or a container's whole subtree) to be merged into
    /// one timestamp-sorted tab on `apply`, independently of `toggle`.
    #[serde(default = "default_ap_merge_toggle")]
    pub merge_toggle: KeyBindings,
    /// On a not-yet-read nested archive, fetches and reveals its contents.
    /// On an already-fetched but folded container, reveals its children
    /// again without re-fetching.
    #[serde(default = "default_ap_expand")]
    pub expand: KeyBindings,
    /// Folds an expanded container's children out of view (the already-
    /// fetched data is kept, not discarded).
    #[serde(default = "default_ap_collapse")]
    pub collapse: KeyBindings,
    #[serde(default = "default_ap_all")]
    pub all: KeyBindings,
    #[serde(default = "default_ap_none")]
    pub none: KeyBindings,
    #[serde(default = "default_ap_apply")]
    pub apply: KeyBindings,
    #[serde(default = "default_ap_cancel")]
    pub cancel: KeyBindings,
    #[serde(default = "default_ap_search")]
    pub search: KeyBindings,
    /// Toggles the selected row for extraction while [`ArchivePickerMode`]
    /// is in search-typeahead mode — `toggle`'s key (`Space` by default)
    /// can't be reused there since it's needed as literal query text.
    ///
    /// [`ArchivePickerMode`]: crate::mode::archive_picker_mode::ArchivePickerMode
    #[serde(default = "default_ap_search_toggle")]
    pub search_toggle: KeyBindings,
    /// Toggles the selected row's merge mark while searching — same
    /// reasoning as `search_toggle`, for `merge_toggle`'s key (`m`).
    ///
    /// Defaults to `Alt+m`, deliberately not a `Ctrl`-chord: outside an
    /// enhanced-keyboard terminal protocol, Ctrl+M and Enter send the same
    /// carriage-return byte, so crossterm reports Ctrl+M as a plain
    /// `KeyCode::Enter` and a `Ctrl+m` binding here would never fire.
    #[serde(default = "default_ap_search_merge_toggle")]
    pub search_merge_toggle: KeyBindings,
    /// Marks every row whose name currently matches the search query for
    /// extraction — unlike `search_toggle` (which only touches the
    /// selected row), this sweeps every match in one press so files
    /// sharing a prefix can all be picked from a single query.
    #[serde(default = "default_ap_search_select_all")]
    pub search_select_all: KeyBindings,
    /// Merge-mark equivalent of `search_select_all`. Defaults to
    /// `Ctrl+Alt+m` — a superset of `search_merge_toggle`'s `Alt+m` rather
    /// than an unrelated key, so the two stay mnemonically paired (`m` for
    /// merge, an extra modifier for "all"). [`ArchivePickerMode`] checks
    /// this binding *before* `search_merge_toggle`'s: [`KeyBinding::matches`]
    /// only requires a binding's own modifiers to be held, so an
    /// Alt-only binding also matches a Ctrl+Alt event, and checking the more
    /// specific one first is what keeps `Ctrl+Alt+m` from being swallowed by
    /// `search_merge_toggle`.
    #[serde(default = "default_ap_search_merge_all")]
    pub search_merge_all: KeyBindings,
}

impl Default for ArchivePickerKeybindings {
    fn default() -> Self {
        Self {
            toggle: default_ap_toggle(),
            merge_toggle: default_ap_merge_toggle(),
            expand: default_ap_expand(),
            collapse: default_ap_collapse(),
            all: default_ap_all(),
            none: default_ap_none(),
            apply: default_ap_apply(),
            cancel: default_ap_cancel(),
            search: default_ap_search(),
            search_toggle: default_ap_search_toggle(),
            search_merge_toggle: default_ap_search_merge_toggle(),
            search_select_all: default_ap_search_select_all(),
            search_merge_all: default_ap_search_merge_all(),
        }
    }
}

#[inline(always)]
fn default_clear_group_style() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('x'), KeyModifiers::NONE)])
}
#[inline(always)]
fn default_add_group() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('a'), KeyModifiers::NONE)])
}

/// Keybindings scoped to `GroupManagementMode` beyond what it already reuses
/// from `filter`/`navigation` (see `filter_mode.rs`'s doc comment on why
/// those are shared rather than duplicated). "Clear style" has no sensible
/// existing key to borrow — `filter.clear_all_filters` means something
/// entirely different (wipe every filter) — so it gets this one dedicated
/// field instead.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GroupKeybindings {
    #[serde(default = "default_clear_group_style")]
    pub clear_group_style: KeyBindings,
    #[serde(default = "default_add_group")]
    pub add_group: KeyBindings,
}

impl Default for GroupKeybindings {
    fn default() -> Self {
        Self {
            clear_group_style: default_clear_group_style(),
            add_group: default_add_group(),
        }
    }
}

#[inline(always)]
fn default_help_close() -> KeyBindings {
    KeyBindings(vec![
        KeyBinding(KeyCode::Char('q'), KeyModifiers::NONE),
        KeyBinding(KeyCode::Esc, KeyModifiers::NONE),
    ])
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
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

#[inline(always)]
fn default_confirm_yes() -> KeyBindings {
    KeyBindings(vec![
        KeyBinding(KeyCode::Char('y'), KeyModifiers::NONE),
        KeyBinding(KeyCode::Enter, KeyModifiers::NONE),
    ])
}
#[inline(always)]
fn default_confirm_no() -> KeyBindings {
    KeyBindings(vec![
        KeyBinding(KeyCode::Char('n'), KeyModifiers::NONE),
        KeyBinding(KeyCode::Esc, KeyModifiers::NONE),
    ])
}
#[inline(always)]
fn default_confirm_always() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('Y'), KeyModifiers::SHIFT)])
}
#[inline(always)]
fn default_confirm_never() -> KeyBindings {
    KeyBindings(vec![KeyBinding(KeyCode::Char('N'), KeyModifiers::SHIFT)])
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ConfirmKeybindings {
    #[serde(default = "default_confirm_yes")]
    pub yes: KeyBindings,
    #[serde(default = "default_confirm_no")]
    pub no: KeyBindings,
    #[serde(default = "default_confirm_always")]
    pub always: KeyBindings,
    #[serde(default = "default_confirm_never")]
    pub never: KeyBindings,
}

impl Default for ConfirmKeybindings {
    fn default() -> Self {
        Self {
            yes: default_confirm_yes(),
            no: default_confirm_no(),
            always: default_confirm_always(),
            never: default_confirm_never(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct UiKeybindings {
    #[serde(default = "default_toggle_sidebar")]
    pub toggle_sidebar: KeyBindings,
    #[serde(default = "default_toggle_mode_bar")]
    pub toggle_mode_bar: KeyBindings,
    #[serde(default = "default_toggle_borders")]
    pub toggle_borders: KeyBindings,
    #[serde(default = "default_toggle_wrap")]
    pub toggle_wrap: KeyBindings,
    #[serde(default = "default_toggle_relative_line_numbers")]
    pub toggle_relative_line_numbers: KeyBindings,
    #[serde(default = "default_toggle_groups_panel")]
    pub toggle_groups_panel: KeyBindings,
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
            toggle_relative_line_numbers: default_toggle_relative_line_numbers(),
            toggle_groups_panel: default_toggle_groups_panel(),
            exit: default_ui_exit(),
        }
    }
}

/// A user-defined keybinding that runs a fixed command line when pressed —
/// e.g. binding a key to `load-filters ~/logs/filters/draco-mars.json`.
/// Checked in Normal Mode, ahead of every built-in action, so a custom
/// binding can deliberately override one (`Keybindings::validate` warns
/// about the collision at startup, but doesn't block it).
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CustomCommandBinding {
    /// Key that triggers this command.
    pub key: KeyBindings,
    /// Full command to run, exactly as typed after `:` in command mode
    /// (e.g. `"load-filters ~/logs/filters/draco-mars.json"` — no leading `:`).
    pub command: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Keybindings {
    #[serde(default)]
    pub navigation: NavigationKeybindings,
    #[serde(default)]
    pub normal: NormalKeybindings,
    #[serde(default)]
    pub filter: FilterKeybindings,
    #[serde(default)]
    pub group: GroupKeybindings,
    #[serde(default)]
    pub global: GlobalKeybindings,
    #[serde(default)]
    pub comment: CommentKeybindings,
    #[serde(default)]
    pub visual_line: VisualLineKeybindings,
    #[serde(default)]
    pub visual: VisualKeybindings,
    #[serde(default)]
    pub search: SearchKeybindings,
    #[serde(default)]
    pub filter_edit: FilterEditKeybindings,
    #[serde(default)]
    pub command: CommandModeKeybindings,
    #[serde(default)]
    pub docker_select: DockerSelectKeybindings,
    #[serde(default)]
    pub dlt_select: DltSelectKeybindings,
    #[serde(default)]
    pub value_colors: ValueColorsKeybindings,
    #[serde(default)]
    pub select_fields: SelectFieldsKeybindings,
    #[serde(default)]
    pub archive_picker: ArchivePickerKeybindings,
    #[serde(default)]
    pub help: HelpKeybindings,
    #[serde(default)]
    pub confirm: ConfirmKeybindings,
    #[serde(default)]
    pub ui: UiKeybindings,
    /// User-defined keybindings that run a fixed command line — see
    /// [`CustomCommandBinding`].
    #[serde(default)]
    pub custom: Vec<CustomCommandBinding>,
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
            ("normal.start_of_line", &self.normal.start_of_line),
            ("normal.end_of_line", &self.normal.end_of_line),
            ("normal.command_mode", &self.normal.command_mode),
            ("normal.filter_mode", &self.normal.filter_mode),
            ("normal.group_mode", &self.normal.group_mode),
            ("normal.toggle_filtering", &self.normal.toggle_filtering),
            (
                "normal.toggle_highlight_mode",
                &self.normal.toggle_highlight_mode,
            ),
            ("normal.enter_ui_mode", &self.normal.enter_ui_mode),
            ("normal.filter_include", &self.normal.filter_include),
            (
                "normal.filter_include_auto",
                &self.normal.filter_include_auto,
            ),
            ("normal.filter_exclude", &self.normal.filter_exclude),
            ("normal.go_to_top_chord", &self.normal.go_to_top_chord),
            ("normal.go_to_bottom", &self.normal.go_to_bottom),
            ("normal.mark_line", &self.normal.mark_line),
            (
                "normal.expand_continuation",
                &self.normal.expand_continuation,
            ),
            (
                "normal.collapse_continuation",
                &self.normal.collapse_continuation,
            ),
            ("normal.toggle_marks_only", &self.normal.toggle_marks_only),
            ("normal.yank_line", &self.normal.yank_line),
            ("normal.yank_marked", &self.normal.yank_marked),
            ("normal.visual_mode", &self.normal.visual_mode),
            ("normal.visual_char", &self.normal.visual_char),
            ("normal.search_forward", &self.normal.search_forward),
            ("normal.search_backward", &self.normal.search_backward),
            ("normal.next_match", &self.normal.next_match),
            ("normal.prev_match", &self.normal.prev_match),
            ("normal.show_keybindings", &self.normal.show_keybindings),
            ("normal.clear_all", &self.normal.clear_all),
            ("normal.edit_comment", &self.normal.edit_comment),
            ("normal.delete_comment", &self.normal.delete_comment),
            ("normal.comment_line", &self.normal.comment_line),
            ("normal.next_error", &self.normal.next_error),
            ("normal.prev_error", &self.normal.prev_error),
            ("normal.next_warning", &self.normal.next_warning),
            ("normal.prev_warning", &self.normal.prev_warning),
            ("global.quit", &self.global.quit),
            ("global.next_tab", &self.global.next_tab),
            ("global.prev_tab", &self.global.prev_tab),
            ("global.close_tab", &self.global.close_tab),
            ("global.new_tab", &self.global.new_tab),
            ("global.file_switcher", &self.global.file_switcher),
        ];

        let filter_actions: &[(&str, &KeyBindings)] = &[
            ("navigation.scroll_down", &nav.scroll_down),
            ("navigation.scroll_up", &nav.scroll_up),
            ("navigation.half_page_down", &nav.half_page_down),
            ("navigation.half_page_up", &nav.half_page_up),
            ("navigation.page_down", &nav.page_down),
            ("navigation.page_up", &nav.page_up),
            ("normal.go_to_top_chord", &self.normal.go_to_top_chord),
            ("normal.go_to_bottom", &self.normal.go_to_bottom),
            ("filter.toggle_filter", &self.filter.toggle_filter),
            ("filter.delete_filter", &self.filter.delete_filter),
            ("filter.move_filter_up", &self.filter.move_filter_up),
            ("filter.move_filter_down", &self.filter.move_filter_down),
            ("filter.edit_filter", &self.filter.edit_filter),
            ("filter.set_color", &self.filter.set_color),
            ("filter.toggle_all_filters", &self.filter.toggle_all_filters),
            ("filter.clear_all_filters", &self.filter.clear_all_filters),
            ("filter.add_include_filter", &self.filter.add_include_filter),
            (
                "filter.add_include_filter_auto",
                &self.filter.add_include_filter_auto,
            ),
            ("filter.add_exclude_filter", &self.filter.add_exclude_filter),
            ("filter.add_date_filter", &self.filter.add_date_filter),
            (
                "filter.add_highlight_filter",
                &self.filter.add_highlight_filter,
            ),
            ("filter.search", &self.filter.search),
            ("filter.exit_mode", &self.filter.exit_mode),
            ("global.quit", &self.global.quit),
            ("global.next_tab", &self.global.next_tab),
            ("global.prev_tab", &self.global.prev_tab),
            ("global.file_switcher", &self.global.file_switcher),
        ];

        let group_actions: &[(&str, &KeyBindings)] = &[
            ("navigation.scroll_down", &nav.scroll_down),
            ("navigation.scroll_up", &nav.scroll_up),
            ("filter.toggle_all_filters", &self.filter.toggle_all_filters),
            ("filter.edit_filter", &self.filter.edit_filter),
            ("filter.exit_mode", &self.filter.exit_mode),
            ("group.clear_group_style", &self.group.clear_group_style),
            ("group.add_group", &self.group.add_group),
        ];

        let visual_line_actions: &[(&str, &KeyBindings)] = &[
            ("navigation.scroll_down", &nav.scroll_down),
            ("navigation.scroll_up", &nav.scroll_up),
            ("visual_line.comment", &self.visual_line.comment),
            ("visual_line.yank", &self.visual_line.yank),
            ("visual_line.mark", &self.visual_line.mark),
            ("visual_line.search", &self.visual_line.search),
            ("visual_line.exit", &self.visual_line.exit),
        ];

        let visual_char_actions: &[(&str, &KeyBindings)] = &[
            ("visual.move_left", &self.visual.move_left),
            ("visual.move_right", &self.visual.move_right),
            ("visual.word_forward", &self.visual.word_forward),
            ("visual.word_backward", &self.visual.word_backward),
            ("visual.word_end", &self.visual.word_end),
            ("visual.word_forward_big", &self.visual.word_forward_big),
            ("visual.word_backward_big", &self.visual.word_backward_big),
            ("visual.word_end_big", &self.visual.word_end_big),
            ("visual.start_of_line", &self.visual.start_of_line),
            ("visual.first_nonblank", &self.visual.first_nonblank),
            ("visual.end_of_line", &self.visual.end_of_line),
            ("visual.find_forward", &self.visual.find_forward),
            ("visual.find_backward", &self.visual.find_backward),
            ("visual.till_forward", &self.visual.till_forward),
            ("visual.till_backward", &self.visual.till_backward),
            ("visual.repeat_motion", &self.visual.repeat_motion),
            ("visual.repeat_motion_rev", &self.visual.repeat_motion_rev),
            ("visual.filter_include", &self.visual.filter_include),
            ("visual.filter_exclude", &self.visual.filter_exclude),
            ("visual.search", &self.visual.search),
            ("visual.start_selection", &self.visual.start_selection),
            ("visual.yank", &self.visual.yank),
            ("visual.exit", &self.visual.exit),
            ("navigation.scroll_down", &nav.scroll_down),
            ("navigation.scroll_up", &nav.scroll_up),
            ("navigation.half_page_down", &nav.half_page_down),
            ("navigation.half_page_up", &nav.half_page_up),
            ("navigation.page_down", &nav.page_down),
            ("navigation.page_up", &nav.page_up),
            ("normal.go_to_bottom", &self.normal.go_to_bottom),
            ("normal.go_to_top_chord", &self.normal.go_to_top_chord),
        ];

        let docker_select_actions: &[(&str, &KeyBindings)] = &[
            ("navigation.scroll_down", &nav.scroll_down),
            ("navigation.scroll_up", &nav.scroll_up),
            ("docker_select.confirm", &self.docker_select.confirm),
            ("docker_select.cancel", &self.docker_select.cancel),
        ];

        let dlt_select_actions: &[(&str, &KeyBindings)] = &[
            ("navigation.scroll_down", &nav.scroll_down),
            ("navigation.scroll_up", &nav.scroll_up),
            ("dlt_select.confirm", &self.dlt_select.confirm),
            ("dlt_select.cancel", &self.dlt_select.cancel),
            ("dlt_select.delete", &self.dlt_select.delete),
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
            ("select_fields.reset", &self.select_fields.reset),
            ("select_fields.apply", &self.select_fields.apply),
            ("select_fields.cancel", &self.select_fields.cancel),
            ("select_fields.search", &self.select_fields.search),
        ];

        let archive_picker_actions: &[(&str, &KeyBindings)] = &[
            ("navigation.scroll_down", &nav.scroll_down),
            ("navigation.scroll_up", &nav.scroll_up),
            ("archive_picker.toggle", &self.archive_picker.toggle),
            (
                "archive_picker.merge_toggle",
                &self.archive_picker.merge_toggle,
            ),
            ("archive_picker.expand", &self.archive_picker.expand),
            ("archive_picker.collapse", &self.archive_picker.collapse),
            ("archive_picker.all", &self.archive_picker.all),
            ("archive_picker.none", &self.archive_picker.none),
            ("archive_picker.apply", &self.archive_picker.apply),
            ("archive_picker.cancel", &self.archive_picker.cancel),
            ("archive_picker.search", &self.archive_picker.search),
            (
                "archive_picker.search_toggle",
                &self.archive_picker.search_toggle,
            ),
            (
                "archive_picker.search_merge_toggle",
                &self.archive_picker.search_merge_toggle,
            ),
            (
                "archive_picker.search_select_all",
                &self.archive_picker.search_select_all,
            ),
            (
                "archive_picker.search_merge_all",
                &self.archive_picker.search_merge_all,
            ),
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
            (
                "ui.toggle_relative_line_numbers",
                &self.ui.toggle_relative_line_numbers,
            ),
            ("ui.toggle_groups_panel", &self.ui.toggle_groups_panel),
            ("ui.exit", &self.ui.exit),
            ("global.quit", &self.global.quit),
            ("global.next_tab", &self.global.next_tab),
            ("global.prev_tab", &self.global.prev_tab),
            ("global.file_switcher", &self.global.file_switcher),
        ];

        // Custom command bindings are checked in Normal Mode, so they share
        // that scope's conflict check — appended dynamically since, unlike
        // every other group, `custom` is a user-sized list, not a fixed set
        // of named fields.
        let custom_labels: Vec<String> = self
            .custom
            .iter()
            .enumerate()
            .map(|(i, c)| format!("custom[{i}] ({})", c.command))
            .collect();
        let mut normal_and_custom: Vec<(&str, &KeyBindings)> = normal_actions.to_vec();
        normal_and_custom.extend(
            self.custom
                .iter()
                .zip(custom_labels.iter())
                .map(|(c, label)| (label.as_str(), &c.key)),
        );
        check_conflicts(&normal_and_custom, &mut conflicts);
        check_conflicts(filter_actions, &mut conflicts);
        check_conflicts(group_actions, &mut conflicts);
        check_conflicts(visual_line_actions, &mut conflicts);
        check_conflicts(visual_char_actions, &mut conflicts);
        check_conflicts(docker_select_actions, &mut conflicts);
        check_conflicts(dlt_select_actions, &mut conflicts);
        check_conflicts(value_colors_actions, &mut conflicts);
        check_conflicts(select_fields_actions, &mut conflicts);
        check_conflicts(archive_picker_actions, &mut conflicts);
        check_conflicts(help_actions, &mut conflicts);
        check_conflicts(ui_actions, &mut conflicts);

        conflicts
    }
}

/// Check all pairs in `actions` for overlapping key bindings and append
/// human-readable descriptions to `out`.
pub(super) fn check_conflicts(actions: &[(&str, &KeyBindings)], out: &mut Vec<String>) {
    for i in 0..actions.len() {
        for j in (i + 1)..actions.len() {
            let (name_a, kb_a) = actions[i];
            let (name_b, kb_b) = actions[j];
            if kb_a.has_overlap(kb_b) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};

    fn kb(key: KeyCode) -> KeyBindings {
        KeyBindings(vec![KeyBinding(key, KeyModifiers::NONE)])
    }

    fn kb2(key1: KeyCode, key2: KeyCode) -> KeyBindings {
        KeyBindings(vec![
            KeyBinding(key1, KeyModifiers::NONE),
            KeyBinding(key2, KeyModifiers::NONE),
        ])
    }

    #[test]
    fn test_keybindings_matches_any() {
        let bindings = kb2(KeyCode::Char('j'), KeyCode::Down);
        assert!(bindings.matches(KeyCode::Char('j'), KeyModifiers::NONE));
        assert!(bindings.matches(KeyCode::Down, KeyModifiers::NONE));
        assert!(!bindings.matches(KeyCode::Char('k'), KeyModifiers::NONE));
    }

    #[test]
    fn test_keybindings_empty_never_matches() {
        assert!(!KeyBindings(vec![]).matches(KeyCode::Char('j'), KeyModifiers::NONE));
    }

    #[test]
    fn test_keybindings_display_single() {
        assert_eq!(kb(KeyCode::Char('j')).display(), "j");
    }

    #[test]
    fn test_keybindings_display_multiple() {
        assert_eq!(kb2(KeyCode::Char('j'), KeyCode::Down).display(), "j/Down");
    }

    #[test]
    fn test_keybindings_display_empty() {
        assert_eq!(KeyBindings(vec![]).display(), "");
    }

    #[test]
    fn test_has_overlap_true() {
        let a = kb2(KeyCode::Char('j'), KeyCode::Down);
        let b = kb(KeyCode::Down);
        assert!(a.has_overlap(&b));
    }

    #[test]
    fn test_has_overlap_false() {
        assert!(!kb(KeyCode::Char('j')).has_overlap(&kb(KeyCode::Char('k'))));
    }

    #[test]
    fn test_has_overlap_empty() {
        assert!(!KeyBindings(vec![]).has_overlap(&kb(KeyCode::Char('j'))));
    }

    #[test]
    fn test_keybindings_deserialize_single_string() {
        let bindings: KeyBindings = serde_json::from_str(r#""j""#).unwrap();
        assert_eq!(
            bindings.0,
            vec![KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE)]
        );
    }

    #[test]
    fn test_keybindings_deserialize_array() {
        let bindings: KeyBindings = serde_json::from_str(r#"["j", "Down"]"#).unwrap();
        assert_eq!(
            bindings.0,
            vec![
                KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE),
                KeyBinding(KeyCode::Down, KeyModifiers::NONE),
            ]
        );
    }

    #[test]
    fn test_keybindings_serialize_single_as_string() {
        let json = serde_json::to_string(&kb(KeyCode::Char('j'))).unwrap();
        assert_eq!(json, r#""j""#);
    }

    #[test]
    fn test_keybindings_serialize_multiple_as_array() {
        let json = serde_json::to_string(&kb2(KeyCode::Char('j'), KeyCode::Down)).unwrap();
        assert_eq!(json, r#"["j","Down"]"#);
    }

    #[test]
    fn test_check_conflicts_detects_overlap() {
        let a = kb(KeyCode::Char('j'));
        let b = kb(KeyCode::Char('j'));
        let actions: &[(&str, &KeyBindings)] = &[("action.a", &a), ("action.b", &b)];
        let mut out = Vec::new();
        check_conflicts(actions, &mut out);
        assert_eq!(out.len(), 1);
        assert!(out[0].contains("action.a"));
        assert!(out[0].contains("action.b"));
        assert!(out[0].contains("'j'"));
    }

    #[test]
    fn test_check_conflicts_no_overlap() {
        let a = kb(KeyCode::Char('j'));
        let b = kb(KeyCode::Char('k'));
        let actions: &[(&str, &KeyBindings)] = &[("action.a", &a), ("action.b", &b)];
        let mut out = Vec::new();
        check_conflicts(actions, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn test_default_keybindings_no_conflicts() {
        let conflicts = Keybindings::default().validate();
        assert!(
            conflicts.is_empty(),
            "Default keybindings have conflicts: {:?}",
            conflicts
        );
    }

    #[test]
    fn test_default_normal_toggle_highlight_mode_is_shift_h() {
        let nk = NormalKeybindings::default();
        assert!(
            nk.toggle_highlight_mode
                .matches(KeyCode::Char('H'), KeyModifiers::NONE)
        );
    }

    #[test]
    fn test_default_filter_add_highlight_is_h() {
        let fk = FilterKeybindings::default();
        assert!(
            fk.add_highlight_filter
                .matches(KeyCode::Char('h'), KeyModifiers::NONE)
        );
    }

    #[test]
    fn test_validate_detects_conflict_if_highlight_bindings_collide() {
        let mut kb = Keybindings::default();
        kb.normal.toggle_highlight_mode = kb.filter.add_highlight_filter.clone();
        let conflicts = kb.validate();
        assert!(
            !conflicts.is_empty(),
            "rebinding both to the same key must be reported as a conflict"
        );
    }

    #[test]
    fn test_default_filter_search_is_slash() {
        let fk = FilterKeybindings::default();
        assert!(fk.search.matches(KeyCode::Char('/'), KeyModifiers::NONE));
    }

    #[test]
    fn test_validate_detects_conflict_if_search_binding_collides_with_filter_action() {
        let mut kb = Keybindings::default();
        kb.filter.search = kb.filter.edit_filter.clone();
        let conflicts = kb.validate();
        assert!(
            !conflicts.is_empty(),
            "rebinding filter.search onto an existing filter-mode action's key must be reported as a conflict"
        );
    }

    #[test]
    fn test_validate_detects_conflict_if_search_collides_with_gg_chord() {
        let mut kb = Keybindings::default();
        kb.filter.search = kb.normal.go_to_top_chord.clone();
        let conflicts = kb.validate();
        assert!(
            !conflicts.is_empty(),
            "filter mode now also consumes normal.go_to_top_chord — colliding filter.search with it must be reported"
        );
    }

    #[test]
    fn test_default_select_fields_search_is_slash() {
        let sf = SelectFieldsKeybindings::default();
        assert!(sf.search.matches(KeyCode::Char('/'), KeyModifiers::NONE));
    }

    #[test]
    fn test_validate_detects_conflict_if_select_fields_search_collides_with_toggle() {
        let mut kb = Keybindings::default();
        kb.select_fields.search = kb.select_fields.toggle.clone();
        let conflicts = kb.validate();
        assert!(
            !conflicts.is_empty(),
            "rebinding select_fields.search onto an existing select-fields action's key must be reported as a conflict"
        );
    }

    #[test]
    fn test_validate_detects_conflict_if_select_fields_reset_collides_with_all() {
        let mut kb = Keybindings::default();
        kb.select_fields.reset = kb.select_fields.all.clone();
        let conflicts = kb.validate();
        assert!(
            !conflicts.is_empty(),
            "rebinding select_fields.reset onto an existing select-fields action's key must be reported as a conflict"
        );
    }

    #[test]
    fn test_archive_picker_keybindings_default() {
        let ap = ArchivePickerKeybindings::default();
        assert!(ap.toggle.matches(KeyCode::Char(' '), KeyModifiers::NONE));
        assert!(
            ap.merge_toggle
                .matches(KeyCode::Char('m'), KeyModifiers::NONE)
        );
        assert!(ap.expand.matches(KeyCode::Right, KeyModifiers::NONE));
        assert!(ap.collapse.matches(KeyCode::Left, KeyModifiers::NONE));
        assert!(ap.all.matches(KeyCode::Char('a'), KeyModifiers::NONE));
        assert!(ap.none.matches(KeyCode::Char('n'), KeyModifiers::NONE));
        assert!(ap.apply.matches(KeyCode::Enter, KeyModifiers::NONE));
        assert!(ap.cancel.matches(KeyCode::Esc, KeyModifiers::NONE));
        assert!(ap.search.matches(KeyCode::Char('/'), KeyModifiers::NONE));
        assert!(
            ap.search_toggle
                .matches(KeyCode::Char('e'), KeyModifiers::CONTROL)
        );
        assert!(
            ap.search_merge_toggle
                .matches(KeyCode::Char('m'), KeyModifiers::ALT)
        );
        assert!(
            ap.search_select_all
                .matches(KeyCode::Char('a'), KeyModifiers::CONTROL)
        );
        assert!(ap.search_merge_all.matches(
            KeyCode::Char('m'),
            KeyModifiers::CONTROL | KeyModifiers::ALT
        ));
    }

    #[test]
    fn test_validate_detects_conflict_if_archive_picker_search_toggle_collides_with_search_merge_toggle()
     {
        let mut kb = Keybindings::default();
        kb.archive_picker.search_toggle = kb.archive_picker.search_merge_toggle.clone();
        let conflicts = kb.validate();
        assert!(
            !conflicts.is_empty(),
            "rebinding archive_picker.search_toggle onto archive_picker.search_merge_toggle's key must be reported as a conflict"
        );
    }

    #[test]
    fn test_validate_detects_conflict_if_archive_picker_search_select_all_collides_with_search_toggle()
     {
        let mut kb = Keybindings::default();
        kb.archive_picker.search_select_all = kb.archive_picker.search_toggle.clone();
        let conflicts = kb.validate();
        assert!(
            !conflicts.is_empty(),
            "rebinding archive_picker.search_select_all onto archive_picker.search_toggle's key must be reported as a conflict"
        );
    }

    #[test]
    fn test_archive_picker_search_merge_all_default_does_not_collide_with_search_merge_toggle() {
        // Alt+m vs Ctrl+Alt+m — different modifiers, must not be flagged
        // even though both are bound to the 'm' character.
        let kb = Keybindings::default();
        let conflicts = kb.validate();
        assert!(
            !conflicts
                .iter()
                .any(|c| c.contains("archive_picker.search_merge_all")),
            "Alt+m and Ctrl+Alt+m must not conflict: {conflicts:?}"
        );
    }

    #[test]
    fn test_archive_picker_search_merge_defaults_do_not_collide_with_enter() {
        // A physical Ctrl+M keypress is delivered as a plain `Enter` key
        // event by terminals that don't support an enhanced keyboard
        // protocol — see the doc comments on
        // `ArchivePickerKeybindings::search_merge_toggle`/`search_merge_all`.
        // Neither default may be a bare `Ctrl+m`, or it could never fire.
        let kb = Keybindings::default();
        assert!(
            !kb.archive_picker
                .search_merge_toggle
                .matches(KeyCode::Enter, KeyModifiers::NONE),
            "search_merge_toggle's default must not match what a physical Ctrl+M keypress \
             is actually reported as"
        );
        assert!(
            !kb.archive_picker
                .search_merge_all
                .matches(KeyCode::Enter, KeyModifiers::NONE),
            "search_merge_all's default must not match what a physical Ctrl+M keypress \
             is actually reported as"
        );
        assert!(
            !kb.archive_picker
                .search_merge_toggle
                .matches(KeyCode::Char('m'), KeyModifiers::CONTROL),
            "search_merge_toggle's default must not be a bare Ctrl+m"
        );
        assert!(
            !kb.archive_picker
                .search_merge_all
                .matches(KeyCode::Char('m'), KeyModifiers::CONTROL),
            "search_merge_all's default must require Alt too, not bare Ctrl+m"
        );
    }

    #[test]
    fn test_validate_detects_conflict_if_archive_picker_merge_toggle_collides_with_toggle() {
        let mut kb = Keybindings::default();
        kb.archive_picker.merge_toggle = kb.archive_picker.toggle.clone();
        let conflicts = kb.validate();
        assert!(
            !conflicts.is_empty(),
            "rebinding archive_picker.merge_toggle onto an existing archive-picker action's key must be reported as a conflict"
        );
    }

    #[test]
    fn test_validate_detects_conflict_if_archive_picker_expand_collides_with_collapse() {
        let mut kb = Keybindings::default();
        kb.archive_picker.expand = kb.archive_picker.collapse.clone();
        let conflicts = kb.validate();
        assert!(
            !conflicts.is_empty(),
            "rebinding archive_picker.expand onto archive_picker.collapse's key must be reported as a conflict"
        );
    }

    #[test]
    fn test_custom_binding_defaults_to_empty() {
        let kb = Keybindings::default();
        assert!(kb.custom.is_empty());
    }

    #[test]
    fn test_validate_detects_conflict_if_custom_binding_collides_with_normal_action() {
        let mut kb = Keybindings::default();
        kb.custom.push(CustomCommandBinding {
            key: kb.normal.filter_include.clone(),
            command: "load-filters ~/logs/filters/draco-mars.json".to_string(),
        });
        let conflicts = kb.validate();
        assert!(
            !conflicts.is_empty(),
            "a custom binding reusing normal.filter_include's key must be reported as a conflict"
        );
    }

    #[test]
    fn test_validate_detects_conflict_between_two_custom_bindings() {
        let mut kb = Keybindings::default();
        let key = KeyBindings(vec![KeyBinding(KeyCode::F(2), KeyModifiers::NONE)]);
        kb.custom.push(CustomCommandBinding {
            key: key.clone(),
            command: "load-filters a.json".to_string(),
        });
        kb.custom.push(CustomCommandBinding {
            key,
            command: "load-filters b.json".to_string(),
        });
        let conflicts = kb.validate();
        assert!(
            !conflicts.is_empty(),
            "two custom bindings sharing the same key must be reported as a conflict"
        );
    }

    #[test]
    fn test_validate_does_not_flag_custom_binding_on_an_unused_key() {
        let mut kb = Keybindings::default();
        kb.custom.push(CustomCommandBinding {
            key: KeyBindings(vec![KeyBinding(KeyCode::F(2), KeyModifiers::NONE)]),
            command: "load-filters ~/logs/filters/draco-mars.json".to_string(),
        });
        let conflicts = kb.validate();
        assert!(
            !conflicts.iter().any(|c| c.contains("custom[0]")),
            "an unused key should not be reported as a conflict: {conflicts:?}"
        );
    }

    #[test]
    fn test_validate_does_not_conflict_archive_picker_with_select_fields() {
        let mut kb = Keybindings::default();
        // Rebind archive_picker.toggle onto whatever key select_fields.toggle
        // uses (they happen to share the same default, Space, but that's
        // exactly the point — the two scopes are independent now, so this
        // must not be reported as a conflict).
        kb.archive_picker.toggle = kb.select_fields.toggle.clone();
        let conflicts = kb.validate();
        assert!(
            !conflicts
                .iter()
                .any(|c| c.contains("archive_picker.toggle") && c.contains("select_fields")),
            "archive_picker and select_fields must be independent scopes: {conflicts:?}"
        );
    }

    #[test]
    fn test_keybindings_deserialize_without_highlight_fields_uses_defaults() {
        let nk: NormalKeybindings = serde_json::from_str("{}").unwrap();
        assert!(
            nk.toggle_highlight_mode
                .matches(KeyCode::Char('H'), KeyModifiers::NONE)
        );
        let fk: FilterKeybindings = serde_json::from_str("{}").unwrap();
        assert!(
            fk.add_highlight_filter
                .matches(KeyCode::Char('h'), KeyModifiers::NONE)
        );
    }

    #[test]
    fn test_default_scroll_bindings() {
        let nav = NavigationKeybindings::default();
        assert!(
            nav.scroll_down
                .matches(KeyCode::Char('j'), KeyModifiers::NONE)
        );
        assert!(nav.scroll_down.matches(KeyCode::Down, KeyModifiers::NONE));
        assert!(
            nav.scroll_up
                .matches(KeyCode::Char('k'), KeyModifiers::NONE)
        );
        assert!(nav.scroll_up.matches(KeyCode::Up, KeyModifiers::NONE));
    }
}
