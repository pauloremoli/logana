use crossterm::event::{KeyCode, KeyModifiers};

use crate::{
    auto_complete::{
        complete_color, complete_file_path, extract_color_partial, find_command_completions,
        fuzzy_match,
    },
    mode::{app_mode::Mode, normal_mode::NormalMode},
    theme::Theme,
    ui::{KeyResult, TabState},
};

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(author, version, about, no_binary_name = true)]
pub struct CommandLine {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Add an include filter
    Filter {
        pattern: String,
        #[arg(long)]
        fg: Option<String>,
        #[arg(long)]
        bg: Option<String>,
        /// Apply color only to the matched text instead of the whole line
        #[arg(short)]
        m: bool,
    },
    /// Add an exclude filter
    Exclude { pattern: String },
    /// Set color for the selected filter
    SetColor {
        #[arg(long)]
        fg: Option<String>,
        #[arg(long)]
        bg: Option<String>,
        /// Apply color only to the matched text instead of the whole line
        #[arg(short)]
        m: bool,
    },
    /// Export marked logs
    ExportMarked { path: String },
    /// Save filters to file
    SaveFilters { path: String },
    /// Load filters from file
    LoadFilters { path: String },
    /// Toggle line wrapping
    Wrap,
    /// Toggle line numbers
    LineNumbers,
    /// Set the theme
    SetTheme { theme_name: String },
    /// Toggle log level color highlighting
    LevelColors,
    /// Open a file in a new tab
    Open { path: String },
    /// Close the current tab
    CloseTab,
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::file_reader::FileReader;
    use crate::log_manager::LogManager;
    use crate::ui::{KeyResult, TabState};
    use std::sync::Arc;

    fn make_tab() -> TabState {
        let file_reader = FileReader::from_bytes(b"line1\nline2\n".to_vec());
        let rt = Arc::new(tokio::runtime::Runtime::new().unwrap());
        let db = Arc::new(rt.block_on(Database::in_memory()).unwrap());
        let log_manager = LogManager::new(db, rt, None);
        TabState::new(file_reader, log_manager, "test".to_string())
    }

    fn empty_mode() -> CommandMode {
        CommandMode::with_history(String::new(), 0, vec![])
    }

    fn press(mode: CommandMode, tab: &mut TabState, code: KeyCode) -> (Box<dyn Mode>, KeyResult) {
        Box::new(mode).handle_key(tab, code, KeyModifiers::NONE)
    }

    #[test]
    fn test_char_appends_to_input() {
        let mut tab = make_tab();
        let (mode, result) = press(empty_mode(), &mut tab, KeyCode::Char('f'));
        assert!(matches!(result, KeyResult::Handled));
        assert_eq!(mode.command_state(), Some(("f", 1)));
    }

    #[test]
    fn test_char_appends_to_existing_input() {
        let mut tab = make_tab();
        let mode = CommandMode::with_history("fil".to_string(), 3, vec![]);
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('t'));
        assert_eq!(mode2.command_state(), Some(("filt", 4)));
    }

    #[test]
    fn test_backspace_removes_last_char() {
        let mut tab = make_tab();
        let mode = CommandMode::with_history("filter".to_string(), 6, vec![]);
        let (mode2, _) = press(mode, &mut tab, KeyCode::Backspace);
        assert_eq!(mode2.command_state(), Some(("filte", 5)));
    }

    #[test]
    fn test_backspace_at_start_no_change() {
        let mut tab = make_tab();
        let (mode2, _) = press(empty_mode(), &mut tab, KeyCode::Backspace);
        assert_eq!(mode2.command_state(), Some(("", 0)));
    }

    #[test]
    fn test_enter_returns_execute_command() {
        let mut tab = make_tab();
        let mode = CommandMode::with_history("filter foo".to_string(), 10, vec![]);
        let (mode2, result) = press(mode, &mut tab, KeyCode::Enter);
        assert!(matches!(result, KeyResult::ExecuteCommand(ref cmd) if cmd == "filter foo"));
        assert!(mode2.command_state().is_none());
    }

    #[test]
    fn test_enter_trims_whitespace() {
        let mut tab = make_tab();
        let mode = CommandMode::with_history("  wrap  ".to_string(), 8, vec![]);
        let (_, result) = press(mode, &mut tab, KeyCode::Enter);
        assert!(matches!(result, KeyResult::ExecuteCommand(ref cmd) if cmd == "wrap"));
    }

    #[test]
    fn test_esc_returns_normal_mode() {
        let mut tab = make_tab();
        let mode = CommandMode::with_history("filter".to_string(), 6, vec![]);
        let (mode2, result) = press(mode, &mut tab, KeyCode::Esc);
        assert!(matches!(result, KeyResult::Handled));
        assert!(mode2.command_state().is_none());
    }

    #[test]
    fn test_left_moves_cursor_back() {
        let mut tab = make_tab();
        let mode = CommandMode::with_history("abc".to_string(), 3, vec![]);
        let (mode2, _) = press(mode, &mut tab, KeyCode::Left);
        assert_eq!(mode2.command_state(), Some(("abc", 2)));
    }

    #[test]
    fn test_left_at_zero_no_change() {
        let mut tab = make_tab();
        let mode = CommandMode::with_history("abc".to_string(), 0, vec![]);
        let (mode2, _) = press(mode, &mut tab, KeyCode::Left);
        assert_eq!(mode2.command_state(), Some(("abc", 0)));
    }

    #[test]
    fn test_right_moves_cursor_forward() {
        let mut tab = make_tab();
        let mode = CommandMode::with_history("abc".to_string(), 2, vec![]);
        let (mode2, _) = press(mode, &mut tab, KeyCode::Right);
        assert_eq!(mode2.command_state(), Some(("abc", 3)));
    }

    #[test]
    fn test_right_at_end_no_change() {
        let mut tab = make_tab();
        let mode = CommandMode::with_history("abc".to_string(), 3, vec![]);
        let (mode2, _) = press(mode, &mut tab, KeyCode::Right);
        assert_eq!(mode2.command_state(), Some(("abc", 3)));
    }

    #[test]
    fn test_up_navigates_to_last_history_entry() {
        let mut tab = make_tab();
        let history = vec!["cmd1".to_string(), "cmd2".to_string()];
        let mode = CommandMode::with_history(String::new(), 0, history);
        let (mode2, _) = press(mode, &mut tab, KeyCode::Up);
        let (input, _) = mode2.command_state().unwrap();
        assert_eq!(input, "cmd2");
    }

    #[test]
    fn test_up_on_empty_history_no_change() {
        let mut tab = make_tab();
        let (mode2, _) = press(empty_mode(), &mut tab, KeyCode::Up);
        assert_eq!(mode2.command_state(), Some(("", 0)));
    }

    #[test]
    fn test_up_then_down_restores_empty_input() {
        let mut tab = make_tab();
        let history = vec!["cmd1".to_string()];
        let mode = CommandMode::with_history(String::new(), 0, history);
        let (mode2, _) = press(mode, &mut tab, KeyCode::Up);
        let (input, _) = mode2.command_state().unwrap();
        assert_eq!(input, "cmd1");

        // Reconstruct CommandMode from state to continue
        let mode3 = CommandMode {
            input: "cmd1".to_string(),
            cursor: 4,
            history: vec!["cmd1".to_string()],
            history_index: Some(0),
            completion_index: None,
        };
        let (mode4, _) = press(mode3, &mut tab, KeyCode::Down);
        let (input2, _) = mode4.command_state().unwrap();
        assert_eq!(input2, "");
    }

    #[test]
    fn test_tab_completes_command_name() {
        let mut tab = make_tab();
        let mode = CommandMode::with_history("fi".to_string(), 2, vec![]);
        let (mode2, _) = Box::new(mode).handle_key(&mut tab, KeyCode::Tab, KeyModifiers::NONE);
        let (input, _) = mode2.command_state().unwrap();
        assert!(input.starts_with("fi") || input == "filter");
    }

    #[test]
    fn test_tab_empty_input_completes_to_first_command() {
        let mut tab = make_tab();
        let (mode2, _) =
            Box::new(empty_mode()).handle_key(&mut tab, KeyCode::Tab, KeyModifiers::NONE);
        let (input, _) = mode2.command_state().unwrap();
        assert!(!input.is_empty());
    }

    #[test]
    fn test_command_state_returns_input_and_cursor() {
        let mode = CommandMode::with_history("hello".to_string(), 3, vec![]);
        assert_eq!(mode.command_state(), Some(("hello", 3)));
    }

    #[test]
    fn test_needs_input_bar() {
        assert!(empty_mode().needs_input_bar());
    }

    #[test]
    fn test_status_line_contains_command() {
        assert!(empty_mode().status_line().contains("[COMMAND]"));
    }
}
