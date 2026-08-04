use std::sync::Arc;

use crate::filters::system_time_to_date;
use crate::ingestion::FileReader;
use crate::parser::{LogFormatParser, detect_format};
use crate::ui::{YearMap, build_continuation_map};

/// A source's detected format plus the derived structures ([`YearMap`],
/// continuation map) needed to merge-sort it against other sources —
/// everything [`crate::ui::TabState::detect_and_apply_format`] computes,
/// minus the tab-only side effects (default hidden fields, notifications)
/// that don't apply to a source that's about to be merged rather than
/// opened on its own.
#[derive(Debug, Clone, Default)]
pub struct DetectedFormat {
    pub format: Option<Arc<dyn LogFormatParser>>,
    pub year_map: Option<Arc<YearMap>>,
    pub continuation_map: Option<Arc<Vec<usize>>>,
}

/// Builds the year map (only when the format's timestamps lack a year) and
/// continuation map for an already-known `format`. This is the derivation
/// half of [`detect_format_for_reader`], factored out so that a manually
/// assigned format (`TabState::apply_format`, used by `:schema`) gets the
/// same derived structures an auto-detected one would — otherwise the
/// continuation map from whatever format was previously active stays
/// around and keeps gating line visibility by a "parent" line that the
/// current parser no longer recognizes.
pub fn derive_format_structures(
    reader: &FileReader,
    format: Option<&dyn LogFormatParser>,
) -> (Option<Arc<Vec<usize>>>, Option<Arc<YearMap>>) {
    let continuation_map = format.map(|p| Arc::new(build_continuation_map(reader, p)));
    let year_map = format.and_then(|p| {
        if p.timestamp_has_year() {
            return None;
        }
        let start_year = system_time_to_date(reader.mtime())
            .map(|d| d.year())
            .unwrap_or_else(|| time::OffsetDateTime::now_utc().year());
        Some(Arc::new(YearMap::build(reader, p, start_year)))
    });
    (continuation_map, year_map)
}

/// Samples up to 200 lines of `reader`, detects its format, and — if a
/// format was detected — builds the year map (only when the format's
/// timestamps lack a year) and continuation map.
pub fn detect_format_for_reader(reader: &FileReader) -> DetectedFormat {
    let limit = reader.line_count().min(200);
    if limit == 0 {
        return DetectedFormat::default();
    }
    let sample: Vec<&[u8]> = (0..limit).map(|j| reader.get_line(j)).collect();
    let format: Option<Arc<dyn LogFormatParser>> = detect_format(&sample).map(Arc::from);
    let (continuation_map, year_map) = derive_format_structures(reader, format.as_deref());
    DetectedFormat {
        format,
        year_map,
        continuation_map,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_format_for_reader_empty_reader_returns_none() {
        let reader = FileReader::from_bytes(vec![]);
        let detected = detect_format_for_reader(&reader);
        assert!(detected.format.is_none());
        assert!(detected.year_map.is_none());
        assert!(detected.continuation_map.is_none());
    }

    #[test]
    fn test_detect_format_for_reader_recognized_json_lines() {
        let data = br#"{"timestamp":"2024-01-01T00:00:00Z","level":"INFO","msg":"hello"}
{"timestamp":"2024-01-01T00:00:01Z","level":"WARN","msg":"world"}
"#
        .to_vec();
        let reader = FileReader::from_bytes(data);
        let detected = detect_format_for_reader(&reader);
        assert!(detected.format.is_some());
        assert!(detected.continuation_map.is_some());
    }

    #[test]
    fn test_detect_format_for_reader_unrecognized_content_returns_none_format() {
        let data = b"just some random text\nwith no recognizable structure\n".to_vec();
        let reader = FileReader::from_bytes(data);
        let detected = detect_format_for_reader(&reader);
        assert!(detected.format.is_none());
        assert!(detected.continuation_map.is_none());
    }
}
