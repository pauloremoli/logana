use crate::gui::{actions::Action, app::App, column_widths::Snapshot};
use crate::shared::TabId;
use gpui_kit::gpui::Context;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Key {
    source: TabId,
    generation: u64,
    count: usize,
    show_keys: bool,
}

impl Key {
    fn matches(&self, tab: &crate::shared::TabState) -> bool {
        self.source == tab.id
            && self.generation == tab.cache.parse_gen
            && self.count == tab.file_reader.line_count()
            && self.show_keys == tab.display.show_keys
    }
}

impl App {
    pub(super) fn sync_table_preparation(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.state.active_tab() else {
            self.table_preparation_key = None;
            self.table_preparation_task = None;
            return;
        };
        let key = Key {
            source: tab.id,
            generation: tab.cache.parse_gen,
            count: tab.file_reader.line_count(),
            show_keys: tab.display.show_keys,
        };
        if self.table_preparation_key.as_ref() == Some(&key) {
            return;
        }
        let snapshot = Snapshot::capture(tab);
        let fixed = self.state.column_widths;
        self.table_preparation_key = Some(key.clone());
        self.table_preparation_task = None;
        let source = key.source;
        let generation = key.generation;
        let worker = cx
            .background_executor()
            .spawn(async move { snapshot.prepare(source, generation) });
        self.table_preparation_task = Some(cx.spawn(async move |this, cx| {
            let (names, widths, fitted) = worker.await;
            this.update(cx, |app, cx| {
                if app.table_preparation_key.as_ref() != Some(&key)
                    || !app.state.active_tab().is_some_and(|tab| key.matches(tab))
                {
                    return;
                }
                let tab = &mut app.state.tabs[app.state.active_tab];
                tab.cache.field_names = Some((key.generation, names.clone()));
                if let Some(editor) = app
                    .state
                    .editor
                    .as_mut()
                    .filter(|e| e.action == Action::Columns && e.source == Some(key.source))
                {
                    let mut choices = names;
                    if tab.display.format.is_none() && !choices.iter().any(|name| name == "message")
                    {
                        choices.push("message".into());
                    }
                    editor.defaults = choices.iter().map(|name| (name.clone(), true)).collect();
                    for name in choices {
                        if !editor.fields.iter().any(|(field, _)| field == &name) {
                            let enabled = !tab.display.hidden_fields.contains(&name);
                            editor.fields.push((name, enabled));
                        }
                    }
                }
                app.field_widths = Some(widths);
                if app.state.column_widths == fixed {
                    app.state.column_widths = fitted;
                }
                app.table_preparation_task = None;
                cx.notify();
            })
            .ok();
        }));
    }
}
