pub mod actions;
pub mod app;
pub mod color;
pub mod column_widths;
pub mod effect;
pub mod key;
pub mod level;
pub mod message;
pub mod runtime;
pub mod state;
pub mod status;
pub mod theme;
pub mod time_range;
pub mod update;
pub mod view;

pub mod palette;

#[cfg(all(test, feature = "gui-test"))]
mod palette_ui_tests;

mod file_actions;

mod save_logs;

mod export;

mod live;

mod sources;

mod defaults;

mod mcp;
