use std::collections::HashSet;
use std::sync::Arc;

use crate::parser::LogFormatParser;
use crate::ui::FieldLayout;

#[derive(Debug, Clone, Copy, PartialEq, Default, strum::EnumString, strum::Display)]
#[strum(serialize_all = "lowercase")]
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
