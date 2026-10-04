use crate::shared::TabState;

/// Identifies which of the log table's fixed-width columns a resize drag
/// targets. `Message` has no fixed width (it always fills whatever space
/// remains), so it isn't resizable and has no variant here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizableColumn {
    LineNo,
    Time,
    Level,
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
/// Caps how many visible lines `fit_time_width` samples for the longest
/// timestamp — same sampling-with-a-cap philosophy as
/// `TabState::build_field_index`/`build_field_value_counts`, just a
/// smaller cap since this runs synchronously on every file load rather
/// than lazily while a panel is open.
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

/// Auto-fit column widths for `tab`'s content, run once when a file
/// loads (not on every render — sampling is cheap but not free). `#`
/// fits the file's real line count exactly (O(1), no sampling needed);
/// Time and Level sample up to `FIT_SAMPLE_CAP` lines for the longest values.
pub fn fit_column_widths(tab: &TabState) -> ColumnWidths {
    ColumnWidths {
        line_no: fit_line_no_width(tab),
        time: fit_time_width(tab),
        level: fit_level_width(tab),
    }
}

fn fit_level_width(tab: &TabState) -> f32 {
    let mut width = text_width("Level") + COL_PADDING_PX;
    for line in 0..tab.file_reader.line_count().min(FIT_SAMPLE_CAP) {
        let bytes = tab.file_reader.get_line(line);
        let level = crate::gui::level::classify_line_level(tab, &bytes);
        let label = match level {
            crate::parser::LogLevel::Unknown => String::new(),
            crate::parser::LogLevel::Warning => "WARN".to_string(),
            _ => format!("{level:?}").to_uppercase(),
        };
        width = width.max(text_width(&label) + COL_PADDING_PX);
    }
    width
}

fn fit_line_no_width(tab: &TabState) -> f32 {
    let digits = tab.file_reader.line_count().max(1).to_string().len() as f32;
    ((digits + 1.) * CHAR_WIDTH_PX + COL_PADDING_PX).max(COL_LINE_NO_MIN_WIDTH)
}

fn fit_time_width(tab: &TabState) -> f32 {
    let Some(parser) = tab.display.format.as_deref() else {
        return COL_TIME_MIN_WIDTH;
    };
    let sample_len = tab.filter.visible_indices.len().min(FIT_SAMPLE_CAP);
    let max_len = (0..sample_len)
        .filter_map(|pos| tab.filter.visible_indices.get_opt(pos))
        .filter_map(|line_idx| {
            let bytes = tab.file_reader.get_line(line_idx);
            parser
                .parse_line(&bytes)
                .and_then(|parts| parts.timestamp.map(text_width))
        })
        .fold(0.0_f32, f32::max);
    (max_len + COL_PADDING_PX).max(COL_TIME_MIN_WIDTH)
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
    let mut fitted = std::collections::BTreeMap::new();
    for field in fields.iter().filter(|field| {
        !matches!(
            field.as_str(),
            "timestamp" | "time" | "ts" | "level" | "lvl" | "message" | "msg"
        )
    }) {
        fitted.insert(field.clone(), text_width(field) + COL_PADDING_PX);
    }
    if let (Some(width), Some(merged)) = (fitted.get_mut("source"), tab.merged.as_ref()) {
        for label in &merged.source_labels {
            let prefix = if tab.display.show_keys {
                text_width("source=")
            } else {
                0.
            };
            *width = width.max(text_width(label) + prefix + COL_PADDING_PX);
        }
    }
    if let Some(parser) = tab.display.format.as_ref() {
        for line in 0..tab.file_reader.line_count().min(FIT_SAMPLE_CAP) {
            let bytes = tab.file_reader.get_line(line);
            if let Some(parts) = parser.parse_line(&bytes) {
                for (field, width) in &mut fitted {
                    if let Some(value) = crate::filters::resolve_field(field, &parts) {
                        let prefix = if tab.display.show_keys && !value.is_empty() {
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
    let widths = std::sync::Arc::new(fitted);
    *cache = Some(FieldWidthCache {
        key,
        widths: widths.clone(),
    });
    TableWidths {
        fixed,
        fields: widths,
    }
}

/// Applies a resize drag's pixel delta to `column`, clamped so a column
/// can never shrink below `MIN_COLUMN_WIDTH_PX`.
pub fn apply_width_delta(widths: &mut ColumnWidths, column: ResizableColumn, delta: f32) {
    let width = match column {
        ResizableColumn::LineNo => &mut widths.line_no,
        ResizableColumn::Time => &mut widths.time,
        ResizableColumn::Level => &mut widths.level,
    };
    *width = (*width + delta).max(MIN_COLUMN_WIDTH_PX);
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
