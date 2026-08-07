use std::collections::HashSet;
use std::sync::Arc;

use crate::parser::LogFormatParser;
use crate::ui::FieldLayout;

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Default,
    strum::EnumString,
    strum::Display,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
#[strum(serialize_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum SidebarSide {
    #[default]
    Right,
    Left,
}

impl SidebarSide {
    pub fn is_left(self) -> bool {
        self == SidebarSide::Left
    }
}

pub struct DisplayConfig {
    pub wrap: bool,
    pub show_line_numbers: bool,
    pub relative_line_numbers: bool,
    pub show_sidebar: bool,
    pub sidebar_width: u16,
    pub sidebar_side: SidebarSide,
    pub show_mode_bar: bool,
    pub show_borders: bool,
    pub raw_mode: bool,
    pub show_keys: bool,
    pub format: Option<Arc<dyn LogFormatParser>>,
    pub hidden_fields: HashSet<String>,
    pub field_layout: FieldLayout,
    pub level_colors_disabled: HashSet<String>,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            wrap: true,
            show_line_numbers: true,
            relative_line_numbers: false,
            show_sidebar: true,
            sidebar_width: 30,
            sidebar_side: SidebarSide::Right,
            show_mode_bar: true,
            show_borders: true,
            raw_mode: false,
            show_keys: false,
            format: None,
            hidden_fields: HashSet::new(),
            field_layout: FieldLayout::default(),
            level_colors_disabled: HashSet::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn sidebar_side_parses_left() {
        assert_eq!(SidebarSide::from_str("left").unwrap(), SidebarSide::Left);
    }

    #[test]
    fn sidebar_side_parses_right() {
        assert_eq!(SidebarSide::from_str("right").unwrap(), SidebarSide::Right);
    }

    #[test]
    fn sidebar_side_displays_lowercase() {
        assert_eq!(SidebarSide::Left.to_string(), "left");
        assert_eq!(SidebarSide::Right.to_string(), "right");
    }

    #[test]
    fn sidebar_side_rejects_invalid() {
        assert!(SidebarSide::from_str("top").is_err());
        assert!(SidebarSide::from_str("Left").is_err());
    }

    #[test]
    fn sidebar_side_is_left() {
        assert!(SidebarSide::Left.is_left());
        assert!(!SidebarSide::Right.is_left());
    }
}
