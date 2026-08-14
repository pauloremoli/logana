use crate::config::Keybindings;
use crate::filters::ColorConfig;
use crate::mode::app_mode::{Mode, ModeRenderState, status_entry};
use crate::mode::filter_mode::{
    append_color_flags, open_command, quote_command_arg, toggle_all_filters,
};
use crate::mode::normal_mode::NormalMode;
use crate::theme::Theme;
use crate::ui::KeyResult;
use crate::ui::TabState;
use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

/// Mirrors `FilterManagementMode`, scoped to a single selected filter group.
/// Entered by clicking a Groups-section row in the sidebar. Selection is by
/// name (not index) since `LogManager::group_names()` is a recomputed,
/// sorted list that can reorder as filters change groups between frames.
#[derive(Debug)]
pub struct GroupManagementMode {
    pub selected_group: String,
    /// Live typeahead query; non-empty narrows the Groups section to matching names.
    pub search: String,
    /// True while capturing raw text input for `search` (gates all other bound keys).
    pub searching: bool,
    /// Selection to restore if search is cancelled with `Esc`.
    pub pre_search_selected: Option<String>,
}

impl GroupManagementMode {
    pub fn new(selected_group: String) -> Self {
        Self {
            selected_group,
            search: String::new(),
            searching: false,
            pre_search_selected: None,
        }
    }

    /// Returns to group mode at `name` while preserving the in-progress
    /// search — unlike `stay_at_group`, which always resets back to a
    /// non-searching mode. Used for navigation (`j`/`k`) within the narrowed
    /// list while `searching` is active.
    fn stay_searching(&self, name: String) -> (Box<dyn Mode>, KeyResult) {
        (
            Box::new(GroupManagementMode {
                selected_group: name,
                search: self.search.clone(),
                searching: true,
                pre_search_selected: self.pre_search_selected.clone(),
            }),
            KeyResult::Handled,
        )
    }

    /// Group names matching the current search query — every known group
    /// when not searching or the query is empty.
    fn narrowed_group_names(&self, tab: &TabState) -> Vec<String> {
        crate::ui::widgets::sidebar::narrowed_group_names(
            &tab.log_manager.group_names(),
            &self.search,
        )
    }

    /// Handles a key while `searching` is active: raw text capture for the
    /// query plus single-step navigation within the narrowed list. Mirrors
    /// `FilterManagementMode::handle_search_key`.
    fn handle_search_key(
        mut self: Box<Self>,
        tab: &mut TabState,
        kb: &Keybindings,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        if kb.search.confirm.matches(key, modifiers) {
            return stay_at_group(self.selected_group.clone(), tab);
        }
        if kb.search.cancel.matches(key, modifiers) {
            let restore = self.pre_search_selected.clone().unwrap_or_default();
            return stay_at_group(restore, tab);
        }
        if kb.navigation.scroll_down.matches(key, modifiers) {
            let narrowed = self.narrowed_group_names(tab);
            let pos = narrowed
                .iter()
                .position(|n| n == &self.selected_group)
                .unwrap_or(0);
            let next = narrowed
                .get(pos + 1)
                .or_else(|| narrowed.last())
                .cloned()
                .unwrap_or_else(|| self.selected_group.clone());
            return self.stay_searching(next);
        }
        if kb.navigation.scroll_up.matches(key, modifiers) {
            let narrowed = self.narrowed_group_names(tab);
            let pos = narrowed
                .iter()
                .position(|n| n == &self.selected_group)
                .unwrap_or(0);
            let prev = narrowed
                .get(pos.saturating_sub(1))
                .cloned()
                .unwrap_or_else(|| self.selected_group.clone());
            return self.stay_searching(prev);
        }
        match key {
            KeyCode::Backspace => {
                self.search.pop();
            }
            KeyCode::Char(c) if !modifiers.contains(KeyModifiers::CONTROL) => {
                self.search.push(c);
            }
            _ => return (self, KeyResult::Ignored),
        }
        let narrowed = self.narrowed_group_names(tab);
        if !narrowed.iter().any(|n| n == &self.selected_group) {
            self.selected_group = narrowed.first().cloned().unwrap_or_default();
        }
        (self, KeyResult::Handled)
    }
}

/// Returns to group mode with `name` selected. Mirrors `filter_mode::stay_at`.
fn stay_at_group(name: String, tab: &mut TabState) -> (Box<dyn Mode>, KeyResult) {
    tab.interaction.g_key_pressed = false;
    (Box::new(GroupManagementMode::new(name)), KeyResult::Handled)
}

/// Builds a `"group <name> --fg ... --bg ... [-l]"` command prefilled with
/// the group's current style, mirroring `filter_mode::build_color_command`.
fn build_group_style_command(name: &str, cc: Option<&ColorConfig>) -> String {
    let mut cmd = format!("group {}", quote_command_arg(name));
    if let Some(cfg) = cc {
        append_color_flags(&mut cmd, &Some(cfg.clone()), true);
    }
    cmd
}

impl GroupManagementMode {
    fn navigate(&self, tab: &mut TabState, delta: i64) -> (Box<dyn Mode>, KeyResult) {
        let names = tab.log_manager.group_names();
        if names.is_empty() {
            return stay_at_group(self.selected_group.clone(), tab);
        }
        let pos = names
            .iter()
            .position(|n| n == &self.selected_group)
            .unwrap_or(0);
        let next_pos = if delta < 0 {
            pos.saturating_sub(delta.unsigned_abs() as usize)
        } else {
            (pos + delta as usize).min(names.len() - 1)
        };
        stay_at_group(names[next_pos].clone(), tab)
    }

    /// Toggles the selected group on/off, flipping to the opposite of its
    /// current state. For a group with filters, "current state" is "any
    /// member filter enabled" (mirroring `cmd_toggle_group`), and every
    /// member flips together. A zero-filter group (one that exists only via
    /// a predefined style) has no filters to derive a state from or flip, so
    /// it falls back to — and persists — its own stored enabled flag instead
    /// of silently doing nothing.
    async fn toggle_group(&self, tab: &mut TabState) -> (Box<dyn Mode>, KeyResult) {
        let has_filters = tab
            .log_manager
            .get_filters()
            .iter()
            .any(|f| f.group.as_deref() == Some(self.selected_group.as_str()));
        let currently_enabled = if has_filters {
            tab.log_manager
                .get_filters()
                .iter()
                .any(|f| f.group.as_deref() == Some(self.selected_group.as_str()) && f.enabled)
        } else {
            crate::filters::group_enabled(tab.log_manager.get_group_styles(), &self.selected_group)
        };
        let new_state = !currently_enabled;
        tab.log_manager
            .set_filters_enabled_by_group(&self.selected_group, new_state)
            .await;
        tab.log_manager
            .set_group_enabled(&self.selected_group, new_state)
            .await;
        tab.begin_filter_refresh();
        stay_at_group(self.selected_group.clone(), tab)
    }

    /// Deletes the selected group: every filter in it plus its predefined
    /// style. Selection moves to whatever group now sits at the deleted
    /// group's old position, mirroring `FilterManagementMode::delete_filter`'s
    /// clamp-to-remaining behavior.
    async fn delete_group(&self, tab: &mut TabState) -> (Box<dyn Mode>, KeyResult) {
        let names = tab.log_manager.group_names();
        let pos = names
            .iter()
            .position(|n| n == &self.selected_group)
            .unwrap_or(0);
        tab.log_manager.remove_group(&self.selected_group).await;
        tab.begin_filter_refresh();
        let remaining = tab.log_manager.group_names();
        let target = remaining
            .get(pos)
            .or_else(|| remaining.last())
            .cloned()
            .unwrap_or_default();
        stay_at_group(target, tab)
    }

    fn edit_group_style(&self, tab: &mut TabState) -> (Box<dyn Mode>, KeyResult) {
        let cc =
            crate::filters::group_style(tab.log_manager.get_group_styles(), &self.selected_group)
                .cloned();
        let cmd = build_group_style_command(&self.selected_group, cc.as_ref());
        open_command(tab, cmd)
    }

    async fn clear_group_style(&self, tab: &mut TabState) -> (Box<dyn Mode>, KeyResult) {
        tab.log_manager
            .clear_group_style(&self.selected_group)
            .await;
        tab.begin_filter_refresh();
        stay_at_group(self.selected_group.clone(), tab)
    }

    /// Opens `CommandMode` prefilled with `"group "` for the user to type a
    /// new group's name (and optionally a style), mirroring
    /// `FilterManagementMode::add_include_filter`'s bare `"filter "` prefill.
    fn add_group(tab: &mut TabState) -> (Box<dyn Mode>, KeyResult) {
        open_command(tab, "group ".to_string())
    }
}

#[async_trait]
impl Mode for GroupManagementMode {
    async fn handle_key(
        self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        let kb = tab.interaction.keybindings.clone();

        if self.searching {
            return self.handle_search_key(tab, &kb, key, modifiers);
        }

        if kb.filter.exit_mode.matches(key, modifiers) {
            tab.interaction.g_key_pressed = false;
            return (Box::new(NormalMode::default()), KeyResult::Handled);
        }
        if kb.filter.search.matches(key, modifiers) {
            tab.interaction.g_key_pressed = false;
            return (
                Box::new(GroupManagementMode {
                    selected_group: self.selected_group.clone(),
                    search: String::new(),
                    searching: true,
                    pre_search_selected: Some(self.selected_group.clone()),
                }),
                KeyResult::Handled,
            );
        }
        if kb.navigation.scroll_down.matches(key, modifiers) {
            return self.navigate(tab, 1);
        }
        if kb.navigation.scroll_up.matches(key, modifiers) {
            return self.navigate(tab, -1);
        }
        if kb.navigation.half_page_up.matches(key, modifiers) {
            let half = (tab.filter.groups_visible_height / 2).max(1) as i64;
            return self.navigate(tab, -half);
        }
        if kb.navigation.half_page_down.matches(key, modifiers) {
            let half = (tab.filter.groups_visible_height / 2).max(1) as i64;
            return self.navigate(tab, half);
        }
        if kb.navigation.page_up.matches(key, modifiers) {
            let page = tab.filter.groups_visible_height.max(1) as i64;
            return self.navigate(tab, -page);
        }
        if kb.navigation.page_down.matches(key, modifiers) {
            let page = tab.filter.groups_visible_height.max(1) as i64;
            return self.navigate(tab, page);
        }
        if kb.normal.go_to_bottom.matches(key, modifiers) {
            let names = tab.log_manager.group_names();
            let target = names
                .last()
                .cloned()
                .unwrap_or_else(|| self.selected_group.clone());
            return stay_at_group(target, tab);
        }
        if kb.normal.go_to_top_chord.matches(key, modifiers) {
            if tab.interaction.g_key_pressed {
                let names = tab.log_manager.group_names();
                let target = names
                    .first()
                    .cloned()
                    .unwrap_or_else(|| self.selected_group.clone());
                return stay_at_group(target, tab);
            }
            tab.interaction.g_key_pressed = true;
            return (self, KeyResult::Handled);
        }
        if kb.filter.toggle_filter.matches(key, modifiers) {
            return self.toggle_group(tab).await;
        }
        if kb.filter.toggle_all_filters.matches(key, modifiers) {
            toggle_all_filters(tab).await;
            return stay_at_group(self.selected_group.clone(), tab);
        }
        if kb.filter.delete_filter.matches(key, modifiers) {
            return self.delete_group(tab).await;
        }
        if kb.filter.edit_filter.matches(key, modifiers) {
            return self.edit_group_style(tab);
        }
        if kb.group.clear_group_style.matches(key, modifiers) {
            return self.clear_group_style(tab).await;
        }
        if kb.group.add_group.matches(key, modifiers) {
            return Self::add_group(tab);
        }

        tab.interaction.g_key_pressed = false;
        (self, KeyResult::Ignored)
    }

    fn mode_bar_content(&self, kb: &Keybindings, theme: &Theme) -> Line<'static> {
        let mut spans: Vec<Span<'static>> = vec![Span::styled(
            "[GROUP]  ",
            Style::default()
                .fg(theme.text_highlight_fg)
                .add_modifier(Modifier::BOLD),
        )];
        status_entry(
            &mut spans,
            format!(
                "{}/{}",
                kb.navigation.scroll_up.display(),
                kb.navigation.scroll_down.display()
            ),
            "nav",
            theme,
        );
        status_entry(
            &mut spans,
            format!(
                "{}/{}",
                kb.filter.toggle_filter.display(),
                kb.filter.toggle_all_filters.display()
            ),
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
        status_entry(
            &mut spans,
            kb.group.clear_group_style.display(),
            "clear style",
            theme,
        );
        status_entry(&mut spans, kb.group.add_group.display(), "add", theme);
        status_entry(&mut spans, kb.filter.search.display(), "search", theme);
        status_entry(&mut spans, kb.filter.exit_mode.display(), "exit", theme);
        Line::from(spans)
    }

    fn render_state(&self) -> ModeRenderState {
        ModeRenderState::GroupManagement {
            selected_group: self.selected_group.clone(),
            search: self.search.clone(),
            searching: self.searching,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::db::LogManager;
    use crate::filters::{FilterOptions, FilterType};
    use crate::ingestion::FileReader;
    use std::sync::Arc;

    async fn make_tab() -> TabState {
        let file_reader = FileReader::from_bytes(b"line".to_vec());
        let db = Arc::new(Database::in_memory().await.unwrap());
        let lm = LogManager::new(db, None).await;
        TabState::new(file_reader, lm, "test".to_string())
    }

    async fn add_filter(tab: &mut TabState, pattern: &str, group: &str, enabled: bool) {
        tab.log_manager
            .add_filter_with_color(
                pattern.to_string(),
                FilterType::Include,
                FilterOptions::default().line_mode().group(group),
            )
            .await;
        if !enabled {
            let id = tab.log_manager.get_filters().last().unwrap().id;
            tab.log_manager.toggle_filter(id).await;
        }
    }

    async fn press(
        mode: GroupManagementMode,
        tab: &mut TabState,
        code: KeyCode,
    ) -> (Box<dyn Mode>, KeyResult) {
        Box::new(mode)
            .handle_key(tab, code, KeyModifiers::NONE)
            .await
    }

    async fn press_mod(
        mode: GroupManagementMode,
        tab: &mut TabState,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        Box::new(mode).handle_key(tab, code, modifiers).await
    }

    /// Adds `n` single-filter groups named `g00`, `g01`, ... so
    /// `LogManager::group_names()`'s alphabetical sort matches index order.
    async fn add_n_groups(tab: &mut TabState, n: usize) {
        for i in 0..n {
            // Distinct patterns: `add_filter_with_color` dedups by
            // `(pattern, filter_type)`, so a shared pattern would just keep
            // updating one filter's group instead of creating `n` of them.
            add_filter(tab, &format!("x{i}"), &format!("g{i:02}"), true).await;
        }
    }

    fn expect_selected(state: ModeRenderState) -> String {
        match state {
            ModeRenderState::GroupManagement { selected_group, .. } => selected_group,
            other => panic!("expected GroupManagement, got {:?}", other),
        }
    }

    #[test]
    fn test_new_sets_selected_group() {
        let mode = GroupManagementMode::new("net".to_string());
        assert_eq!(mode.selected_group, "net");
    }

    #[tokio::test]
    async fn test_navigate_down_moves_to_next_group_alphabetically() {
        let mut tab = make_tab().await;
        add_filter(&mut tab, "a", "alpha", true).await;
        add_filter(&mut tab, "b", "beta", true).await;
        let mode = GroupManagementMode::new("alpha".to_string());
        let (mode2, result) = press(mode, &mut tab, KeyCode::Char('j')).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(matches!(
            mode2.render_state(),
            ModeRenderState::GroupManagement { selected_group, .. } if selected_group == "beta"
        ));
    }

    #[tokio::test]
    async fn test_navigate_down_at_last_group_stays() {
        let mut tab = make_tab().await;
        add_filter(&mut tab, "a", "alpha", true).await;
        add_filter(&mut tab, "b", "beta", true).await;
        let mode = GroupManagementMode::new("beta".to_string());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('j')).await;
        assert!(matches!(
            mode2.render_state(),
            ModeRenderState::GroupManagement { selected_group, .. } if selected_group == "beta"
        ));
    }

    #[tokio::test]
    async fn test_navigate_up_at_first_group_stays() {
        let mut tab = make_tab().await;
        add_filter(&mut tab, "a", "alpha", true).await;
        add_filter(&mut tab, "b", "beta", true).await;
        let mode = GroupManagementMode::new("alpha".to_string());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('k')).await;
        assert!(matches!(
            mode2.render_state(),
            ModeRenderState::GroupManagement { selected_group, .. } if selected_group == "alpha"
        ));
    }

    #[tokio::test]
    async fn test_space_disables_group_when_any_filter_enabled() {
        let mut tab = make_tab().await;
        add_filter(&mut tab, "a", "alpha", true).await;
        add_filter(&mut tab, "b", "alpha", false).await;
        let mode = GroupManagementMode::new("alpha".to_string());
        press(mode, &mut tab, KeyCode::Char(' ')).await;
        assert!(tab.log_manager.get_filters().iter().all(|f| !f.enabled));
    }

    #[tokio::test]
    async fn test_space_enables_group_when_all_disabled() {
        let mut tab = make_tab().await;
        add_filter(&mut tab, "a", "alpha", false).await;
        add_filter(&mut tab, "b", "alpha", false).await;
        let mode = GroupManagementMode::new("alpha".to_string());
        press(mode, &mut tab, KeyCode::Char(' ')).await;
        assert!(tab.log_manager.get_filters().iter().all(|f| f.enabled));
    }

    #[tokio::test]
    async fn test_space_toggle_only_affects_selected_group() {
        let mut tab = make_tab().await;
        add_filter(&mut tab, "a", "alpha", true).await;
        add_filter(&mut tab, "b", "beta", true).await;
        let mode = GroupManagementMode::new("alpha".to_string());
        press(mode, &mut tab, KeyCode::Char(' ')).await;
        let filters = tab.log_manager.get_filters();
        assert!(!filters.iter().find(|f| f.pattern == "a").unwrap().enabled);
        assert!(filters.iter().find(|f| f.pattern == "b").unwrap().enabled);
    }

    #[tokio::test]
    async fn test_shift_a_toggles_every_group_not_just_selected() {
        let mut tab = make_tab().await;
        add_filter(&mut tab, "a", "alpha", true).await;
        add_filter(&mut tab, "b", "beta", true).await;
        let mode = GroupManagementMode::new("alpha".to_string());
        let (mode2, result) = press(mode, &mut tab, KeyCode::Char('A')).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(tab.log_manager.get_filters().iter().all(|f| !f.enabled));
        assert_eq!(expect_selected(mode2.render_state()), "alpha");
    }

    #[tokio::test]
    async fn test_shift_a_enables_every_group_when_all_disabled() {
        let mut tab = make_tab().await;
        add_filter(&mut tab, "a", "alpha", false).await;
        add_filter(&mut tab, "b", "beta", false).await;
        let mode = GroupManagementMode::new("alpha".to_string());
        press(mode, &mut tab, KeyCode::Char('A')).await;
        assert!(tab.log_manager.get_filters().iter().all(|f| f.enabled));
    }

    #[tokio::test]
    async fn test_toggle_on_group_with_zero_filters_flips_stored_state() {
        let mut tab = make_tab().await;
        tab.log_manager
            .set_group_style("empty", Some("red"), None, true)
            .await;
        let mode = GroupManagementMode::new("empty".to_string());
        let (mode2, result) = press(mode, &mut tab, KeyCode::Char(' ')).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(matches!(
            mode2.render_state(),
            ModeRenderState::GroupManagement { selected_group, .. } if selected_group == "empty"
        ));
        // Newly created groups default to enabled, so one press disables it.
        assert!(!crate::filters::group_enabled(
            tab.log_manager.get_group_styles(),
            "empty"
        ));
    }

    #[tokio::test]
    async fn test_toggling_zero_filter_group_twice_restores_enabled_state() {
        let mut tab = make_tab().await;
        tab.log_manager
            .set_group_style("empty", Some("red"), None, true)
            .await;
        let mode = GroupManagementMode::new("empty".to_string());
        let (mode, _) = press(mode, &mut tab, KeyCode::Char(' ')).await;
        let (_, _) = mode
            .handle_key(&mut tab, KeyCode::Char(' '), KeyModifiers::NONE)
            .await;
        assert!(crate::filters::group_enabled(
            tab.log_manager.get_group_styles(),
            "empty"
        ));
    }

    #[tokio::test]
    async fn test_edit_style_opens_command_mode_prefilled() {
        let mut tab = make_tab().await;
        tab.log_manager
            .set_group_style("net", Some("red"), Some("blue"), true)
            .await;
        let mode = GroupManagementMode::new("net".to_string());
        let (mode2, result) = press(mode, &mut tab, KeyCode::Char('e')).await;
        assert!(matches!(result, KeyResult::Handled));
        match mode2.render_state() {
            ModeRenderState::Command { input, .. } => {
                assert!(input.starts_with("group net"), "got: {input:?}");
                assert!(input.contains("--fg"), "got: {input:?}");
                assert!(input.contains("--bg"), "got: {input:?}");
            }
            other => panic!("expected Command mode, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_c_is_not_bound_to_edit_style() {
        let mut tab = make_tab().await;
        tab.log_manager
            .set_group_style("net", Some("red"), Some("blue"), true)
            .await;
        let mode = GroupManagementMode::new("net".to_string());
        let (mode2, result) = press(mode, &mut tab, KeyCode::Char('c')).await;
        assert!(matches!(result, KeyResult::Ignored));
        assert!(matches!(
            mode2.render_state(),
            ModeRenderState::GroupManagement { .. }
        ));
    }

    #[tokio::test]
    async fn test_edit_style_with_no_existing_style_opens_bare_group_command() {
        let mut tab = make_tab().await;
        add_filter(&mut tab, "a", "net", true).await;
        let mode = GroupManagementMode::new("net".to_string());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('e')).await;
        match mode2.render_state() {
            ModeRenderState::Command { input, .. } => {
                assert_eq!(input, "group net");
            }
            other => panic!("expected Command mode, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_a_opens_command_mode_prefilled_with_bare_group() {
        let mut tab = make_tab().await;
        let mode = GroupManagementMode::new(String::new());
        let (mode2, result) = press(mode, &mut tab, KeyCode::Char('a')).await;
        assert!(matches!(result, KeyResult::Handled));
        match mode2.render_state() {
            ModeRenderState::Command { input, .. } => {
                assert_eq!(input, "group ");
            }
            other => panic!("expected Command mode, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_edit_style_quotes_group_name_with_whitespace() {
        let mut tab = make_tab().await;
        tab.log_manager
            .set_group_style("my group", Some("red"), None, true)
            .await;
        let mode = GroupManagementMode::new("my group".to_string());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('e')).await;
        match mode2.render_state() {
            ModeRenderState::Command { input, .. } => {
                assert!(input.starts_with("group \"my group\""), "got: {input:?}");
            }
            other => panic!("expected Command mode, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_clear_style_removes_style_and_stays_in_mode() {
        let mut tab = make_tab().await;
        tab.log_manager
            .set_group_style("net", Some("red"), None, true)
            .await;
        let mode = GroupManagementMode::new("net".to_string());
        let (mode2, result) = press(mode, &mut tab, KeyCode::Char('x')).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(crate::filters::group_style(tab.log_manager.get_group_styles(), "net").is_none());
        assert!(matches!(
            mode2.render_state(),
            ModeRenderState::GroupManagement { selected_group, .. } if selected_group == "net"
        ));
    }

    #[tokio::test]
    async fn test_d_deletes_group_and_its_filters() {
        let mut tab = make_tab().await;
        add_filter(&mut tab, "a", "alpha", true).await;
        add_filter(&mut tab, "b", "alpha", true).await;
        add_filter(&mut tab, "c", "beta", true).await;
        let mode = GroupManagementMode::new("alpha".to_string());
        let (mode2, result) = press(mode, &mut tab, KeyCode::Char('d')).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(!tab.log_manager.group_names().contains(&"alpha".to_string()));
        let patterns: Vec<_> = tab
            .log_manager
            .get_filters()
            .iter()
            .map(|f| f.pattern.clone())
            .collect();
        assert_eq!(patterns, vec!["c".to_string()]);
        assert_eq!(expect_selected(mode2.render_state()), "beta");
    }

    #[tokio::test]
    async fn test_d_deletes_group_style_too() {
        let mut tab = make_tab().await;
        tab.log_manager
            .set_group_style("empty", Some("red"), None, true)
            .await;
        let mode = GroupManagementMode::new("empty".to_string());
        press(mode, &mut tab, KeyCode::Char('d')).await;
        assert!(!tab.log_manager.group_names().contains(&"empty".to_string()));
        assert!(crate::filters::group_style(tab.log_manager.get_group_styles(), "empty").is_none());
    }

    #[tokio::test]
    async fn test_d_on_last_group_selects_previous() {
        let mut tab = make_tab().await;
        add_filter(&mut tab, "a", "alpha", true).await;
        add_filter(&mut tab, "b", "beta", true).await;
        let mode = GroupManagementMode::new("beta".to_string());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('d')).await;
        assert_eq!(expect_selected(mode2.render_state()), "alpha");
    }

    #[tokio::test]
    async fn test_d_on_only_group_leaves_empty_selection() {
        let mut tab = make_tab().await;
        add_filter(&mut tab, "a", "alpha", true).await;
        let mode = GroupManagementMode::new("alpha".to_string());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('d')).await;
        assert_eq!(expect_selected(mode2.render_state()), "");
        assert!(tab.log_manager.group_names().is_empty());
    }

    #[tokio::test]
    async fn test_exit_returns_normal_mode() {
        let mut tab = make_tab().await;
        let mode = GroupManagementMode::new("net".to_string());
        let (mode2, result) = press(mode, &mut tab, KeyCode::Esc).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(matches!(mode2.render_state(), ModeRenderState::Normal));
    }

    #[tokio::test]
    async fn test_unbound_key_is_ignored() {
        let mut tab = make_tab().await;
        let mode = GroupManagementMode::new("net".to_string());
        let (mode2, result) = press(mode, &mut tab, KeyCode::Char('z')).await;
        assert!(matches!(result, KeyResult::Ignored));
        assert!(matches!(
            mode2.render_state(),
            ModeRenderState::GroupManagement { selected_group, .. } if selected_group == "net"
        ));
    }

    #[test]
    fn test_mode_bar_content_lists_actions() {
        let mode = GroupManagementMode::new("net".to_string());
        let kb = Keybindings::default();
        let theme = Theme::default();
        let line = mode.mode_bar_content(&kb, &theme);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("GROUP"));
        assert!(text.contains("toggle"));
        assert!(text.contains("delete"));
        assert!(text.contains("edit"));
        assert!(text.contains("clear style"));
        assert!(text.contains("add"));
        assert!(text.contains("search"));
        assert!(text.contains("exit"));
    }

    #[test]
    fn test_render_state_matches_selected_group() {
        let mode = GroupManagementMode::new("sys".to_string());
        assert!(matches!(
            mode.render_state(),
            ModeRenderState::GroupManagement { selected_group, .. } if selected_group == "sys"
        ));
    }

    #[tokio::test]
    async fn test_ctrl_d_half_page_down_group() {
        let mut tab = make_tab().await;
        add_n_groups(&mut tab, 20).await;
        tab.filter.groups_visible_height = 10;
        let mode = GroupManagementMode::new("g00".to_string());
        let (mode2, _) = press_mod(mode, &mut tab, KeyCode::Char('d'), KeyModifiers::CONTROL).await;
        assert_eq!(expect_selected(mode2.render_state()), "g05");
    }

    #[tokio::test]
    async fn test_ctrl_u_half_page_up_group() {
        let mut tab = make_tab().await;
        add_n_groups(&mut tab, 20).await;
        tab.filter.groups_visible_height = 10;
        let mode = GroupManagementMode::new("g15".to_string());
        let (mode2, _) = press_mod(mode, &mut tab, KeyCode::Char('u'), KeyModifiers::CONTROL).await;
        assert_eq!(expect_selected(mode2.render_state()), "g10");
    }

    #[tokio::test]
    async fn test_page_down_moves_by_visible_height_group() {
        let mut tab = make_tab().await;
        add_n_groups(&mut tab, 30).await;
        tab.filter.groups_visible_height = 10;
        let mode = GroupManagementMode::new("g00".to_string());
        let (mode2, _) = press(mode, &mut tab, KeyCode::PageDown).await;
        assert_eq!(expect_selected(mode2.render_state()), "g10");
    }

    #[tokio::test]
    async fn test_page_up_moves_by_visible_height_group() {
        let mut tab = make_tab().await;
        add_n_groups(&mut tab, 30).await;
        tab.filter.groups_visible_height = 10;
        let mode = GroupManagementMode::new("g25".to_string());
        let (mode2, _) = press(mode, &mut tab, KeyCode::PageUp).await;
        assert_eq!(expect_selected(mode2.render_state()), "g15");
    }

    #[tokio::test]
    async fn test_page_down_clamps_at_last_group() {
        let mut tab = make_tab().await;
        add_n_groups(&mut tab, 5).await;
        tab.filter.groups_visible_height = 10;
        let mode = GroupManagementMode::new("g00".to_string());
        let (mode2, _) = press(mode, &mut tab, KeyCode::PageDown).await;
        assert_eq!(expect_selected(mode2.render_state()), "g04");
    }

    #[tokio::test]
    async fn test_bare_shift_g_jumps_to_last_group() {
        let mut tab = make_tab().await;
        add_n_groups(&mut tab, 10).await;
        let mode = GroupManagementMode::new("g00".to_string());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('G')).await;
        assert_eq!(expect_selected(mode2.render_state()), "g09");
    }

    #[tokio::test]
    async fn test_bare_g_jumps_to_first_group() {
        let mut tab = make_tab().await;
        add_n_groups(&mut tab, 10).await;
        let mode = GroupManagementMode::new("g05".to_string());
        let (mode, result) = press(mode, &mut tab, KeyCode::Char('g')).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(
            tab.interaction.g_key_pressed,
            "first 'g' should arm the chord"
        );
        let (mode, _) = mode
            .handle_key(&mut tab, KeyCode::Char('g'), KeyModifiers::NONE)
            .await;
        assert!(
            !tab.interaction.g_key_pressed,
            "second 'g' should complete and disarm the chord"
        );
        assert_eq!(expect_selected(mode.render_state()), "g00");
    }

    #[tokio::test]
    async fn test_slash_enters_search_mode_with_empty_query() {
        let mut tab = make_tab().await;
        add_n_groups(&mut tab, 3).await;
        let mode = GroupManagementMode::new("g00".to_string());
        let (mode, result) = press(mode, &mut tab, KeyCode::Char('/')).await;
        assert!(matches!(result, KeyResult::Handled));
        match mode.render_state() {
            ModeRenderState::GroupManagement {
                search, searching, ..
            } => {
                assert_eq!(search, "");
                assert!(searching);
            }
            other => panic!("expected GroupManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_typing_while_searching_narrows_and_moves_off_nonmatching_selection() {
        let mut tab = make_tab().await;
        add_filter(&mut tab, "a", "alpha", true).await;
        add_filter(&mut tab, "b", "beta", true).await;
        let mut mode = GroupManagementMode::new("alpha".to_string());
        mode.searching = true;
        let (mode, _) = press(mode, &mut tab, KeyCode::Char('b')).await;
        match mode.render_state() {
            ModeRenderState::GroupManagement {
                selected_group,
                search,
                ..
            } => {
                assert_eq!(search, "b");
                assert_eq!(selected_group, "beta");
            }
            other => panic!("expected GroupManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_action_letter_goes_to_query_while_searching_not_triggered() {
        let mut tab = make_tab().await;
        tab.log_manager
            .set_group_style("net", Some("red"), None, true)
            .await;
        let mut mode = GroupManagementMode::new("net".to_string());
        mode.searching = true;
        // 'e' is normally bound to edit_group_style — while searching it must
        // be captured as query text instead, mirroring `FilterManagementMode`.
        let (mode, result) = press(mode, &mut tab, KeyCode::Char('e')).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(matches!(
            mode.render_state(),
            ModeRenderState::GroupManagement { .. }
        ));
        match mode.render_state() {
            ModeRenderState::GroupManagement { search, .. } => assert_eq!(search, "e"),
            other => panic!("expected GroupManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_d_goes_to_query_while_searching_instead_of_deleting() {
        let mut tab = make_tab().await;
        add_filter(&mut tab, "a", "net", true).await;
        let mut mode = GroupManagementMode::new("net".to_string());
        mode.searching = true;
        let (mode, _) = press(mode, &mut tab, KeyCode::Char('d')).await;
        assert!(tab.log_manager.group_names().contains(&"net".to_string()));
        match mode.render_state() {
            ModeRenderState::GroupManagement { search, .. } => assert_eq!(search, "d"),
            other => panic!("expected GroupManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_backspace_removes_last_search_char() {
        let mut tab = make_tab().await;
        add_n_groups(&mut tab, 3).await;
        let mut mode = GroupManagementMode::new("g00".to_string());
        mode.searching = true;
        mode.search = "ab".to_string();
        let (mode, _) = press(mode, &mut tab, KeyCode::Backspace).await;
        match mode.render_state() {
            ModeRenderState::GroupManagement { search, .. } => assert_eq!(search, "a"),
            other => panic!("expected GroupManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_enter_confirms_search_and_exits_searching() {
        let mut tab = make_tab().await;
        add_n_groups(&mut tab, 3).await;
        let mut mode = GroupManagementMode::new("g00".to_string());
        mode.searching = true;
        mode.search = "g02".to_string();
        let (mode, _) = press(mode, &mut tab, KeyCode::Enter).await;
        match mode.render_state() {
            ModeRenderState::GroupManagement {
                selected_group,
                search,
                searching,
            } => {
                assert_eq!(selected_group, "g00");
                assert_eq!(search, "", "confirming search must un-narrow the sidebar");
                assert!(!searching);
            }
            other => panic!("expected GroupManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_esc_cancels_search_and_restores_original_selection() {
        let mut tab = make_tab().await;
        add_filter(&mut tab, "a", "alpha", true).await;
        add_filter(&mut tab, "b", "beta", true).await;
        add_filter(&mut tab, "c", "candy", true).await;
        let mut mode = GroupManagementMode::new("alpha".to_string());
        mode.searching = true;
        mode.pre_search_selected = Some("alpha".to_string());
        mode.search = "c".to_string();
        let (mode, result) = press(mode, &mut tab, KeyCode::Esc).await;
        assert!(matches!(result, KeyResult::Handled));
        match mode.render_state() {
            ModeRenderState::GroupManagement {
                selected_group,
                search,
                searching,
            } => {
                assert_eq!(
                    selected_group, "alpha",
                    "Esc must restore the pre-search selection"
                );
                assert_eq!(search, "");
                assert!(!searching);
            }
            other => panic!("expected GroupManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_j_navigates_within_narrowed_list_while_searching() {
        let mut tab = make_tab().await;
        add_filter(&mut tab, "a", "car", true).await;
        add_filter(&mut tab, "b", "cat", true).await;
        add_filter(&mut tab, "c", "dog", true).await;
        let mut mode = GroupManagementMode::new("car".to_string());
        mode.searching = true;
        mode.search = "ca".to_string();
        let (mode, _) = press(mode, &mut tab, KeyCode::Char('j')).await;
        match mode.render_state() {
            ModeRenderState::GroupManagement {
                selected_group,
                searching,
                ..
            } => {
                assert_eq!(
                    selected_group, "cat",
                    "only 'car'/'cat' match 'ca', not 'dog'"
                );
                assert!(searching, "navigating within search must stay searching");
            }
            other => panic!("expected GroupManagement, got {:?}", other),
        }
    }
}
