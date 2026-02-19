use crossterm::event::{KeyCode, KeyModifiers};

use crate::mode::app_mode::Mode;
use crate::mode::command_mode::CommandMode;
use crate::mode::normal_mode::NormalMode;
use crate::types::FilterType;

use crate::ui::KeyResult;
use crate::ui::TabState;

#[derive(Debug)]
pub struct FilterManagementMode {
    pub selected_filter_index: usize,
}

impl Mode for FilterManagementMode {
    fn handle_key(
        self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        _modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        if matches!(key, KeyCode::Tab | KeyCode::BackTab) {
            return (self, KeyResult::Ignored);
        }

        let selected = self.selected_filter_index;

        match key {
            KeyCode::Esc => (Box::new(NormalMode), KeyResult::Handled),
            KeyCode::Up => (
                Box::new(FilterManagementMode {
                    selected_filter_index: selected.saturating_sub(1),
                }),
                KeyResult::Handled,
            ),
            KeyCode::Down => {
                let num_filters = tab.log_manager.get_filters().len();
                let new_idx = if num_filters > 0 {
                    (selected + 1).min(num_filters - 1)
                } else {
                    0
                };
                (
                    Box::new(FilterManagementMode {
                        selected_filter_index: new_idx,
                    }),
                    KeyResult::Handled,
                )
            }
            KeyCode::Char(' ') => {
                let filter_id = tab.log_manager.get_filters().get(selected).map(|f| f.id);
                if let Some(id) = filter_id {
                    tab.log_manager.toggle_filter(id);
                    tab.refresh_visible();
                }
                (
                    Box::new(FilterManagementMode {
                        selected_filter_index: selected,
                    }),
                    KeyResult::Handled,
                )
            }
            KeyCode::Char('d') => {
                let filter_id = tab.log_manager.get_filters().get(selected).map(|f| f.id);
                if let Some(id) = filter_id {
                    tab.log_manager.remove_filter(id);
                    tab.refresh_visible();
                    let remaining_len = tab.log_manager.get_filters().len();
                    let new_idx = if remaining_len > 0 && selected >= remaining_len {
                        remaining_len - 1
                    } else {
                        selected
                    };
                    (
                        Box::new(FilterManagementMode {
                            selected_filter_index: new_idx,
                        }),
                        KeyResult::Handled,
                    )
                } else {
                    (
                        Box::new(FilterManagementMode {
                            selected_filter_index: selected,
                        }),
                        KeyResult::Handled,
                    )
                }
            }
            KeyCode::Char('K') => {
                let filter_id = tab.log_manager.get_filters().get(selected).map(|f| f.id);
                if let Some(id) = filter_id {
                    tab.log_manager.move_filter_up(id);
                    tab.refresh_visible();
                    let new_idx = selected.saturating_sub(1);
                    (
                        Box::new(FilterManagementMode {
                            selected_filter_index: new_idx,
                        }),
                        KeyResult::Handled,
                    )
                } else {
                    (
                        Box::new(FilterManagementMode {
                            selected_filter_index: selected,
                        }),
                        KeyResult::Handled,
                    )
                }
            }
            KeyCode::Char('J') => {
                let filter_id = tab.log_manager.get_filters().get(selected).map(|f| f.id);
                if let Some(id) = filter_id {
                    tab.log_manager.move_filter_down(id);
                    tab.refresh_visible();
                    let total = tab.log_manager.get_filters().len();
                    let new_idx = if selected + 1 < total {
                        selected + 1
                    } else {
                        selected
                    };
                    (
                        Box::new(FilterManagementMode {
                            selected_filter_index: new_idx,
                        }),
                        KeyResult::Handled,
                    )
                } else {
                    (
                        Box::new(FilterManagementMode {
                            selected_filter_index: selected,
                        }),
                        KeyResult::Handled,
                    )
                }
            }
            KeyCode::Char('e') => {
                let filter_info = tab.log_manager.get_filters().get(selected).map(|f| {
                    (
                        f.id,
                        f.filter_type.clone(),
                        f.color_config.clone(),
                        f.pattern.clone(),
                    )
                });
                if let Some((id, ft, cc, pattern)) = filter_info {
                    tab.editing_filter_id = Some(id);
                    tab.filter_context = Some(selected);
                    let mut cmd = if ft == FilterType::Include {
                        String::from("filter")
                    } else {
                        String::from("exclude")
                    };
                    if ft == FilterType::Include {
                        if let Some(cfg) = &cc {
                            if let Some(fg) = cfg.fg {
                                cmd.push_str(&format!(" --fg {:?}", fg));
                            }
                            if let Some(bg) = cfg.bg {
                                cmd.push_str(&format!(" --bg {:?}", bg));
                            }
                            if cfg.match_only {
                                cmd.push_str(" -m");
                            }
                        }
                    }
                    cmd.push(' ');
                    cmd.push_str(&pattern);
                    let len = cmd.len();
                    let history = tab.command_history.clone();
                    (
                        Box::new(CommandMode::with_history(cmd, len, history)),
                        KeyResult::Handled,
                    )
                } else {
                    (
                        Box::new(FilterManagementMode {
                            selected_filter_index: selected,
                        }),
                        KeyResult::Handled,
                    )
                }
            }
            KeyCode::Char('c') => {
                let color_config = tab
                    .log_manager
                    .get_filters()
                    .get(selected)
                    .and_then(|f| f.color_config.clone());
                tab.filter_context = Some(selected);
                let mut cmd = String::from("set-color");
                if let Some(cfg) = color_config {
                    if let Some(fg) = cfg.fg {
                        cmd.push_str(&format!(" --fg {:?}", fg));
                    }
                    if let Some(bg) = cfg.bg {
                        cmd.push_str(&format!(" --bg {:?}", bg));
                    }
                }
                let len = cmd.len();
                let history = tab.command_history.clone();
                (
                    Box::new(CommandMode::with_history(cmd, len, history)),
                    KeyResult::Handled,
                )
            }
            KeyCode::Char('i') => {
                let history = tab.command_history.clone();
                (
                    Box::new(CommandMode::with_history("filter ".to_string(), 7, history)),
                    KeyResult::Handled,
                )
            }
            KeyCode::Char('x') => {
                let history = tab.command_history.clone();
                (
                    Box::new(CommandMode::with_history(
                        "exclude ".to_string(),
                        8,
                        history,
                    )),
                    KeyResult::Handled,
                )
            }
            _ => (
                Box::new(FilterManagementMode {
                    selected_filter_index: selected,
                }),
                KeyResult::Handled,
            ),
        }
    }

    fn status_line(&self) -> &str {
        "[FILTER] [i]nclude | e[x]clude | Space => toggle | [d]elete | [e]dit | set [c]olor | [J/K] move down/up | Esc => normal mode"
    }

    fn selected_filter_index(&self) -> Option<usize> {
        Some(self.selected_filter_index)
    }
}

// ---------------------------------------------------------------------------
// FilterEditMode
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct FilterEditMode {
    pub filter_id: Option<usize>,
    pub filter_input: String,
}

impl Mode for FilterEditMode {
    fn handle_key(
        self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        _modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        if matches!(key, KeyCode::Tab | KeyCode::BackTab) {
            return (self, KeyResult::Ignored);
        }
        match key {
            KeyCode::Enter => {
                if let Some(id) = self.filter_id {
                    tab.log_manager.edit_filter(id, self.filter_input);
                    tab.refresh_visible();
                }
                (
                    Box::new(FilterManagementMode {
                        selected_filter_index: 0,
                    }),
                    KeyResult::Handled,
                )
            }
            KeyCode::Esc => (
                Box::new(FilterManagementMode {
                    selected_filter_index: 0,
                }),
                KeyResult::Handled,
            ),
            KeyCode::Backspace => {
                let mut input = self.filter_input;
                input.pop();
                (
                    Box::new(FilterEditMode {
                        filter_id: self.filter_id,
                        filter_input: input,
                    }),
                    KeyResult::Handled,
                )
            }
            KeyCode::Char(c) => {
                let mut input = self.filter_input;
                input.push(c);
                (
                    Box::new(FilterEditMode {
                        filter_id: self.filter_id,
                        filter_input: input,
                    }),
                    KeyResult::Handled,
                )
            }
            _ => (self, KeyResult::Handled),
        }
    }

    fn status_line(&self) -> &str {
        "[FILTER EDIT] Esc => cancel | Enter => save"
    }
}
