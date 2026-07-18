use crate::{
    commands::auto_complete::{complete_file_path, fuzzy_match, schema_completion_names},
    config::Keybindings,
    mode::app_mode::{Mode, ModeRenderState, status_entry},
    mode::normal_mode::NormalMode,
    theme::Theme,
    ui::{KeyResult, TabState},
};
use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use std::collections::HashMap;

/// One row: a known format name (custom schema or built-in) and its
/// currently configured default filter path, if any.
#[derive(Debug, Clone)]
pub struct DefaultFilterRow {
    pub name: String,
    pub is_custom: bool,
    pub path: Option<String>,
}

/// Free-text path editor for the currently selected row, with the same
/// Tab-cycling autocomplete UX as the command line's file-path completion
/// (`CommandMode`'s `completion_query`/`completion_index`).
#[derive(Debug, Clone)]
pub struct PathEditState {
    pub input: String,
    pub cursor: usize,
    /// Text as typed before the first Tab press this "session" — completions
    /// are always computed from this (not the possibly-already-cycled
    /// `input`), so repeated Tabs keep cycling the same candidate list.
    pub query: Option<String>,
    /// Index into the completions computed from `query`, for highlighting
    /// the selected candidate in the hint list.
    pub completion_index: Option<usize>,
}

#[derive(Debug)]
pub struct DefaultFiltersMode {
    pub rows: Vec<DefaultFilterRow>,
    pub search: String,
    /// Index into the *visible* (filtered) row list.
    pub selected: usize,
    pub editing: Option<PathEditState>,
}

impl DefaultFiltersMode {
    /// Builds rows from every known format name (custom schemas first
    /// alphabetical, then built-ins alphabetical — same ordering/grouping
    /// `:schema`'s autocomplete uses), paired with the currently configured
    /// mapping.
    pub fn new(custom_names: &[String], current: &HashMap<String, String>) -> Self {
        let rows = schema_completion_names(custom_names)
            .into_iter()
            .map(|(name, is_custom)| DefaultFilterRow {
                path: current.get(&name).cloned(),
                name,
                is_custom,
            })
            .collect();
        DefaultFiltersMode {
            rows,
            search: String::new(),
            selected: 0,
            editing: None,
        }
    }

    fn visible_rows(&self) -> Vec<usize> {
        if self.search.is_empty() {
            return (0..self.rows.len()).collect();
        }
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| fuzzy_match(&self.search, &r.name))
            .map(|(i, _)| i)
            .collect()
    }

    fn clamp_selected(&mut self) {
        let count = self.visible_rows().len();
        if count == 0 {
            self.selected = 0;
        } else if self.selected >= count {
            self.selected = count - 1;
        }
    }

    fn handle_list_key(
        mut self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        let kb = &tab.interaction.keybindings;

        if kb.navigation.scroll_down.matches(key, modifiers) {
            let count = self.visible_rows().len();
            if count > 0 {
                self.selected = (self.selected + 1).min(count - 1);
            }
            return (self, KeyResult::Handled);
        }
        if kb.navigation.scroll_up.matches(key, modifiers) {
            self.selected = self.selected.saturating_sub(1);
            return (self, KeyResult::Handled);
        }
        if matches!(key, KeyCode::Enter) {
            let rows = self.visible_rows();
            if let Some(&idx) = rows.get(self.selected) {
                let existing = self.rows[idx].path.clone().unwrap_or_default();
                let cursor = existing.chars().count();
                self.editing = Some(PathEditState {
                    input: existing,
                    cursor,
                    query: None,
                    completion_index: None,
                });
            }
            return (self, KeyResult::Handled);
        }
        if matches!(key, KeyCode::Char('d'))
            && !modifiers.contains(KeyModifiers::CONTROL)
            && self.search.is_empty()
        {
            let rows = self.visible_rows();
            if let Some(&idx) = rows.get(self.selected)
                && self.rows[idx].path.is_some()
            {
                self.rows[idx].path = None;
                let format = self.rows[idx].name.clone();
                return (self, KeyResult::SetDefaultFilterFile { format, path: None });
            }
            return (self, KeyResult::Handled);
        }
        if matches!(key, KeyCode::Esc) {
            if !self.search.is_empty() {
                self.search.clear();
                self.selected = 0;
                return (self, KeyResult::Handled);
            }
            return (Box::new(NormalMode::default()), KeyResult::Handled);
        }
        match key {
            KeyCode::Char(c) if !modifiers.contains(KeyModifiers::CONTROL) => {
                self.search.push(c);
                self.selected = 0;
                self.clamp_selected();
                (self, KeyResult::Handled)
            }
            KeyCode::Backspace => {
                self.search.pop();
                self.selected = 0;
                self.clamp_selected();
                (self, KeyResult::Handled)
            }
            _ => (self, KeyResult::Ignored),
        }
    }

    fn handle_edit_key(
        mut self: Box<Self>,
        _tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        match key {
            KeyCode::Enter => {
                let rows = self.visible_rows();
                let Some(&idx) = rows.get(self.selected) else {
                    self.editing = None;
                    return (self, KeyResult::Handled);
                };
                let input = self.editing.as_ref().unwrap().input.trim().to_string();
                let path = if input.is_empty() { None } else { Some(input) };
                self.rows[idx].path = path.clone();
                let format = self.rows[idx].name.clone();
                self.editing = None;
                (self, KeyResult::SetDefaultFilterFile { format, path })
            }
            KeyCode::Esc => {
                self.editing = None;
                (self, KeyResult::Handled)
            }
            KeyCode::Tab => {
                let editing = self.editing.as_mut().unwrap();
                if editing.query.is_none() {
                    editing.query = Some(editing.input.clone());
                }
                let completions = complete_file_path(editing.query.as_deref().unwrap());
                if !completions.is_empty() {
                    let idx = match editing.completion_index {
                        None => 0,
                        Some(i) => (i + 1) % completions.len(),
                    };
                    editing.completion_index = Some(idx);
                    editing.input = completions[idx].clone();
                    editing.cursor = editing.input.chars().count();
                }
                (self, KeyResult::Handled)
            }
            KeyCode::BackTab => {
                let editing = self.editing.as_mut().unwrap();
                if editing.query.is_none() {
                    editing.query = Some(editing.input.clone());
                }
                let completions = complete_file_path(editing.query.as_deref().unwrap());
                if !completions.is_empty() {
                    let idx = match editing.completion_index {
                        None | Some(0) => completions.len() - 1,
                        Some(i) => i - 1,
                    };
                    editing.completion_index = Some(idx);
                    editing.input = completions[idx].clone();
                    editing.cursor = editing.input.chars().count();
                }
                (self, KeyResult::Handled)
            }
            KeyCode::Backspace => {
                let editing = self.editing.as_mut().unwrap();
                if let Some(query) = editing.query.take() {
                    editing.input = query;
                    editing.cursor = editing.input.chars().count();
                    editing.completion_index = None;
                }
                if editing.cursor > 0 {
                    let byte_pos = editing
                        .input
                        .char_indices()
                        .nth(editing.cursor - 1)
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    let end_pos = editing
                        .input
                        .char_indices()
                        .nth(editing.cursor)
                        .map(|(i, _)| i)
                        .unwrap_or(editing.input.len());
                    editing.input.replace_range(byte_pos..end_pos, "");
                    editing.cursor -= 1;
                }
                (self, KeyResult::Handled)
            }
            KeyCode::Left => {
                let editing = self.editing.as_mut().unwrap();
                editing.cursor = editing.cursor.saturating_sub(1);
                (self, KeyResult::Handled)
            }
            KeyCode::Right => {
                let editing = self.editing.as_mut().unwrap();
                let max = editing.input.chars().count();
                editing.cursor = (editing.cursor + 1).min(max);
                (self, KeyResult::Handled)
            }
            KeyCode::Char(c)
                if !modifiers.contains(KeyModifiers::CONTROL)
                    && !modifiers.contains(KeyModifiers::ALT) =>
            {
                let editing = self.editing.as_mut().unwrap();
                if let Some(query) = editing.query.take() {
                    editing.input = query;
                    editing.cursor = editing.input.chars().count();
                    editing.completion_index = None;
                }
                let byte_pos = editing
                    .input
                    .char_indices()
                    .nth(editing.cursor)
                    .map(|(i, _)| i)
                    .unwrap_or(editing.input.len());
                editing.input.insert(byte_pos, c);
                editing.cursor += 1;
                (self, KeyResult::Handled)
            }
            _ => (self, KeyResult::Handled),
        }
    }
}

#[async_trait]
impl Mode for DefaultFiltersMode {
    async fn handle_key(
        self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        if self.editing.is_some() {
            self.handle_edit_key(tab, key, modifiers)
        } else {
            self.handle_list_key(tab, key, modifiers)
        }
    }

    fn mode_bar_content(&self, _kb: &Keybindings, theme: &Theme) -> Line<'static> {
        let title_style = Style::default()
            .fg(theme.text_highlight_fg)
            .add_modifier(Modifier::BOLD);
        if self.editing.is_some() {
            let mut spans: Vec<Span<'static>> =
                vec![Span::styled("[DEFAULT FILTERS: EDIT]  ", title_style)];
            status_entry(&mut spans, "Tab".to_string(), "complete", theme);
            status_entry(&mut spans, "Enter".to_string(), "save", theme);
            status_entry(&mut spans, "Esc".to_string(), "cancel", theme);
            Line::from(spans)
        } else {
            let mut spans: Vec<Span<'static>> =
                vec![Span::styled("[DEFAULT FILTERS]  ", title_style)];
            status_entry(&mut spans, "Enter".to_string(), "edit", theme);
            status_entry(&mut spans, "d".to_string(), "clear", theme);
            status_entry(&mut spans, "Esc".to_string(), "close", theme);
            Line::from(spans)
        }
    }

    fn render_state(&self) -> ModeRenderState {
        ModeRenderState::DefaultFilters {
            rows: self.rows.clone(),
            search: self.search.clone(),
            selected: self.selected,
            editing: self.editing.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::db::LogManager;
    use crate::ingestion::FileReader;
    use std::sync::Arc;

    async fn make_tab() -> TabState {
        let file_reader = FileReader::from_bytes(b"line\n".to_vec());
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        TabState::new(file_reader, log_manager, "test".to_string())
    }

    fn sample_mode() -> DefaultFiltersMode {
        let custom_names = vec!["zeta".to_string(), "acme".to_string()];
        let mut current = HashMap::new();
        current.insert("acme".to_string(), "/tmp/acme.json".to_string());
        DefaultFiltersMode::new(&custom_names, &current)
    }

    async fn press(
        mode: DefaultFiltersMode,
        tab: &mut TabState,
        key: KeyCode,
    ) -> (Box<dyn Mode>, KeyResult) {
        Box::new(mode)
            .handle_key(tab, key, KeyModifiers::NONE)
            .await
    }

    async fn press_dyn(
        mode: Box<dyn Mode>,
        tab: &mut TabState,
        key: KeyCode,
    ) -> (Box<dyn Mode>, KeyResult) {
        mode.handle_key(tab, key, KeyModifiers::NONE).await
    }

    fn extract(
        state: ModeRenderState,
    ) -> (Vec<DefaultFilterRow>, String, usize, Option<PathEditState>) {
        match state {
            ModeRenderState::DefaultFilters {
                rows,
                search,
                selected,
                editing,
            } => (rows, search, selected, editing),
            other => panic!("expected DefaultFilters, got {other:?}"),
        }
    }

    #[test]
    fn test_new_orders_custom_first_alphabetical_then_builtin() {
        let mode = sample_mode();
        let custom: Vec<&str> = mode
            .rows
            .iter()
            .filter(|r| r.is_custom)
            .map(|r| r.name.as_str())
            .collect();
        assert_eq!(custom, vec!["acme", "zeta"]);
        assert!(mode.rows.iter().any(|r| !r.is_custom && r.name == "syslog"));
    }

    #[test]
    fn test_new_pairs_existing_mapping() {
        let mode = sample_mode();
        let acme = mode.rows.iter().find(|r| r.name == "acme").unwrap();
        assert_eq!(acme.path.as_deref(), Some("/tmp/acme.json"));
        let zeta = mode.rows.iter().find(|r| r.name == "zeta").unwrap();
        assert_eq!(zeta.path, None);
    }

    #[test]
    fn test_visible_rows_no_search() {
        let mode = sample_mode();
        assert_eq!(mode.visible_rows().len(), mode.rows.len());
    }

    #[test]
    fn test_visible_rows_with_search_filters_by_name() {
        let mut mode = sample_mode();
        mode.search = "acme".to_string();
        let rows = mode.visible_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(mode.rows[rows[0]].name, "acme");
    }

    #[tokio::test]
    async fn test_navigate_down() {
        let mut tab = make_tab().await;
        let mode = sample_mode();
        let (mode, _) = press(mode, &mut tab, KeyCode::Char('j')).await;
        let (_, _, selected, _) = extract(mode.render_state());
        assert_eq!(selected, 1);
    }

    #[tokio::test]
    async fn test_navigate_up_at_top() {
        let mut tab = make_tab().await;
        let mode = sample_mode();
        let (mode, _) = press(mode, &mut tab, KeyCode::Char('k')).await;
        let (_, _, selected, _) = extract(mode.render_state());
        assert_eq!(selected, 0);
    }

    #[tokio::test]
    async fn test_enter_opens_edit_with_existing_path_prefilled() {
        let mut tab = make_tab().await;
        let mode = sample_mode(); // selected=0 is "acme" (custom-first)
        let (mode, result) = press(mode, &mut tab, KeyCode::Enter).await;
        assert!(matches!(result, KeyResult::Handled));
        let (_, _, _, editing) = extract(mode.render_state());
        let editing = editing.unwrap();
        assert_eq!(editing.input, "/tmp/acme.json");
        assert_eq!(editing.cursor, 14);
    }

    #[tokio::test]
    async fn test_enter_opens_edit_with_empty_input_when_no_existing_mapping() {
        let mut tab = make_tab().await;
        let mode = sample_mode();
        let (mode, _) = press(mode, &mut tab, KeyCode::Char('j')).await; // move to "zeta"
        let (mode, _) = press_dyn(mode, &mut tab, KeyCode::Enter).await;
        let (_, _, _, editing) = extract(mode.render_state());
        let editing = editing.unwrap();
        assert_eq!(editing.input, String::new());
        assert_eq!(editing.cursor, 0);
    }

    #[tokio::test]
    async fn test_typing_in_edit_state_inserts_chars_at_cursor() {
        let mut tab = make_tab().await;
        let mode = sample_mode();
        let (mode, _) = press(mode, &mut tab, KeyCode::Enter).await; // edit "acme"
        let (mode, _) = press_dyn(mode, &mut tab, KeyCode::Char('x')).await;
        let (_, _, _, editing) = extract(mode.render_state());
        assert_eq!(editing.unwrap().input, "/tmp/acme.jsonx");
    }

    #[tokio::test]
    async fn test_backspace_in_edit_state_removes_char() {
        let mut tab = make_tab().await;
        let mode = sample_mode();
        let (mode, _) = press(mode, &mut tab, KeyCode::Enter).await;
        let (mode, _) = press_dyn(mode, &mut tab, KeyCode::Backspace).await;
        let (_, _, _, editing) = extract(mode.render_state());
        assert_eq!(editing.unwrap().input, "/tmp/acme.jso");
    }

    #[tokio::test]
    async fn test_tab_in_edit_state_cycles_file_path_completions() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.json"), "[]").unwrap();
        let mut tab = make_tab().await;
        let mut mode = sample_mode();
        mode.editing = Some(PathEditState {
            input: dir.path().to_str().unwrap().to_string() + "/",
            cursor: 0,
            query: None,
            completion_index: None,
        });
        let (mode, _) = press(mode, &mut tab, KeyCode::Tab).await;
        let (_, _, _, editing) = extract(mode.render_state());
        let editing = editing.unwrap();
        assert!(editing.input.ends_with("a.json"));
        assert_eq!(editing.completion_index, Some(0));
    }

    #[tokio::test]
    async fn test_tab_cycles_forward_through_multiple_completions_and_wraps() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.json"), "[]").unwrap();
        std::fs::write(dir.path().join("b.json"), "[]").unwrap();
        let mut tab = make_tab().await;
        let mut mode = sample_mode();
        mode.editing = Some(PathEditState {
            input: dir.path().to_str().unwrap().to_string() + "/",
            cursor: 0,
            query: None,
            completion_index: None,
        });
        let (mode, _) = press(mode, &mut tab, KeyCode::Tab).await;
        let (mode, _) = press_dyn(mode, &mut tab, KeyCode::Tab).await;
        let (mode, _) = press_dyn(mode, &mut tab, KeyCode::Tab).await;
        let (_, _, _, editing) = extract(mode.render_state());
        // Two candidates: third Tab wraps back to index 0.
        assert_eq!(editing.unwrap().completion_index, Some(0));
    }

    #[tokio::test]
    async fn test_backtab_cycles_backward() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.json"), "[]").unwrap();
        std::fs::write(dir.path().join("b.json"), "[]").unwrap();
        let mut tab = make_tab().await;
        let mut mode = sample_mode();
        mode.editing = Some(PathEditState {
            input: dir.path().to_str().unwrap().to_string() + "/",
            cursor: 0,
            query: None,
            completion_index: None,
        });
        let (mode, _) = press(mode, &mut tab, KeyCode::Tab).await; // index 0
        let (mode, _) = press_dyn(mode, &mut tab, KeyCode::Tab).await; // index 1
        let (mode, _) = press_dyn(mode, &mut tab, KeyCode::BackTab).await; // back to 0
        let (_, _, _, editing) = extract(mode.render_state());
        assert_eq!(editing.unwrap().completion_index, Some(0));
    }

    #[tokio::test]
    async fn test_backtab_from_none_wraps_to_last() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.json"), "[]").unwrap();
        std::fs::write(dir.path().join("b.json"), "[]").unwrap();
        let mut tab = make_tab().await;
        let mut mode = sample_mode();
        mode.editing = Some(PathEditState {
            input: dir.path().to_str().unwrap().to_string() + "/",
            cursor: 0,
            query: None,
            completion_index: None,
        });
        let (mode, _) = press(mode, &mut tab, KeyCode::BackTab).await;
        let (_, _, _, editing) = extract(mode.render_state());
        assert_eq!(editing.unwrap().completion_index, Some(1));
    }

    #[tokio::test]
    async fn test_typing_after_tab_reverts_to_original_query_before_inserting() {
        let mut tab = make_tab().await;
        let mode = sample_mode(); // editing "acme" would start at "/tmp/acme.json"
        let (mode, _) = press(mode, &mut tab, KeyCode::Enter).await;
        let (mode, _) = press_dyn(mode, &mut tab, KeyCode::Tab).await; // cycles input away from the typed query
        let (mode, _) = press_dyn(mode, &mut tab, KeyCode::Char('x')).await;
        let (_, _, _, editing) = extract(mode.render_state());
        let editing = editing.unwrap();
        // Typing must discard the Tab-selected candidate and resume from
        // what was actually typed ("/tmp/acme.json"), not from wherever Tab
        // had cycled to — same as CommandMode's file-path completion.
        assert_eq!(editing.input, "/tmp/acme.jsonx");
        assert_eq!(editing.completion_index, None);
        assert_eq!(editing.query, None);
    }

    #[tokio::test]
    async fn test_backspace_after_tab_reverts_to_original_query_before_removing() {
        let mut tab = make_tab().await;
        let mode = sample_mode();
        let (mode, _) = press(mode, &mut tab, KeyCode::Enter).await;
        let (mode, _) = press_dyn(mode, &mut tab, KeyCode::Tab).await;
        let (mode, _) = press_dyn(mode, &mut tab, KeyCode::Backspace).await;
        let (_, _, _, editing) = extract(mode.render_state());
        let editing = editing.unwrap();
        assert_eq!(editing.input, "/tmp/acme.jso");
        assert_eq!(editing.completion_index, None);
    }

    #[tokio::test]
    async fn test_enter_while_editing_emits_set_default_filter_file_with_path() {
        let mut tab = make_tab().await;
        let mode = sample_mode();
        let (mode, _) = press(mode, &mut tab, KeyCode::Enter).await;
        let (mode, _) = press_dyn(mode, &mut tab, KeyCode::Char('x')).await;
        let (mode, result) = press_dyn(mode, &mut tab, KeyCode::Enter).await;
        match result {
            KeyResult::SetDefaultFilterFile { format, path } => {
                assert_eq!(format, "acme");
                assert_eq!(path, Some("/tmp/acme.jsonx".to_string()));
            }
            other => panic!("expected SetDefaultFilterFile, got {other:?}"),
        }
        let (_, _, _, editing) = extract(mode.render_state());
        assert!(editing.is_none());
    }

    #[tokio::test]
    async fn test_enter_while_editing_empty_input_emits_clear() {
        let mut tab = make_tab().await;
        let mut mode = sample_mode();
        mode.editing = Some(PathEditState {
            input: String::new(),
            cursor: 0,
            query: None,
            completion_index: None,
        });
        let (_, result) = press(mode, &mut tab, KeyCode::Enter).await;
        match result {
            KeyResult::SetDefaultFilterFile { format, path } => {
                assert_eq!(format, "acme");
                assert_eq!(path, None);
            }
            other => panic!("expected SetDefaultFilterFile, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_esc_while_editing_cancels_without_emitting_key_result() {
        let mut tab = make_tab().await;
        let mode = sample_mode();
        let (mode, _) = press(mode, &mut tab, KeyCode::Enter).await;
        let (mode, result) = press_dyn(mode, &mut tab, KeyCode::Esc).await;
        assert!(matches!(result, KeyResult::Handled));
        let (rows, _, _, editing) = extract(mode.render_state());
        assert!(editing.is_none());
        // Row's path is unchanged (still "/tmp/acme.json"), edit was discarded.
        assert_eq!(
            rows.iter().find(|r| r.name == "acme").unwrap().path,
            Some("/tmp/acme.json".to_string())
        );
    }

    #[tokio::test]
    async fn test_delete_key_on_row_clears_immediately_without_entering_edit() {
        let mut tab = make_tab().await;
        let mode = sample_mode(); // selected=0 is "acme", has a mapping
        let (mode, result) = press(mode, &mut tab, KeyCode::Char('d')).await;
        match result {
            KeyResult::SetDefaultFilterFile { format, path } => {
                assert_eq!(format, "acme");
                assert_eq!(path, None);
            }
            other => panic!("expected SetDefaultFilterFile, got {other:?}"),
        }
        let (rows, _, _, editing) = extract(mode.render_state());
        assert!(editing.is_none());
        assert_eq!(rows.iter().find(|r| r.name == "acme").unwrap().path, None);
    }

    #[tokio::test]
    async fn test_delete_key_on_row_without_mapping_is_noop() {
        let mut tab = make_tab().await;
        let mode = sample_mode();
        let (mode, _) = press(mode, &mut tab, KeyCode::Char('j')).await; // "zeta", no mapping
        let (_, result) = press_dyn(mode, &mut tab, KeyCode::Char('d')).await;
        assert!(matches!(result, KeyResult::Handled));
    }

    #[tokio::test]
    async fn test_esc_with_search_clears_search_first() {
        let mut tab = make_tab().await;
        let mut mode = sample_mode();
        mode.search = "ac".to_string();
        let (mode, result) = press(mode, &mut tab, KeyCode::Esc).await;
        assert!(matches!(result, KeyResult::Handled));
        let (_, search, _, _) = extract(mode.render_state());
        assert!(search.is_empty());
    }

    #[tokio::test]
    async fn test_esc_without_search_or_edit_closes_popup() {
        let mut tab = make_tab().await;
        let mode = sample_mode();
        let (mode, result) = press(mode, &mut tab, KeyCode::Esc).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(!matches!(
            mode.render_state(),
            ModeRenderState::DefaultFilters { .. }
        ));
    }

    #[tokio::test]
    async fn test_typing_activates_search_and_resets_selection() {
        let mut tab = make_tab().await;
        let mode = sample_mode();
        let (mode, _) = press(mode, &mut tab, KeyCode::Char('j')).await; // selected=1
        let (mode, _) = press_dyn(mode, &mut tab, KeyCode::Char('z')).await;
        let (_, search, selected, _) = extract(mode.render_state());
        assert_eq!(search, "z");
        assert_eq!(selected, 0);
    }

    #[tokio::test]
    async fn test_backspace_in_list_state_removes_search_char() {
        let mut tab = make_tab().await;
        let mut mode = sample_mode();
        mode.search = "ze".to_string();
        let (mode, _) = press(mode, &mut tab, KeyCode::Backspace).await;
        let (_, search, _, _) = extract(mode.render_state());
        assert_eq!(search, "z");
    }

    #[tokio::test]
    async fn test_unrecognized_key_returns_ignored() {
        let mut tab = make_tab().await;
        let mode = sample_mode();
        let (_, result) = press(mode, &mut tab, KeyCode::F(2)).await;
        assert!(matches!(result, KeyResult::Ignored));
    }

    #[tokio::test]
    async fn test_mode_bar_content_list_state() {
        let mode = sample_mode();
        let kb = Keybindings::default();
        let theme = Theme::default();
        let line = mode.mode_bar_content(&kb, &theme);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("DEFAULT FILTERS"));
    }

    #[tokio::test]
    async fn test_mode_bar_content_edit_state() {
        let mut mode = sample_mode();
        mode.editing = Some(PathEditState {
            input: String::new(),
            cursor: 0,
            query: None,
            completion_index: None,
        });
        let kb = Keybindings::default();
        let theme = Theme::default();
        let line = mode.mode_bar_content(&kb, &theme);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("EDIT"));
    }
}
