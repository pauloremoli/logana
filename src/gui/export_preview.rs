use crate::{
    db::Comment,
    gui::{actions::Action, app::App, preview},
    ingestion::FileReader,
    parser::LogFormatParser,
    shared::{FieldLayout, TabId},
};
use gpui_kit::gpui::Context;
use std::{collections::HashSet, sync::Arc};

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Key {
    epoch: u64,
    source: TabId,
    values: Vec<String>,
}

pub struct Sample {
    pub output: String,
    pub notes: usize,
    pub bookmarks: usize,
    pub skipped: usize,
}

struct Snapshot {
    template: String,
    filename: String,
    comments: Vec<Comment>,
    marks: Vec<usize>,
    reader: FileReader,
    parser: Option<Arc<dyn LogFormatParser>>,
    layout: FieldLayout,
    hidden: HashSet<String>,
    show_keys: bool,
    fields: Vec<(String, String)>,
}

fn render(mut snapshot: Snapshot) -> Result<Sample, String> {
    let template = crate::commands::load_template(&snapshot.template)?;
    let mut skipped = 0;
    let mut keep = |index: &usize| {
        let valid = *index < snapshot.reader.line_count()
            && !snapshot
                .reader
                .get_raw_line_prefix(*index, preview::MAX_LINE_BYTES)
                .1;
        if !valid {
            skipped += 1;
        }
        valid
    };
    snapshot.marks.retain(&mut keep);
    for comment in &mut snapshot.comments {
        comment.line_indices.retain(&mut keep);
    }
    snapshot
        .comments
        .retain(|comment| !comment.line_indices.is_empty());
    let fields: Vec<_> = snapshot
        .fields
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    let output = crate::commands::render_export(
        &template,
        &crate::commands::ExportData {
            filename: &snapshot.filename,
            comments: &snapshot.comments,
            marked_indices: snapshot.marks.clone(),
            file_reader: &snapshot.reader,
            parser: snapshot.parser.as_deref(),
            field_layout: &snapshot.layout,
            hidden_fields: &snapshot.hidden,
            show_keys: snapshot.show_keys,
            footer_fields: &fields,
        },
    );
    Ok(Sample {
        output: preview::text(&output),
        notes: snapshot.comments.len(),
        bookmarks: snapshot.marks.len(),
        skipped,
    })
}

impl App {
    pub(super) fn sync_export_preview(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self
            .state
            .editor
            .as_ref()
            .filter(|editor| editor.action == Action::Export)
        else {
            self.export_preview_key = None;
            self.export_preview_task = None;
            self.export_preview = None;
            return;
        };
        let Some(source) = editor.source else {
            return;
        };
        let key = Key {
            epoch: editor.epoch,
            source,
            values: editor.values.clone(),
        };
        if self.export_preview_key.as_ref() == Some(&key) {
            return;
        }
        self.export_preview_key = Some(key.clone());
        self.export_preview_task = None;
        self.export_preview = None;
        let Some(tab) = self.state.tabs.iter().find(|tab| tab.id == source) else {
            return;
        };
        let snapshot = Snapshot {
            template: editor.values[0].clone(),
            filename: preview::text(tab.log_manager.source_file().unwrap_or("stream")),
            comments: tab
                .comment_manager
                .get()
                .iter()
                .take(3)
                .map(|comment| Comment {
                    text: preview::text(&comment.text),
                    line_indices: comment.line_indices.iter().copied().take(8).collect(),
                })
                .collect(),
            marks: tab.mark_manager.indices().take(8).collect(),
            reader: tab.file_reader.clone(),
            parser: if tab.display.raw_mode {
                None
            } else {
                tab.display.format.clone()
            },
            layout: tab.display.field_layout.clone(),
            hidden: tab.display.hidden_fields.clone(),
            show_keys: tab.display.show_keys,
            fields: editor
                .fields
                .iter()
                .enumerate()
                .take(preview::MAX_FIELDS)
                .map(|(index, (key, _))| (key.clone(), preview::text(&editor.values[index + 6])))
                .collect(),
        };
        let worker = cx
            .background_executor()
            .spawn(async move { render(snapshot) });
        self.export_preview_task = Some(cx.spawn(async move |this, cx| {
            let result = worker.await;
            this.update(cx, |app, cx| {
                if app.export_preview_key.as_ref() == Some(&key)
                    && app.state.editor.as_ref().is_some_and(|editor| {
                        editor.action == Action::Export
                            && editor.epoch == key.epoch
                            && editor.source == Some(key.source)
                            && editor.values == key.values
                    })
                {
                    app.export_preview = Some(result);
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
    fn export_samples_preserve_source_numbers_and_skip_oversized_rows() {
        let reader = FileReader::from_bytes(
            format!(
                "value1\n{}\nvalue3\n",
                "x".repeat(preview::MAX_LINE_BYTES + 1)
            )
            .into_bytes(),
        );
        let snapshot = Snapshot {
            template: "markdown".into(),
            filename: "sample.log".into(),
            comments: vec![Comment {
                text: "Note1".into(),
                line_indices: vec![2],
            }],
            marks: vec![0, 1, 9],
            reader,
            parser: None,
            layout: FieldLayout::default(),
            hidden: HashSet::new(),
            show_keys: false,
            fields: vec![("context".into(), "value4".into())],
        };
        let sample = render(snapshot).unwrap();
        assert!(sample.output.contains("3: value3"));
        assert!(sample.output.contains("1: value1"));
        assert!(sample.output.contains("Note1"));
        assert!(sample.output.contains("value4"));
        assert_eq!((sample.notes, sample.bookmarks, sample.skipped), (1, 1, 2));
    }
}
