use std::collections::HashSet;
use std::sync::Arc;

use crate::parser::LogFormatParser;
use crate::types::FieldLayout;

pub struct DisplayConfig {
    pub wrap: bool,
    pub show_line_numbers: bool,
    pub show_sidebar: bool,
    pub sidebar_width: u16,
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
