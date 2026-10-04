//! Terminal frontend and compatibility re-exports of shared application data.
pub use crate::shared::theme::{
    Theme, ValueColorGroup, ValueColors, color_to_string, complete_theme, parse_color,
};
pub use crate::shared::value_colors::*;
pub use crate::shared::*;
#[cfg(feature = "tui")]
mod app;
#[cfg(feature = "tui")]
mod commands;
#[cfg(feature = "tui")]
mod input;
#[cfg(feature = "tui")]
mod input_handler;
#[cfg(feature = "tui")]
mod render;
#[cfg(feature = "tui")]
mod tabs;
#[cfg(feature = "tui")]
pub mod widgets;
#[cfg(feature = "tui")]
pub use app::App;
