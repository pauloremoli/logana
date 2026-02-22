use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::style::Color;
use std::collections::HashSet;

use crate::{
    auto_complete::fuzzy_match,
    mode::app_mode::{Mode, ModeRenderState},
    mode::normal_mode::NormalMode,
    ui::{KeyResult, TabState},
};

/// A single toggleable colour category (leaf node).
#[derive(Debug, Clone)]
pub struct ValueColorEntry {
    /// Internal key (e.g. "http_get", "status_2xx").
    pub key: String,
    /// Human-readable display name.
    pub label: String,
    /// The colour associated with this category.
    pub color: Color,
    /// Whether this category is currently enabled.
    pub enabled: bool,
}

/// A group header that contains child entries.
#[derive(Debug, Clone)]
pub struct ValueColorGroup {
    pub label: String,
    pub children: Vec<ValueColorEntry>,
}

/// Flat row used for rendering and navigation.
#[derive(Debug, Clone)]
pub enum ValueColorRow {
    Group(usize),
    Entry(usize, usize),
}

#[derive(Debug)]
pub struct ValueColorsMode {
    pub groups: Vec<ValueColorGroup>,
    pub search: String,
    /// Index into the *visible* (filtered) row list.
    pub selected: usize,
    /// Snapshot of disabled keys on entry — restored on Esc cancel.
    original_disabled: HashSet<String>,
}

impl ValueColorsMode {
    pub fn new(groups: Vec<ValueColorGroup>, original_disabled: HashSet<String>) -> Self {
        ValueColorsMode {
            groups,
            search: String::new(),
            selected: 0,
            original_disabled,
        }
    }

    /// Build the flat list of visible rows, applying fuzzy search.
    pub fn visible_rows(&self) -> Vec<ValueColorRow> {
        let mut rows = Vec::new();
        for (gi, group) in self.groups.iter().enumerate() {
            if self.search.is_empty() {
                rows.push(ValueColorRow::Group(gi));
                for (ei, _) in group.children.iter().enumerate() {
                    rows.push(ValueColorRow::Entry(gi, ei));
                }
            } else {
                // Collect children that match the search.
                let matching: Vec<usize> = group
                    .children
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| {
                        let haystack = format!("{} {}", group.label, e.label);
                        fuzzy_match(&self.search, &haystack)
                    })
                    .map(|(i, _)| i)
                    .collect();
                if !matching.is_empty() {
                    rows.push(ValueColorRow::Group(gi));
                    for ei in matching {
                        rows.push(ValueColorRow::Entry(gi, ei));
                    }
                }
            }
        }
        rows
    }

    /// Tri-state: all children enabled → true, none → false, mixed → None.
    pub fn group_enabled(&self, gi: usize) -> Option<bool> {
        let group = &self.groups[gi];
        let all = group.children.iter().all(|e| e.enabled);
        let none = group.children.iter().all(|e| !e.enabled);
        if all {
            Some(true)
        } else if none {
            Some(false)
        } else {
            None
        }
    }

    fn clamp_selected(&mut self) {
        let count = self.visible_rows().len();
        if count == 0 {
            self.selected = 0;
        } else if self.selected >= count {
            self.selected = count - 1;
        }
    }
}

#[async_trait]
impl Mode for ValueColorsMode {
    async fn handle_key(
        mut self: Box<Self>,
        _tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        // Esc: clear search first, then cancel.
        if key == KeyCode::Esc {
            if !self.search.is_empty() {
                self.search.clear();
                self.selected = 0;
                return (self, KeyResult::Handled);
            }
            return (
                Box::new(NormalMode),
                KeyResult::ApplyValueColors(self.original_disabled.clone()),
            );
        }

        if key == KeyCode::Enter {
            let disabled: HashSet<String> = self
                .groups
                .iter()
                .flat_map(|g| g.children.iter())
                .filter(|e| !e.enabled)
                .map(|e| e.key.clone())
                .collect();
            return (Box::new(NormalMode), KeyResult::ApplyValueColors(disabled));
        }

        // When search is empty, j/k/Space/a/n work as navigation/toggle.
        // When search is non-empty, printable chars go to search;
        // j/k/Space still navigate and toggle.
        match key {
            KeyCode::Char('j') | KeyCode::Down => {
                let count = self.visible_rows().len();
                if count > 0 {
                    self.selected = (self.selected + 1).min(count - 1);
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
            }
            KeyCode::Char(' ') => {
                let rows = self.visible_rows();
                if let Some(row) = rows.get(self.selected) {
                    match row {
                        ValueColorRow::Group(gi) => {
                            let gi = *gi;
                            // Toggle all children: if all enabled → disable all, otherwise enable all.
                            let target = !self.group_enabled(gi).unwrap_or(false);
                            for child in &mut self.groups[gi].children {
                                child.enabled = target;
                            }
                        }
                        ValueColorRow::Entry(gi, ei) => {
                            let (gi, ei) = (*gi, *ei);
                            self.groups[gi].children[ei].enabled =
                                !self.groups[gi].children[ei].enabled;
                        }
                    }
                }
            }
            KeyCode::Char('a') if self.search.is_empty() => {
                for group in &mut self.groups {
                    for child in &mut group.children {
                        child.enabled = true;
                    }
                }
            }
            KeyCode::Char('n') if self.search.is_empty() => {
                for group in &mut self.groups {
                    for child in &mut group.children {
                        child.enabled = false;
                    }
                }
            }
            KeyCode::Char(c) if !modifiers.contains(KeyModifiers::CONTROL) => {
                self.search.push(c);
                self.selected = 0;
                self.clamp_selected();
            }
            KeyCode::Backspace => {
                self.search.pop();
                self.selected = 0;
                self.clamp_selected();
            }
            _ => {}
        }
        (self, KeyResult::Handled)
    }

    fn status_line(&self) -> &str {
        "[VALUE COLORS] Space=toggle | a=all | n=none | Enter=apply | Esc=cancel | type to search"
    }

    fn render_state(&self) -> ModeRenderState {
        ModeRenderState::ValueColors {
            groups: self.groups.clone(),
            search: self.search.clone(),
            selected: self.selected,
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
    use crate::ui::TabState;
    use std::sync::Arc;

    async fn make_tab() -> TabState {
        let file_reader = FileReader::from_bytes(b"test line\n".to_vec());
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        TabState::new(file_reader, log_manager, "test".to_string())
    }

    fn sample_groups() -> Vec<ValueColorGroup> {
        vec![
            ValueColorGroup {
                label: "HTTP methods".to_string(),
                children: vec![
                    ValueColorEntry {
                        key: "http_get".to_string(),
                        label: "GET".to_string(),
                        color: Color::Green,
                        enabled: true,
                    },
                    ValueColorEntry {
                        key: "http_post".to_string(),
                        label: "POST".to_string(),
                        color: Color::Cyan,
                        enabled: true,
                    },
                ],
            },
            ValueColorGroup {
                label: "Status codes".to_string(),
                children: vec![
                    ValueColorEntry {
                        key: "status_2xx".to_string(),
                        label: "2xx".to_string(),
                        color: Color::Green,
                        enabled: true,
                    },
                    ValueColorEntry {
                        key: "status_4xx".to_string(),
                        label: "4xx".to_string(),
                        color: Color::Yellow,
                        enabled: false,
                    },
                ],
            },
            ValueColorGroup {
                label: "Identifiers".to_string(),
                children: vec![ValueColorEntry {
                    key: "uuid".to_string(),
                    label: "UUIDs".to_string(),
                    color: Color::Magenta,
                    enabled: true,
                }],
            },
        ]
    }

    async fn press(
        mode: ValueColorsMode,
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

    #[tokio::test]
    async fn test_visible_rows_no_search() {
        let mode = ValueColorsMode::new(sample_groups(), HashSet::new());
        let rows = mode.visible_rows();
        // 3 groups + 2 + 2 + 1 entries = 8 rows
        assert_eq!(rows.len(), 8);
        assert!(matches!(rows[0], ValueColorRow::Group(0)));
        assert!(matches!(rows[1], ValueColorRow::Entry(0, 0)));
        assert!(matches!(rows[3], ValueColorRow::Group(1)));
    }

    #[tokio::test]
    async fn test_visible_rows_with_search() {
        let mut mode = ValueColorsMode::new(sample_groups(), HashSet::new());
        mode.search = "get".to_string();
        let rows = mode.visible_rows();
        // Only "HTTP methods" group with "GET" entry
        assert_eq!(rows.len(), 2);
        assert!(matches!(rows[0], ValueColorRow::Group(0)));
        assert!(matches!(rows[1], ValueColorRow::Entry(0, 0)));
    }

    #[tokio::test]
    async fn test_navigate_down() {
        let mut tab = make_tab().await;
        let mode = ValueColorsMode::new(sample_groups(), HashSet::new());
        let (mode, _) = press(mode, &mut tab, KeyCode::Char('j')).await;
        match mode.render_state() {
            ModeRenderState::ValueColors { selected, .. } => assert_eq!(selected, 1),
            other => panic!("expected ValueColors, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_navigate_up_at_top() {
        let mut tab = make_tab().await;
        let mode = ValueColorsMode::new(sample_groups(), HashSet::new());
        let (mode, _) = press(mode, &mut tab, KeyCode::Char('k')).await;
        match mode.render_state() {
            ModeRenderState::ValueColors { selected, .. } => assert_eq!(selected, 0),
            other => panic!("expected ValueColors, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_toggle_group_disables_all_children() {
        let mut tab = make_tab().await;
        let mode = ValueColorsMode::new(sample_groups(), HashSet::new());
        // selected=0 is "HTTP methods" group, both children enabled → toggle disables
        let (mode, _) = press(mode, &mut tab, KeyCode::Char(' ')).await;
        match mode.render_state() {
            ModeRenderState::ValueColors { groups, .. } => {
                assert!(groups[0].children.iter().all(|c| !c.enabled));
            }
            other => panic!("expected ValueColors, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_toggle_group_enables_when_mixed() {
        let mut tab = make_tab().await;
        let groups = sample_groups(); // Status codes group has mixed (2xx=on, 4xx=off)
        let mode = ValueColorsMode::new(groups, HashSet::new());
        // Navigate to "Status codes" group (row index 3)
        let (mode, _) = press(mode, &mut tab, KeyCode::Char('j')).await;
        let (mode, _) = press_dyn(mode, &mut tab, KeyCode::Char('j')).await;
        let (mode, _) = press_dyn(mode, &mut tab, KeyCode::Char('j')).await;
        // Now at row 3 = Group(1) = Status codes
        let (mode, _) = press_dyn(mode, &mut tab, KeyCode::Char(' ')).await;
        match mode.render_state() {
            ModeRenderState::ValueColors { groups, .. } => {
                // Mixed → should enable all
                assert!(groups[1].children.iter().all(|c| c.enabled));
            }
            other => panic!("expected ValueColors, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_toggle_entry() {
        let mut tab = make_tab().await;
        let mode = ValueColorsMode::new(sample_groups(), HashSet::new());
        // Navigate to row 1 = Entry(0, 0) = GET
        let (mode, _) = press(mode, &mut tab, KeyCode::Char('j')).await;
        let (mode, _) = press_dyn(mode, &mut tab, KeyCode::Char(' ')).await;
        match mode.render_state() {
            ModeRenderState::ValueColors { groups, .. } => {
                assert!(!groups[0].children[0].enabled);
            }
            other => panic!("expected ValueColors, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_enable_all() {
        let mut tab = make_tab().await;
        let mode = ValueColorsMode::new(sample_groups(), HashSet::new());
        let (mode, _) = press(mode, &mut tab, KeyCode::Char('a')).await;
        match mode.render_state() {
            ModeRenderState::ValueColors { groups, .. } => {
                assert!(groups.iter().all(|g| g.children.iter().all(|c| c.enabled)));
            }
            other => panic!("expected ValueColors, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_disable_all() {
        let mut tab = make_tab().await;
        let mode = ValueColorsMode::new(sample_groups(), HashSet::new());
        let (mode, _) = press(mode, &mut tab, KeyCode::Char('n')).await;
        match mode.render_state() {
            ModeRenderState::ValueColors { groups, .. } => {
                assert!(groups.iter().all(|g| g.children.iter().all(|c| !c.enabled)));
            }
            other => panic!("expected ValueColors, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_enter_collects_disabled() {
        let mut tab = make_tab().await;
        let mode = ValueColorsMode::new(sample_groups(), HashSet::new());
        let (_, result) = press(mode, &mut tab, KeyCode::Enter).await;
        match result {
            KeyResult::ApplyValueColors(disabled) => {
                // Only status_4xx was disabled in sample_groups
                assert!(disabled.contains("status_4xx"));
                assert!(!disabled.contains("http_get"));
                assert!(!disabled.contains("uuid"));
            }
            _ => panic!("expected ApplyValueColors"),
        }
    }

    #[tokio::test]
    async fn test_esc_with_search_clears_search() {
        let mut tab = make_tab().await;
        let mut mode = ValueColorsMode::new(sample_groups(), HashSet::new());
        mode.search = "http".to_string();
        let (mode, result) = press(mode, &mut tab, KeyCode::Esc).await;
        assert!(matches!(result, KeyResult::Handled));
        match mode.render_state() {
            ModeRenderState::ValueColors { search, .. } => assert!(search.is_empty()),
            other => panic!("expected ValueColors, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_esc_without_search_restores_original() {
        let mut tab = make_tab().await;
        let mut original = HashSet::new();
        original.insert("status_5xx".to_string());
        let mode = ValueColorsMode::new(sample_groups(), original.clone());
        let (_, result) = press(mode, &mut tab, KeyCode::Esc).await;
        match result {
            KeyResult::ApplyValueColors(disabled) => {
                assert_eq!(disabled, original);
            }
            _ => panic!("expected ApplyValueColors"),
        }
    }

    #[tokio::test]
    async fn test_typing_activates_search() {
        let mut tab = make_tab().await;
        let mode = ValueColorsMode::new(sample_groups(), HashSet::new());
        // Type 'g' — should go into search
        let (mode, _) = press(mode, &mut tab, KeyCode::Char('g')).await;
        match mode.render_state() {
            ModeRenderState::ValueColors { search, .. } => assert_eq!(search, "g"),
            other => panic!("expected ValueColors, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_backspace_removes_search_char() {
        let mut tab = make_tab().await;
        let mut mode = ValueColorsMode::new(sample_groups(), HashSet::new());
        mode.search = "ge".to_string();
        let (mode, _) = press(mode, &mut tab, KeyCode::Backspace).await;
        match mode.render_state() {
            ModeRenderState::ValueColors { search, .. } => assert_eq!(search, "g"),
            other => panic!("expected ValueColors, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_group_enabled_all() {
        let mode = ValueColorsMode::new(sample_groups(), HashSet::new());
        assert_eq!(mode.group_enabled(0), Some(true)); // HTTP: all enabled
    }

    #[tokio::test]
    async fn test_group_enabled_mixed() {
        let mode = ValueColorsMode::new(sample_groups(), HashSet::new());
        assert_eq!(mode.group_enabled(1), None); // Status: mixed
    }

    #[tokio::test]
    async fn test_group_enabled_none() {
        let mut groups = sample_groups();
        for child in &mut groups[0].children {
            child.enabled = false;
        }
        let mode = ValueColorsMode::new(groups, HashSet::new());
        assert_eq!(mode.group_enabled(0), Some(false));
    }

    #[tokio::test]
    async fn test_status_line() {
        let mode = ValueColorsMode::new(sample_groups(), HashSet::new());
        assert!(mode.status_line().contains("[VALUE COLORS]"));
    }

    #[tokio::test]
    async fn test_search_filters_to_matching_groups() {
        let mut mode = ValueColorsMode::new(sample_groups(), HashSet::new());
        mode.search = "uuid".to_string();
        let rows = mode.visible_rows();
        // Only "Identifiers" group + "UUIDs" entry
        assert_eq!(rows.len(), 2);
        assert!(matches!(rows[0], ValueColorRow::Group(2)));
        assert!(matches!(rows[1], ValueColorRow::Entry(2, 0)));
    }
}
