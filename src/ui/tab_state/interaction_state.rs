use std::sync::Arc;

use crate::config::Keybindings;
use crate::mode::app_mode::Mode;
use crate::mode::normal_mode::NormalMode;

pub struct InteractionState {
    pub mode: Box<dyn Mode>,
    pub g_key_pressed: bool,
    pub command_error: Option<String>,
    pub notification: Option<String>,
    pub notification_set_at: Option<std::time::Instant>,
    pub command_history: Vec<String>,
    pub keybindings: Arc<Keybindings>,
    /// Row count of the archive picker popup's last-rendered content area —
    /// set from the render layer each frame (mirrors
    /// `FilterState::sidebar_visible_height`), read by `ArchivePickerMode`
    /// for `PageUp`/`PageDown`/half-page navigation sizing.
    pub archive_picker_visible_height: usize,
}

impl Default for InteractionState {
    fn default() -> Self {
        Self {
            mode: Box::new(NormalMode::default()),
            g_key_pressed: false,
            command_error: None,
            notification: None,
            notification_set_at: None,
            command_history: Vec::new(),
            keybindings: Arc::new(Keybindings::default()),
            archive_picker_visible_height: 0,
        }
    }
}
