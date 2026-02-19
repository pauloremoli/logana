use crossterm::event::{KeyCode, KeyModifiers};

use crate::{
    auto_complete::{
        complete_color, complete_file_path, extract_color_partial, find_command_completions,
    },
    mode::{app_mode::Mode, normal_mode::NormalMode},
    theme::{Theme, fuzzy_match},
    ui::{KeyResult, TabState},
};

#[derive(Debug)]
pub struct CommandMode {
    pub input: String,
    pub cursor: usize,
    pub history: Vec<String>,
    pub history_index: Option<usize>,
    pub completion_index: Option<usize>,
}

impl CommandMode {
    pub fn with_history(input: String, cursor: usize, history: Vec<String>) -> Self {
        CommandMode {
            input,
            cursor,
            history,
            history_index: None,
            completion_index: None,
        }
    }
}

impl Mode for CommandMode {
    fn handle_key(
        mut self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        _modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        match key {
            KeyCode::Enter => {
                let cmd = self.input.trim().to_string();
                return (Box::new(NormalMode), KeyResult::ExecuteCommand(cmd));
            }
            KeyCode::Esc => {
                tab.filter_context = None;
                tab.editing_filter_id = None;
                return (Box::new(NormalMode), KeyResult::Handled);
            }
            KeyCode::Backspace => {
                if self.cursor > 0 && !self.input.is_empty() {
                    self.input.remove(self.cursor - 1);
                    self.cursor -= 1;
                    self.completion_index = None;
                }
            }
            KeyCode::Char(c) => {
                self.input.insert(self.cursor, c);
                self.cursor += 1;
                tab.command_error = None;
                self.completion_index = None;
                self.history_index = None;
            }
            KeyCode::Left => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                }
            }
            KeyCode::Right => {
                if self.cursor < self.input.len() {
                    self.cursor += 1;
                }
            }
            KeyCode::Up => {
                if self.history.is_empty() {
                    return (self, KeyResult::Handled);
                }
                let new_index = match self.history_index {
                    None => Some(self.history.len() - 1),
                    Some(0) => Some(0),
                    Some(i) => Some(i - 1),
                };
                if let Some(i) = new_index {
                    self.input = self.history[i].clone();
                    self.cursor = self.input.len();
                    self.history_index = Some(i);
                }
            }
            KeyCode::Down => {
                if self.history.is_empty() {
                    return (self, KeyResult::Handled);
                }
                let new_index = match self.history_index {
                    None => return (self, KeyResult::Handled),
                    Some(i) if i + 1 >= self.history.len() => {
                        self.input = String::new();
                        self.cursor = 0;
                        self.history_index = None;
                        return (self, KeyResult::Handled);
                    }
                    Some(i) => Some(i + 1),
                };
                if let Some(i) = new_index {
                    self.input = self.history[i].clone();
                    self.cursor = self.input.len();
                    self.history_index = Some(i);
                }
            }
            KeyCode::Tab => {
                let trimmed = self.input.trim().to_string();

                // Color completion for --fg/--bg arguments
                if let Some(partial) = extract_color_partial(&trimmed) {
                    let completions = complete_color(partial);
                    if !completions.is_empty() {
                        let idx = self.completion_index.unwrap_or(0) % completions.len();
                        let color_name = completions[idx];
                        let prefix = if partial.is_empty() {
                            trimmed.clone()
                        } else {
                            trimmed[..trimmed.len() - partial.len()].to_string()
                        };
                        self.input = format!("{}{}", prefix, color_name);
                        self.cursor = self.input.len();
                        self.completion_index = Some(idx + 1);
                        return (self, KeyResult::Handled);
                    }
                }

                // File path completion
                let file_commands = ["open", "load-filters", "save-filters", "export-marked"];
                let file_cmd = file_commands
                    .iter()
                    .find(|cmd| trimmed.starts_with(&format!("{} ", cmd)));
                if let Some(&cmd) = file_cmd {
                    let partial = trimmed[cmd.len()..].trim_start();
                    let completions = complete_file_path(partial);
                    if !completions.is_empty() {
                        let idx = self.completion_index.unwrap_or(0) % completions.len();
                        let completed = completions[idx].clone();
                        self.input = format!("{} {}", cmd, completed);
                        self.cursor = self.input.len();
                        self.completion_index = Some(idx + 1);
                    }
                    return (self, KeyResult::Handled);
                }

                // set-theme completion with fuzzy match
                if let Some(after_prefix) = trimmed.strip_prefix("set-theme") {
                    let partial = after_prefix.trim_start();
                    let mut themes = Theme::list_available_themes();
                    if !partial.is_empty() {
                        themes.retain(|t| fuzzy_match(partial, t));
                    }
                    if !themes.is_empty() {
                        let idx = self.completion_index.unwrap_or(0) % themes.len();
                        let theme_name = themes[idx].clone();
                        self.input = format!("set-theme {}", theme_name);
                        self.cursor = self.input.len();
                        self.completion_index = Some(idx + 1);
                    }
                    return (self, KeyResult::Handled);
                }

                // Command name completion
                let completions = find_command_completions(&trimmed);
                if !completions.is_empty() {
                    let idx = self.completion_index.unwrap_or(0) % completions.len();
                    self.input = completions[idx].to_string();
                    self.cursor = self.input.len();
                    self.completion_index = Some(idx + 1);
                }
            }
            _ => {}
        }
        (self, KeyResult::Handled)
    }

    fn status_line(&self) -> &str {
        "[COMMAND] filter | exclude | set-color | export-marked | save-filters | load-filters | wrap | set-theme | level-colors | open | close-tab | Esc | Enter"
    }

    fn command_state(&self) -> Option<(&str, usize)> {
        Some((&self.input, self.cursor))
    }

    fn needs_input_bar(&self) -> bool {
        true
    }
}
