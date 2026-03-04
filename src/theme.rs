use anyhow::Context;
use ratatui::style::Color;
use serde::de::Error;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::str::FromStr;

use crate::auto_complete::fuzzy_match;

/// Themes embedded into the binary at compile time.
/// Lookup order: user config dir → local `themes/` (dev) → here.
static BUNDLED_THEMES: &[(&str, &str)] = &[
    ("atomic", include_str!("../themes/atomic.json")),
    ("dracula", include_str!("../themes/dracula.json")),
    ("gruvbox-dark", include_str!("../themes/gruvbox-dark.json")),
    ("jandedobbeleer", include_str!("../themes/jandedobbeleer.json")),
    ("monokai", include_str!("../themes/monokai.json")),
    ("nord", include_str!("../themes/nord.json")),
    ("paradox", include_str!("../themes/paradox.json")),
    ("solarized", include_str!("../themes/solarized.json")),
    ("tokyonight", include_str!("../themes/tokyonight.json")),
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValueColors {
    #[serde(
        serialize_with = "color_to_str",
        deserialize_with = "color_from_str",
        default = "default_http_get"
    )]
    pub http_get: Color,
    #[serde(
        serialize_with = "color_to_str",
        deserialize_with = "color_from_str",
        default = "default_http_post"
    )]
    pub http_post: Color,
    #[serde(
        serialize_with = "color_to_str",
        deserialize_with = "color_from_str",
        default = "default_http_put"
    )]
    pub http_put: Color,
    #[serde(
        serialize_with = "color_to_str",
        deserialize_with = "color_from_str",
        default = "default_http_delete"
    )]
    pub http_delete: Color,
    #[serde(
        serialize_with = "color_to_str",
        deserialize_with = "color_from_str",
        default = "default_http_patch"
    )]
    pub http_patch: Color,
    #[serde(
        serialize_with = "color_to_str",
        deserialize_with = "color_from_str",
        default = "default_http_other"
    )]
    pub http_other: Color,
    #[serde(
        serialize_with = "color_to_str",
        deserialize_with = "color_from_str",
        default = "default_status_2xx"
    )]
    pub status_2xx: Color,
    #[serde(
        serialize_with = "color_to_str",
        deserialize_with = "color_from_str",
        default = "default_status_3xx"
    )]
    pub status_3xx: Color,
    #[serde(
        serialize_with = "color_to_str",
        deserialize_with = "color_from_str",
        default = "default_status_4xx"
    )]
    pub status_4xx: Color,
    #[serde(
        serialize_with = "color_to_str",
        deserialize_with = "color_from_str",
        default = "default_status_5xx"
    )]
    pub status_5xx: Color,
    #[serde(
        serialize_with = "color_to_str",
        deserialize_with = "color_from_str",
        default = "default_ip_address"
    )]
    pub ip_address: Color,
    #[serde(
        serialize_with = "color_to_str",
        deserialize_with = "color_from_str",
        default = "default_uuid"
    )]
    pub uuid: Color,
    /// Runtime-only set of disabled category keys (not serialized to theme JSON).
    #[serde(skip)]
    pub disabled: HashSet<String>,
}

fn default_http_get() -> Color {
    Color::Rgb(80, 250, 123)
}
fn default_http_post() -> Color {
    Color::Rgb(139, 233, 253)
}
fn default_http_put() -> Color {
    Color::Rgb(255, 184, 108)
}
fn default_http_delete() -> Color {
    Color::Rgb(255, 85, 85)
}
fn default_http_patch() -> Color {
    Color::Rgb(189, 147, 249)
}
fn default_http_other() -> Color {
    Color::Rgb(98, 114, 164)
}
fn default_status_2xx() -> Color {
    Color::Rgb(80, 250, 123)
}
fn default_status_3xx() -> Color {
    Color::Rgb(139, 233, 253)
}
fn default_status_4xx() -> Color {
    Color::Rgb(255, 184, 108)
}
fn default_status_5xx() -> Color {
    Color::Rgb(255, 85, 85)
}
fn default_ip_address() -> Color {
    Color::Rgb(189, 147, 249)
}
fn default_uuid() -> Color {
    Color::Rgb(108, 113, 196)
}

impl Default for ValueColors {
    fn default() -> Self {
        ValueColors {
            http_get: default_http_get(),
            http_post: default_http_post(),
            http_put: default_http_put(),
            http_delete: default_http_delete(),
            http_patch: default_http_patch(),
            http_other: default_http_other(),
            status_2xx: default_status_2xx(),
            status_3xx: default_status_3xx(),
            status_4xx: default_status_4xx(),
            status_5xx: default_status_5xx(),
            ip_address: default_ip_address(),
            uuid: default_uuid(),
            disabled: HashSet::new(),
        }
    }
}

/// A group of related value-color categories.
pub struct ValueColorGroup {
    pub label: &'static str,
    pub children: Vec<(&'static str, &'static str, Color)>,
}

impl ValueColors {
    /// Returns categories organised into groups.
    pub fn grouped_categories(&self) -> Vec<ValueColorGroup> {
        vec![
            ValueColorGroup {
                label: "HTTP methods",
                children: vec![
                    ("http_get", "GET", self.http_get),
                    ("http_post", "POST", self.http_post),
                    ("http_put", "PUT", self.http_put),
                    ("http_delete", "DELETE", self.http_delete),
                    ("http_patch", "PATCH", self.http_patch),
                    ("http_other", "HEAD/OPTIONS", self.http_other),
                ],
            },
            ValueColorGroup {
                label: "Status codes",
                children: vec![
                    ("status_2xx", "2xx", self.status_2xx),
                    ("status_3xx", "3xx", self.status_3xx),
                    ("status_4xx", "4xx", self.status_4xx),
                    ("status_5xx", "5xx", self.status_5xx),
                ],
            },
            ValueColorGroup {
                label: "Network",
                children: vec![("ip_address", "IP addresses", self.ip_address)],
            },
            ValueColorGroup {
                label: "Identifiers",
                children: vec![("uuid", "UUIDs", self.uuid)],
            },
        ]
    }

    pub fn is_disabled(&self, key: &str) -> bool {
        self.disabled.contains(key)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Theme {
    #[serde(serialize_with = "color_to_str", deserialize_with = "color_from_str")]
    pub root_bg: Color,
    #[serde(serialize_with = "color_to_str", deserialize_with = "color_from_str")]
    pub border: Color,
    #[serde(serialize_with = "color_to_str", deserialize_with = "color_from_str")]
    pub border_title: Color,
    #[serde(serialize_with = "color_to_str", deserialize_with = "color_from_str")]
    pub text: Color,
    #[serde(serialize_with = "color_to_str", deserialize_with = "color_from_str")]
    pub text_highlight: Color,
    /// Foreground colour used for the currently-selected (cursor) line.
    /// Should contrast well against the `border` colour used as the cursor background.
    #[serde(
        serialize_with = "color_to_str",
        deserialize_with = "color_from_str",
        default = "default_cursor_fg"
    )]
    pub cursor_fg: Color,
    #[serde(
        serialize_with = "color_to_str",
        deserialize_with = "color_from_str",
        default = "default_trace_fg"
    )]
    pub trace_fg: Color,
    #[serde(
        serialize_with = "color_to_str",
        deserialize_with = "color_from_str",
        default = "default_debug_fg"
    )]
    pub debug_fg: Color,
    #[serde(
        serialize_with = "color_to_str",
        deserialize_with = "color_from_str",
        default = "default_notice_fg"
    )]
    pub notice_fg: Color,
    #[serde(serialize_with = "color_to_str", deserialize_with = "color_from_str")]
    pub error_fg: Color,
    #[serde(serialize_with = "color_to_str", deserialize_with = "color_from_str")]
    pub warning_fg: Color,
    #[serde(
        serialize_with = "color_to_str",
        deserialize_with = "color_from_str",
        default = "default_fatal_fg"
    )]
    pub fatal_fg: Color,
    #[serde(
        serialize_with = "color_to_str",
        deserialize_with = "color_from_str",
        default = "default_search_fg"
    )]
    pub search_fg: Color,
    #[serde(
        serialize_with = "color_to_str",
        deserialize_with = "color_from_str",
        default = "default_visual_select_bg"
    )]
    pub visual_select_bg: Color,
    #[serde(
        serialize_with = "color_to_str",
        deserialize_with = "color_from_str",
        default = "default_visual_select_fg"
    )]
    pub visual_select_fg: Color,
    #[serde(
        serialize_with = "color_to_str",
        deserialize_with = "color_from_str",
        default = "default_mark_bg"
    )]
    pub mark_bg: Color,
    #[serde(
        serialize_with = "color_to_str",
        deserialize_with = "color_from_str",
        default = "default_mark_fg"
    )]
    pub mark_fg: Color,
    #[serde(
        serialize_with = "colors_to_str_vec",
        deserialize_with = "colors_from_str_vec"
    )]
    pub process_colors: Vec<Color>,
    #[serde(default)]
    pub value_colors: ValueColors,
}

fn color_to_str<S>(color: &Color, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&color.to_string())
}

fn color_from_str<'de, D>(deserializer: D) -> Result<Color, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct ColorVisitor;

    impl<'de> serde::de::Visitor<'de> for ColorVisitor {
        type Value = Color;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a color string (e.g., \"#RRGGBB\") or an RGB array [u8; 3]")
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Color::from_str(v).map_err(E::custom)
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: serde::de::SeqAccess<'de>,
        {
            let r = seq
                .next_element()?
                .ok_or_else(|| A::Error::invalid_length(0, &self))?;
            let g = seq
                .next_element()?
                .ok_or_else(|| A::Error::invalid_length(1, &self))?;
            let b = seq
                .next_element()?
                .ok_or_else(|| A::Error::invalid_length(2, &self))?;
            Ok(Color::Rgb(r, g, b))
        }
    }

    deserializer.deserialize_any(ColorVisitor)
}

fn colors_to_str_vec<S>(colors: &[Color], serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let strs: Vec<String> = colors.iter().map(|c| c.to_string()).collect();
    strs.serialize(serializer)
}

fn colors_from_str_vec<'de, D>(deserializer: D) -> Result<Vec<Color>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct ColorVecVisitor;

    impl<'de> serde::de::Visitor<'de> for ColorVecVisitor {
        type Value = Vec<Color>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a sequence of color strings or RGB arrays")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: serde::de::SeqAccess<'de>,
        {
            let mut colors = Vec::new();
            while let Some(element) = seq.next_element_seed(ColorDeserializer)? {
                colors.push(element);
            }
            Ok(colors)
        }
    }

    deserializer.deserialize_seq(ColorVecVisitor)
}

struct ColorDeserializer;

impl<'de> serde::de::DeserializeSeed<'de> for ColorDeserializer {
    type Value = Color;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        color_from_str(deserializer)
    }
}

fn default_trace_fg() -> Color {
    Color::Rgb(98, 114, 164) // dimmed/gray (Dracula comment color)
}
fn default_debug_fg() -> Color {
    Color::Rgb(139, 233, 253) // cyan (Dracula cyan)
}
fn default_notice_fg() -> Color {
    Color::Rgb(248, 248, 242) // same as default text (Dracula foreground)
}
fn default_fatal_fg() -> Color {
    Color::Rgb(255, 85, 85) // bright red (same as error, Dracula red)
}
fn default_cursor_fg() -> Color {
    Color::Rgb(28, 28, 28)
}
fn default_search_fg() -> Color {
    Color::Rgb(28, 28, 28)
}
fn default_visual_select_bg() -> Color {
    Color::Rgb(68, 71, 90)
}
fn default_visual_select_fg() -> Color {
    Color::Rgb(248, 248, 242)
}
fn default_mark_bg() -> Color {
    Color::Rgb(70, 60, 15)
}
fn default_mark_fg() -> Color {
    Color::Rgb(248, 248, 242)
}

impl Theme {
    /// Returns the names of all available themes: bundled, local `themes/`, and
    /// `~/.config/logana/themes/`. User-config and local names shadow bundled ones.
    pub fn list_available_themes() -> Vec<String> {
        Self::list_available_themes_from(dirs::config_dir().as_deref())
    }

    fn list_available_themes_from(config_dir: Option<&Path>) -> Vec<String> {
        let mut set: std::collections::HashSet<String> =
            BUNDLED_THEMES.iter().map(|(name, _)| name.to_string()).collect();

        let mut add_from_dir = |dir: &Path| {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.extension().and_then(|e| e.to_str()) == Some("json") {
                        if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                            set.insert(stem.to_string());
                        }
                    }
                }
            }
        };

        add_from_dir(Path::new("themes"));
        if let Some(dir) = config_dir {
            add_from_dir(&dir.join("logana/themes"));
        }

        let mut themes: Vec<String> = set.into_iter().collect();
        themes.sort();
        themes
    }

    pub fn from_file<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        Self::from_file_with_config_dir(path, dirs::config_dir().as_deref())
    }

    fn from_file_with_config_dir<P: AsRef<Path>>(
        path: P,
        config_dir: Option<&Path>,
    ) -> anyhow::Result<Self> {
        let config_path = config_dir.map(|d| d.join("logana").join("themes").join(&path));
        let local_path = Path::new("themes").join(&path);

        let data = if config_path.as_ref().is_some_and(|p| p.exists()) {
            let cp = config_path.unwrap();
            fs::read_to_string(&cp)
                .with_context(|| format!("Failed to read theme from {:?}", cp))?
        } else if local_path.exists() {
            fs::read_to_string(&local_path)
                .with_context(|| format!("Failed to read theme from {:?}", local_path))?
        } else {
            let stem = path
                .as_ref()
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            BUNDLED_THEMES
                .iter()
                .find(|(name, _)| *name == stem)
                .map(|(_, json)| json.to_string())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Theme {:?} not found in config dir, local themes/, or bundled themes",
                        path.as_ref()
                    )
                })?
        };
        let config: Theme = serde_json::from_str(&data)?;
        Ok(config)
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme::from_file("dracula.json").unwrap_or_else(|_| Theme {
            root_bg: Color::Rgb(40, 42, 54),
            border: Color::Rgb(98, 114, 164),
            border_title: Color::Rgb(248, 248, 242),
            text: Color::Rgb(248, 248, 242),
            text_highlight: Color::Rgb(255, 184, 108),
            cursor_fg: Color::Rgb(28, 28, 28),
            trace_fg: default_trace_fg(),
            debug_fg: default_debug_fg(),
            notice_fg: default_notice_fg(),
            error_fg: Color::Rgb(255, 85, 85),
            warning_fg: Color::Rgb(241, 250, 140),
            fatal_fg: default_fatal_fg(),
            search_fg: default_search_fg(),
            visual_select_bg: default_visual_select_bg(),
            visual_select_fg: default_visual_select_fg(),
            mark_bg: default_mark_bg(),
            mark_fg: default_mark_fg(),
            process_colors: vec![
                Color::Rgb(255, 85, 85),
                Color::Rgb(80, 250, 123),
                Color::Rgb(255, 184, 108),
                Color::Rgb(189, 147, 249),
                Color::Rgb(255, 121, 198),
                Color::Rgb(139, 233, 253),
            ],
            value_colors: ValueColors::default(),
        })
    }
}

/// Complete a partial theme name using fuzzy matching against themes found on the filesystem.
pub fn complete_theme(partial: &str) -> Vec<String> {
    let themes = Theme::list_available_themes();
    if partial.is_empty() {
        themes
    } else {
        themes
            .into_iter()
            .filter(|t| fuzzy_match(partial, t))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // ── ValueColors defaults ────────────────────────────────────────────

    #[test]
    fn test_value_colors_default() {
        let vc = ValueColors::default();
        assert_eq!(vc.http_get, Color::Rgb(80, 250, 123));
        assert_eq!(vc.http_post, Color::Rgb(139, 233, 253));
        assert_eq!(vc.http_put, Color::Rgb(255, 184, 108));
        assert_eq!(vc.http_delete, Color::Rgb(255, 85, 85));
        assert_eq!(vc.http_patch, Color::Rgb(189, 147, 249));
        assert_eq!(vc.http_other, Color::Rgb(98, 114, 164));
        assert_eq!(vc.status_2xx, Color::Rgb(80, 250, 123));
        assert_eq!(vc.status_3xx, Color::Rgb(139, 233, 253));
        assert_eq!(vc.status_4xx, Color::Rgb(255, 184, 108));
        assert_eq!(vc.status_5xx, Color::Rgb(255, 85, 85));
        assert_eq!(vc.ip_address, Color::Rgb(189, 147, 249));
        assert_eq!(vc.uuid, Color::Rgb(108, 113, 196));
        assert!(vc.disabled.is_empty());
    }

    // ── ValueColors::grouped_categories ─────────────────────────────────

    #[test]
    fn test_grouped_categories_structure() {
        let vc = ValueColors::default();
        let groups = vc.grouped_categories();
        assert_eq!(groups.len(), 4);
        assert_eq!(groups[0].label, "HTTP methods");
        assert_eq!(groups[0].children.len(), 6);
        assert_eq!(groups[1].label, "Status codes");
        assert_eq!(groups[1].children.len(), 4);
        assert_eq!(groups[2].label, "Network");
        assert_eq!(groups[2].children.len(), 1);
        assert_eq!(groups[3].label, "Identifiers");
        assert_eq!(groups[3].children.len(), 1);
    }

    #[test]
    fn test_grouped_categories_keys_and_labels() {
        let vc = ValueColors::default();
        let groups = vc.grouped_categories();
        // HTTP methods group
        let http = &groups[0].children;
        assert_eq!(http[0].0, "http_get");
        assert_eq!(http[0].1, "GET");
        assert_eq!(http[5].0, "http_other");
        assert_eq!(http[5].1, "HEAD/OPTIONS");
        // Status codes group
        let status = &groups[1].children;
        assert_eq!(status[0].0, "status_2xx");
        assert_eq!(status[3].0, "status_5xx");
        // Network
        assert_eq!(groups[2].children[0].0, "ip_address");
        // Identifiers
        assert_eq!(groups[3].children[0].0, "uuid");
    }

    #[test]
    fn test_grouped_categories_uses_current_colors() {
        let mut vc = ValueColors::default();
        vc.http_get = Color::Rgb(1, 2, 3);
        let groups = vc.grouped_categories();
        assert_eq!(groups[0].children[0].2, Color::Rgb(1, 2, 3));
    }

    // ── ValueColors::is_disabled ────────────────────────────────────────

    #[test]
    fn test_is_disabled_false_by_default() {
        let vc = ValueColors::default();
        assert!(!vc.is_disabled("http_get"));
        assert!(!vc.is_disabled("uuid"));
    }

    #[test]
    fn test_is_disabled_true_when_in_set() {
        let mut vc = ValueColors::default();
        vc.disabled.insert("http_get".to_string());
        assert!(vc.is_disabled("http_get"));
        assert!(!vc.is_disabled("http_post"));
    }

    // ── ValueColors serde ───────────────────────────────────────────────

    #[test]
    fn test_value_colors_serde_roundtrip() {
        let original = ValueColors::default();
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: ValueColors = serde_json::from_str(&json).unwrap();
        assert_eq!(original, deserialized);
    }

    #[test]
    fn test_value_colors_disabled_not_serialized() {
        let mut vc = ValueColors::default();
        vc.disabled.insert("http_get".to_string());
        let json = serde_json::to_string(&vc).unwrap();
        assert!(!json.contains("disabled"));
        let deserialized: ValueColors = serde_json::from_str(&json).unwrap();
        assert!(deserialized.disabled.is_empty());
    }

    #[test]
    fn test_value_colors_partial_json_uses_defaults() {
        let json = r##"{"http_get": "#FF0000"}"##;
        let vc: ValueColors = serde_json::from_str(json).unwrap();
        assert_eq!(vc.http_get, Color::Rgb(255, 0, 0));
        // Other fields should use defaults
        assert_eq!(vc.http_post, default_http_post());
        assert_eq!(vc.uuid, default_uuid());
    }

    // ── Theme defaults ──────────────────────────────────────────────────

    #[test]
    fn test_theme_default_level_colors() {
        let theme = Theme::default();
        assert_eq!(theme.trace_fg, Color::Rgb(98, 114, 164));
        assert_eq!(theme.debug_fg, Color::Rgb(139, 233, 253));
        assert_eq!(theme.notice_fg, Color::Rgb(248, 248, 242));
        assert_eq!(theme.fatal_fg, Color::Rgb(255, 85, 85));
        assert_eq!(theme.cursor_fg, Color::Rgb(28, 28, 28));
        assert_eq!(theme.search_fg, Color::Rgb(28, 28, 28));
        assert_eq!(theme.visual_select_bg, Color::Rgb(68, 71, 90));
        assert_eq!(theme.visual_select_fg, Color::Rgb(248, 248, 242));
        assert_eq!(theme.mark_bg, Color::Rgb(70, 60, 15));
        assert_eq!(theme.mark_fg, Color::Rgb(248, 248, 242));
    }

    #[test]
    fn test_theme_default_base_colors() {
        let theme = Theme::default();
        assert_eq!(theme.error_fg, Color::Rgb(255, 85, 85));
        assert_eq!(theme.warning_fg, Color::Rgb(241, 250, 140));
        assert!(!theme.process_colors.is_empty());
    }

    // ── Theme serde ─────────────────────────────────────────────────────

    #[test]
    fn test_theme_serde_roundtrip() {
        let original = Theme::default();
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: Theme = serde_json::from_str(&json).unwrap();
        assert_eq!(original, deserialized);
    }

    #[test]
    fn test_theme_deserialize_hex_color() {
        let json = serde_json::to_string(&Theme::default()).unwrap();
        // The serialized form uses ratatui's color string format
        let theme: Theme = serde_json::from_str(&json).unwrap();
        assert_eq!(theme.root_bg, Color::Rgb(40, 42, 54));
    }

    #[test]
    fn test_theme_deserialize_rgb_array() {
        let mut json_value: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&Theme::default()).unwrap()).unwrap();
        // Replace a color with RGB array format
        json_value["root_bg"] = serde_json::json!([100, 200, 50]);
        let theme: Theme = serde_json::from_value(json_value).unwrap();
        assert_eq!(theme.root_bg, Color::Rgb(100, 200, 50));
    }

    #[test]
    fn test_theme_deserialize_missing_optional_fields_use_defaults() {
        // Build a minimal theme JSON without trace_fg, debug_fg, notice_fg, fatal_fg, cursor_fg
        let json = r##"{
            "root_bg": "#282a36",
            "border": "#6272a4",
            "border_title": "#f8f8f2",
            "text": "#f8f8f2",
            "text_highlight": "#ffb86c",
            "error_fg": "#ff5555",
            "warning_fg": "#f1fa8c",
            "process_colors": ["#ff5555", "#50fa7b"]
        }"##;
        let theme: Theme = serde_json::from_str(json).unwrap();
        assert_eq!(theme.trace_fg, default_trace_fg());
        assert_eq!(theme.debug_fg, default_debug_fg());
        assert_eq!(theme.notice_fg, default_notice_fg());
        assert_eq!(theme.fatal_fg, default_fatal_fg());
        assert_eq!(theme.cursor_fg, default_cursor_fg());
        assert_eq!(theme.search_fg, default_search_fg());
        assert_eq!(theme.visual_select_bg, default_visual_select_bg());
        assert_eq!(theme.visual_select_fg, default_visual_select_fg());
        assert_eq!(theme.mark_bg, default_mark_bg());
        assert_eq!(theme.mark_fg, default_mark_fg());
        assert_eq!(theme.value_colors, ValueColors::default());
    }

    #[test]
    fn test_theme_deserialize_process_colors_rgb_arrays() {
        let mut json_value: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&Theme::default()).unwrap()).unwrap();
        json_value["process_colors"] = serde_json::json!([[10, 20, 30], [40, 50, 60]]);
        let theme: Theme = serde_json::from_value(json_value).unwrap();
        assert_eq!(theme.process_colors.len(), 2);
        assert_eq!(theme.process_colors[0], Color::Rgb(10, 20, 30));
        assert_eq!(theme.process_colors[1], Color::Rgb(40, 50, 60));
    }

    // ── Theme::from_file ────────────────────────────────────────────────

    #[test]
    fn test_theme_from_file_nonexistent() {
        let result = Theme::from_file("nonexistent_theme_xyz123.json");
        assert!(result.is_err());
    }

    #[test]
    fn test_theme_from_file_valid() {
        let temp = tempdir().unwrap();
        let theme_dir = temp.path().join("logana").join("themes");
        fs::create_dir_all(&theme_dir).unwrap();
        let theme_json = serde_json::to_string(&Theme::default()).unwrap();
        fs::write(theme_dir.join("test_theme.json"), &theme_json).unwrap();

        let result = Theme::from_file_with_config_dir("test_theme.json", Some(temp.path()));

        assert!(result.is_ok());
        assert_eq!(result.unwrap().root_bg, Color::Rgb(40, 42, 54));
    }

    #[test]
    fn test_theme_from_file_invalid_json() {
        let temp = tempdir().unwrap();
        let theme_dir = temp.path().join("logana").join("themes");
        fs::create_dir_all(&theme_dir).unwrap();
        fs::write(theme_dir.join("broken.json"), "not valid json {{{").unwrap();

        let result = Theme::from_file_with_config_dir("broken.json", Some(temp.path()));

        assert!(result.is_err());
    }

    // ── Theme::list_available_themes ────────────────────────────────────

    #[test]
    fn test_theme_loading_from_config_dir() {
        let temp_dir = tempdir().unwrap();
        let themes_dir = temp_dir.path().join("logana/themes");
        fs::create_dir_all(&themes_dir).unwrap();
        fs::write(themes_dir.join("mytheme.json"), "{}").unwrap();

        let themes = Theme::list_available_themes_from(Some(temp_dir.path()));
        assert!(themes.contains(&"mytheme".to_string()));
    }

    #[test]
    fn test_list_available_themes_ignores_non_json() {
        let temp_dir = tempdir().unwrap();
        let themes_dir = temp_dir.path().join("logana/themes");
        fs::create_dir_all(&themes_dir).unwrap();
        fs::write(themes_dir.join("readme.txt"), "not a theme").unwrap();
        fs::write(themes_dir.join("valid.json"), "{}").unwrap();

        let themes = Theme::list_available_themes_from(Some(temp_dir.path()));
        assert!(themes.contains(&"valid".to_string()));
        assert!(!themes.contains(&"readme".to_string()));
        assert!(!themes.contains(&"readme.txt".to_string()));
    }

    // ── complete_theme ──────────────────────────────────────────────────

    #[test]
    fn test_complete_theme_empty_returns_available_themes() {
        let themes = complete_theme("");
        for t in &themes {
            assert!(!t.is_empty());
        }
    }

    #[test]
    fn test_complete_theme_no_match_returns_empty() {
        let results = complete_theme("zzznomatch9999");
        assert!(results.is_empty());
    }

    #[test]
    fn test_complete_theme_fuzzy_match() {
        let temp_dir = tempdir().unwrap();
        let themes_dir = temp_dir.path().join("logana/themes");
        fs::create_dir_all(&themes_dir).unwrap();
        fs::write(themes_dir.join("monokai.json"), "{}").unwrap();
        fs::write(themes_dir.join("solarized.json"), "{}").unwrap();

        let all = Theme::list_available_themes_from(Some(temp_dir.path()));
        let results: Vec<String> = all.into_iter().filter(|t| fuzzy_match("mono", t)).collect();
        assert!(results.contains(&"monokai".to_string()));
        assert!(!results.contains(&"solarized".to_string()));
    }

    // ── color serde helpers ─────────────────────────────────────────────

    #[test]
    fn test_color_deserialize_string() {
        #[derive(Deserialize)]
        struct Wrapper {
            #[serde(deserialize_with = "color_from_str")]
            color: Color,
        }
        let w: Wrapper = serde_json::from_str(r##"{"color": "#ff0000"}"##).unwrap();
        assert_eq!(w.color, Color::Rgb(255, 0, 0));
    }

    #[test]
    fn test_color_deserialize_rgb_array() {
        #[derive(Deserialize)]
        struct Wrapper {
            #[serde(deserialize_with = "color_from_str")]
            color: Color,
        }
        let w: Wrapper = serde_json::from_str(r#"{"color": [10, 20, 30]}"#).unwrap();
        assert_eq!(w.color, Color::Rgb(10, 20, 30));
    }

    #[test]
    fn test_color_deserialize_incomplete_array() {
        #[derive(Deserialize)]
        struct Wrapper {
            #[serde(deserialize_with = "color_from_str")]
            _color: Color,
        }
        let result: Result<Wrapper, _> = serde_json::from_str(r#"{"_color": [10, 20]}"#);
        assert!(result.is_err());
    }

    #[test]
    fn test_colors_vec_roundtrip() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct Wrapper {
            #[serde(
                serialize_with = "colors_to_str_vec",
                deserialize_with = "colors_from_str_vec"
            )]
            colors: Vec<Color>,
        }
        let original = Wrapper {
            colors: vec![Color::Rgb(1, 2, 3), Color::Rgb(4, 5, 6)],
        };
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: Wrapper = serde_json::from_str(&json).unwrap();
        assert_eq!(original, deserialized);
    }
}
