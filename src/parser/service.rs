//! Service logs: sequence service <timestamp> pid level/component/feature, message.
use std::sync::LazyLock;

use regex::Regex;

use super::{DisplayParts, FieldSemantic, LogFormatParser, push_extra_field, push_field_as};

static LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"^([0-9A-Fa-f]+)\s+(\S+)\s+<([^>]+)>\s+([0-9A-Fa-f]+)\s+",
        r"([^/\s]+)/([^/\s]+)/([^,\s]+),[ \t]?(.*)$"
    ))
    .unwrap()
});

#[derive(Debug)]
pub struct ServiceParser;

impl LogFormatParser for ServiceParser {
    fn name(&self) -> &str {
        "service"
    }

    fn parse_line<'a>(&self, line: &'a [u8]) -> Option<DisplayParts<'a>> {
        let text = std::str::from_utf8(line)
            .ok()?
            .trim_end_matches(['\r', '\n']);
        let captures = LINE.captures(text)?;
        let value = |index| captures.get(index).unwrap().as_str();
        let mut extra_fields = Vec::new();
        push_extra_field(&mut extra_fields, "id", value(1));
        push_field_as(&mut extra_fields, FieldSemantic::Pid, value(4));
        push_field_as(&mut extra_fields, FieldSemantic::Component, value(6));
        push_field_as(&mut extra_fields, FieldSemantic::Feature, value(7));
        Some(DisplayParts {
            timestamp: Some(value(3)),
            target: Some(value(2)),
            level: Some(value(5)),
            message: Some(value(8)),
            extra_fields,
            ..Default::default()
        })
    }

    fn collect_field_names(&self, _lines: &[&[u8]]) -> Vec<String> {
        [
            "id",
            "target",
            "timestamp",
            "pid",
            "level",
            "component",
            "feature",
            "message",
        ]
        .into_iter()
        .map(String::from)
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_service_and_rejects_unrelated_or_incomplete_lines() {
        let line = b"04 sample-source <2024-01-01T21:54:53.283856Z> 1AF INF/Module/WorkerTask, StateChange: connected";
        // Process identifiers can be hexadecimal.
        let parts = ServiceParser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024-01-01T21:54:53.283856Z"));
        assert_eq!(parts.target, Some("sample-source"));
        assert_eq!(parts.level, Some("INF"));
        assert_eq!(parts.message, Some("StateChange: connected"));
        assert!(
            parts
                .extra_fields
                .contains(&(FieldSemantic::Pid, "pid", "1AF"))
        );
        assert!(
            parts
                .extra_fields
                .contains(&(FieldSemantic::Component, "component", "Module"))
        );
        assert!(
            parts
                .extra_fields
                .contains(&(FieldSemantic::Feature, "feature", "WorkerTask"))
        );
        for line in [
            b"INFO hello".as_slice(),
            b"04 service <timestamp> 1AF INF/Module",
            b"\xff",
        ] {
            assert!(ServiceParser.parse_line(line).is_none());
        }
        assert_eq!(
            super::super::detect_format(&[line]).unwrap().name(),
            "service"
        );
    }
}
