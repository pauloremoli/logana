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
        let long = "東京".repeat(MAX_TEXT_CHARS);
        assert_eq!(text(&long).chars().count(), MAX_TEXT_CHARS + 1);
        let line = br#"{"key1":"value1","message":"value2"}"#;
        let parser = crate::parser::detect_format(&[line]).unwrap();
        assert!(fields(parser.as_ref(), line).contains(&("key1".into(), "value1".into())));
        assert!(fields(parser.as_ref(), b"unstructured").is_empty());
        assert!(fields(parser.as_ref(), &vec![b'x'; MAX_LINE_BYTES + 1]).is_empty());
    }
}
