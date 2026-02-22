use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyModifiers};

use crate::{
    auto_complete::{
        complete_color, complete_file_path, extract_color_partial, find_command_completions,
        fuzzy_match,
    },
    mode::{app_mode::Mode, filter_mode::FilterManagementMode, normal_mode::NormalMode},
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
    /// Remove all filter definitions
    ClearFilters,
    /// Disable all filters without removing them
    DisableFilters,
    /// Enable all filters
    EnableFilters,
    /// Toggle global filtering on/off
    Filtering,
    /// Hide a JSON field by name or 0-based index
    HideField { field: String },
    /// Show a hidden JSON field by name or 0-based index
    ShowField { field: String },
    /// Clear all hidden fields
    ShowAllFields,
    /// Open a modal to select which JSON fields to display
    SelectFields,
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

#[async_trait]
impl Mode for CommandMode {
    async fn handle_key(
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
                tab.editing_filter_id = None;
                if let Some(idx) = tab.filter_context.take() {
                    return (
                        Box::new(FilterManagementMode {
                            selected_filter_index: idx,
                        }),
                        KeyResult::Handled,
                    );
                }
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

                // File path completion — use the un-right-trimmed input so that a trailing
                // space after the command name (e.g. "open ") is preserved for detection.
                let file_commands = ["open", "load-filters", "save-filters", "export-marked"];
                let input_ltrimmed = self.input.trim_start();
                let file_cmd = file_commands
                    .iter()
                    .find(|cmd| input_ltrimmed.starts_with(&format!("{} ", cmd)));
                if let Some(&cmd) = file_cmd {
                    let partial = input_ltrimmed[cmd.len()..].trim_start();
                    // When nothing is typed yet, default to the directory of the currently open file.
                    let default_dir: String;
                    let effective_partial: &str = if partial.is_empty() {
                        default_dir = tab
                            .log_manager
                            .source_file()
                            .and_then(|p| std::path::Path::new(p).parent())
                            .and_then(|d| d.to_str())
                            .map(|d| format!("{}/", d))
                            .unwrap_or_default();
                        &default_dir
                    } else {
                        partial
                    };
                    let completions = complete_file_path(effective_partial);
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

    async fn make_tab() -> TabState {
        let file_reader = FileReader::from_bytes(b"line1\nline2\n".to_vec());
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        TabState::new(file_reader, log_manager, "test".to_string())
    }

    fn empty_mode() -> CommandMode {
        CommandMode::with_history(String::new(), 0, vec![])
    }

    async fn press(
        mode: CommandMode,
        tab: &mut TabState,
        code: KeyCode,
    ) -> (Box<dyn Mode>, KeyResult) {
        Box::new(mode)
            .handle_key(tab, code, KeyModifiers::NONE)
            .await
    }

    #[tokio::test]
    async fn test_char_appends_to_input() {
        let mut tab = make_tab().await;
        let (mode, result) = press(empty_mode(), &mut tab, KeyCode::Char('f')).await;
        assert!(matches!(result, KeyResult::Handled));
        assert_eq!(mode.command_state(), Some(("f", 1)));
    }

    #[tokio::test]
    async fn test_char_appends_to_existing_input() {
        let mut tab = make_tab().await;
        let mode = CommandMode::with_history("fil".to_string(), 3, vec![]);
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('t')).await;
        assert_eq!(mode2.command_state(), Some(("filt", 4)));
    }

    #[tokio::test]
    async fn test_backspace_removes_last_char() {
        let mut tab = make_tab().await;
        let mode = CommandMode::with_history("filter".to_string(), 6, vec![]);
        let (mode2, _) = press(mode, &mut tab, KeyCode::Backspace).await;
        assert_eq!(mode2.command_state(), Some(("filte", 5)));
    }

    #[tokio::test]
    async fn test_backspace_at_start_no_change() {
        let mut tab = make_tab().await;
        let (mode2, _) = press(empty_mode(), &mut tab, KeyCode::Backspace).await;
        assert_eq!(mode2.command_state(), Some(("", 0)));
    }

    #[tokio::test]
    async fn test_enter_returns_execute_command() {
        let mut tab = make_tab().await;
        let mode = CommandMode::with_history("filter foo".to_string(), 10, vec![]);
        let (mode2, result) = press(mode, &mut tab, KeyCode::Enter).await;
        assert!(matches!(result, KeyResult::ExecuteCommand(ref cmd) if cmd == "filter foo"));
        assert!(mode2.command_state().is_none());
    }

    #[tokio::test]
    async fn test_enter_trims_whitespace() {
        let mut tab = make_tab().await;
        let mode = CommandMode::with_history("  wrap  ".to_string(), 8, vec![]);
        let (_, result) = press(mode, &mut tab, KeyCode::Enter).await;
        assert!(matches!(result, KeyResult::ExecuteCommand(ref cmd) if cmd == "wrap"));
    }

    #[tokio::test]
    async fn test_esc_without_filter_context_returns_normal_mode() {
        let mut tab = make_tab().await;
        let mode = CommandMode::with_history("filter".to_string(), 6, vec![]);
        let (mode2, result) = press(mode, &mut tab, KeyCode::Esc).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(mode2.command_state().is_none());
        assert!(mode2.status_line().contains("[NORMAL]"));
    }

    #[tokio::test]
    async fn test_esc_with_filter_context_returns_filter_management_mode() {
        let mut tab = make_tab().await;
        tab.filter_context = Some(2);
        let mode = CommandMode::with_history("set-color --fg Red".to_string(), 18, vec![]);
        let (mode2, result) = press(mode, &mut tab, KeyCode::Esc).await;
        assert!(matches!(result, KeyResult::Handled));
        // filter_context consumed
        assert!(tab.filter_context.is_none());
        // returned to filter management mode, not normal mode
        assert!(mode2.status_line().contains("[FILTER]"));
    }

    #[tokio::test]
    async fn test_left_moves_cursor_back() {
        let mut tab = make_tab().await;
        let mode = CommandMode::with_history("abc".to_string(), 3, vec![]);
        let (mode2, _) = press(mode, &mut tab, KeyCode::Left).await;
        assert_eq!(mode2.command_state(), Some(("abc", 2)));
    }

    #[tokio::test]
    async fn test_left_at_zero_no_change() {
        let mut tab = make_tab().await;
        let mode = CommandMode::with_history("abc".to_string(), 0, vec![]);
        let (mode2, _) = press(mode, &mut tab, KeyCode::Left).await;
        assert_eq!(mode2.command_state(), Some(("abc", 0)));
    }

    #[tokio::test]
    async fn test_right_moves_cursor_forward() {
        let mut tab = make_tab().await;
        let mode = CommandMode::with_history("abc".to_string(), 2, vec![]);
        let (mode2, _) = press(mode, &mut tab, KeyCode::Right).await;
        assert_eq!(mode2.command_state(), Some(("abc", 3)));
    }

    #[tokio::test]
    async fn test_right_at_end_no_change() {
        let mut tab = make_tab().await;
        let mode = CommandMode::with_history("abc".to_string(), 3, vec![]);
        let (mode2, _) = press(mode, &mut tab, KeyCode::Right).await;
        assert_eq!(mode2.command_state(), Some(("abc", 3)));
    }

    #[tokio::test]
    async fn test_up_navigates_to_last_history_entry() {
        let mut tab = make_tab().await;
        let history = vec!["cmd1".to_string(), "cmd2".to_string()];
        let mode = CommandMode::with_history(String::new(), 0, history);
        let (mode2, _) = press(mode, &mut tab, KeyCode::Up).await;
        let (input, _) = mode2.command_state().unwrap();
        assert_eq!(input, "cmd2");
    }

    #[tokio::test]
    async fn test_up_on_empty_history_no_change() {
        let mut tab = make_tab().await;
        let (mode2, _) = press(empty_mode(), &mut tab, KeyCode::Up).await;
        assert_eq!(mode2.command_state(), Some(("", 0)));
    }

    #[tokio::test]
    async fn test_up_then_down_restores_empty_input() {
        let mut tab = make_tab().await;
        let history = vec!["cmd1".to_string()];
        let mode = CommandMode::with_history(String::new(), 0, history);
        let (mode2, _) = press(mode, &mut tab, KeyCode::Up).await;
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
        let (mode4, _) = press(mode3, &mut tab, KeyCode::Down).await;
        let (input2, _) = mode4.command_state().unwrap();
        assert_eq!(input2, "");
    }

    #[tokio::test]
    async fn test_tab_completes_command_name() {
        let mut tab = make_tab().await;
        let mode = CommandMode::with_history("fi".to_string(), 2, vec![]);
        let (mode2, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Tab, KeyModifiers::NONE)
            .await;
        let (input, _) = mode2.command_state().unwrap();
        assert!(input.starts_with("fi") || input == "filter");
    }

    #[tokio::test]
    async fn test_tab_empty_input_completes_to_first_command() {
        let mut tab = make_tab().await;
        let (mode2, _) = Box::new(empty_mode())
            .handle_key(&mut tab, KeyCode::Tab, KeyModifiers::NONE)
            .await;
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

    #[tokio::test]
    async fn test_tab_open_with_no_path_defaults_to_open_file_directory() {
        use crate::log_manager::LogManager;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        std::fs::write(path.join("server.log"), b"data").unwrap();

        // Tab with a source_file in the temp dir
        let source = path.join("existing.log");
        let file_reader = crate::file_reader::FileReader::from_bytes(b"line1\n".to_vec());
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, Some(source.to_str().unwrap().to_string())).await;
        let mut tab = TabState::new(file_reader, log_manager, "existing.log".to_string());

        // "open " with trailing space but no path → Tab should list files in source_file's directory
        let mode = CommandMode::with_history("open ".to_string(), 5, vec![]);
        let (mode2, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Tab, KeyModifiers::NONE)
            .await;
        let (input, _) = mode2.command_state().unwrap();
        assert!(
            input.starts_with("open "),
            "Input should start with 'open '"
        );
        assert!(
            input.contains(path.to_str().unwrap()),
            "Should complete into the open file's directory, got: {input}"
        );
    }

    #[tokio::test]
    async fn test_tab_open_with_fuzzy_path_matches() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        std::fs::write(path.join("application.log"), b"data").unwrap();
        std::fs::write(path.join("error.txt"), b"data").unwrap();

        // "alog" fuzzy-matches "application.log" (a…l…o…g)
        let partial = format!("{}/alog", path.to_str().unwrap());
        let mode =
            CommandMode::with_history(format!("open {}", partial), partial.len() + 5, vec![]);
        let mut tab = make_tab().await;
        let (mode2, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Tab, KeyModifiers::NONE)
            .await;
        let (input, _) = mode2.command_state().unwrap();
        assert!(
            input.ends_with("application.log"),
            "Fuzzy 'alog' should match application.log, got: {input}"
        );
    }
}
