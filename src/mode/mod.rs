//! Mode trait and all mode implementations for the TUI.
//!
//! Each mode owns its key-handling logic and state. The active mode is stored
//! as `Box<dyn Mode>` on [`crate::ui::TabState`]. Unhandled keys return
//! `KeyResult::Ignored` to fall through to global key handling; commands
//! are triggered via `KeyResult::ExecuteCommand`.

pub mod app_mode;
pub mod command_mode;
pub mod comment_mode;
pub mod docker_select_mode;
pub mod filter_mode;
pub mod keybindings_help_mode;
pub mod normal_mode;
pub mod search_mode;
pub mod select_fields_mode;
pub mod ui_mode;
pub mod value_colors_mode;
pub mod visual_mode;
