use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::config::Keybindings;
use crate::mode::app_mode::{Mode, ModeRenderState, status_entry};
use crate::mode::command_mode::CommandMode;
use crate::mode::normal_mode::NormalMode;
use crate::theme::Theme;
use crate::types::FilterType;

use crate::ui::KeyResult;
use crate::ui::TabState;

#[derive(Debug)]
pub struct FilterManagementMode {
    pub selected_filter_index: usize,
}

#[async_trait]
impl Mode for FilterManagementMode {
    async fn handle_key(
        self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        // Clone the Arc so we can mutate `tab` freely below.
        let kb = tab.keybindings.clone();

        // Tab / Shift+Tab always pass through to the global handler.
        if kb.global.next_tab.matches(key, modifiers) || kb.global.prev_tab.matches(key, modifiers)
        {
            return (self, KeyResult::Ignored);
        }

        let selected = self.selected_filter_index;

        if kb.filter.exit_mode.matches(key, modifiers) {
            return (Box::new(NormalMode::default()), KeyResult::Handled);
        }

        if kb.navigation.scroll_up.matches(key, modifiers) {
            return (
                Box::new(FilterManagementMode {
                    selected_filter_index: selected.saturating_sub(1),
                }),
                KeyResult::Handled,
            );
        }

        if kb.navigation.scroll_down.matches(key, modifiers) {
            let num_filters = tab.log_manager.get_filters().len();
            let new_idx = if num_filters > 0 {
                (selected + 1).min(num_filters - 1)
            } else {
                0
            };
            return (
                Box::new(FilterManagementMode {
                    selected_filter_index: new_idx,
                }),
                KeyResult::Handled,
            );
        }

        if kb.filter.toggle_filter.matches(key, modifiers) {
            let filter_id = tab.log_manager.get_filters().get(selected).map(|f| f.id);
            if let Some(id) = filter_id {
                tab.log_manager.toggle_filter(id).await;
                tab.refresh_visible();
            }
            return (
                Box::new(FilterManagementMode {
                    selected_filter_index: selected,
                }),
                KeyResult::Handled,
            );
        }

        if kb.filter.delete_filter.matches(key, modifiers) {
            let filter_id = tab.log_manager.get_filters().get(selected).map(|f| f.id);
            if let Some(id) = filter_id {
                tab.log_manager.remove_filter(id).await;
                tab.refresh_visible();
                let remaining_len = tab.log_manager.get_filters().len();
                let new_idx = if remaining_len > 0 && selected >= remaining_len {
                    remaining_len - 1
                } else {
                    selected
                };
                return (
                    Box::new(FilterManagementMode {
                        selected_filter_index: new_idx,
                    }),
                    KeyResult::Handled,
                );
            }
            return (
                Box::new(FilterManagementMode {
                    selected_filter_index: selected,
                }),
                KeyResult::Handled,
            );
        }

        if kb.filter.move_filter_up.matches(key, modifiers) {
            let filter_id = tab.log_manager.get_filters().get(selected).map(|f| f.id);
            if let Some(id) = filter_id {
                tab.log_manager.move_filter_up(id).await;
                tab.refresh_visible();
                let new_idx = selected.saturating_sub(1);
                return (
                    Box::new(FilterManagementMode {
                        selected_filter_index: new_idx,
                    }),
                    KeyResult::Handled,
                );
            }
            return (
                Box::new(FilterManagementMode {
                    selected_filter_index: selected,
                }),
                KeyResult::Handled,
            );
        }

        if kb.filter.move_filter_down.matches(key, modifiers) {
            let filter_id = tab.log_manager.get_filters().get(selected).map(|f| f.id);
            if let Some(id) = filter_id {
                tab.log_manager.move_filter_down(id).await;
                tab.refresh_visible();
                let total = tab.log_manager.get_filters().len();
                let new_idx = if selected + 1 < total {
                    selected + 1
                } else {
                    selected
                };
                return (
                    Box::new(FilterManagementMode {
                        selected_filter_index: new_idx,
                    }),
                    KeyResult::Handled,
                );
            }
            return (
                Box::new(FilterManagementMode {
                    selected_filter_index: selected,
                }),
                KeyResult::Handled,
            );
        }

        if kb.filter.edit_filter.matches(key, modifiers) {
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
                let cmd = if let Some(expr) = pattern.strip_prefix(crate::date_filter::DATE_PREFIX)
                {
                    format!("date-filter {}", expr)
                } else {
                    let mut c = if ft == FilterType::Include {
                        String::from("filter")
                    } else {
                        String::from("exclude")
                    };
                    if ft == FilterType::Include
                        && let Some(cfg) = &cc
                    {
                        if let Some(fg) = cfg.fg {
                            c.push_str(&format!(" --fg {}", fg));
                        }
                        if let Some(bg) = cfg.bg {
                            c.push_str(&format!(" --bg {}", bg));
                        }
                        if !cfg.match_only {
                            c.push_str(" -l");
                        }
                    }
                    c.push(' ');
                    c.push_str(&pattern);
                    c
                };
                let len = cmd.len();
                let history = tab.command_history.clone();
                tab.command_error = None;
                return (
                    Box::new(CommandMode::with_history(cmd, len, history)),
                    KeyResult::Handled,
                );
            }
            return (
                Box::new(FilterManagementMode {
                    selected_filter_index: selected,
                }),
                KeyResult::Handled,
            );
        }

        if kb.filter.set_color.matches(key, modifiers) {
            let color_config = tab
                .log_manager
                .get_filters()
                .get(selected)
                .and_then(|f| f.color_config.clone());
            tab.filter_context = Some(selected);
            let mut cmd = String::from("set-color");
            if let Some(cfg) = color_config {
                if let Some(fg) = cfg.fg {
                    cmd.push_str(&format!(" --fg {}", fg));
                }
                if let Some(bg) = cfg.bg {
                    cmd.push_str(&format!(" --bg {}", bg));
                }
            }
            let len = cmd.len();
            let history = tab.command_history.clone();
            tab.command_error = None;
            return (
                Box::new(CommandMode::with_history(cmd, len, history)),
                KeyResult::Handled,
            );
        }

        if kb.filter.toggle_all_filters.matches(key, modifiers) {
            let any_enabled = tab.log_manager.get_filters().iter().any(|f| f.enabled);
            if any_enabled {
                tab.log_manager.disable_all_filters().await;
            } else {
                tab.log_manager.enable_all_filters().await;
            }
            tab.refresh_visible();
            return (
                Box::new(FilterManagementMode {
                    selected_filter_index: selected,
                }),
                KeyResult::Handled,
            );
        }

        if kb.filter.clear_all_filters.matches(key, modifiers) {
            tab.log_manager.clear_filters().await;
            tab.refresh_visible();
            return (
                Box::new(FilterManagementMode {
                    selected_filter_index: 0,
                }),
                KeyResult::Handled,
            );
        }

        if kb.filter.add_date_filter.matches(key, modifiers) {
            let history = tab.command_history.clone();
            tab.command_error = None;
            return (
                Box::new(CommandMode::with_history(
                    "date-filter ".to_string(),
                    12,
                    history,
                )),
                KeyResult::Handled,
            );
        }

        // Unrecognised key — stay in filter mode.
        (
            Box::new(FilterManagementMode {
                selected_filter_index: selected,
            }),
            KeyResult::Handled,
        )
    }

    fn status_line(&self) -> &str {
        "[FILTER] <t> date  <Space> toggle  <d> delete  <e> edit  <c> color  <J/K> move  <A> tog.all  <C> clear  <Esc> exit"
    }

    fn dynamic_status_line(&self, kb: &Keybindings, theme: &Theme) -> Line<'static> {
        let mut spans: Vec<Span<'static>> = vec![Span::styled(
            "[FILTER]  ",
            Style::default()
                .fg(theme.text_highlight)
                .add_modifier(Modifier::BOLD),
        )];
        status_entry(
            &mut spans,
            kb.filter.add_date_filter.display(),
            "date",
            theme,
        );
        status_entry(
            &mut spans,
            kb.filter.toggle_filter.display(),
            "toggle",
            theme,
        );
        status_entry(
            &mut spans,
            kb.filter.delete_filter.display(),
            "delete",
            theme,
        );
        status_entry(&mut spans, kb.filter.edit_filter.display(), "edit", theme);
        status_entry(&mut spans, kb.filter.set_color.display(), "color", theme);
        // Move up/down: <K/J>
        spans.push(Span::styled("<", Style::default().fg(theme.border)));
        spans.push(Span::styled(
            kb.filter.move_filter_up.display(),
            Style::default()
                .fg(theme.text_highlight)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled("/", Style::default().fg(theme.border)));
        spans.push(Span::styled(
            kb.filter.move_filter_down.display(),
            Style::default()
                .fg(theme.text_highlight)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled("> move  ", Style::default().fg(theme.text)));
        status_entry(
            &mut spans,
            kb.filter.toggle_all_filters.display(),
            "tog.all",
            theme,
        );
        status_entry(
            &mut spans,
            kb.filter.clear_all_filters.display(),
            "clear",
            theme,
        );
        status_entry(&mut spans, kb.filter.exit_mode.display(), "exit", theme);
        Line::from(spans)
    }

    fn render_state(&self) -> ModeRenderState {
        ModeRenderState::FilterManagement {
            selected_index: self.selected_filter_index,
        }
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

#[async_trait]
impl Mode for FilterEditMode {
    async fn handle_key(
        self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        let kb = tab.keybindings.clone();

        if kb.global.next_tab.matches(key, modifiers) || kb.global.prev_tab.matches(key, modifiers)
        {
            return (self, KeyResult::Ignored);
        }
        if kb.filter_edit.confirm.matches(key, modifiers) {
            if let Some(id) = self.filter_id {
                tab.log_manager.edit_filter(id, self.filter_input).await;
                tab.refresh_visible();
            }
            return (
                Box::new(FilterManagementMode {
                    selected_filter_index: 0,
                }),
                KeyResult::Handled,
            );
        }
        if kb.filter_edit.cancel.matches(key, modifiers) {
            return (
                Box::new(FilterManagementMode {
                    selected_filter_index: 0,
                }),
                KeyResult::Handled,
            );
        }
        match key {
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
        "[FILTER EDIT] <Esc> cancel  <Enter> save"
    }

    fn dynamic_status_line(&self, kb: &Keybindings, theme: &Theme) -> Line<'static> {
        let mut spans: Vec<Span<'static>> = vec![Span::styled(
            "[FILTER EDIT]  ",
            Style::default()
                .fg(theme.text_highlight)
                .add_modifier(Modifier::BOLD),
        )];
        status_entry(&mut spans, kb.filter_edit.cancel.display(), "cancel", theme);
        status_entry(&mut spans, kb.filter_edit.confirm.display(), "save", theme);
        Line::from(spans)
    }

    fn render_state(&self) -> ModeRenderState {
        ModeRenderState::FilterEdit
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

    async fn make_tab(lines: &[&str]) -> TabState {
        let data = lines.join("\n").into_bytes();
        let file_reader = FileReader::from_bytes(data);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        TabState::new(file_reader, log_manager, "test".to_string())
    }

    async fn add_filter(tab: &mut TabState, pattern: &str, filter_type: FilterType) {
        tab.log_manager
            .add_filter_with_color(pattern.to_string(), filter_type, None, None, true)
            .await;
        tab.refresh_visible();
    }

    fn filter_mode(idx: usize) -> FilterManagementMode {
        FilterManagementMode {
            selected_filter_index: idx,
        }
    }

    async fn press(
        mode: FilterManagementMode,
        tab: &mut TabState,
        code: KeyCode,
    ) -> (Box<dyn Mode>, KeyResult) {
        Box::new(mode)
            .handle_key(tab, code, KeyModifiers::NONE)
            .await
    }

    #[tokio::test]
    async fn test_esc_transitions_to_normal_mode() {
        let mut tab = make_tab(&["line"]).await;
        let (mode, result) = press(filter_mode(0), &mut tab, KeyCode::Esc).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(!matches!(
            mode.render_state(),
            ModeRenderState::Command { .. }
        ));
        assert!(!matches!(
            mode.render_state(),
            ModeRenderState::FilterManagement { .. }
        ));
    }

    #[tokio::test]
    async fn test_tab_returns_ignored() {
        let mut tab = make_tab(&["line"]).await;
        let (_, result) = press(filter_mode(0), &mut tab, KeyCode::Tab).await;
        assert!(matches!(result, KeyResult::Ignored));
    }

    #[tokio::test]
    async fn test_backtab_returns_ignored() {
        let mut tab = make_tab(&["line"]).await;
        let (_, result) = press(filter_mode(0), &mut tab, KeyCode::BackTab).await;
        assert!(matches!(result, KeyResult::Ignored));
    }

    #[tokio::test]
    async fn test_up_decrements_selected_index() {
        let mut tab = make_tab(&["a", "b"]).await;
        add_filter(&mut tab, "a", FilterType::Include).await;
        add_filter(&mut tab, "b", FilterType::Include).await;
        let (mode, _) = press(filter_mode(1), &mut tab, KeyCode::Up).await;
        match mode.render_state() {
            ModeRenderState::FilterManagement { selected_index } => assert_eq!(selected_index, 0),
            other => panic!("expected FilterManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_up_saturates_at_zero() {
        let mut tab = make_tab(&["a"]).await;
        let (mode, _) = press(filter_mode(0), &mut tab, KeyCode::Up).await;
        match mode.render_state() {
            ModeRenderState::FilterManagement { selected_index } => assert_eq!(selected_index, 0),
            other => panic!("expected FilterManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_down_increments_selected_index() {
        let mut tab = make_tab(&["a", "b"]).await;
        add_filter(&mut tab, "a", FilterType::Include).await;
        add_filter(&mut tab, "b", FilterType::Include).await;
        let (mode, _) = press(filter_mode(0), &mut tab, KeyCode::Down).await;
        match mode.render_state() {
            ModeRenderState::FilterManagement { selected_index } => assert_eq!(selected_index, 1),
            other => panic!("expected FilterManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_down_clamps_at_last_filter() {
        let mut tab = make_tab(&["a", "b"]).await;
        add_filter(&mut tab, "a", FilterType::Include).await;
        add_filter(&mut tab, "b", FilterType::Include).await;
        let (mode, _) = press(filter_mode(1), &mut tab, KeyCode::Down).await;
        match mode.render_state() {
            ModeRenderState::FilterManagement { selected_index } => assert_eq!(selected_index, 1),
            other => panic!("expected FilterManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_space_toggles_filter() {
        let mut tab = make_tab(&["a", "b"]).await;
        add_filter(&mut tab, "a", FilterType::Include).await;
        let id = tab.log_manager.get_filters()[0].id;
        assert!(tab.log_manager.get_filters()[0].enabled);
        press(filter_mode(0), &mut tab, KeyCode::Char(' ')).await;
        assert!(
            !tab.log_manager
                .get_filters()
                .iter()
                .find(|f| f.id == id)
                .unwrap()
                .enabled
        );
    }

    #[tokio::test]
    async fn test_d_deletes_filter() {
        let mut tab = make_tab(&["a", "b"]).await;
        add_filter(&mut tab, "a", FilterType::Include).await;
        assert_eq!(tab.log_manager.get_filters().len(), 1);
        press(filter_mode(0), &mut tab, KeyCode::Char('d')).await;
        assert_eq!(tab.log_manager.get_filters().len(), 0);
    }

    #[tokio::test]
    async fn test_d_with_no_filters_no_panic() {
        let mut tab = make_tab(&["line"]).await;
        let (mode, result) = press(filter_mode(0), &mut tab, KeyCode::Char('d')).await;
        assert!(matches!(result, KeyResult::Handled));
        match mode.render_state() {
            ModeRenderState::FilterManagement { selected_index } => assert_eq!(selected_index, 0),
            other => panic!("expected FilterManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_e_opens_command_mode_with_filter_pattern() {
        let mut tab = make_tab(&["error", "warn"]).await;
        add_filter(&mut tab, "error", FilterType::Include).await;
        let (mode, _) = press(filter_mode(0), &mut tab, KeyCode::Char('e')).await;
        match mode.render_state() {
            ModeRenderState::Command { input, .. } => {
                assert!(input.contains("error"));
            }
            other => panic!("expected Command, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_c_opens_set_color_command() {
        let mut tab = make_tab(&["line"]).await;
        add_filter(&mut tab, "error", FilterType::Include).await;
        let (mode, _) = press(filter_mode(0), &mut tab, KeyCode::Char('c')).await;
        match mode.render_state() {
            ModeRenderState::Command { input, .. } => {
                assert!(input.starts_with("set-color"));
            }
            other => panic!("expected Command, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_capital_k_moves_filter_up() {
        let mut tab = make_tab(&["a", "b"]).await;
        add_filter(&mut tab, "first", FilterType::Include).await;
        add_filter(&mut tab, "second", FilterType::Include).await;
        let (mode, result) = press(filter_mode(1), &mut tab, KeyCode::Char('K')).await;
        assert!(matches!(result, KeyResult::Handled));
        match mode.render_state() {
            ModeRenderState::FilterManagement { selected_index } => assert_eq!(selected_index, 0),
            other => panic!("expected FilterManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_capital_j_moves_filter_down() {
        let mut tab = make_tab(&["a", "b"]).await;
        add_filter(&mut tab, "first", FilterType::Include).await;
        add_filter(&mut tab, "second", FilterType::Include).await;
        let (mode, result) = press(filter_mode(0), &mut tab, KeyCode::Char('J')).await;
        assert!(matches!(result, KeyResult::Handled));
        match mode.render_state() {
            ModeRenderState::FilterManagement { selected_index } => assert_eq!(selected_index, 1),
            other => panic!("expected FilterManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_capital_a_disables_all_when_some_enabled() {
        let mut tab = make_tab(&["error", "warn"]).await;
        add_filter(&mut tab, "error", FilterType::Include).await;
        add_filter(&mut tab, "warn", FilterType::Include).await;
        assert!(tab.log_manager.get_filters().iter().all(|f| f.enabled));

        press(filter_mode(0), &mut tab, KeyCode::Char('A')).await;
        assert!(tab.log_manager.get_filters().iter().all(|f| !f.enabled));
    }

    #[tokio::test]
    async fn test_capital_a_enables_all_when_all_disabled() {
        let mut tab = make_tab(&["error", "warn"]).await;
        add_filter(&mut tab, "error", FilterType::Include).await;
        add_filter(&mut tab, "warn", FilterType::Include).await;
        tab.log_manager.disable_all_filters().await;
        assert!(tab.log_manager.get_filters().iter().all(|f| !f.enabled));

        press(filter_mode(0), &mut tab, KeyCode::Char('A')).await;
        assert!(tab.log_manager.get_filters().iter().all(|f| f.enabled));
    }

    #[tokio::test]
    async fn test_capital_c_clears_all_filters() {
        let mut tab = make_tab(&["error", "warn"]).await;
        add_filter(&mut tab, "error", FilterType::Include).await;
        add_filter(&mut tab, "warn", FilterType::Include).await;
        assert_eq!(tab.log_manager.get_filters().len(), 2);

        press(filter_mode(0), &mut tab, KeyCode::Char('C')).await;
        assert!(tab.log_manager.get_filters().is_empty());
    }

    #[tokio::test]
    async fn test_capital_c_resets_selected_index_to_zero() {
        let mut tab = make_tab(&["error", "warn"]).await;
        add_filter(&mut tab, "error", FilterType::Include).await;
        add_filter(&mut tab, "warn", FilterType::Include).await;

        let (mode, _) = press(filter_mode(1), &mut tab, KeyCode::Char('C')).await;
        match mode.render_state() {
            ModeRenderState::FilterManagement { selected_index } => assert_eq!(selected_index, 0),
            other => panic!("expected FilterManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_command_error_cleared_on_date_filter_shortcut() {
        let mut tab = make_tab(&["line"]).await;
        tab.command_error = Some("previous error".to_string());
        press(filter_mode(0), &mut tab, KeyCode::Char('t')).await;
        assert!(tab.command_error.is_none());
    }

    #[tokio::test]
    async fn test_command_error_cleared_on_edit_filter() {
        let mut tab = make_tab(&["line"]).await;
        add_filter(&mut tab, "error", FilterType::Include).await;
        tab.command_error = Some("previous error".to_string());
        press(filter_mode(0), &mut tab, KeyCode::Char('e')).await;
        assert!(tab.command_error.is_none());
    }

    #[tokio::test]
    async fn test_status_line_contains_filter() {
        assert!(filter_mode(0).status_line().contains("[FILTER]"));
    }

    #[tokio::test]
    async fn test_selected_filter_index_returns_current() {
        let mode = filter_mode(3);
        match mode.render_state() {
            ModeRenderState::FilterManagement { selected_index } => assert_eq!(selected_index, 3),
            other => panic!("expected FilterManagement, got {:?}", other),
        }
    }

    // FilterEditMode tests

    fn edit_mode(filter_id: Option<usize>, input: &str) -> FilterEditMode {
        FilterEditMode {
            filter_id,
            filter_input: input.to_string(),
        }
    }

    async fn press_edit(
        mode: FilterEditMode,
        tab: &mut TabState,
        code: KeyCode,
    ) -> (Box<dyn Mode>, KeyResult) {
        Box::new(mode)
            .handle_key(tab, code, KeyModifiers::NONE)
            .await
    }

    #[tokio::test]
    async fn test_edit_char_appends_to_input() {
        let mut tab = make_tab(&["line"]).await;
        let mode = edit_mode(None, "err");
        let (mode2, _) = press_edit(mode, &mut tab, KeyCode::Char('o')).await;
        assert!(mode2.status_line().contains("[FILTER EDIT]"));
    }

    #[tokio::test]
    async fn test_edit_backspace_removes_char() {
        let mut tab = make_tab(&["line"]).await;
        let mode = edit_mode(None, "error");
        let (mode2, result) = press_edit(mode, &mut tab, KeyCode::Backspace).await;
        assert!(matches!(result, KeyResult::Handled));
        // Mode should still be FilterEditMode
        assert!(mode2.status_line().contains("[FILTER EDIT]"));
    }

    #[tokio::test]
    async fn test_edit_esc_transitions_to_filter_mode() {
        let mut tab = make_tab(&["line"]).await;
        let mode = edit_mode(None, "error");
        let (mode2, result) = press_edit(mode, &mut tab, KeyCode::Esc).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(matches!(
            mode2.render_state(),
            ModeRenderState::FilterManagement { .. }
        ));
    }

    #[tokio::test]
    async fn test_edit_tab_returns_ignored() {
        let mut tab = make_tab(&["line"]).await;
        let mode = edit_mode(None, "err");
        let (_, result) = press_edit(mode, &mut tab, KeyCode::Tab).await;
        assert!(matches!(result, KeyResult::Ignored));
    }

    #[tokio::test]
    async fn test_edit_enter_applies_filter_change() {
        let mut tab = make_tab(&["warn", "error"]).await;
        add_filter(&mut tab, "warn", FilterType::Include).await;
        let id = tab.log_manager.get_filters()[0].id;
        let mode = edit_mode(Some(id), "error");
        let (mode2, result) = press_edit(mode, &mut tab, KeyCode::Enter).await;
        assert!(matches!(result, KeyResult::Handled));
        // Should transition to FilterManagementMode
        assert!(matches!(
            mode2.render_state(),
            ModeRenderState::FilterManagement { .. }
        ));
        assert_eq!(tab.log_manager.get_filters()[0].pattern, "error");
    }
}
