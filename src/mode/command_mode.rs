use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::{
    auto_complete::{
        FieldCompletion, complete_color, complete_field_name, complete_field_value,
        complete_file_path, extract_color_partial, extract_field_partial, find_command_completions,
        fuzzy_match,
    },
    commands::FILE_PATH_COMMANDS,
    config::Keybindings,
    mode::{
        app_mode::{Mode, ModeRenderState, status_entry},
        filter_mode::FilterManagementMode,
        normal_mode::NormalMode,
    },
    theme::Theme,
    ui::{KeyResult, TabState},
};

use clap::{Parser, Subcommand};

/// If the input is `export ... -t <partial>` or `--template <partial>`,
/// returns the partial template name for completion.
fn extract_template_partial(input: &str) -> Option<&str> {
    let trimmed = input.trim();
    if !trimmed.starts_with("export") {
        return None;
    }
    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    if tokens.len() < 2 {
        return None;
    }
    let last = tokens[tokens.len() - 1];
    let second_last = tokens[tokens.len() - 2];

    if second_last == "-t" || second_last == "--template" {
        return Some(last);
    }
    if (last == "-t" || last == "--template") && input.ends_with(' ') {
        return Some("");
    }
    None
}

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
        /// Apply color to the whole line instead of only the matched text
        #[arg(short = 'l')]
        line_mode: bool,
        /// Treat pattern as key=value and match against the named parsed field
        #[arg(long = "field")]
        field: bool,
    },
    /// Add an exclude filter
    Exclude {
        pattern: String,
        /// Treat pattern as key=value and match against the named parsed field
        #[arg(long = "field")]
        field: bool,
    },
    /// Set color for the selected filter
    SetColor {
        #[arg(long)]
        fg: Option<String>,
        #[arg(long)]
        bg: Option<String>,
        /// Apply color to the whole line instead of only the matched text
        #[arg(short = 'l')]
        line_mode: bool,
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
    /// List running Docker containers and attach to one
    Docker,
    /// Toggle value-based color coding (HTTP methods, status codes, IPs, UUIDs)
    ValueColors,
    /// Export analysis (comments + marked lines) using a template
    Export {
        path: String,
        #[arg(short, long, default_value = "markdown")]
        template: String,
    },
    /// Filter log lines by timestamp range or comparison
    DateFilter {
        /// Date filter expression (e.g. "01:00 .. 02:00", "> 2024-02-22")
        expr: Vec<String>,
        #[arg(long)]
        fg: Option<String>,
        #[arg(long)]
        bg: Option<String>,
        /// Apply color to the whole line instead of only the timestamp
        #[arg(short = 'l')]
        line_mode: bool,
    },
    /// Toggle tail mode (always scroll to last line on new content)
    Tail,
    /// Show field keys alongside values (e.g. key=value) in structured log display
    ShowKeys,
    /// Hide field keys and show only values in structured log display
    HideKeys,
    /// Toggle raw mode — disable the format parser and show unformatted log lines
    Raw,
    /// Stop all incoming data for the current tab (file watcher and/or stream)
    Stop,
    /// Pause applying incoming data to the view (watcher/stream keeps running)
    Pause,
    /// Resume applying incoming data after a pause
    Resume,
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

    /// Compute the list of full replacement strings for the current input.
    /// Returns completions for whichever tier matches first:
    /// field → color → file path → theme → command name.
    fn compute_completions(&self, tab: &TabState) -> Vec<String> {
        let trimmed = self.input.trim().to_string();

        // Field name/value completion for `filter --field` and `exclude --field`.
        // Use trim_start() (left-trim only) to preserve any trailing space, which
        // signals that the preceding token is complete and the next one starts.
        let input_ls = self.input.trim_start();
        if let Some(fc) = extract_field_partial(input_ls) {
            let field_index = tab.build_field_index();
            let completions: Vec<String> = match &fc {
                FieldCompletion::Name(partial) => {
                    let prefix_end = input_ls.len() - partial.len();
                    let prefix = &input_ls[..prefix_end];
                    complete_field_name(partial, &field_index)
                        .into_iter()
                        .map(|n| format!("{prefix}{n}="))
                        .collect()
                }
                FieldCompletion::Value { field, partial } => {
                    let prefix_end = input_ls.len() - partial.len();
                    let prefix = &input_ls[..prefix_end];
                    complete_field_value(field, partial, &field_index)
                        .into_iter()
                        .map(|v| format!("{prefix}{v}"))
                        .collect()
                }
            };
            if !completions.is_empty() {
                return completions;
            }
        }

        // Color completion for --fg/--bg arguments
        if let Some(partial) = extract_color_partial(&trimmed) {
            let completions = complete_color(partial);
            if !completions.is_empty() {
                let prefix = if partial.is_empty() {
                    trimmed.clone()
                } else {
                    trimmed[..trimmed.len() - partial.len()].to_string()
                };
                return completions
                    .into_iter()
                    .map(|c| format!("{}{}", prefix, c))
                    .collect();
            }
        }

        // Template completion for export -t/--template
        if let Some(partial) = extract_template_partial(&trimmed) {
            let completions = crate::export::complete_template(partial);
            if !completions.is_empty() {
                let prefix = if partial.is_empty() {
                    trimmed.clone()
                } else {
                    trimmed[..trimmed.len() - partial.len()].to_string()
                };
                return completions
                    .into_iter()
                    .map(|c| format!("{}{}", prefix, c))
                    .collect();
            }
        }

        // File path completion
        let input_ltrimmed = self.input.trim_start();
        let file_cmd = FILE_PATH_COMMANDS
            .iter()
            .find(|cmd| input_ltrimmed.starts_with(&format!("{} ", cmd)));
        if let Some(&cmd) = file_cmd {
            let partial = input_ltrimmed[cmd.len()..].trim_start();
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
                return completions
                    .into_iter()
                    .map(|c| format!("{} {}", cmd, c))
                    .collect();
            }
            return vec![];
        }

        // Theme completion
        if let Some(after_prefix) = trimmed.strip_prefix("set-theme") {
            let partial = after_prefix.trim_start();
            let mut themes = Theme::list_available_themes();
            if !partial.is_empty() {
                themes.retain(|t| fuzzy_match(partial, t));
            }
            if !themes.is_empty() {
                return themes
                    .into_iter()
                    .map(|t| format!("set-theme {}", t))
                    .collect();
            }
            return vec![];
        }

        // Command name completion
        find_command_completions(&trimmed)
            .into_iter()
            .map(|s| s.to_string())
            .collect()
    }
}

#[async_trait]
impl Mode for CommandMode {
    async fn handle_key(
        mut self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        let kb = tab.keybindings.command.clone();
        if kb.confirm.matches(key, modifiers) {
            let cmd = self.input.trim().to_string();
            return (
                Box::new(NormalMode::default()),
                KeyResult::ExecuteCommand(cmd),
            );
        }
        if kb.cancel.matches(key, modifiers) {
            tab.editing_filter_id = None;
            if let Some(idx) = tab.filter_context.take() {
                return (
                    Box::new(FilterManagementMode {
                        selected_filter_index: idx,
                    }),
                    KeyResult::Handled,
                );
            }
            return (Box::new(NormalMode::default()), KeyResult::Handled);
        }
        match key {
            KeyCode::Backspace => {
                if self.cursor > 0 && !self.input.is_empty() {
                    self.input.remove(self.cursor - 1);
                    self.cursor -= 1;
                    self.completion_index = None;
                }
            }
            KeyCode::Char(' ') if self.completion_index.is_some() => {
                let idx = self.completion_index.unwrap();
                let completions = self.compute_completions(tab);
                if let Some(text) = completions.into_iter().nth(idx) {
                    self.input = text;
                    self.cursor = self.input.len();
                }
                self.completion_index = None;
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
                let completions = self.compute_completions(tab);
                if !completions.is_empty() {
                    let idx = match self.completion_index {
                        None => 0,
                        Some(i) => (i + 1) % completions.len(),
                    };
                    self.completion_index = Some(idx);
                }
            }
            KeyCode::BackTab => {
                let completions = self.compute_completions(tab);
                if !completions.is_empty() {
                    let idx = match self.completion_index {
                        None | Some(0) => completions.len() - 1,
                        Some(i) => i - 1,
                    };
                    self.completion_index = Some(idx);
                }
            }
            _ => {}
        }
        (self, KeyResult::Handled)
    }

    fn mode_bar_content(&self, kb: &Keybindings, theme: &Theme) -> Line<'static> {
        let mut spans: Vec<Span<'static>> = vec![Span::styled(
            "[COMMAND]  ",
            Style::default()
                .fg(theme.text_highlight_fg)
                .add_modifier(Modifier::BOLD),
        )];
        status_entry(&mut spans, kb.command.cancel.display(), "cancel", theme);
        status_entry(&mut spans, kb.command.confirm.display(), "execute", theme);
        status_entry(&mut spans, "Tab".to_string(), "complete", theme);
        Line::from(spans)
    }

    fn render_state(&self) -> ModeRenderState {
        ModeRenderState::Command {
            input: self.input.clone(),
            cursor: self.cursor,
            completion_index: self.completion_index,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::file_reader::FileReader;
    use crate::log_manager::LogManager;
    use crate::mode::app_mode::ModeRenderState;
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

    /// Extract (input, cursor) from a mode's render_state if it is Command.
    /// Returns None when the mode is not CommandMode.
    fn command_state(mode: &dyn Mode) -> Option<(String, usize)> {
        match mode.render_state() {
            ModeRenderState::Command { input, cursor, .. } => Some((input, cursor)),
            _ => None,
        }
    }

    /// Extract completion_index from a Command render_state.
    fn completion_index(mode: &dyn Mode) -> Option<usize> {
        match mode.render_state() {
            ModeRenderState::Command {
                completion_index, ..
            } => completion_index,
            _ => None,
        }
    }

    #[tokio::test]
    async fn test_char_appends_to_input() {
        let mut tab = make_tab().await;
        let (mode, result) = press(empty_mode(), &mut tab, KeyCode::Char('f')).await;
        assert!(matches!(result, KeyResult::Handled));
        assert_eq!(command_state(mode.as_ref()), Some(("f".to_string(), 1)));
    }

    #[tokio::test]
    async fn test_char_appends_to_existing_input() {
        let mut tab = make_tab().await;
        let mode = CommandMode::with_history("fil".to_string(), 3, vec![]);
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('t')).await;
        assert_eq!(command_state(mode2.as_ref()), Some(("filt".to_string(), 4)));
    }

    #[tokio::test]
    async fn test_backspace_removes_last_char() {
        let mut tab = make_tab().await;
        let mode = CommandMode::with_history("filter".to_string(), 6, vec![]);
        let (mode2, _) = press(mode, &mut tab, KeyCode::Backspace).await;
        assert_eq!(
            command_state(mode2.as_ref()),
            Some(("filte".to_string(), 5))
        );
    }

    #[tokio::test]
    async fn test_backspace_at_start_no_change() {
        let mut tab = make_tab().await;
        let (mode2, _) = press(empty_mode(), &mut tab, KeyCode::Backspace).await;
        assert_eq!(command_state(mode2.as_ref()), Some(("".to_string(), 0)));
    }

    #[tokio::test]
    async fn test_enter_returns_execute_command() {
        let mut tab = make_tab().await;
        let mode = CommandMode::with_history("filter foo".to_string(), 10, vec![]);
        let (mode2, result) = press(mode, &mut tab, KeyCode::Enter).await;
        assert!(matches!(result, KeyResult::ExecuteCommand(ref cmd) if cmd == "filter foo"));
        assert!(!matches!(
            mode2.render_state(),
            ModeRenderState::Command { .. }
        ));
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
        assert!(!matches!(
            mode2.render_state(),
            ModeRenderState::Command { .. }
        ));
        assert!(matches!(mode2.render_state(), ModeRenderState::Normal));
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
        assert!(matches!(
            mode2.render_state(),
            ModeRenderState::FilterManagement { .. }
        ));
    }

    #[tokio::test]
    async fn test_left_moves_cursor_back() {
        let mut tab = make_tab().await;
        let mode = CommandMode::with_history("abc".to_string(), 3, vec![]);
        let (mode2, _) = press(mode, &mut tab, KeyCode::Left).await;
        assert_eq!(command_state(mode2.as_ref()), Some(("abc".to_string(), 2)));
    }

    #[tokio::test]
    async fn test_left_at_zero_no_change() {
        let mut tab = make_tab().await;
        let mode = CommandMode::with_history("abc".to_string(), 0, vec![]);
        let (mode2, _) = press(mode, &mut tab, KeyCode::Left).await;
        assert_eq!(command_state(mode2.as_ref()), Some(("abc".to_string(), 0)));
    }

    #[tokio::test]
    async fn test_right_moves_cursor_forward() {
        let mut tab = make_tab().await;
        let mode = CommandMode::with_history("abc".to_string(), 2, vec![]);
        let (mode2, _) = press(mode, &mut tab, KeyCode::Right).await;
        assert_eq!(command_state(mode2.as_ref()), Some(("abc".to_string(), 3)));
    }

    #[tokio::test]
    async fn test_right_at_end_no_change() {
        let mut tab = make_tab().await;
        let mode = CommandMode::with_history("abc".to_string(), 3, vec![]);
        let (mode2, _) = press(mode, &mut tab, KeyCode::Right).await;
        assert_eq!(command_state(mode2.as_ref()), Some(("abc".to_string(), 3)));
    }

    #[tokio::test]
    async fn test_up_navigates_to_last_history_entry() {
        let mut tab = make_tab().await;
        let history = vec!["cmd1".to_string(), "cmd2".to_string()];
        let mode = CommandMode::with_history(String::new(), 0, history);
        let (mode2, _) = press(mode, &mut tab, KeyCode::Up).await;
        let (input, _) = command_state(mode2.as_ref()).unwrap();
        assert_eq!(input, "cmd2");
    }

    #[tokio::test]
    async fn test_up_on_empty_history_no_change() {
        let mut tab = make_tab().await;
        let (mode2, _) = press(empty_mode(), &mut tab, KeyCode::Up).await;
        assert_eq!(command_state(mode2.as_ref()), Some(("".to_string(), 0)));
    }

    #[tokio::test]
    async fn test_up_then_down_restores_empty_input() {
        let mut tab = make_tab().await;
        let history = vec!["cmd1".to_string()];
        let mode = CommandMode::with_history(String::new(), 0, history);
        let (mode2, _) = press(mode, &mut tab, KeyCode::Up).await;
        let (input, _) = command_state(mode2.as_ref()).unwrap();
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
        let (input2, _) = command_state(mode4.as_ref()).unwrap();
        assert_eq!(input2, "");
    }

    #[tokio::test]
    async fn test_tab_highlights_without_changing_input() {
        let mut tab = make_tab().await;
        let mode = CommandMode::with_history("fi".to_string(), 2, vec![]);
        let (mode2, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Tab, KeyModifiers::NONE)
            .await;
        // Input stays unchanged
        let (input, _) = command_state(mode2.as_ref()).unwrap();
        assert_eq!(input, "fi");
        // But completion_index is set
        assert_eq!(completion_index(mode2.as_ref()), Some(0));
    }

    #[tokio::test]
    async fn test_tab_empty_input_highlights_first_command() {
        let mut tab = make_tab().await;
        let (mode2, _) = Box::new(empty_mode())
            .handle_key(&mut tab, KeyCode::Tab, KeyModifiers::NONE)
            .await;
        // Input stays empty
        let (input, _) = command_state(mode2.as_ref()).unwrap();
        assert_eq!(input, "");
        // But completion_index is set
        assert_eq!(completion_index(mode2.as_ref()), Some(0));
    }

    #[tokio::test]
    async fn test_tab_cycles_completion_index() {
        let mut tab = make_tab().await;
        // "fi" matches "filter", "filtering", etc.
        let mode = CommandMode::with_history("fi".to_string(), 2, vec![]);
        let (mode2, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Tab, KeyModifiers::NONE)
            .await;
        assert_eq!(completion_index(mode2.as_ref()), Some(0));

        let (mode3, _) = mode2
            .handle_key(&mut tab, KeyCode::Tab, KeyModifiers::NONE)
            .await;
        assert_eq!(completion_index(mode3.as_ref()), Some(1));
    }

    #[tokio::test]
    async fn test_backtab_cycles_backward() {
        let mut tab = make_tab().await;
        let mode = CommandMode::with_history("fi".to_string(), 2, vec![]);
        // Tab → index 0
        let (mode2, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Tab, KeyModifiers::NONE)
            .await;
        assert_eq!(completion_index(mode2.as_ref()), Some(0));
        // BackTab from 0 → wraps to last
        let (mode3, _) = mode2
            .handle_key(&mut tab, KeyCode::BackTab, KeyModifiers::NONE)
            .await;
        let idx = completion_index(mode3.as_ref()).unwrap();
        assert!(idx > 0, "BackTab from 0 should wrap to last index");
    }

    #[tokio::test]
    async fn test_typing_resets_completion_index() {
        let mut tab = make_tab().await;
        let mode = CommandMode::with_history("fi".to_string(), 2, vec![]);
        let (mode2, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Tab, KeyModifiers::NONE)
            .await;
        assert!(completion_index(mode2.as_ref()).is_some());
        // Typing a char resets completion
        let (mode3, _) = mode2
            .handle_key(&mut tab, KeyCode::Char('l'), KeyModifiers::NONE)
            .await;
        assert!(completion_index(mode3.as_ref()).is_none());
    }

    #[test]
    fn test_command_state_returns_input_and_cursor() {
        let mode = CommandMode::with_history("hello".to_string(), 3, vec![]);
        assert_eq!(command_state(&mode), Some(("hello".to_string(), 3)));
    }

    #[test]
    fn test_render_state_is_command() {
        assert!(matches!(
            empty_mode().render_state(),
            ModeRenderState::Command { .. }
        ));
    }

    #[test]
    fn test_mode_bar_content_contains_command() {
        assert!(matches!(
            empty_mode().render_state(),
            ModeRenderState::Command { .. }
        ));
    }

    #[tokio::test]
    async fn test_tab_open_with_no_path_highlights_completion() {
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

        // "open " with trailing space but no path → Tab highlights first completion
        let mode = CommandMode::with_history("open ".to_string(), 5, vec![]);
        let (mode2, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Tab, KeyModifiers::NONE)
            .await;
        let (input, _) = command_state(mode2.as_ref()).unwrap();
        // Input stays unchanged
        assert_eq!(input, "open ");
        // Completion is highlighted
        assert_eq!(completion_index(mode2.as_ref()), Some(0));

        // Space accepts the highlighted completion into input
        let (mode3, result) = mode2
            .handle_key(&mut tab, KeyCode::Char(' '), KeyModifiers::NONE)
            .await;
        assert!(matches!(result, KeyResult::Handled));
        let (accepted, _) = command_state(mode3.as_ref()).unwrap();
        assert!(
            accepted.starts_with("open "),
            "Should start with 'open ', got: {accepted}"
        );
        assert!(
            accepted.contains(path.to_str().unwrap()),
            "Should complete into the open file's directory, got: {accepted}"
        );
        assert!(completion_index(mode3.as_ref()).is_none());
    }

    #[tokio::test]
    async fn test_tab_open_with_fuzzy_path_highlights() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        std::fs::write(path.join("application.log"), b"data").unwrap();
        std::fs::write(path.join("error.txt"), b"data").unwrap();

        // "alog" fuzzy-matches "application.log" (a…l…o…g)
        let partial = format!("{}/alog", path.to_str().unwrap());
        let input_str = format!("open {}", partial);
        let cursor = input_str.len();
        let mode = CommandMode::with_history(input_str.clone(), cursor, vec![]);
        let mut tab = make_tab().await;
        let (mode2, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Tab, KeyModifiers::NONE)
            .await;
        // Input stays unchanged after Tab
        let (input, _) = command_state(mode2.as_ref()).unwrap();
        assert_eq!(input, input_str);
        assert_eq!(completion_index(mode2.as_ref()), Some(0));

        // Space accepts the highlighted completion into input
        let (mode3, result) = mode2
            .handle_key(&mut tab, KeyCode::Char(' '), KeyModifiers::NONE)
            .await;
        assert!(matches!(result, KeyResult::Handled));
        let (accepted, _) = command_state(mode3.as_ref()).unwrap();
        assert!(
            accepted.ends_with("application.log"),
            "Should accept application.log, got: {accepted}"
        );
        assert!(completion_index(mode3.as_ref()).is_none());
    }

    #[tokio::test]
    async fn test_space_accepts_completion_into_input() {
        let mut tab = make_tab().await;
        // Type "fi" then Tab to highlight first completion
        let mode = CommandMode::with_history("fi".to_string(), 2, vec![]);
        let (mode2, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Tab, KeyModifiers::NONE)
            .await;
        // Input stays "fi", completion_index is set
        assert_eq!(command_state(mode2.as_ref()).unwrap().0, "fi");
        assert_eq!(completion_index(mode2.as_ref()), Some(0));

        // Press Space — should accept highlighted completion into input
        let (mode3, result) = mode2
            .handle_key(&mut tab, KeyCode::Char(' '), KeyModifiers::NONE)
            .await;
        assert!(matches!(result, KeyResult::Handled));
        // Still in command mode with accepted text
        let input_after_accept = command_state(mode3.as_ref()).unwrap().0.clone();
        assert!(
            !input_after_accept.is_empty(),
            "Input should be filled with accepted completion"
        );
        assert!(completion_index(mode3.as_ref()).is_none());

        // Enter — should execute the command
        let (mode4, result2) = mode3
            .handle_key(&mut tab, KeyCode::Enter, KeyModifiers::NONE)
            .await;
        assert!(matches!(result2, KeyResult::ExecuteCommand(_)));
        assert!(!matches!(
            mode4.render_state(),
            ModeRenderState::Command { .. }
        ));
    }

    #[tokio::test]
    async fn test_enter_during_completion_executes_raw_input() {
        let mut tab = make_tab().await;
        // Type "wrap" then Tab to highlight completion
        let mode = CommandMode::with_history("wrap".to_string(), 4, vec![]);
        let (mode2, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Tab, KeyModifiers::NONE)
            .await;
        assert_eq!(completion_index(mode2.as_ref()), Some(0));

        // Enter always executes the current input text, ignoring completion highlight
        let (mode3, result) = mode2
            .handle_key(&mut tab, KeyCode::Enter, KeyModifiers::NONE)
            .await;
        assert!(
            matches!(result, KeyResult::ExecuteCommand(ref cmd) if cmd == "wrap"),
            "Enter should execute the input text directly"
        );
        assert!(!matches!(
            mode3.render_state(),
            ModeRenderState::Command { .. }
        ));
    }

    #[tokio::test]
    async fn test_space_without_completion_inserts_space() {
        let mut tab = make_tab().await;
        let mode = CommandMode::with_history("filter".to_string(), 6, vec![]);
        // No Tab pressed, so completion_index is None
        let (mode2, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Char(' '), KeyModifiers::NONE)
            .await;
        let (input, cursor) = command_state(mode2.as_ref()).unwrap();
        assert_eq!(input, "filter ");
        assert_eq!(cursor, 7);
    }

    async fn make_json_tab() -> TabState {
        let json_lines = b"{\"level\":\"info\",\"target\":\"app\",\"message\":\"hello\"}\n\
              {\"level\":\"error\",\"target\":\"db\",\"message\":\"fail\"}\n"
            .to_vec();
        let file_reader = crate::file_reader::FileReader::from_bytes(json_lines);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        TabState::new(file_reader, log_manager, "test.json".to_string())
    }

    #[tokio::test]
    async fn test_field_completion_name_after_space() {
        let mut tab = make_json_tab().await;
        // "filter --field " (trailing space) → field name completions
        let mode = CommandMode::with_history("filter --field ".to_string(), 14, vec![]);
        let (mode2, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Tab, KeyModifiers::NONE)
            .await;
        // completion_index must be set (completions were found)
        assert!(
            completion_index(mode2.as_ref()).is_some(),
            "Tab after '--field ' should produce field name completions"
        );
        // Space accepts the highlighted completion; input must include field name + '='
        let (mode3, _) = mode2
            .handle_key(&mut tab, KeyCode::Char(' '), KeyModifiers::NONE)
            .await;
        let (accepted, _) = command_state(mode3.as_ref()).unwrap();
        assert!(
            accepted.starts_with("filter --field "),
            "Accepted completion should preserve prefix, got: {accepted}"
        );
        assert!(
            accepted.contains('='),
            "Accepted field name completion should include '=' ready for value entry, got: {accepted}"
        );
    }

    #[tokio::test]
    async fn test_field_completion_value_after_eq() {
        let mut tab = make_json_tab().await;
        // "filter --field level=" → value completions
        let input = "filter --field level=".to_string();
        let cursor = input.len();
        let mode = CommandMode::with_history(input, cursor, vec![]);
        let (mode2, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Tab, KeyModifiers::NONE)
            .await;
        assert!(
            completion_index(mode2.as_ref()).is_some(),
            "Tab after 'level=' should produce value completions"
        );
        // Accepted completion should contain a known level value
        let (mode3, _) = mode2
            .handle_key(&mut tab, KeyCode::Char(' '), KeyModifiers::NONE)
            .await;
        let (accepted, _) = command_state(mode3.as_ref()).unwrap();
        assert!(
            accepted.contains("level=info") || accepted.contains("level=error"),
            "Accepted value completion should be a known level, got: {accepted}"
        );
    }
}
