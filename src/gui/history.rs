//! Undo reversible edits, scoped to stable tab identities.
use crate::{
    db::{Comment, LogManager},
    shared::{DisplayConfig, TabId, TabState},
};
#[derive(Clone)]
pub struct Snapshot {
    pub id: TabId,
    manager: LogManager,
    display: DisplayConfig,
    marks: Vec<usize>,
    notes: Vec<Comment>,
    enabled: bool,
    marks_only: bool,
}
impl Snapshot {
    pub fn new(tab: &TabState) -> Self {
        Self {
            id: tab.id,
            manager: tab.log_manager.clone(),
            display: tab.display.clone(),
            marks: tab.mark_manager.get_indices(),
            notes: tab.comment_manager.get().to_vec(),
            enabled: tab.filter.enabled,
            marks_only: tab.filter.show_marks_only,
        }
    }
    pub fn changed(&self, tab: &TabState) -> bool {
        self.manager.get_filters() != tab.log_manager.get_filters()
            || self.manager.get_group_styles() != tab.log_manager.get_group_styles()
            || self.marks != tab.mark_manager.get_indices()
            || self.notes != tab.comment_manager.get()
            || self.enabled != tab.filter.enabled
            || self.marks_only != tab.filter.show_marks_only
            || self.display.hidden_fields != tab.display.hidden_fields
            || self.display.field_layout.columns != tab.display.field_layout.columns
            || self.display.wrap != tab.display.wrap
            || self.display.raw_mode != tab.display.raw_mode
            || self.display.show_keys != tab.display.show_keys
            || self.display.show_sidebar != tab.display.show_sidebar
            || self.display.sidebar_side != tab.display.sidebar_side
            || self.display.show_line_numbers != tab.display.show_line_numbers
            || self.display.relative_line_numbers != tab.display.relative_line_numbers
            || self.display.collapse_continuations != tab.display.collapse_continuations
            || self.display.level_colors_disabled != tab.display.level_colors_disabled
    }
    pub fn restore(self, tab: &mut TabState) -> crate::gui::effect::FilterEdit {
        tab.log_manager = self.manager;
        tab.display = self.display;
        tab.mark_manager.set(self.marks);
        tab.comment_manager.set(self.notes);
        tab.filter.enabled = self.enabled;
        tab.filter.show_marks_only = self.marks_only;
        tab.filter.saved_view = None;
        tab.filter.cached_scan = None;
        tab.invalidate_parse_cache();
        tab.begin_filter_refresh();
        crate::gui::effect::FilterEdit::Restore {
            manager: tab.log_manager.clone(),
        }
    }
}
#[derive(Default)]
pub struct History {
    pub undo: Vec<Snapshot>,
    pub redo: Vec<Snapshot>,
}
impl History {
    pub fn record(&mut self, snapshot: Snapshot) {
        self.undo.push(snapshot);
        self.redo.clear();
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
    }
}
pub fn apply(state: &mut crate::gui::state::GuiState, redo: bool) -> crate::gui::effect::Effect {
    let Some(id) = state.active_tab().map(|tab| tab.id) else {
        return crate::gui::effect::Effect::None;
    };
    let history = state.history.entry(id).or_default();
    let snapshot = if redo {
        history.redo.pop()
    } else {
        history.undo.pop()
    };
    let Some(snapshot) = snapshot else {
        return crate::gui::effect::Effect::None;
    };
    let tab = state.tabs.iter_mut().find(|tab| tab.id == id).unwrap();
    let current = Snapshot::new(tab);
    if redo {
        history.undo.push(current);
    } else {
        history.redo.push(current);
    }
    crate::gui::effect::Effect::PersistFilterEdit(snapshot.restore(tab))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn undo_redo_restore_edits_and_clear_redo_after_a_new_edit() {
        let db = std::sync::Arc::new(crate::db::Database::in_memory().await.unwrap());
        let manager = LogManager::new(db.clone(), None).await;
        let tab = TabState::new(
            crate::ingestion::FileReader::from_bytes(b"line\n".to_vec()),
            manager,
            "test".into(),
        );
        let mut state = crate::gui::state::GuiState::new(db);
        state.tabs.push(tab);
        let original = state.tabs[0].display.show_sidebar;
        crate::gui::update::update(&mut state, crate::gui::message::Message::SidebarToggled);
        assert_ne!(state.tabs[0].display.show_sidebar, original);
        apply(&mut state, false);
        assert_eq!(state.tabs[0].display.show_sidebar, original);
        apply(&mut state, true);
        assert_ne!(state.tabs[0].display.show_sidebar, original);
        apply(&mut state, false);
        crate::gui::update::update(&mut state, crate::gui::message::Message::SidebarSideToggled);
        assert!(state.history[&state.tabs[0].id].redo.is_empty());
    }
}
