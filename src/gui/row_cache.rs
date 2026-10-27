//! Bounded presentation data shared by table rendering and horizontal sizing.
use super::{
    column_widths::TableWidths,
    view::log_pane::{DisplayedRow, displayed_row},
};
use crate::{
    ingestion::file_reader::ReaderSnapshot,
    shared::{TabId, TabState},
};
use gpui_kit::gpui::{TextRun, Window, font, px};
use std::{collections::HashMap, rc::Rc, sync::Arc};

#[derive(PartialEq)]
struct Key {
    source: TabId,
    generation: u64,
    parser: Option<usize>,
    raw: bool,
    show_keys: bool,
    fields: Vec<String>,
}
struct Row {
    data: Rc<DisplayedRow>,
    widths: Option<(u32, Vec<f32>)>,
}
#[derive(PartialEq)]
struct FitKey {
    lines: Vec<usize>,
    widths: TableWidths,
    message_width: u32,
    font_size: u32,
}
#[derive(Default)]
pub(crate) struct RowCache {
    key: Option<Key>,
    snapshot: Option<ReaderSnapshot>,
    rows: HashMap<usize, Row>,
    bytes: usize,
    fitted: Option<(FitKey, TableWidths)>,
    #[cfg(test)]
    pub measurements: usize,
}

const MAX_ROWS: usize = 1024;
// Detector ranges and row metadata are additionally bounded by the text and row limits.
const MAX_BYTES: usize = 32 * 1024 * 1024;

impl RowCache {
    fn prepare(&mut self, tab: &TabState, fields: &[String]) {
        let unchanged = self.key.as_ref().is_some_and(|key| {
            key.source == tab.id
                && key.generation == tab.cache.parse_gen
                && key.parser
                    == tab
                        .display
                        .format
                        .as_ref()
                        .map(|p| Arc::as_ptr(p) as *const () as usize)
                && key.raw == tab.display.raw_mode
                && key.show_keys == tab.display.show_keys
                && key.fields == fields
        }) && self
            .snapshot
            .as_ref()
            .is_some_and(|s| s.matches(&tab.file_reader));
        if unchanged {
            return;
        }
        self.key = Some(Key {
            source: tab.id,
            generation: tab.cache.parse_gen,
            parser: tab
                .display
                .format
                .as_ref()
                .map(|p| Arc::as_ptr(p) as *const () as usize),
            raw: tab.display.raw_mode,
            show_keys: tab.display.show_keys,
            fields: fields.to_vec(),
        });
        self.snapshot = Some(tab.file_reader.cache_snapshot());
        self.rows.clear();
        self.bytes = 0;
        self.fitted = None;
    }

    pub fn row(&mut self, tab: &TabState, line: usize, fields: &[String]) -> Rc<DisplayedRow> {
        self.prepare(tab, fields);
        if let Some(row) = self.rows.get(&line) {
            return row.data.clone();
        }
        let data = Rc::new(displayed_row(tab, line, fields));
        let bytes = data.values.iter().map(String::len).sum::<usize>()
            + data.target.as_ref().map_or(0, String::len);
        if self.rows.len() >= MAX_ROWS || self.bytes.saturating_add(bytes) > MAX_BYTES {
            self.rows.clear();
            self.bytes = 0;
            self.fitted = None;
        }
        if bytes <= MAX_BYTES {
            self.bytes += bytes;
            self.rows.insert(
                line,
                Row {
                    data: data.clone(),
                    widths: None,
                },
            );
        }
        data
    }

    pub fn fit(
        &mut self,
        tab: &TabState,
        lines: Vec<usize>,
        fields: &[String],
        widths: &TableWidths,
        message_width: f32,
        window: &Window,
    ) -> TableWidths {
        self.prepare(tab, fields);
        let font_size = f32::from(window.rem_size()) * 12. / 13.;
        let key = FitKey {
            lines,
            widths: widths.clone(),
            message_width: message_width.to_bits(),
            font_size: font_size.to_bits(),
        };
        if let Some((old, fitted)) = &self.fitted
            && old == &key
        {
            return fitted.clone();
        }
        let mut fitted = widths.clone();
        for &line in &key.lines {
            let data = self.row(tab, line, fields);
            let measured = self
                .rows
                .get(&line)
                .and_then(|row| row.widths.as_ref())
                .filter(|(size, _)| *size == key.font_size)
                .map(|(_, widths)| widths.clone());
            let measured = measured.unwrap_or_else(|| {
                #[cfg(test)]
                {
                    self.measurements += 1;
                }
                data.values
                    .iter()
                    .zip(fields)
                    .map(|(text, field)| {
                        if !matches!(field.as_str(), "message" | "msg") {
                            return 0.;
                        }
                        text.lines()
                            .map(|text| {
                                let run = TextRun {
                                    len: text.len(),
                                    font: font("monospace"),
                                    color: Default::default(),
                                    background_color: None,
                                    underline: None,
                                    strikethrough: None,
                                };
                                f32::from(
                                    window
                                        .text_system()
                                        .shape_line(
                                            text.to_owned().into(),
                                            px(font_size),
                                            &[run],
                                            None,
                                        )
                                        .width,
                                ) + 16.
                            })
                            .fold(0., f32::max)
                    })
                    .collect::<Vec<_>>()
            });
            if let Some(row) = self.rows.get_mut(&line) {
                row.widths = Some((key.font_size, measured.clone()));
            }
            for (field, measured) in fields.iter().zip(measured) {
                if matches!(field.as_str(), "message" | "msg") {
                    let width = fitted
                        .fields
                        .get(field)
                        .copied()
                        .unwrap_or(message_width)
                        .max(measured);
                    Arc::make_mut(&mut fitted.fields).insert(field.clone(), width);
                }
            }
        }
        self.fitted = Some((key, fitted.clone()));
        fitted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::{Database, LogManager},
        ingestion::FileReader,
    };

    async fn tab(bytes: &[u8]) -> TabState {
        let db = Arc::new(Database::in_memory().await.unwrap());
        TabState::new(
            FileReader::from_bytes(bytes.to_vec()),
            LogManager::new(db, None).await,
            "cache.log".into(),
        )
    }

    #[tokio::test]
    async fn presentation_cache_reuses_rows_and_invalidates_changed_content_and_columns() {
        let mut tab = tab(br#"{"message":"first","key1":"example"}"#).await;
        tab.display.format = Some(Arc::from(
            crate::parser::find_builtin_parser("json").unwrap(),
        ));
        let mut cache = RowCache::default();
        let fields = vec!["message".into(), "key1".into()];
        let first = cache.row(&tab, 0, &fields);
        assert!(Rc::ptr_eq(&first, &cache.row(&tab, 0, &fields)));
        assert_eq!(first.values, &["first", "example"]);
        tab.display.show_keys = true;
        assert_eq!(cache.row(&tab, 0, &fields).values[1], "key1=example");
        tab.display.raw_mode = true;
        assert!(cache.row(&tab, 0, &fields).values[0].starts_with('{'));
        tab.display.raw_mode = false;
        let reversed = vec!["key1".into(), "message".into()];
        assert_eq!(cache.row(&tab, 0, &reversed).values[1], "first");
        // Same-size replacement must not reuse the old source's values.
        tab.file_reader =
            FileReader::from_bytes(br#"{"message":"other","key1":"example"}"#.to_vec());
        assert_eq!(cache.row(&tab, 0, &fields).values[0], "other");
        let before = cache.row(&tab, 0, &fields);
        tab.invalidate_parse_cache();
        assert!(!Rc::ptr_eq(&before, &cache.row(&tab, 0, &fields)));
    }

    #[tokio::test]
    async fn presentation_cache_tracks_partial_appends_without_retaining_file_buffers() {
        let mut tab = tab(b"first").await;
        let fields = vec!["message".into()];
        let mut cache = RowCache::default();
        assert_eq!(cache.row(&tab, 0, &fields).values[0], "first");
        tab.file_reader.append_bytes(b" more");
        assert_eq!(tab.file_reader.line_count(), 1);
        assert_eq!(cache.row(&tab, 0, &fields).values[0], "first more");
    }

    #[tokio::test]
    async fn scrolling_cache_keeps_a_bounded_number_of_rows() {
        let bytes = "message\n".repeat(MAX_ROWS + 5);
        let tab = tab(bytes.as_bytes()).await;
        let fields = vec!["message".into()];
        let mut cache = RowCache::default();
        for line in 0..tab.file_reader.line_count() {
            cache.row(&tab, line, &fields);
        }
        assert!(cache.rows.len() <= MAX_ROWS);
        assert!(cache.bytes <= MAX_BYTES);
        assert_eq!(cache.row(&tab, MAX_ROWS, &fields).values[0], "message");
    }
}
