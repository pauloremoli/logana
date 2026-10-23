use crate::parser::LogFormatParser;

pub const MAX_LINE_BYTES: usize = 64 * 1024;
pub const MAX_FIELDS: usize = 32;
pub const MAX_TEXT_CHARS: usize = 4096;

pub fn text(value: &str) -> String {
    let mut chars = value.chars();
    let mut bounded: String = chars.by_ref().take(MAX_TEXT_CHARS).collect();
    if chars.next().is_some() {
        bounded.push('…');
    }
    bounded
}

pub fn fields(parser: &dyn LogFormatParser, line: &[u8]) -> Vec<(String, String)> {
    if line.len() > MAX_LINE_BYTES {
        return Vec::new();
    }
    let Some(parts) = parser.parse_line(line) else {
        return Vec::new();
    };
    parser
        .collect_field_names(&[line])
        .into_iter()
        .take(MAX_FIELDS)
        .filter_map(|name| {
            crate::shared::field_layout::get_col(&parts, &name, false, None)
                .map(|value| (name, value))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn row_samples_bound_unicode_text_and_parsing() {
        assert_eq!(text("value1"), "value1");
        let reader = crate::ingestion::FileReader::from_bytes(b"value1\nvalue2".to_vec());
        let (prefix, truncated) = reader.get_raw_line_prefix(0, 3);
        assert_eq!(&*prefix, b"val");
        assert!(truncated);
        assert!(!reader.get_raw_line_prefix(1, 100).1);
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), b"value1\nvalue2\n").unwrap();
        let paged = crate::ingestion::FileReader::try_new_paged(
            file.path().to_str().unwrap(),
            std::sync::Arc::new(file.path().canonicalize().unwrap()),
        )
        .unwrap()
        .unwrap();
        let merged = crate::ingestion::FileReader::from_merged(
            std::sync::Arc::new(vec![crate::ingestion::file_reader::MergedEntry {
                source_idx: 0,
                line_idx: 1,
                sort_key: [0; 23],
            }]),
            std::sync::Arc::new(vec![paged.clone()]),
        );
        for source in [&paged, &merged] {
            let (prefix, truncated) = source.get_raw_line_prefix(0, 3);
            assert_eq!(&*prefix, b"val");
            assert!(truncated);
            assert!(!source.get_raw_line_prefix(0, 100).1);
        }
        let long = "東京".repeat(MAX_TEXT_CHARS);
        assert_eq!(text(&long).chars().count(), MAX_TEXT_CHARS + 1);
        let line = br#"{"key1":"value1","message":"value2"}"#;
        let parser = crate::parser::detect_format(&[line]).unwrap();
        assert!(fields(parser.as_ref(), line).contains(&("key1".into(), "value1".into())));
        assert!(fields(parser.as_ref(), b"unstructured").is_empty());
        assert!(fields(parser.as_ref(), &vec![b'x'; MAX_LINE_BYTES + 1]).is_empty());
    }
}
