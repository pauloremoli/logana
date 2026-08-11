use crate::config::Keybindings;
use crate::filters::ColorConfig;
use crate::mode::app_mode::{Mode, ModeRenderState, status_entry};
use crate::mode::filter_mode::{append_color_flags, open_command, quote_command_arg};
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
}

impl GroupManagementMode {
    pub fn new(selected_group: String) -> Self {
        Self { selected_group }
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

    /// Toggles every filter in the selected group on/off together,
    /// replicating `cmd_toggle_group`'s any-enabled logic exactly (flip to
    /// the opposite of "any filter in the group is currently enabled"). A
    /// zero-filter group (one that exists only via a predefined style) is a
    /// silent no-op here rather than the CLI command's hard error, since a
    /// keypress in this mode has no natural place to surface an error.
    async fn toggle_group(&self, tab: &mut TabState) -> (Box<dyn Mode>, KeyResult) {
        let any_enabled = tab
            .log_manager
            .get_filters()
            .iter()
            .any(|f| f.group.as_deref() == Some(self.selected_group.as_str()) && f.enabled);
        tab.log_manager
            .set_filters_enabled_by_group(&self.selected_group, !any_enabled)
            .await;
        tab.begin_filter_refresh();
        stay_at_group(self.selected_group.clone(), tab)
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

        if kb.filter.exit_mode.matches(key, modifiers) {
            tab.interaction.g_key_pressed = false;
            return (Box::new(NormalMode::default()), KeyResult::Handled);
        }
        if kb.navigation.scroll_down.matches(key, modifiers) {
            return self.navigate(tab, 1);
        }
        if kb.navigation.scroll_up.matches(key, modifiers) {
            return self.navigate(tab, -1);
        }
        if kb.filter.toggle_all_filters.matches(key, modifiers)
            || kb.filter.toggle_filter.matches(key, modifiers)
        {
            return self.toggle_group(tab).await;
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
        status_entry(&mut spans, kb.filter.edit_filter.display(), "edit", theme);
        status_entry(
            &mut spans,
            kb.group.clear_group_style.display(),
            "clear style",
            theme,
        );
        status_entry(&mut spans, kb.group.add_group.display(), "add", theme);
        status_entry(&mut spans, kb.filter.exit_mode.display(), "exit", theme);
        Line::from(spans)
    }

    fn render_state(&self) -> ModeRenderState {
        ModeRenderState::GroupManagement {
            selected_group: self.selected_group.clone(),
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
            ModeRenderState::GroupManagement { selected_group } if selected_group == "beta"
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
            ModeRenderState::GroupManagement { selected_group } if selected_group == "beta"
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
            ModeRenderState::GroupManagement { selected_group } if selected_group == "alpha"
        ));
    }

    #[tokio::test]
    async fn test_toggle_disables_when_any_filter_enabled() {
        let mut tab = make_tab().await;
        add_filter(&mut tab, "a", "alpha", true).await;
        add_filter(&mut tab, "b", "alpha", false).await;
        let mode = GroupManagementMode::new("alpha".to_string());
        press(mode, &mut tab, KeyCode::Char('A')).await;
        assert!(tab.log_manager.get_filters().iter().all(|f| !f.enabled));
    }

    #[tokio::test]
    async fn test_toggle_enables_when_all_disabled() {
        let mut tab = make_tab().await;
        add_filter(&mut tab, "a", "alpha", false).await;
        add_filter(&mut tab, "b", "alpha", false).await;
        let mode = GroupManagementMode::new("alpha".to_string());
        press(mode, &mut tab, KeyCode::Char('A')).await;
        assert!(tab.log_manager.get_filters().iter().all(|f| f.enabled));
    }

    #[tokio::test]
    async fn test_space_toggles_group_same_as_toggle_all_key() {
        let mut tab = make_tab().await;
        add_filter(&mut tab, "a", "alpha", true).await;
        add_filter(&mut tab, "b", "alpha", false).await;
        let mode = GroupManagementMode::new("alpha".to_string());
        press(mode, &mut tab, KeyCode::Char(' ')).await;
        assert!(tab.log_manager.get_filters().iter().all(|f| !f.enabled));
    }

    #[tokio::test]
    async fn test_toggle_on_group_with_zero_filters_is_noop() {
        let mut tab = make_tab().await;
        tab.log_manager
            .set_group_style("empty", Some("red"), None, true)
            .await;
        let mode = GroupManagementMode::new("empty".to_string());
        let (mode2, result) = press(mode, &mut tab, KeyCode::Char('A')).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(matches!(
            mode2.render_state(),
            ModeRenderState::GroupManagement { selected_group } if selected_group == "empty"
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
            ModeRenderState::GroupManagement { selected_group } if selected_group == "net"
        ));
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
            ModeRenderState::GroupManagement { selected_group } if selected_group == "net"
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
        assert!(text.contains("edit"));
        assert!(text.contains("clear style"));
        assert!(text.contains("add"));
        assert!(text.contains("exit"));
    }

    #[test]
    fn test_render_state_matches_selected_group() {
        let mode = GroupManagementMode::new("sys".to_string());
        assert!(matches!(
            mode.render_state(),
            ModeRenderState::GroupManagement { selected_group } if selected_group == "sys"
        ));
    }
}
