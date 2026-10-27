pub mod actions;
pub mod app;
pub mod color;
pub mod column_widths;
pub mod effect;
pub mod key;
pub mod level;
pub mod menus;
pub mod message;
pub mod runtime;
pub mod state;
pub mod status;
pub mod text_selection;
pub mod theme;
pub mod time_range;
pub mod update;
pub mod view;

mod export_preview;
pub mod palette;
pub mod preview;
mod schema_preview;

#[cfg(all(test, feature = "gui-test"))]
mod palette_ui_tests;

mod file_actions;

mod save_logs;

mod export;

mod live;

mod sources;

mod defaults;

mod mcp;

mod keyboard;

pub mod persistence;

mod loading;

mod startup;

mod merge;

mod history;

mod field_discovery;

mod table_preparation;

mod schema_preparation;

mod facet_query;

mod archive_window;

mod path_picker;

mod row_cache;
