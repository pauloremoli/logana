use anyhow::Context;
use ratatui::style::Color;
use serde::de::Error;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::str::FromStr;
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
    #[serde(serialize_with = "color_to_str", deserialize_with = "color_from_str")]
    pub error_fg: Color,
    #[serde(serialize_with = "color_to_str", deserialize_with = "color_from_str")]
    pub warning_fg: Color,
    #[serde(
        serialize_with = "colors_to_str_vec",
        deserialize_with = "colors_from_str_vec"
    )]
    pub process_colors: Vec<Color>,
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

impl Theme {
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
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::file_reader::FileReader;
    use crate::log_manager::LogManager;
    use crate::ui::App;
    use std::env;
    use std::sync::Arc;
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

        let rt = Arc::new(tokio::runtime::Runtime::new().unwrap());
        let db = Arc::new(rt.block_on(Database::in_memory()).unwrap());
        let log_manager = LogManager::new(db, rt, None);
        let file_reader = FileReader::from_bytes(vec![]);
        let app = App::new(log_manager, file_reader, Theme::default());

        assert!(app.available_themes.contains(&"mytheme".to_string()));

        unsafe {
            env::remove_var("XDG_CONFIG_HOME");
        }
    }
}
