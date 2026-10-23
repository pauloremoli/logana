use crate::gui::app::App;
use crate::shared::{FieldValueCounts, TabId};
use gpui_kit::gpui::{Context, Task};
use std::sync::Arc;

pub(super) struct Query {
    source: TabId,
    counts: Arc<FieldValueCounts>,
    text: String,
    result: Arc<FieldValueCounts>,
    task: Option<Task<()>>,
}

fn search(counts: &FieldValueCounts, query: &str) -> FieldValueCounts {
    let mut result = FieldValueCounts::default();
    for name in &counts.names {
        let Some(values) = counts.counts.get(name) else {
            continue;
        };
        let field_matches = name.to_lowercase().contains(query);
        let values: Vec<_> = values
            .iter()
            .filter(|(value, _)| field_matches || value.to_lowercase().contains(query))
            .cloned()
            .collect();
        if !values.is_empty() {
            result.names.push(name.clone());
            result.counts.insert(name.clone(), values);
        }
    }
    result
}

impl App {
    pub(super) fn sync_facet_query(&mut self, cx: &mut Context<Self>) {
        let text = self
            .facet_search
            .as_ref()
            .map(|input| input.read(cx).value().to_lowercase())
            .unwrap_or_default();
        let Some(tab) = self
            .state
            .active_tab()
            .filter(|_| self.state.facet_browser_open && !text.is_empty())
        else {
            self.facet_query = None;
            return;
        };
        let source = tab.id;
        let Some(counts) = self
            .facet_counts
            .get(&source)
            .and_then(|cache| cache.counts.as_ref())
            .cloned()
        else {
            return;
        };
        if self.facet_query.as_ref().is_some_and(|query| {
            query.source == source && query.text == text && Arc::ptr_eq(&query.counts, &counts)
        }) {
            return;
        }
        let result = self
            .facet_query
            .take()
            .filter(|query| query.source == source && Arc::ptr_eq(&query.counts, &counts))
            .map(|query| query.result)
            .unwrap_or_else(|| counts.clone());
        let captured = counts.clone();
        let pattern = text.clone();
        let worker = cx
            .background_executor()
            .spawn(async move { Arc::new(search(&captured, &pattern)) });
        let expected = counts.clone();
        self.facet_query = Some(Query {
            source,
            counts,
            text: text.clone(),
            result,
            task: None,
        });
        let task = cx.spawn(async move |this, cx| {
            let result = worker.await;
            this.update(cx, |app, cx| {
                if let Some(query) = &mut app.facet_query
                    && query.source == source
                    && query.text == text
                    && Arc::ptr_eq(&query.counts, &expected)
                {
                    query.result = result;
                    query.task = None;
                    cx.notify();
                }
            })
            .ok();
        });
        self.facet_query.as_mut().unwrap().task = Some(task);
    }

    pub(super) fn facet_display_counts(&self) -> Option<(&FieldValueCounts, bool)> {
        let tab = self.state.active_tab()?;
        let counts = self.facet_counts.get(&tab.id)?.counts.as_ref()?;
        if let Some(query) = &self.facet_query
            && query.source == tab.id
            && Arc::ptr_eq(&query.counts, counts)
        {
            return Some((&query.result, query.task.is_some()));
        }
        Some((counts, false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn searches_field_names_and_values_without_changing_order_or_counts() {
        let counts = FieldValueCounts {
            names: vec!["key1".into(), "key2".into()],
            counts: std::collections::HashMap::from([
                (
                    "key1".into(),
                    vec![("Value1".into(), 7), ("value2".into(), 3)],
                ),
                ("key2".into(), vec![("value3".into(), 1)]),
            ]),
        };
        assert_eq!(search(&counts, "key").names, counts.names);
        assert_eq!(
            search(&counts, "value1").counts["key1"],
            vec![("Value1".into(), 7)]
        );
        assert!(search(&counts, "absent").names.is_empty());
        assert_eq!(counts.counts["key1"].len(), 2);
    }
}
