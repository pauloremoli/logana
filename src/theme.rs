use ratatui::style::Color;
use std::fs;
use std::path::Path;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use anyhow::Context;
use serde::de::Error;
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
    #[serde(serialize_with = "colors_to_str_vec", deserialize_with = "colors_from_str_vec")]
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
            let r = seq.next_element()?.ok_or_else(|| A::Error::invalid_length(0, &self))?;
            let g = seq.next_element()?.ok_or_else(|| A::Error::invalid_length(1, &self))?;
            let b = seq.next_element()?.ok_or_else(|| A::Error::invalid_length(2, &self))?;
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
        let config_dir = dirs::config_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not find config directory"))?
            .join("logsmith-rs")
            .join("themes");
        let config_path = config_dir.join(&path);

        let data = if config_path.exists() {
            fs::read_to_string(&config_path)
                .with_context(|| format!("Failed to read theme from {:?}", config_path))?
        } else {
            fs::read_to_string(&path)
                .with_context(|| format!("Failed to read theme from {:?}", path.as_ref()))?
        };
        let config: Theme = serde_json::from_str(&data)?;
        Ok(config)
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme::from_file("themes/dracula.json").unwrap_or_else(|_| Theme {
            root_bg: Color::Rgb(40, 42, 54),
            border: Color::Rgb(98, 114, 164),
            border_title: Color::Rgb(248, 248, 242),
            text: Color::Rgb(248, 248, 242),
            text_highlight: Color::Rgb(255, 184, 108),
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
    use std::env;
    use tempfile::tempdir;
    use crate::analyzer::LogAnalyzer;
    use crate::ui::App;

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

        let app = App::new(LogAnalyzer::new(), Theme::default());

        assert!(app.available_themes.contains(&"mytheme".to_string()));

        unsafe {
            env::remove_var("XDG_CONFIG_HOME");
        }
    }
}