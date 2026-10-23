use crate::gui::{actions::Action, app::App, preview};
use crate::shared::TabId;
use gpui_kit::gpui::Context;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Key {
    epoch: u64,
    source: TabId,
    schema: String,
}

pub struct Sample {
    pub examined: usize,
    pub matched: usize,
    pub skipped: usize,
    pub raw: String,
    pub fields: Vec<(String, String)>,
    pub unstructured: bool,
}

fn sample(name: &str, rows: Vec<Vec<u8>>, skipped: usize) -> Result<Sample, String> {
    let parser = if name == "none" {
        None
    } else {
        Some(crate::parser::resolve_parser(name)?)
    };
    let mut result = Sample {
        examined: rows.len(),
        matched: 0,
        skipped,
        raw: String::new(),
        fields: Vec::new(),
        unstructured: parser.is_none(),
    };
    for row in rows {
        if result.raw.is_empty() {
            result.raw = preview::text(&String::from_utf8_lossy(&row));
        }
        if let Some(parser) = &parser {
            let fields = preview::fields(parser.as_ref(), &row);
            if !fields.is_empty() {
                result.matched += 1;
                if result.fields.is_empty() {
                    result.fields = fields;
                }
            }
        }
    }
    Ok(result)
}

impl App {
    pub(super) fn sync_schema_preview(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self
            .state
            .editor
            .as_ref()
            .filter(|editor| editor.action == Action::Schema)
        else {
            self.schema_preview_key = None;
            self.schema_preview_task = None;
            self.schema_preview = None;
            return;
        };
        let Some(source) = editor.source else {
            return;
        };
        let key = Key {
            epoch: editor.epoch,
            source,
            schema: editor.values[0].clone(),
        };
        if self.schema_preview_key.as_ref() == Some(&key) {
            return;
        }
        self.schema_preview_key = Some(key.clone());
        self.schema_preview_task = None;
        self.schema_preview = None;
        let Some(tab) = self.state.tabs.iter().find(|tab| tab.id == source) else {
            return;
        };
        let reader = tab.file_reader.clone();
        let name = key.schema.clone();
        let worker = cx.background_executor().spawn(async move {
            let mut skipped = 0;
            let rows = (0..reader.line_count().min(8))
                .filter_map(|i| {
                    let (bytes, truncated) = reader.get_raw_line_prefix(i, preview::MAX_LINE_BYTES);
                    if truncated {
                        skipped += 1;
                        None
                    } else {
                        Some(bytes.to_vec())
                    }
                })
                .collect();
            sample(&name, rows, skipped)
        });
        self.schema_preview_task = Some(cx.spawn(async move |this, cx| {
            let result = worker.await;
            this.update(cx, |app, cx| {
                if app.schema_preview_key.as_ref() == Some(&key)
                    && app.state.editor.as_ref().is_some_and(|editor| {
                        editor.action == Action::Schema
                            && editor.epoch == key.epoch
                            && editor.source == Some(key.source)
                            && editor.values[0] == key.schema
                    })
                {
                    app.schema_preview = Some(result);
                    cx.notify();
                }
            })
            .ok();
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schema_samples_report_matches_raw_mode_and_invalid_choices() {
        let row = br#"{"key1":"value1","message":"value2"}"#.to_vec();
        let name = crate::parser::detect_format(&[&row])
            .unwrap()
            .name()
            .to_string();
        let parsed = sample(&name, vec![row.clone(), b"plain".to_vec()], 1).unwrap();
        assert_eq!((parsed.examined, parsed.matched, parsed.skipped), (2, 1, 1));
        assert!(
            parsed
                .fields
                .iter()
                .any(|(name, value)| name == "key1" && value == "value1")
        );
        assert!(sample("none", vec![row], 0).unwrap().unstructured);
        assert!(sample("unknown-schema", vec![], 0).is_err());
        assert_eq!(sample("none", vec![], 0).unwrap().examined, 0);
    }
}
