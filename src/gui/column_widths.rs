use crate::shared::TabState;

/// Identifies a column and its width when the drag starts.
#[derive(Debug, Clone, PartialEq)]
pub enum ResizableColumn {
    LineNo,
    Time,
    Level,
    Field(String, f32),
}

/// The log table's current fixed-column widths — either the auto-fit
/// default (`fit_column_widths`, recomputed whenever a file loads) or
/// whatever the user has since dragged a column's handle to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColumnWidths {
    pub line_no: f32,
    pub time: f32,
    pub level: f32,
}

pub(super) struct Snapshot {
    reader: crate::ingestion::FileReader,
    parser: Option<std::sync::Arc<dyn crate::parser::LogFormatParser>>,
    visible: Vec<usize>,
    labels: Vec<String>,
    merged: bool,
    show_keys: bool,
}

/// A column can never be dragged narrower than this — small enough to
/// still show its header glyph, not so small a stray drag collapses it
/// to nothing.
pub const MIN_COLUMN_WIDTH_PX: f32 = 32.0;

/// "WARNING"/"NOTICE", the longest `LogLevel` uppercase labels, plus the
/// pill's own padding — level values are a small, fixed vocabulary, so
/// (unlike `#`/Time) there's no real content-fit benefit to sampling it.
const COL_LEVEL_WIDTH: f32 = 84.0;

const COL_LINE_NO_MIN_WIDTH: f32 = 23.2;
const COL_TIME_MIN_WIDTH: f32 = 44.8;
/// A rough average glyph width (px) at the table's default text size,
/// used to turn a sampled character count into a pixel estimate. The GUI
/// uses a monospaced font, but this is an approximation, not a measured
/// value — good enough to size columns in the right ballpark, with manual
/// resize (dragging a column's handle) available to correct it.
const CHAR_WIDTH_PX: f32 = 7.2;
const COL_PADDING_PX: f32 = 16.0;
/// Limits the row sample used for column fitting.
const FIT_SAMPLE_CAP: usize = 500;

impl Default for ColumnWidths {
    fn default() -> Self {
        Self {
            line_no: COL_LINE_NO_MIN_WIDTH,
            time: COL_TIME_MIN_WIDTH,
            level: COL_LEVEL_WIDTH,
        }
    }
}

/// Fit fixed columns against a captured row sample.
pub fn fit_column_widths(tab: &TabState) -> ColumnWidths {
    Snapshot::capture(tab).fixed_widths()
}

#[cfg(test)]
fn fit_line_no_width(tab: &TabState) -> f32 {
    Snapshot::capture(tab).fixed_widths().line_no
}
#[cfg(test)]
fn fit_time_width(tab: &TabState) -> f32 {
    Snapshot::capture(tab).fixed_widths().time
}

impl Snapshot {
    pub(super) fn capture(tab: &TabState) -> Self {
        Self {
            reader: tab.file_reader.clone(),
            parser: tab.display.format.clone(),
            visible: tab
                .filter
                .visible_indices
                .iter()
                .take(FIT_SAMPLE_CAP)
                .collect(),
            labels: tab
                .merged
                .as_ref()
                .map(|m| m.source_labels.clone())
                .unwrap_or_default(),
            merged: tab.merged.is_some(),
            show_keys: tab.display.show_keys,
        }
    }

    pub(super) fn names(&self) -> Vec<String> {
        let indices: Vec<_> = (0..self.reader.line_count().min(200)).collect();
        TabState::field_names_for(&self.reader, self.parser.as_deref(), &indices, self.merged)
    }

    pub(super) fn fixed_widths(&self) -> ColumnWidths {
        let digits = self.reader.line_count().max(1).to_string().len() as f32;
        let mut level = text_width("Level") + COL_PADDING_PX;
        let mut time = COL_TIME_MIN_WIDTH;
        if let Some(parser) = self.parser.as_deref() {
            for line in 0..self.reader.line_count().min(FIT_SAMPLE_CAP) {
                let bytes = self.reader.get_line(line);
                let classified = parser
                    .parse_line(&bytes)
                    .and_then(|parts| parts.level)
                    .map(|raw| crate::parser::classify_level(Some(parser), raw))
                    .unwrap_or_default();
                let label = match classified {
                    crate::parser::LogLevel::Unknown => String::new(),
                    crate::parser::LogLevel::Warning => "WARN".to_string(),
                    _ => format!("{classified:?}").to_uppercase(),
                };
                level = level.max(text_width(&label) + COL_PADDING_PX);
            }
            for &line in &self.visible {
                let bytes = self.reader.get_line(line);
                if let Some(timestamp) = parser.parse_line(&bytes).and_then(|parts| parts.timestamp)
                {
                    time = time.max(text_width(timestamp) + COL_PADDING_PX);
                }
            }
        }
        ColumnWidths {
            line_no: ((digits + 1.) * CHAR_WIDTH_PX + COL_PADDING_PX).max(COL_LINE_NO_MIN_WIDTH),
            time,
            level,
        }
    }
}

/// Content widths are cached independently of viewport size and resize handles.
#[derive(Clone, Debug, PartialEq)]
pub struct TableWidths {
    pub fixed: ColumnWidths,
    pub fields: std::sync::Arc<std::collections::BTreeMap<String, f32>>,
}
impl std::ops::Deref for TableWidths {
    type Target = ColumnWidths;
    fn deref(&self) -> &Self::Target {
        &self.fixed
    }
}
impl From<ColumnWidths> for TableWidths {
    fn from(fixed: ColumnWidths) -> Self {
        Self {
            fixed,
            fields: Default::default(),
        }
    }
}
pub struct FieldWidthCache {
    key: (crate::shared::TabId, u64, usize, bool, Vec<String>),
    widths: std::sync::Arc<std::collections::BTreeMap<String, f32>>,
}
fn text_width(text: &str) -> f32 {
    use unicode_width::UnicodeWidthStr;
    text.lines().map(UnicodeWidthStr::width).max().unwrap_or(0) as f32 * CHAR_WIDTH_PX
}
pub fn table_widths(
    cache: &mut Option<FieldWidthCache>,
    tab: &TabState,
    fields: &[String],
    fixed: ColumnWidths,
) -> TableWidths {
    let key = (
        tab.id,
        tab.cache.parse_gen,
        tab.file_reader.line_count(),
        tab.display.show_keys,
        fields.to_vec(),
    );
    if let Some(entry) = cache.as_ref().filter(|entry| entry.key == key) {
        return TableWidths {
            fixed,
            fields: entry.widths.clone(),
        };
    }
    let widths = Snapshot::capture(tab).field_widths(fields);
    *cache = Some(FieldWidthCache {
        key,
        widths: widths.clone(),
    });
    TableWidths {
        fixed,
        fields: widths,
    }
}

fn header_widths(fields: &[String]) -> std::collections::BTreeMap<String, f32> {
    fields
        .iter()
        .filter(|field| {
            !matches!(
                field.as_str(),
                "timestamp" | "time" | "ts" | "level" | "lvl" | "message" | "msg"
            )
        })
        .map(|field| (field.clone(), text_width(field) + COL_PADDING_PX))
        .collect()
}

impl Snapshot {
    fn field_widths(
        &self,
        fields: &[String],
    ) -> std::sync::Arc<std::collections::BTreeMap<String, f32>> {
        let mut fitted = header_widths(fields);
        if let Some(width) = fitted.get_mut("source") {
            for label in &self.labels {
                let prefix = if self.show_keys {
                    text_width("source=")
                } else {
                    0.
                };
                *width = width.max(text_width(label) + prefix + COL_PADDING_PX);
            }
        }
        if let Some(parser) = self.parser.as_ref() {
            for line in 0..self.reader.line_count().min(FIT_SAMPLE_CAP) {
                let bytes = self.reader.get_line(line);
                if let Some(parts) = parser.parse_line(&bytes) {
                    for (field, width) in &mut fitted {
                        if let Some(value) = crate::filters::resolve_field(field, &parts) {
                            let prefix = if self.show_keys && !value.is_empty() {
                                text_width(field) + CHAR_WIDTH_PX
                            } else {
                                0.
                            };
                            *width = width.max(text_width(value) + prefix + COL_PADDING_PX);
                        }
                    }
                }
            }
        }
        std::sync::Arc::new(fitted)
    }

    pub(super) fn prepare(
        &self,
        source: crate::shared::TabId,
        generation: u64,
    ) -> (Vec<String>, FieldWidthCache, ColumnWidths) {
        let fields = self.names();
        let cache = FieldWidthCache {
            key: (
                source,
                generation,
                self.reader.line_count(),
                self.show_keys,
                fields.clone(),
            ),
            widths: self.field_widths(&fields),
        };
        (fields, cache, self.fixed_widths())
    }
}

pub fn prepared_table_widths(
    cache: &Option<FieldWidthCache>,
    tab: &TabState,
    fields: &[String],
    fixed: ColumnWidths,
) -> TableWidths {
    let key = (
        tab.id,
        tab.cache.parse_gen,
        tab.file_reader.line_count(),
        tab.display.show_keys,
        fields.to_vec(),
    );
    let widths = cache
        .as_ref()
        .filter(|entry| entry.key == key)
        .map(|entry| entry.widths.clone())
        .unwrap_or_else(|| std::sync::Arc::new(header_widths(fields)));
    TableWidths {
        fixed,
        fields: widths,
    }
}

pub fn cached_field_names(tab: &TabState) -> Vec<String> {
    tab.cache
        .field_names
        .as_ref()
        .filter(|(generation, _)| *generation == tab.cache.parse_gen)
        .map(|(_, names)| names.clone())
        .or_else(|| tab.display.field_layout.columns.clone())
        .or_else(|| {
            tab.cache
                .field_names
                .as_ref()
                .map(|(_, names)| names.clone())
        })
        .unwrap_or_default()
}

/// Applies a resize drag's pixel delta to `column`, clamped so a column
/// can never shrink below `MIN_COLUMN_WIDTH_PX`.
pub fn apply_width_delta(widths: &mut ColumnWidths, column: ResizableColumn, delta: f32) {
    let width = match column {
        ResizableColumn::LineNo => &mut widths.line_no,
        ResizableColumn::Time => &mut widths.time,
        ResizableColumn::Level => &mut widths.level,
        ResizableColumn::Field(..) => return,
    };
    *width = (*width + delta).max(MIN_COLUMN_WIDTH_PX);
}

/// Unrecognized files use their own layout, separate from detected formats.
pub fn format_key(tab: &TabState) -> String {
    tab.display
        .format
        .as_ref()
        .map(|parser| parser.name().to_owned())
        .unwrap_or_else(|| "__unstructured__".into())
}

impl TableWidths {
    pub fn restore(&mut self, saved: &std::collections::BTreeMap<String, f32>) {
        for (field, width) in saved {
            if !width.is_finite() || *width < MIN_COLUMN_WIDTH_PX {
                continue;
            }
            match field.as_str() {
                "#" => self.fixed.line_no = *width,
                "timestamp" | "time" | "ts" => self.fixed.time = *width,
                "level" | "lvl" => self.fixed.level = *width,
                _ => {
                    std::sync::Arc::make_mut(&mut self.fields).insert(field.clone(), *width);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Database, LogManager};
    use crate::ingestion::FileReader;
    use std::sync::Arc;

    async fn tab_with_lines(lines: &[&str]) -> TabState {
        let data = lines.join("\n").into_bytes();
        let reader = FileReader::from_bytes(data);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        TabState::new(reader, log_manager, "test.log".to_string())
    }

    #[tokio::test]
    async fn line_no_width_fits_the_real_digit_count() {
        let lines: Vec<String> = (0..150).map(|i| i.to_string()).collect();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let tab = tab_with_lines(&refs).await;
        // Three digits plus the bookmark marker slot must fit without clipping.
        assert_eq!(fit_line_no_width(&tab), 44.8);
    }

    #[tokio::test]
    async fn line_no_width_grows_for_many_digits() {
        let lines: Vec<String> = (0..200_000).map(|i| i.to_string()).collect();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let tab = tab_with_lines(&refs).await;
        // Six digits plus the bookmark marker exceed the minimum width.
        assert!(fit_line_no_width(&tab) > COL_LINE_NO_MIN_WIDTH);
    }

    #[tokio::test]
    async fn time_width_is_the_floor_without_a_detected_format() {
        let tab = tab_with_lines(&["a", "b"]).await;
        assert_eq!(fit_time_width(&tab), COL_TIME_MIN_WIDTH);
    }

    #[tokio::test]
    async fn time_width_fits_the_longest_sampled_timestamp() {
        let mut tab = tab_with_lines(&[
            "<134>Oct 11 22:14:15 myhost sshd[1234]: short",
            "<134>Oct 1 2:14:15 myhost sshd[1234]: shorter-date",
        ])
        .await;
        tab.display.format = Some(Arc::new(crate::parser::SyslogParser::default()));
        // "Oct 11 22:14:15" is 15 chars -> 15 * 7.2 + 16 = 124.
        assert_eq!(fit_time_width(&tab), 15.0 * CHAR_WIDTH_PX + COL_PADDING_PX);
    }

    #[tokio::test]
    async fn field_widths_fit_values_keys_and_unicode_and_reuse_cache() {
        let tab =
            tab_with_lines(&[r#"{"level":"INFO","message":"hello","id":"42","city":"東京"}"#])
                .await;
        let fields = vec!["id".into(), "city".into()];
        let mut tab = tab;
        tab.display.show_keys = false;
        let mut cache = None;
        let widths = table_widths(&mut cache, &tab, &fields, fit_column_widths(&tab));
        assert_eq!(widths.fields["id"], 2. * CHAR_WIDTH_PX + COL_PADDING_PX);
        assert_eq!(widths.fields["city"], 4. * CHAR_WIDTH_PX + COL_PADDING_PX);
        assert!(widths.level < COL_LEVEL_WIDTH);
        let cached = table_widths(&mut cache, &tab, &fields, widths.fixed);
        assert!(std::sync::Arc::ptr_eq(&widths.fields, &cached.fields));
        tab.display.show_keys = true;
        let keyed = table_widths(&mut cache, &tab, &fields, widths.fixed);
        assert_eq!(keyed.fields["id"], 5. * CHAR_WIDTH_PX + COL_PADDING_PX);
        assert!((keyed.fields["city"] - (9. * CHAR_WIDTH_PX + COL_PADDING_PX)).abs() < 0.001);
        assert!(!std::sync::Arc::ptr_eq(&widths.fields, &keyed.fields));
    }

    #[test]
    fn apply_width_delta_grows_and_shrinks() {
        let mut widths = ColumnWidths::default();
        apply_width_delta(&mut widths, ResizableColumn::Time, 20.0);
        assert_eq!(widths.time, ColumnWidths::default().time + 20.0);
        apply_width_delta(&mut widths, ResizableColumn::Time, -20.0);
        assert!((widths.time - ColumnWidths::default().time).abs() < 0.001);
    }

    #[test]
    fn apply_width_delta_never_shrinks_below_the_floor() {
        let mut widths = ColumnWidths::default();
        apply_width_delta(&mut widths, ResizableColumn::Level, -1000.0);
        assert_eq!(widths.level, MIN_COLUMN_WIDTH_PX);
    }
}
