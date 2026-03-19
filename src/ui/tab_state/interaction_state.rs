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
        }
    }
}
