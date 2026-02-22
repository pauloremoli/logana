use anyhow::Context;
use ratatui::style::Color;
use serde::de::Error;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::str::FromStr;

use crate::auto_complete::fuzzy_match;

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
    #[serde(serialize_with = "color_to_str", deserialize_with = "color_from_str")]
    pub error_fg: Color,
    #[serde(serialize_with = "color_to_str", deserialize_with = "color_from_str")]
    pub warning_fg: Color,
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

fn default_cursor_fg() -> Color {
    Color::Rgb(28, 28, 28)
}

impl Theme {
    /// Returns the names of all available themes found in the local `themes/` directory
    /// and in `~/.config/logsmith-rs/themes/`.
    pub fn list_available_themes() -> Vec<String> {
        let mut theme_paths = vec![];
        if let Ok(entries) = std::fs::read_dir("themes") {
            for entry in entries.flatten() {
                theme_paths.push(entry.path());
            }
        }
        if let Some(config_dir) = dirs::config_dir() {
            let user_themes_path = config_dir.join("logsmith-rs/themes");
            if let Ok(entries) = std::fs::read_dir(user_themes_path) {
                for entry in entries.flatten() {
                    theme_paths.push(entry.path());
                }
            }
        }
        let mut set = std::collections::HashSet::new();
        for path in theme_paths {
            if path.extension().and_then(|ext| ext.to_str()) == Some("json")
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            {
                set.insert(stem.to_string());
            }
        }
        let mut themes: Vec<String> = set.into_iter().collect();
        themes.sort();
        themes
    }

    pub fn from_file<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let config_path =
            dirs::config_dir().map(|d| d.join("logsmith-rs").join("themes").join(&path));
        let local_path = Path::new("themes").join(&path);

        let data = if config_path.as_ref().is_some_and(|p| p.exists()) {
            let cp = config_path.unwrap();
            fs::read_to_string(&cp)
                .with_context(|| format!("Failed to read theme from {:?}", cp))?
        } else if local_path.exists() {
            fs::read_to_string(&local_path)
                .with_context(|| format!("Failed to read theme from {:?}", local_path))?
        } else {
            anyhow::bail!(
                "Theme {:?} not found in config dir or local themes/",
                path.as_ref()
            );
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
            error_fg: Color::Rgb(255, 85, 85),
            warning_fg: Color::Rgb(241, 250, 140),
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
    use std::env;
    use tempfile::tempdir;

    #[test]
    fn test_theme_loading_from_config_dir() {
        let temp_dir = tempdir().unwrap();
        let config_home = temp_dir.path();
        unsafe {
            env::set_var("XDG_CONFIG_HOME", config_home);
        }

        let themes_dir = config_home.join("logsmith-rs/themes");
        fs::create_dir_all(&themes_dir).unwrap();
        let theme_path = themes_dir.join("mytheme.json");
        fs::write(&theme_path, "{}").unwrap();

        assert!(Theme::list_available_themes().contains(&"mytheme".to_string()));

        unsafe {
            env::remove_var("XDG_CONFIG_HOME");
        }
    }

    #[test]
    fn test_complete_theme_empty_returns_available_themes() {
        // Result depends on filesystem but must not panic
        let themes = complete_theme("");
        // All returned entries must be non-empty strings
        for t in &themes {
            assert!(!t.is_empty());
        }
    }

    #[test]
    fn test_complete_theme_no_match_returns_empty() {
        // An unlikely prefix that won't match any theme name
        let results = complete_theme("zzznomatch9999");
        assert!(results.is_empty());
    }
}
