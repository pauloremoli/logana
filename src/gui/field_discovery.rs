use crate::gui::{actions::Action, app::App};
use crate::shared::{TabId, TabState};
use gpui_kit::component::select::SearchableVec;
use gpui_kit::gpui::Context;
use std::sync::Arc;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Key {
    epoch: u64,
    source: TabId,
    generation: u64,
    count: usize,
}

impl App {
    pub(super) fn sync_field_discovery(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self
            .state
            .editor
            .as_ref()
            .filter(|e| e.action == Action::Filter)
        else {
            self.field_discovery_key = None;
            self.field_discovery_task = None;
            return;
        };
        let Some(tab) = self
            .state
            .tabs
            .iter()
            .find(|tab| Some(tab.id) == editor.source)
        else {
            return;
        };
        let key = Key {
            epoch: editor.epoch,
            source: tab.id,
            generation: tab.cache.parse_gen,
            count: tab.file_reader.line_count(),
        };
        if self.field_discovery_key.as_ref() == Some(&key) {
            return;
        }
        self.field_discovery_key = Some(key.clone());
        self.field_discovery_task = None;
        if self
            .state
            .field_discovery
            .as_ref()
            .is_some_and(|(id, generation, count, _)| {
                (*id, *generation, *count) == (key.source, key.generation, key.count)
            })
        {
            return;
        }
        let reader = tab.file_reader.clone();
        let parser = tab.display.format.clone();
        let worker = cx.background_executor().spawn(async move {
            Arc::new(TabState::build_field_index_for(&reader, parser.as_deref()))
        });
        self.field_discovery_task = Some(cx.spawn(async move |this, cx| {
            let fields = worker.await;
            this.update_in(cx, |app, window, cx| {
                if app.field_discovery_key.as_ref() != Some(&key)
                    || !app.state.tabs.iter().any(|tab| {
                        tab.id == key.source
                            && tab.cache.parse_gen == key.generation
                            && tab.file_reader.line_count() == key.count
                    })
                {
                    return;
                }
                let Some(editor) = app.state.editor.as_mut().filter(|e| {
                    e.action == Action::Filter
                        && e.epoch == key.epoch
                        && e.source == Some(key.source)
                }) else {
                    return;
                };
                app.field_discovery_task = None;
                editor.discovery = fields.clone();
                let mut names = fields.names.clone();
                names.extend(editor.conditions.iter().map(|(field, _)| field.clone()));
                let selected = editor.values[1].clone();
                if !selected.is_empty() {
                    names.push(selected.clone());
                }
                names.sort();
                names.dedup();
                app.state.field_discovery = Some((key.source, key.generation, key.count, fields));
                if let Some(select) = &app.editor_field_select {
                    select.update(cx, |select, cx| {
                        select.set_items(SearchableVec::new(names), window, cx);
                        select.set_selected_value(&selected, window, cx);
                        cx.notify();
                    });
                }
                cx.notify();
            })
            .ok();
        }));
    }
}
