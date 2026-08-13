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
    /// Pending count prefix (e.g. the `4` in `4j`), mirrors `NormalMode.count`.
    pub count: Option<usize>,
    /// Live typeahead query; non-empty narrows the sidebar to matching filters.
    pub search: String,
    /// True while capturing raw text input for `search` (gates all other bound keys).
    pub searching: bool,
    /// Selection to restore if search is cancelled with `Esc`.
    pub pre_search_selected: Option<usize>,
}

/// Returns to filter mode at `idx`. Resets `g_key_pressed` — every action
/// that isn't part of an in-progress `gg` chord cancels that chord, matching
/// `NormalMode`'s discipline.
fn stay_at(idx: usize, tab: &mut TabState) -> (Box<dyn Mode>, KeyResult) {
    tab.interaction.g_key_pressed = false;
    (Box::new(FilterManagementMode::new(idx)), KeyResult::Handled)
}

pub(crate) fn open_command(tab: &mut TabState, cmd: String) -> (Box<dyn Mode>, KeyResult) {
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
    ignore_case: bool,
    group: &Option<String>,
) -> String {
    if let Some(expr) = pattern.strip_prefix(crate::filters::DATE_PREFIX) {
        build_date_filter_command(cc, expr)
    } else if let Some(expr) = pattern.strip_prefix(crate::filters::FIELD_PREFIX) {
        build_field_filter_command(ft, cc, expr, group)
    } else {
        build_text_filter_command(ft, cc, pattern, use_regex, ignore_case, group)
    }
}

fn build_date_filter_command(cc: &Option<ColorConfig>, expr: &str) -> String {
    let mut c = String::from("date-filter");
    append_color_flags(&mut c, cc, true);
    c.push(' ');
    c.push_str(expr);
    c
}

fn build_field_filter_command(
    ft: &FilterType,
    cc: &Option<ColorConfig>,
    expr: &str,
    group: &Option<String>,
) -> String {
    let mut c = filter_command_prefix(ft);
    if matches!(ft, FilterType::Include | FilterType::Highlight) {
        append_color_flags(&mut c, cc, true);
    }
    append_group_flag(&mut c, group);
    match crate::filters::parse_field_filter_expr(expr) {
        Ok((conditions, text)) => {
            for (key, value) in &conditions {
                c.push_str(" --field ");
                c.push_str(key);
                c.push('=');
                c.push_str(value);
            }
            if let Some(t) = text {
                c.push(' ');
                c.push_str(&t);
            }
        }
        Err(_) => {
            c.push_str(" --field ");
            c.push_str(expr);
        }
    }
    c
}

fn build_text_filter_command(
    ft: &FilterType,
    cc: &Option<ColorConfig>,
    pattern: &str,
    use_regex: bool,
    ignore_case: bool,
    group: &Option<String>,
) -> String {
    let mut c = filter_command_prefix(ft);
    if use_regex {
        c.push_str(" --regex");
    }
    if ignore_case {
        c.push_str(" --ignore-case");
    }
    if matches!(ft, FilterType::Include | FilterType::Highlight) {
        append_color_flags(&mut c, cc, true);
    }
    append_group_flag(&mut c, group);
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

fn filter_command_prefix(ft: &FilterType) -> String {
    match ft {
        FilterType::Include => String::from("filter"),
        FilterType::Exclude => String::from("exclude"),
        FilterType::Highlight => String::from("highlight"),
    }
}

pub(crate) fn append_color_flags(
    cmd: &mut String,
    cc: &Option<ColorConfig>,
    include_line_flag: bool,
) {
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

/// Quotes `name` (escaping backslashes/quotes) if it contains whitespace, so
/// it round-trips through `shell_split` as a single token; returned as-is
/// otherwise. Shared by anything embedding a user-chosen name (filter group,
/// etc.) as a bare or flag-valued command argument.
pub(crate) fn quote_command_arg(name: &str) -> String {
    if name.contains(char::is_whitespace) {
        let escaped = name.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{escaped}\"")
    } else {
        name.to_string()
    }
}

/// Append `--group <name>`, quoting the name if it contains whitespace so it
/// round-trips through `shell_split` as a single token.
fn append_group_flag(cmd: &mut String, group: &Option<String>) {
    if let Some(name) = group {
        cmd.push_str(&format!(" --group {}", quote_command_arg(name)));
    }
}

impl FilterManagementMode {
    pub fn new(selected_filter_index: usize) -> Self {
        Self {
            selected_filter_index,
            count: None,
            search: String::new(),
            searching: false,
            pre_search_selected: None,
        }
    }

    /// Returns to filter mode at `idx` while preserving the in-progress
    /// search — unlike `stay_at`, which always resets back to a non-searching
    /// mode. Used for navigation (`j`/`k`) within the narrowed list while
    /// `searching` is active.
    fn stay_searching(&self, idx: usize) -> (Box<dyn Mode>, KeyResult) {
        (
            Box::new(FilterManagementMode {
                selected_filter_index: idx,
                count: None,
                search: self.search.clone(),
                searching: true,
                pre_search_selected: self.pre_search_selected,
            }),
            KeyResult::Handled,
        )
    }

    /// Indices (into the full filter list) of entries matching the current
    /// search query — everything, when not searching or the query is empty.
    fn narrowed_indices(&self, tab: &TabState) -> Vec<usize> {
        crate::ui::widgets::sidebar::narrowed_filter_indices(
            tab.log_manager.get_filters(),
            &tab.filter.match_counts,
            &self.search,
        )
    }

    /// Handles a key while `searching` is active: raw text capture for the
    /// query plus single-step navigation within the narrowed list. No bound
    /// filter-mode action (`e`, `d`, `i`, ...) is checked here — see the
    /// module-level notes on why an explicit entry key gates this.
    fn handle_search_key(
        mut self: Box<Self>,
        tab: &mut TabState,
        kb: &Keybindings,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        if kb.search.confirm.matches(key, modifiers) {
            let narrowed = self.narrowed_indices(tab);
            let full_idx = narrowed
                .get(self.selected_filter_index)
                .copied()
                .unwrap_or(0);
            return stay_at(full_idx, tab);
        }
        if kb.search.cancel.matches(key, modifiers) {
            let restore_idx = self.pre_search_selected.unwrap_or(0);
            return stay_at(restore_idx, tab);
        }
        if kb.navigation.scroll_down.matches(key, modifiers) {
            let narrowed_len = self.narrowed_indices(tab).len();
            let new_idx = if narrowed_len > 0 {
                (self.selected_filter_index + 1).min(narrowed_len - 1)
            } else {
                0
            };
            return self.stay_searching(new_idx);
        }
        if kb.navigation.scroll_up.matches(key, modifiers) {
            let new_idx = self.selected_filter_index.saturating_sub(1);
            return self.stay_searching(new_idx);
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
        let narrowed_len = self.narrowed_indices(tab).len();
        self.selected_filter_index = if self.selected_filter_index < narrowed_len {
            self.selected_filter_index
        } else {
            0
        };
        (self, KeyResult::Handled)
    }

    fn scroll_up(&self, tab: &mut TabState, count: usize) -> (Box<dyn Mode>, KeyResult) {
        stay_at(self.selected_filter_index.saturating_sub(count), tab)
    }

    fn scroll_down(&self, tab: &mut TabState, count: usize) -> (Box<dyn Mode>, KeyResult) {
        let num_filters = tab.log_manager.get_filters().len();
        let new_idx = if num_filters > 0 {
            (self.selected_filter_index + count).min(num_filters - 1)
        } else {
            0
        };
        stay_at(new_idx, tab)
    }

    async fn toggle_filter(&self, tab: &mut TabState) -> (Box<dyn Mode>, KeyResult) {
        let selected = self.selected_filter_index;
        let filter_id = tab.log_manager.get_filters().get(selected).map(|f| f.id);
        if let Some(id) = filter_id {
            tab.log_manager.toggle_filter(id).await;
            tab.begin_filter_refresh();
        }
        stay_at(selected, tab)
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
            return stay_at(new_idx, tab);
        }
        stay_at(selected, tab)
    }

    async fn move_filter_up(&self, tab: &mut TabState) -> (Box<dyn Mode>, KeyResult) {
        let selected = self.selected_filter_index;
        let filter_id = tab.log_manager.get_filters().get(selected).map(|f| f.id);
        if let Some(id) = filter_id {
            tab.log_manager.move_filter_up(id).await;
            tab.begin_filter_refresh();
            return stay_at(selected.saturating_sub(1), tab);
        }
        stay_at(selected, tab)
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
            return stay_at(new_idx, tab);
        }
        stay_at(selected, tab)
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
                f.ignore_case,
                f.group.clone(),
            )
        });
        if let Some((id, ft, cc, pattern, use_regex, ignore_case, group)) = filter_info {
            tab.filter.editing_filter_id = Some(id);
            tab.filter.filter_context = Some(selected);
            let cmd = build_edit_command(&ft, &cc, &pattern, use_regex, ignore_case, &group);
            return open_command(tab, cmd);
        }
        stay_at(selected, tab)
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
        stay_at(self.selected_filter_index, tab)
    }

    async fn toggle_all_filters(&self, tab: &mut TabState) -> (Box<dyn Mode>, KeyResult) {
        let any_enabled = tab.log_manager.get_filters().iter().any(|f| f.enabled);
        if any_enabled {
            tab.log_manager.disable_all_filters().await;
        } else {
            tab.log_manager.enable_all_filters().await;
        }
        tab.begin_filter_refresh();
        stay_at(self.selected_filter_index, tab)
    }

    async fn clear_all_filters(&self, tab: &mut TabState) -> (Box<dyn Mode>, KeyResult) {
        tab.log_manager.clear_filters().await;
        tab.begin_filter_refresh();
        stay_at(0, tab)
    }

    fn add_include_filter(tab: &mut TabState) -> (Box<dyn Mode>, KeyResult) {
        open_command(tab, "filter ".to_string())
    }

    fn add_include_filter_auto(tab: &mut TabState) -> (Box<dyn Mode>, KeyResult) {
        open_command(tab, "filter --auto ".to_string())
    }

    fn add_exclude_filter(tab: &mut TabState) -> (Box<dyn Mode>, KeyResult) {
        open_command(tab, "exclude ".to_string())
    }

    fn add_date_filter(tab: &mut TabState) -> (Box<dyn Mode>, KeyResult) {
        open_command(tab, "date-filter ".to_string())
    }

    fn add_highlight_filter(tab: &mut TabState) -> (Box<dyn Mode>, KeyResult) {
        open_command(tab, "highlight ".to_string())
    }

    fn sidebar_grow(&self, tab: &mut TabState) -> (Box<dyn Mode>, KeyResult) {
        tab.display.sidebar_width = tab.display.sidebar_width.saturating_add(2);
        let (mode, _) = stay_at(self.selected_filter_index, tab);
        (mode, KeyResult::ResizeSidebar(tab.display.sidebar_width))
    }

    fn sidebar_shrink(&self, tab: &mut TabState) -> (Box<dyn Mode>, KeyResult) {
        tab.display.sidebar_width = tab.display.sidebar_width.saturating_sub(2).max(10);
        let (mode, _) = stay_at(self.selected_filter_index, tab);
        (mode, KeyResult::ResizeSidebar(tab.display.sidebar_width))
    }
}

#[async_trait]
impl Mode for FilterManagementMode {
    async fn handle_key(
        mut self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        let kb = tab.interaction.keybindings.clone();

        if self.searching {
            return self.handle_search_key(tab, &kb, key, modifiers);
        }

        if let KeyCode::Char(c @ '1'..='9') = key
            && (modifiers.is_empty() || modifiers == KeyModifiers::SHIFT)
        {
            let digit = (c as u32 - '0' as u32) as usize;
            let n = self
                .count
                .unwrap_or(0)
                .saturating_mul(10)
                .saturating_add(digit);
            self.count = Some(n.min(999_999));
            return (self, KeyResult::Handled);
        }
        if let KeyCode::Char('0') = key
            && self.count.is_some()
            && (modifiers.is_empty() || modifiers == KeyModifiers::SHIFT)
        {
            self.count = Some(self.count.unwrap().saturating_mul(10).min(999_999));
            return (self, KeyResult::Handled);
        }

        if kb.global.next_tab.matches(key, modifiers)
            || kb.global.prev_tab.matches(key, modifiers)
            || kb.global.file_switcher.matches(key, modifiers)
        {
            self.count = None;
            tab.interaction.g_key_pressed = false;
            return (self, KeyResult::Ignored);
        }

        if kb.filter.exit_mode.matches(key, modifiers) {
            tab.interaction.g_key_pressed = false;
            return (Box::new(NormalMode::default()), KeyResult::Handled);
        }
        if kb.filter.search.matches(key, modifiers) {
            self.pre_search_selected = Some(self.selected_filter_index);
            self.search = String::new();
            self.searching = true;
            self.count = None;
            tab.interaction.g_key_pressed = false;
            return (self, KeyResult::Handled);
        }
        if kb.navigation.scroll_up.matches(key, modifiers) {
            let count = self.count.take().unwrap_or(1);
            return self.scroll_up(tab, count);
        }
        if kb.navigation.scroll_down.matches(key, modifiers) {
            let count = self.count.take().unwrap_or(1);
            return self.scroll_down(tab, count);
        }
        if kb.navigation.half_page_up.matches(key, modifiers) {
            let half = (tab.filter.sidebar_visible_height / 2).max(1);
            let count = self.count.take().unwrap_or(1);
            return self.scroll_up(tab, half.saturating_mul(count));
        }
        if kb.navigation.half_page_down.matches(key, modifiers) {
            let half = (tab.filter.sidebar_visible_height / 2).max(1);
            let count = self.count.take().unwrap_or(1);
            return self.scroll_down(tab, half.saturating_mul(count));
        }
        if kb.navigation.page_up.matches(key, modifiers) {
            let page = tab.filter.sidebar_visible_height.max(1);
            let count = self.count.take().unwrap_or(1);
            return self.scroll_up(tab, page.saturating_mul(count));
        }
        if kb.navigation.page_down.matches(key, modifiers) {
            let page = tab.filter.sidebar_visible_height.max(1);
            let count = self.count.take().unwrap_or(1);
            return self.scroll_down(tab, page.saturating_mul(count));
        }
        if kb.normal.go_to_bottom.matches(key, modifiers) {
            let num_filters = tab.log_manager.get_filters().len();
            let idx = match self.count.take() {
                Some(count) => (count.saturating_sub(1)).min(num_filters.saturating_sub(1)),
                None => num_filters.saturating_sub(1),
            };
            return stay_at(idx, tab);
        }
        if kb.normal.go_to_top_chord.matches(key, modifiers) {
            if tab.interaction.g_key_pressed {
                let num_filters = tab.log_manager.get_filters().len();
                let idx = match self.count.take() {
                    Some(count) => (count.saturating_sub(1)).min(num_filters.saturating_sub(1)),
                    None => 0,
                };
                return stay_at(idx, tab);
            }
            tab.interaction.g_key_pressed = true;
            return (self, KeyResult::Handled);
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
        if kb.filter.add_include_filter_auto.matches(key, modifiers) {
            return Self::add_include_filter_auto(tab);
        }
        if kb.filter.add_exclude_filter.matches(key, modifiers) {
            return Self::add_exclude_filter(tab);
        }
        if kb.filter.add_date_filter.matches(key, modifiers) {
            return Self::add_date_filter(tab);
        }
        if kb.filter.add_highlight_filter.matches(key, modifiers) {
            return Self::add_highlight_filter(tab);
        }
        if kb.filter.sidebar_grow.matches(key, modifiers) {
            return self.sidebar_grow(tab);
        }
        if kb.filter.sidebar_shrink.matches(key, modifiers) {
            return self.sidebar_shrink(tab);
        }

        self.count = None;
        tab.interaction.g_key_pressed = false;
        (self, KeyResult::Ignored)
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
            kb.filter.add_include_filter_auto.display(),
            "filter in (auto)",
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
            kb.filter.add_highlight_filter.display(),
            "highlight",
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
        status_entry(&mut spans, kb.filter.search.display(), "search", theme);
        status_entry(&mut spans, kb.filter.exit_mode.display(), "exit", theme);
        Line::from(spans)
    }

    fn render_state(&self) -> ModeRenderState {
        ModeRenderState::FilterManagement {
            selected_index: self.selected_filter_index,
            search: self.search.clone(),
            searching: self.searching,
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
            return (Box::new(FilterManagementMode::new(0)), KeyResult::Handled);
        }
        if kb.filter_edit.cancel.matches(key, modifiers) {
            return (Box::new(FilterManagementMode::new(0)), KeyResult::Handled);
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
        FilterManagementMode::new(idx)
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
    async fn test_sidebar_grow_emits_resize_sidebar_with_new_width() {
        let mut tab = make_tab(&["line"]).await;
        let before = tab.display.sidebar_width;
        let (_, result) = press(filter_mode(0), &mut tab, KeyCode::Char('>')).await;
        assert_eq!(tab.display.sidebar_width, before + 2);
        assert!(matches!(result, KeyResult::ResizeSidebar(w) if w == before + 2));
    }

    #[tokio::test]
    async fn test_sidebar_shrink_emits_resize_sidebar_with_new_width() {
        let mut tab = make_tab(&["line"]).await;
        let before = tab.display.sidebar_width;
        let (_, result) = press(filter_mode(0), &mut tab, KeyCode::Char('<')).await;
        assert_eq!(tab.display.sidebar_width, before - 2);
        assert!(matches!(result, KeyResult::ResizeSidebar(w) if w == before - 2));
    }

    #[tokio::test]
    async fn test_sidebar_shrink_stops_at_minimum_width() {
        let mut tab = make_tab(&["line"]).await;
        tab.display.sidebar_width = 10;
        let (_, result) = press(filter_mode(0), &mut tab, KeyCode::Char('<')).await;
        assert_eq!(tab.display.sidebar_width, 10);
        assert!(matches!(result, KeyResult::ResizeSidebar(10)));
    }

    #[tokio::test]
    async fn test_up_decrements_selected_index() {
        let mut tab = make_tab(&["a", "b"]).await;
        add_filter(&mut tab, "a", FilterType::Include).await;
        add_filter(&mut tab, "b", FilterType::Include).await;
        let (mode, _) = press(filter_mode(1), &mut tab, KeyCode::Up).await;
        match mode.render_state() {
            ModeRenderState::FilterManagement { selected_index, .. } => {
                assert_eq!(selected_index, 0)
            }
            other => panic!("expected FilterManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_up_saturates_at_zero() {
        let mut tab = make_tab(&["a"]).await;
        let (mode, _) = press(filter_mode(0), &mut tab, KeyCode::Up).await;
        match mode.render_state() {
            ModeRenderState::FilterManagement { selected_index, .. } => {
                assert_eq!(selected_index, 0)
            }
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
            ModeRenderState::FilterManagement { selected_index, .. } => {
                assert_eq!(selected_index, 1)
            }
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
            ModeRenderState::FilterManagement { selected_index, .. } => {
                assert_eq!(selected_index, 1)
            }
            other => panic!("expected FilterManagement, got {:?}", other),
        }
    }

    fn filters(n: usize) -> Vec<&'static str> {
        vec!["x"; n]
    }

    async fn add_n_filters(tab: &mut TabState, n: usize) {
        for i in 0..n {
            add_filter(tab, &format!("pattern{i}"), FilterType::Include).await;
        }
    }

    #[tokio::test]
    async fn test_count_4j_moves_down_4() {
        let mut tab = make_tab(&filters(10)).await;
        add_n_filters(&mut tab, 10).await;
        let mut mode = filter_mode(0);
        mode.count = Some(4);
        let (mode, _) = press(mode, &mut tab, KeyCode::Char('j')).await;
        match mode.render_state() {
            ModeRenderState::FilterManagement { selected_index, .. } => {
                assert_eq!(selected_index, 4)
            }
            other => panic!("expected FilterManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_count_10k_moves_up_10_clamped() {
        let mut tab = make_tab(&filters(20)).await;
        add_n_filters(&mut tab, 20).await;
        let mut mode = filter_mode(5);
        mode.count = Some(10);
        let (mode, _) = press(mode, &mut tab, KeyCode::Char('k')).await;
        match mode.render_state() {
            ModeRenderState::FilterManagement { selected_index, .. } => {
                assert_eq!(selected_index, 0)
            }
            other => panic!("expected FilterManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_digit_accumulation_then_j_moves_down_by_typed_count() {
        let mut tab = make_tab(&filters(10)).await;
        add_n_filters(&mut tab, 10).await;
        let (mode, result) = press(filter_mode(0), &mut tab, KeyCode::Char('4')).await;
        assert!(matches!(result, KeyResult::Handled));
        let (mode, _) = mode
            .handle_key(&mut tab, KeyCode::Char('j'), KeyModifiers::NONE)
            .await;
        match mode.render_state() {
            ModeRenderState::FilterManagement { selected_index, .. } => {
                assert_eq!(selected_index, 4)
            }
            other => panic!("expected FilterManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_count_resets_after_being_consumed() {
        let mut tab = make_tab(&filters(20)).await;
        add_n_filters(&mut tab, 20).await;
        let mut mode = filter_mode(0);
        mode.count = Some(4);
        let (mode, _) = press(mode, &mut tab, KeyCode::Char('j')).await;
        // Second 'j' with no new count should move by 1, not reuse the old count of 4.
        let (mode, _) = mode
            .handle_key(&mut tab, KeyCode::Char('j'), KeyModifiers::NONE)
            .await;
        match mode.render_state() {
            ModeRenderState::FilterManagement { selected_index, .. } => {
                assert_eq!(selected_index, 5)
            }
            other => panic!("expected FilterManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_count_resets_on_unrecognized_key() {
        let mut tab = make_tab(&filters(10)).await;
        add_n_filters(&mut tab, 10).await;
        let (mode, _) = press(filter_mode(0), &mut tab, KeyCode::Char('4')).await;
        let (mode, _) = mode
            .handle_key(&mut tab, KeyCode::F(5), KeyModifiers::NONE)
            .await;
        let (mode, _) = mode
            .handle_key(&mut tab, KeyCode::Char('j'), KeyModifiers::NONE)
            .await;
        match mode.render_state() {
            ModeRenderState::FilterManagement { selected_index, .. } => {
                assert_eq!(selected_index, 1)
            }
            other => panic!("expected FilterManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_ctrl_d_half_page_down() {
        let mut tab = make_tab(&filters(20)).await;
        add_n_filters(&mut tab, 20).await;
        tab.filter.sidebar_visible_height = 10;
        let (mode, _) = Box::new(filter_mode(0))
            .handle_key(&mut tab, KeyCode::Char('d'), KeyModifiers::CONTROL)
            .await;
        match mode.render_state() {
            ModeRenderState::FilterManagement { selected_index, .. } => {
                assert_eq!(selected_index, 5)
            }
            other => panic!("expected FilterManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_ctrl_u_half_page_up() {
        let mut tab = make_tab(&filters(20)).await;
        add_n_filters(&mut tab, 20).await;
        tab.filter.sidebar_visible_height = 10;
        let (mode, _) = Box::new(filter_mode(15))
            .handle_key(&mut tab, KeyCode::Char('u'), KeyModifiers::CONTROL)
            .await;
        match mode.render_state() {
            ModeRenderState::FilterManagement { selected_index, .. } => {
                assert_eq!(selected_index, 10)
            }
            other => panic!("expected FilterManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_page_down_moves_by_visible_height() {
        let mut tab = make_tab(&filters(30)).await;
        add_n_filters(&mut tab, 30).await;
        tab.filter.sidebar_visible_height = 10;
        let (mode, _) = press(filter_mode(0), &mut tab, KeyCode::PageDown).await;
        match mode.render_state() {
            ModeRenderState::FilterManagement { selected_index, .. } => {
                assert_eq!(selected_index, 10)
            }
            other => panic!("expected FilterManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_page_up_moves_by_visible_height() {
        let mut tab = make_tab(&filters(30)).await;
        add_n_filters(&mut tab, 30).await;
        tab.filter.sidebar_visible_height = 10;
        let (mode, _) = press(filter_mode(25), &mut tab, KeyCode::PageUp).await;
        match mode.render_state() {
            ModeRenderState::FilterManagement { selected_index, .. } => {
                assert_eq!(selected_index, 15)
            }
            other => panic!("expected FilterManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_page_down_clamps_at_last_filter() {
        let mut tab = make_tab(&filters(5)).await;
        add_n_filters(&mut tab, 5).await;
        tab.filter.sidebar_visible_height = 10;
        let (mode, _) = press(filter_mode(0), &mut tab, KeyCode::PageDown).await;
        match mode.render_state() {
            ModeRenderState::FilterManagement { selected_index, .. } => {
                assert_eq!(selected_index, 4)
            }
            other => panic!("expected FilterManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_count_2_page_down_moves_by_2_pages() {
        let mut tab = make_tab(&filters(50)).await;
        add_n_filters(&mut tab, 50).await;
        tab.filter.sidebar_visible_height = 10;
        let mut mode = filter_mode(0);
        mode.count = Some(2);
        let (mode, _) = press(mode, &mut tab, KeyCode::PageDown).await;
        match mode.render_state() {
            ModeRenderState::FilterManagement { selected_index, .. } => {
                assert_eq!(selected_index, 20)
            }
            other => panic!("expected FilterManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_bare_g_jumps_to_first_filter() {
        let mut tab = make_tab(&filters(10)).await;
        add_n_filters(&mut tab, 10).await;
        let (mode, result) = press(filter_mode(5), &mut tab, KeyCode::Char('g')).await;
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
        match mode.render_state() {
            ModeRenderState::FilterManagement { selected_index, .. } => {
                assert_eq!(selected_index, 0)
            }
            other => panic!("expected FilterManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_bare_shift_g_jumps_to_last_filter() {
        let mut tab = make_tab(&filters(10)).await;
        add_n_filters(&mut tab, 10).await;
        let (mode, _) = press(filter_mode(0), &mut tab, KeyCode::Char('G')).await;
        match mode.render_state() {
            ModeRenderState::FilterManagement { selected_index, .. } => {
                assert_eq!(selected_index, 9)
            }
            other => panic!("expected FilterManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_count_shift_g_jumps_to_index_count_minus_1() {
        let mut tab = make_tab(&filters(20)).await;
        add_n_filters(&mut tab, 20).await;
        let mut mode = filter_mode(0);
        mode.count = Some(5);
        let (mode, _) = press(mode, &mut tab, KeyCode::Char('G')).await;
        match mode.render_state() {
            ModeRenderState::FilterManagement { selected_index, .. } => {
                assert_eq!(selected_index, 4)
            }
            other => panic!("expected FilterManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_count_gg_jumps_to_index_count_minus_1() {
        let mut tab = make_tab(&filters(20)).await;
        add_n_filters(&mut tab, 20).await;
        let mut mode = filter_mode(15);
        mode.count = Some(5);
        let (mode, _) = press(mode, &mut tab, KeyCode::Char('g')).await;
        let (mode, _) = mode
            .handle_key(&mut tab, KeyCode::Char('g'), KeyModifiers::NONE)
            .await;
        match mode.render_state() {
            ModeRenderState::FilterManagement { selected_index, .. } => {
                assert_eq!(selected_index, 4)
            }
            other => panic!("expected FilterManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_g_then_recognized_key_cancels_chord() {
        let mut tab = make_tab(&filters(10)).await;
        add_n_filters(&mut tab, 10).await;
        let (mode, _) = press(filter_mode(0), &mut tab, KeyCode::Char('g')).await;
        assert!(tab.interaction.g_key_pressed);
        let (mode, _) = mode
            .handle_key(&mut tab, KeyCode::Char('j'), KeyModifiers::NONE)
            .await;
        assert!(
            !tab.interaction.g_key_pressed,
            "a non-'g' key should cancel an armed chord"
        );
        // Regular 'j' single-step move, not a 'gg' jump to top.
        match mode.render_state() {
            ModeRenderState::FilterManagement { selected_index, .. } => {
                assert_eq!(selected_index, 1)
            }
            other => panic!("expected FilterManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_g_then_unrecognized_key_cancels_chord() {
        let mut tab = make_tab(&filters(10)).await;
        add_n_filters(&mut tab, 10).await;
        let (mode, _) = press(filter_mode(0), &mut tab, KeyCode::Char('g')).await;
        assert!(tab.interaction.g_key_pressed);
        let _ = mode
            .handle_key(&mut tab, KeyCode::F(5), KeyModifiers::NONE)
            .await;
        assert!(
            !tab.interaction.g_key_pressed,
            "an unrecognized key falling through to the default branch should cancel an armed chord"
        );
    }

    #[tokio::test]
    async fn test_exit_mode_resets_g_key_pressed() {
        let mut tab = make_tab(&filters(10)).await;
        add_n_filters(&mut tab, 10).await;
        let (mode, _) = press(filter_mode(0), &mut tab, KeyCode::Char('g')).await;
        assert!(tab.interaction.g_key_pressed);
        let _ = mode
            .handle_key(&mut tab, KeyCode::Esc, KeyModifiers::NONE)
            .await;
        assert!(
            !tab.interaction.g_key_pressed,
            "exiting filter mode must not leave an armed chord for normal mode to inherit"
        );
    }

    fn extract_search_state(state: ModeRenderState) -> (usize, String) {
        match state {
            ModeRenderState::FilterManagement {
                selected_index,
                search,
                ..
            } => (selected_index, search),
            other => panic!("expected FilterManagement, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_slash_enters_search_mode_with_empty_query() {
        let mut tab = make_tab(&filters(5)).await;
        add_n_filters(&mut tab, 5).await;
        let (mode, result) = press(filter_mode(0), &mut tab, KeyCode::Char('/')).await;
        assert!(matches!(result, KeyResult::Handled));
        let (_, search) = extract_search_state(mode.render_state());
        assert_eq!(search, "");
    }

    #[tokio::test]
    async fn test_typing_while_searching_appends_to_query() {
        let mut tab = make_tab(&["a"]).await;
        add_filter(&mut tab, "errno", FilterType::Include).await;
        add_filter(&mut tab, "timeout", FilterType::Exclude).await;
        let (mode, _) = press(filter_mode(0), &mut tab, KeyCode::Char('/')).await;
        let (mode, _) = mode
            .handle_key(&mut tab, KeyCode::Char('e'), KeyModifiers::NONE)
            .await;
        let (mode, _) = mode
            .handle_key(&mut tab, KeyCode::Char('r'), KeyModifiers::NONE)
            .await;
        let (_, search) = extract_search_state(mode.render_state());
        assert_eq!(search, "er");
    }

    #[tokio::test]
    async fn test_action_letter_goes_to_query_while_searching_not_triggered() {
        let mut tab = make_tab(&["a"]).await;
        add_filter(&mut tab, "errno", FilterType::Include).await;
        let (mode, _) = press(filter_mode(0), &mut tab, KeyCode::Char('/')).await;
        // 'e' is normally bound to edit_filter — while searching it must be
        // captured as query text instead, not open the edit command bar.
        let (mode, _) = mode
            .handle_key(&mut tab, KeyCode::Char('e'), KeyModifiers::NONE)
            .await;
        assert!(matches!(
            mode.render_state(),
            ModeRenderState::FilterManagement { .. }
        ));
        let (_, search) = extract_search_state(mode.render_state());
        assert_eq!(search, "e");
    }

    #[tokio::test]
    async fn test_backspace_removes_last_search_char() {
        let mut tab = make_tab(&["a"]).await;
        add_filter(&mut tab, "errno", FilterType::Include).await;
        let (mode, _) = press(filter_mode(0), &mut tab, KeyCode::Char('/')).await;
        let (mode, _) = mode
            .handle_key(&mut tab, KeyCode::Char('a'), KeyModifiers::NONE)
            .await;
        let (mode, _) = mode
            .handle_key(&mut tab, KeyCode::Char('b'), KeyModifiers::NONE)
            .await;
        let (mode, _) = mode
            .handle_key(&mut tab, KeyCode::Backspace, KeyModifiers::NONE)
            .await;
        let (_, search) = extract_search_state(mode.render_state());
        assert_eq!(search, "a");
    }

    #[tokio::test]
    async fn test_search_selection_resets_to_zero_when_narrowed_out_of_range() {
        let mut tab = make_tab(&["a"]).await;
        add_filter(&mut tab, "aaa", FilterType::Include).await;
        add_filter(&mut tab, "bbb", FilterType::Include).await;
        add_filter(&mut tab, "abc", FilterType::Include).await;
        // Start selected on the narrowed-list's last match (index 2 of 3 matches for "a").
        let mut mode = filter_mode(2);
        mode.searching = true;
        mode.pre_search_selected = Some(2);
        let (mode, _) = press(mode, &mut tab, KeyCode::Char('a')).await;
        // Query "a" still matches all 3 filters (aaa, bbb has no 'a'... wait bbb doesn't match).
        let (selected, search) = extract_search_state(mode.render_state());
        assert_eq!(search, "a");
        assert!(
            selected < 2,
            "selection must stay within the narrowed range"
        );
    }

    #[tokio::test]
    async fn test_enter_confirms_search_and_translates_to_full_list_index() {
        let mut tab = make_tab(&["a"]).await;
        add_filter(&mut tab, "aaa", FilterType::Include).await; // index 0
        add_filter(&mut tab, "bbb", FilterType::Include).await; // index 1, no 'z'
        add_filter(&mut tab, "zzz", FilterType::Include).await; // index 2, matches "z"
        let mut mode = filter_mode(0);
        mode.searching = true;
        mode.search = "z".to_string();
        // Only "zzz" (full-list index 2) matches, so narrowed index 0 == full index 2.
        let (mode, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Enter, KeyModifiers::NONE)
            .await;
        let (selected, search) = extract_search_state(mode.render_state());
        assert_eq!(selected, 2);
        assert_eq!(search, "", "confirming search must un-narrow the sidebar");
    }

    #[tokio::test]
    async fn test_esc_cancels_search_and_restores_original_selection() {
        let mut tab = make_tab(&["a"]).await;
        add_filter(&mut tab, "aaa", FilterType::Include).await;
        add_filter(&mut tab, "bbb", FilterType::Include).await;
        add_filter(&mut tab, "ccc", FilterType::Include).await;
        let mut mode = filter_mode(1);
        mode.searching = true;
        mode.pre_search_selected = Some(1);
        mode.search = "c".to_string();
        let (mode, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Esc, KeyModifiers::NONE)
            .await;
        let (selected, search) = extract_search_state(mode.render_state());
        assert_eq!(selected, 1, "Esc must restore the pre-search selection");
        assert_eq!(search, "");
    }

    #[tokio::test]
    async fn test_j_navigates_within_narrowed_list_while_searching() {
        let mut tab = make_tab(&["a"]).await;
        add_filter(&mut tab, "match1", FilterType::Include).await;
        add_filter(&mut tab, "nope", FilterType::Include).await;
        add_filter(&mut tab, "match2", FilterType::Include).await;
        let mut mode = filter_mode(0);
        mode.searching = true;
        mode.search = "match".to_string();
        let (mode, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Char('j'), KeyModifiers::NONE)
            .await;
        let (selected, search) = extract_search_state(mode.render_state());
        assert_eq!(
            selected, 1,
            "'j' should move to the next match within the narrowed list, not the full list"
        );
        assert_eq!(search, "match", "still searching, list stays narrowed");
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
    async fn test_lowercase_a_opens_command_mode_with_filter_auto_prefill() {
        let mut tab = make_tab(&["a", "b"]).await;
        let (mode, result) = press(filter_mode(0), &mut tab, KeyCode::Char('a')).await;
        assert!(matches!(result, KeyResult::Handled));
        match mode.render_state() {
            ModeRenderState::Command { input, cursor, .. } => {
                assert_eq!(input, "filter --auto ");
                assert_eq!(cursor, input.len());
            }
            other => panic!("expected Command, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_lowercase_h_opens_command_mode_with_highlight_prefill() {
        let mut tab = make_tab(&["a", "b"]).await;
        let (mode, _) = press(filter_mode(0), &mut tab, KeyCode::Char('h')).await;
        match mode.render_state() {
            ModeRenderState::Command { input, .. } => {
                assert_eq!(input, "highlight ");
            }
            other => panic!("expected Command, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_shift_h_does_not_toggle_highlight_mode_in_filter_mode() {
        // The highlight-mode toggle lives in normal mode only (kb.normal.*);
        // filter management mode must leave 'H' unhandled.
        let mut tab = make_tab(&["a", "b"]).await;
        assert!(!tab.filter.highlight_mode);
        press(filter_mode(0), &mut tab, KeyCode::Char('H')).await;
        assert!(
            !tab.filter.highlight_mode,
            "'H' must not toggle highlight mode from within filter management mode"
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
            ModeRenderState::FilterManagement { selected_index, .. } => {
                assert_eq!(selected_index, 0)
            }
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
    async fn test_e_opens_command_mode_preserves_group() {
        let mut tab = make_tab(&["error", "warn"]).await;
        tab.log_manager
            .add_filter_with_color(
                "error".to_string(),
                FilterType::Include,
                FilterOptions::default().group("errors"),
            )
            .await;
        tab.refresh_visible();
        let (mode, _) = press(filter_mode(0), &mut tab, KeyCode::Char('e')).await;
        match mode.render_state() {
            ModeRenderState::Command { input, .. } => {
                assert!(input.contains("--group errors"), "{input}");
            }
            other => panic!("expected Command, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_e_opens_command_mode_with_highlight_pattern() {
        let mut tab = make_tab(&["error", "warn"]).await;
        add_filter(&mut tab, "error", FilterType::Highlight).await;
        let (mode, _) = press(filter_mode(0), &mut tab, KeyCode::Char('e')).await;
        match mode.render_state() {
            ModeRenderState::Command { input, .. } => {
                assert!(
                    input.starts_with("highlight"),
                    "expected highlight prefix, got {input}"
                );
                assert!(input.contains("error"));
            }
            other => panic!("expected Command, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_e_opens_command_mode_preserves_highlight_color() {
        let mut tab = make_tab(&["error", "warn"]).await;
        tab.log_manager
            .add_filter_with_color(
                "error".to_string(),
                FilterType::Highlight,
                FilterOptions::default().fg("red"),
            )
            .await;
        tab.refresh_visible();
        let (mode, _) = press(filter_mode(0), &mut tab, KeyCode::Char('e')).await;
        match mode.render_state() {
            ModeRenderState::Command { input, .. } => {
                assert!(input.starts_with("highlight"), "{input}");
                assert!(input.contains("--fg Red"), "{input}");
            }
            other => panic!("expected Command, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_e_opens_command_mode_preserves_ignore_case() {
        let mut tab = make_tab(&["error", "warn"]).await;
        tab.log_manager
            .add_filter_with_color(
                "ERROR".to_string(),
                FilterType::Include,
                FilterOptions::default().ignore_case(),
            )
            .await;
        tab.refresh_visible();
        let (mode, _) = press(filter_mode(0), &mut tab, KeyCode::Char('e')).await;
        match mode.render_state() {
            ModeRenderState::Command { input, .. } => {
                assert!(input.starts_with("filter"), "{input}");
                assert!(input.contains("--ignore-case"), "{input}");
            }
            other => panic!("expected Command, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_e_opens_command_mode_with_field_filter_pattern() {
        let mut tab = make_tab(&["error", "warn"]).await;
        tab.log_manager
            .add_filter_with_color(
                format!("{}level:error", crate::filters::FIELD_PREFIX),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        tab.refresh_visible();
        let (mode, _) = press(filter_mode(0), &mut tab, KeyCode::Char('e')).await;
        match mode.render_state() {
            ModeRenderState::Command { input, .. } => {
                assert!(input.starts_with("filter"), "{input}");
                assert!(input.contains("--field level=error"), "{input}");
            }
            other => panic!("expected Command, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_e_opens_command_mode_with_compound_field_filter_pattern() {
        let mut tab = make_tab(&["error", "warn"]).await;
        let pattern = crate::filters::encode_field_filter(
            &[
                ("level".to_string(), "INFO".to_string()),
                ("component".to_string(), "Draco".to_string()),
            ],
            Some("Power measuments:"),
        );
        tab.log_manager
            .add_filter_with_color(pattern, FilterType::Include, FilterOptions::default())
            .await;
        tab.refresh_visible();
        let (mode, _) = press(filter_mode(0), &mut tab, KeyCode::Char('e')).await;
        match mode.render_state() {
            ModeRenderState::Command { input, .. } => {
                assert!(input.starts_with("filter"), "{input}");
                assert!(input.contains("--field level=INFO"), "{input}");
                assert!(input.contains("--field component=Draco"), "{input}");
                assert!(input.ends_with("Power measuments:"), "{input}");
            }
            other => panic!("expected Command, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_e_opens_command_mode_quotes_group_with_spaces() {
        let mut tab = make_tab(&["error", "warn"]).await;
        tab.log_manager
            .add_filter_with_color(
                "error".to_string(),
                FilterType::Include,
                FilterOptions::default().group("my errors"),
            )
            .await;
        tab.refresh_visible();
        let (mode, _) = press(filter_mode(0), &mut tab, KeyCode::Char('e')).await;
        match mode.render_state() {
            ModeRenderState::Command { input, .. } => {
                assert!(input.contains("--group \"my errors\""), "{input}");
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

    #[test]
    fn test_mode_bar_content_shows_configured_search_shortcut() {
        let m = filter_mode(0);
        let mut kb = Keybindings::default();
        kb.filter.search = crate::config::KeyBindings(vec![crate::config::KeyBinding(
            KeyCode::Char('?'),
            KeyModifiers::NONE,
        )]);
        let theme = crate::theme::Theme::default();
        let line = m.mode_bar_content(&kb, &theme);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("search"));
        assert!(text.contains('?'));
    }
}
