use crate::config::Keybindings;
use crate::filters::{ColorConfig, FilterType};
use crate::mode::app_mode::{Mode, ModeRenderState, status_entry};
use crate::mode::command_mode::CommandMode;
use crate::mode::normal_mode::NormalMode;
use crate::theme::{Theme, color_to_string};
use crate::ui::KeyResult;
use crate::ui::TabState;
use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

#[derive(Debug)]
pub struct FilterManagementMode {
    pub selected_filter_index: usize,
}

fn stay_at(idx: usize) -> (Box<dyn Mode>, KeyResult) {
    (
        Box::new(FilterManagementMode {
            selected_filter_index: idx,
        }),
        KeyResult::Handled,
    )
}

fn open_command(tab: &mut TabState, cmd: String) -> (Box<dyn Mode>, KeyResult) {
    let len = cmd.len();
    let history = tab.interaction.command_history.clone();
    tab.interaction.command_error = None;
    (
        Box::new(CommandMode::with_history(cmd, len, history)),
        KeyResult::Handled,
    )
}

fn build_edit_command(
    ft: &FilterType,
    cc: &Option<ColorConfig>,
    pattern: &str,
    use_regex: bool,
) -> String {
    if let Some(expr) = pattern.strip_prefix(crate::filters::DATE_PREFIX) {
        build_date_filter_command(cc, expr)
    } else if let Some(expr) = pattern.strip_prefix(crate::filters::FIELD_PREFIX) {
        build_field_filter_command(ft, cc, expr)
    } else {
        build_text_filter_command(ft, cc, pattern, use_regex)
    }
}

fn build_date_filter_command(cc: &Option<ColorConfig>, expr: &str) -> String {
    let mut c = String::from("date-filter");
    append_color_flags(&mut c, cc, true);
    c.push(' ');
    c.push_str(expr);
    c
}

fn build_field_filter_command(ft: &FilterType, cc: &Option<ColorConfig>, expr: &str) -> String {
    let mut c = filter_or_exclude_prefix(ft);
    if *ft == FilterType::Include {
        append_color_flags(&mut c, cc, true);
    }
    c.push_str(" --field ");
    if let Some(colon) = expr.find(':') {
        c.push_str(&expr[..colon]);
        c.push('=');
        c.push_str(&expr[colon + 1..]);
    } else {
        c.push_str(expr);
    }
    c
}

fn build_text_filter_command(
    ft: &FilterType,
    cc: &Option<ColorConfig>,
    pattern: &str,
    use_regex: bool,
) -> String {
    let mut c = filter_or_exclude_prefix(ft);
    if use_regex {
        c.push_str(" --regex");
    }
    if *ft == FilterType::Include {
        append_color_flags(&mut c, cc, true);
    }
    c.push(' ');
    c.push_str(pattern);
    c
}

fn build_color_command(cc: Option<ColorConfig>) -> String {
    let mut cmd = String::from("set-color");
    if let Some(cfg) = cc {
        append_color_flags(&mut cmd, &Some(cfg), true);
    }
    cmd
}

fn filter_or_exclude_prefix(ft: &FilterType) -> String {
    if *ft == FilterType::Include {
        String::from("filter")
    } else {
        String::from("exclude")
    }
}

fn append_color_flags(cmd: &mut String, cc: &Option<ColorConfig>, include_line_flag: bool) {
    if let Some(cfg) = cc {
        if let Some(fg) = cfg.fg {
            cmd.push_str(&format!(" --fg {}", color_to_string(fg)));
        }
        if let Some(bg) = cfg.bg {
            cmd.push_str(&format!(" --bg {}", color_to_string(bg)));
        }
        if include_line_flag && !cfg.match_only {
            cmd.push_str(" -l");
        }
    }
}

impl FilterManagementMode {
    fn scroll_up(&self) -> (Box<dyn Mode>, KeyResult) {
        stay_at(self.selected_filter_index.saturating_sub(1))
    }

    fn scroll_down(&self, tab: &TabState) -> (Box<dyn Mode>, KeyResult) {
        let num_filters = tab.log_manager.get_filters().len();
        let new_idx = if num_filters > 0 {
            (self.selected_filter_index + 1).min(num_filters - 1)
        } else {
            0
        };
        stay_at(new_idx)
    }

    async fn toggle_filter(&self, tab: &mut TabState) -> (Box<dyn Mode>, KeyResult) {
        let selected = self.selected_filter_index;
        let filter_id = tab.log_manager.get_filters().get(selected).map(|f| f.id);
        if let Some(id) = filter_id {
            tab.log_manager.toggle_filter(id).await;
            tab.begin_filter_refresh();
        }
        stay_at(selected)
    }

    async fn delete_filter(&self, tab: &mut TabState) -> (Box<dyn Mode>, KeyResult) {
        let selected = self.selected_filter_index;
        let filter_id = tab.log_manager.get_filters().get(selected).map(|f| f.id);
        if let Some(id) = filter_id {
            tab.log_manager.remove_filter(id).await;
            tab.begin_filter_refresh();
            let remaining_len = tab.log_manager.get_filters().len();
            let new_idx = if remaining_len > 0 && selected >= remaining_len {
                remaining_len - 1
            } else {
                selected
            };
            return stay_at(new_idx);
        }
        stay_at(selected)
    }

    async fn move_filter_up(&self, tab: &mut TabState) -> (Box<dyn Mode>, KeyResult) {
        let selected = self.selected_filter_index;
        let filter_id = tab.log_manager.get_filters().get(selected).map(|f| f.id);
        if let Some(id) = filter_id {
            tab.log_manager.move_filter_up(id).await;
            tab.begin_filter_refresh();
            return stay_at(selected.saturating_sub(1));
        }
        stay_at(selected)
    }

    async fn move_filter_down(&self, tab: &mut TabState) -> (Box<dyn Mode>, KeyResult) {
        let selected = self.selected_filter_index;
        let filter_id = tab.log_manager.get_filters().get(selected).map(|f| f.id);
        if let Some(id) = filter_id {
            tab.log_manager.move_filter_down(id).await;
            tab.begin_filter_refresh();
            let total = tab.log_manager.get_filters().len();
            let new_idx = if selected + 1 < total {
                selected + 1
            } else {
                selected
            };
            return stay_at(new_idx);
        }
        stay_at(selected)
    }

    fn edit_filter(&self, tab: &mut TabState) -> (Box<dyn Mode>, KeyResult) {
        let selected = self.selected_filter_index;
        let filter_info = tab.log_manager.get_filters().get(selected).map(|f| {
            (
                f.id,
                f.filter_type.clone(),
                f.color_config.clone(),
                f.pattern.clone(),
                f.use_regex,
            )
        });
        if let Some((id, ft, cc, pattern, use_regex)) = filter_info {
            tab.filter.editing_filter_id = Some(id);
            tab.filter.filter_context = Some(selected);
            let cmd = build_edit_command(&ft, &cc, &pattern, use_regex);
            return open_command(tab, cmd);
        }
        stay_at(selected)
    }

    fn set_color(&self, tab: &mut TabState) -> (Box<dyn Mode>, KeyResult) {
        let selected = self.selected_filter_index;
        let color_config = tab
            .log_manager
            .get_filters()
            .get(selected)
            .and_then(|f| f.color_config.clone());
        tab.filter.filter_context = Some(selected);
        let cmd = build_color_command(color_config);
        open_command(tab, cmd)
    }

    fn toggle_filtering(&self, tab: &mut TabState) -> (Box<dyn Mode>, KeyResult) {
        tab.filter.enabled = !tab.filter.enabled;
        tab.begin_filter_refresh();
        stay_at(self.selected_filter_index)
    }

    async fn toggle_all_filters(&self, tab: &mut TabState) -> (Box<dyn Mode>, KeyResult) {
        let any_enabled = tab.log_manager.get_filters().iter().any(|f| f.enabled);
        if any_enabled {
            tab.log_manager.disable_all_filters().await;
        } else {
            tab.log_manager.enable_all_filters().await;
        }
        tab.begin_filter_refresh();
        stay_at(self.selected_filter_index)
    }

    async fn clear_all_filters(&self, tab: &mut TabState) -> (Box<dyn Mode>, KeyResult) {
        tab.log_manager.clear_filters().await;
        tab.begin_filter_refresh();
        stay_at(0)
    }

    fn add_include_filter(tab: &mut TabState) -> (Box<dyn Mode>, KeyResult) {
        open_command(tab, "filter ".to_string())
    }

    fn add_exclude_filter(tab: &mut TabState) -> (Box<dyn Mode>, KeyResult) {
        open_command(tab, "exclude ".to_string())
    }

    fn add_date_filter(tab: &mut TabState) -> (Box<dyn Mode>, KeyResult) {
        open_command(tab, "date-filter ".to_string())
    }

    fn sidebar_grow(&self, tab: &mut TabState) -> (Box<dyn Mode>, KeyResult) {
        tab.display.sidebar_width = tab.display.sidebar_width.saturating_add(2);
        stay_at(self.selected_filter_index)
    }

    fn sidebar_shrink(&self, tab: &mut TabState) -> (Box<dyn Mode>, KeyResult) {
        tab.display.sidebar_width = tab.display.sidebar_width.saturating_sub(2).max(10);
        stay_at(self.selected_filter_index)
    }
}

#[async_trait]
impl Mode for FilterManagementMode {
    async fn handle_key(
        self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        let kb = tab.interaction.keybindings.clone();

        if kb.global.next_tab.matches(key, modifiers) || kb.global.prev_tab.matches(key, modifiers)
        {
            return (self, KeyResult::Ignored);
        }

        if kb.filter.exit_mode.matches(key, modifiers) {
            return (Box::new(NormalMode::default()), KeyResult::Handled);
        }
        if kb.navigation.scroll_up.matches(key, modifiers) {
            return self.scroll_up();
        }
        if kb.navigation.scroll_down.matches(key, modifiers) {
            return self.scroll_down(tab);
        }
        if kb.filter.toggle_filter.matches(key, modifiers) {
            return self.toggle_filter(tab).await;
        }
        if kb.filter.delete_filter.matches(key, modifiers) {
            return self.delete_filter(tab).await;
        }
        if kb.filter.move_filter_up.matches(key, modifiers) {
            return self.move_filter_up(tab).await;
        }
        if kb.filter.move_filter_down.matches(key, modifiers) {
            return self.move_filter_down(tab).await;
        }
        if kb.filter.edit_filter.matches(key, modifiers) {
            return self.edit_filter(tab);
        }
        if kb.filter.set_color.matches(key, modifiers) {
            return self.set_color(tab);
        }
        if kb.normal.toggle_filtering.matches(key, modifiers) {
            return self.toggle_filtering(tab);
        }
        if kb.filter.toggle_all_filters.matches(key, modifiers) {
            return self.toggle_all_filters(tab).await;
        }
        if kb.filter.clear_all_filters.matches(key, modifiers) {
            return self.clear_all_filters(tab).await;
        }
        if kb.filter.add_include_filter.matches(key, modifiers) {
            return Self::add_include_filter(tab);
        }
        if kb.filter.add_exclude_filter.matches(key, modifiers) {
            return Self::add_exclude_filter(tab);
        }
        if kb.filter.add_date_filter.matches(key, modifiers) {
            return Self::add_date_filter(tab);
        }
        if kb.filter.sidebar_grow.matches(key, modifiers) {
            return self.sidebar_grow(tab);
        }
        if kb.filter.sidebar_shrink.matches(key, modifiers) {
            return self.sidebar_shrink(tab);
        }

        (
            Box::new(FilterManagementMode {
                selected_filter_index: self.selected_filter_index,
            }),
            KeyResult::Ignored,
        )
    }

    fn mode_bar_content(&self, kb: &Keybindings, theme: &Theme) -> Line<'static> {
        let mut spans: Vec<Span<'static>> = vec![Span::styled(
            "[FILTER]  ",
            Style::default()
                .fg(theme.text_highlight_fg)
                .add_modifier(Modifier::BOLD),
        )];
        status_entry(
            &mut spans,
            kb.filter.add_include_filter.display(),
            "filter in",
            theme,
        );
        status_entry(
            &mut spans,
            kb.filter.add_exclude_filter.display(),
            "filter out",
            theme,
        );
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
        spans.push(Span::styled("<", Style::default().fg(theme.text)));
        spans.push(Span::styled(
            kb.filter.move_filter_up.display(),
            Style::default()
                .fg(theme.text_highlight_fg)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled("/", Style::default().fg(theme.text)));
        spans.push(Span::styled(
            kb.filter.move_filter_down.display(),
            Style::default()
                .fg(theme.text_highlight_fg)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled("> move  ", Style::default().fg(theme.text)));
        status_entry(
            &mut spans,
            kb.normal.toggle_filtering.display(),
            "tog.filtering",
            theme,
        );
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
        spans.push(Span::styled("<", Style::default().fg(theme.text)));
        spans.push(Span::styled(
            kb.filter.sidebar_shrink.display(),
            Style::default()
                .fg(theme.text_highlight_fg)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled("/", Style::default().fg(theme.text)));
        spans.push(Span::styled(
            kb.filter.sidebar_grow.display(),
            Style::default()
                .fg(theme.text_highlight_fg)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled("> resize  ", Style::default().fg(theme.text)));
        status_entry(&mut spans, kb.filter.exit_mode.display(), "exit", theme);
        Line::from(spans)
    }

    fn render_state(&self) -> ModeRenderState {
        ModeRenderState::FilterManagement {
            selected_index: self.selected_filter_index,
        }
    }
}

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
        let kb = tab.interaction.keybindings.clone();

        if kb.global.next_tab.matches(key, modifiers) || kb.global.prev_tab.matches(key, modifiers)
        {
            return (self, KeyResult::Ignored);
        }
        if kb.filter_edit.confirm.matches(key, modifiers) {
            if let Some(id) = self.filter_id {
                tab.log_manager.edit_filter(id, self.filter_input).await;
                tab.begin_filter_refresh();
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

    fn mode_bar_content(&self, kb: &Keybindings, theme: &Theme) -> Line<'static> {
        let mut spans: Vec<Span<'static>> = vec![Span::styled(
            "[FILTER EDIT]  ",
            Style::default()
                .fg(theme.text_highlight_fg)
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
    use crate::db::LogManager;
    use crate::filters::FilterOptions;
    use crate::ingestion::FileReader;
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
            .add_filter_with_color(pattern.to_string(), filter_type, FilterOptions::default())
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
}
