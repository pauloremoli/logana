// ---------------------------------------------------------------------------
// Kubernetes CRI log format parser
// ---------------------------------------------------------------------------
//
// Format: `TIMESTAMP STREAM FLAG MESSAGE`
//
// Where:
//   TIMESTAMP = ISO 8601 (e.g. 2024-01-15T10:30:00.123456789Z)
//   STREAM    = stdout | stderr
//   FLAG      = F (full line) | P (partial line)
//   MESSAGE   = rest of line
//
// Example:
//   2024-01-15T10:30:00.123456789Z stdout F Hello, World!
//   2024-01-15T10:30:00.123456789Z stderr F Error: something failed

use std::collections::HashSet;

use super::timestamp::parse_iso_timestamp;
use super::types::{DisplayParts, LogFormatParser};

/// Zero-copy parser for Kubernetes CRI log format.
#[derive(Debug)]
pub struct KubeCriParser;

fn parse_cri_line(s: &str) -> Option<DisplayParts<'_>> {
    // Parse ISO timestamp
    let (timestamp, consumed) = parse_iso_timestamp(s)?;

    let rest = s.get(consumed..)?;
    let rest = rest.strip_prefix(' ')?;

    // Stream: stdout or stderr
    let (stream, rest) = if let Some(r) = rest.strip_prefix("stdout ") {
        ("stdout", r)
    } else if let Some(r) = rest.strip_prefix("stderr ") {
        ("stderr", r)
    } else {
        return None;
    };

    // Flag: F or P
    let (flag, rest) = if let Some(r) = rest.strip_prefix("F ") {
        ("F", r)
    } else if let Some(r) = rest.strip_prefix("P ") {
        ("P", r)
    } else if rest == "F" {
        // Empty message with F flag
        ("F", "")
    } else if rest == "P" {
        ("P", "")
    } else {
        return None;
    };

    let mut parts = DisplayParts {
        timestamp: Some(timestamp),
        ..Default::default()
    };

    parts.extra_fields.push(("stream", stream));
    if flag == "P" {
        parts.extra_fields.push(("partial", "true"));
    }

    if !rest.is_empty() {
        parts.message = Some(rest);
    }

    Some(parts)
}

impl LogFormatParser for KubeCriParser {
    fn parse_line<'a>(&self, line: &'a [u8]) -> Option<DisplayParts<'a>> {
        let s = std::str::from_utf8(line).ok()?;
        if s.is_empty() {
            return None;
        }
        parse_cri_line(s)
    }

    fn collect_field_names(&self, lines: &[&[u8]]) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut extras = Vec::new();

        for &line in lines {
            if let Some(parts) = self.parse_line(line) {
                for (key, _) in &parts.extra_fields {
                    let k = key.to_string();
                    if seen.insert(k.clone()) {
                        extras.push(k);
                    }
                }
            }
        }

        let mut result = vec!["timestamp".to_string()];
        result.extend(extras);
        result.push("message".to_string());
        result
    }

    fn detect_score(&self, sample: &[&[u8]]) -> f64 {
        if sample.is_empty() {
            return 0.0;
        }
        let mut parsed = 0usize;
        for &line in sample {
            if self.parse_line(line).is_some() {
                parsed += 1;
            }
        }
        if parsed == 0 {
            return 0.0;
        }
        parsed as f64 / sample.len() as f64
    }

    fn name(&self) -> &str {
        "kube-cri"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Basic parsing ─────────────────────────────────────────────────

    #[test]
    fn test_parse_stdout_full() {
        let line = b"2024-01-15T10:30:00.123456789Z stdout F Hello, World!";
        let parser = KubeCriParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024-01-15T10:30:00.123456789Z"));
        assert_eq!(parts.message, Some("Hello, World!"));
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "stream" && *v == "stdout")
        );
        assert!(!parts.extra_fields.iter().any(|(k, _)| *k == "partial"));
    }

    #[test]
    fn test_parse_stderr() {
        let line = b"2024-01-15T10:30:00.123456789Z stderr F Error: something failed";
        let parser = KubeCriParser;
        let parts = parser.parse_line(line).unwrap();
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "stream" && *v == "stderr")
        );
        assert_eq!(parts.message, Some("Error: something failed"));
    }

    #[test]
    fn test_parse_partial_line() {
        let line = b"2024-01-15T10:30:00.123456789Z stdout P partial message...";
        let parser = KubeCriParser;
        let parts = parser.parse_line(line).unwrap();
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "partial" && *v == "true")
        );
        assert_eq!(parts.message, Some("partial message..."));
    }

    #[test]
    fn test_parse_empty_message() {
        let line = b"2024-01-15T10:30:00.123456789Z stdout F";
        let parser = KubeCriParser;
        let parts = parser.parse_line(line).unwrap();
        assert!(parts.message.is_none());
    }

    #[test]
    fn test_parse_with_simple_timestamp() {
        let line = b"2024-01-15T10:30:00Z stdout F simple";
        let parser = KubeCriParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024-01-15T10:30:00Z"));
        assert_eq!(parts.message, Some("simple"));
    }

    // ── Negative cases ────────────────────────────────────────────────

    #[test]
    fn test_parse_empty() {
        let parser = KubeCriParser;
        assert!(parser.parse_line(b"").is_none());
    }

    #[test]
    fn test_parse_not_cri() {
        let parser = KubeCriParser;
        assert!(parser.parse_line(b"just plain text").is_none());
    }

    #[test]
    fn test_parse_json_not_cri() {
        let parser = KubeCriParser;
        assert!(
            parser
                .parse_line(br#"{"level":"INFO","msg":"hello"}"#)
                .is_none()
        );
    }

    #[test]
    fn test_parse_iso_but_not_cri() {
        // ISO timestamp but wrong stream field
        let parser = KubeCriParser;
        assert!(
            parser
                .parse_line(b"2024-01-15T10:30:00Z myhost sshd: msg")
                .is_none()
        );
    }

    #[test]
    fn test_parse_iso_bad_flag() {
        let parser = KubeCriParser;
        assert!(
            parser
                .parse_line(b"2024-01-15T10:30:00Z stdout X msg")
                .is_none()
        );
    }

    // ── detect_score ─────────────────────────────────────────────────

    #[test]
    fn test_detect_score_all_cri() {
        let parser = KubeCriParser;
        let lines: Vec<&[u8]> = vec![
            b"2024-01-15T10:30:00Z stdout F msg1",
            b"2024-01-15T10:30:01Z stderr F msg2",
        ];
        let score = parser.detect_score(&lines);
        assert!((score - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_detect_score_mixed() {
        let parser = KubeCriParser;
        let lines: Vec<&[u8]> = vec![b"2024-01-15T10:30:00Z stdout F msg1", b"not cri"];
        let score = parser.detect_score(&lines);
        assert!((score - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_detect_score_none() {
        let parser = KubeCriParser;
        let lines: Vec<&[u8]> = vec![b"plain text", b"more text"];
        let score = parser.detect_score(&lines);
        assert!((score - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_detect_score_empty() {
        let parser = KubeCriParser;
        let lines: Vec<&[u8]> = vec![];
        let score = parser.detect_score(&lines);
        assert!((score - 0.0).abs() < 0.001);
    }

    // ── collect_field_names ──────────────────────────────────────────

    #[test]
    fn test_collect_field_names() {
        let parser = KubeCriParser;
        let lines: Vec<&[u8]> = vec![
            b"2024-01-15T10:30:00Z stdout F msg",
            b"2024-01-15T10:30:01Z stderr P partial",
        ];
        let names = parser.collect_field_names(&lines);
        assert_eq!(names[0], "timestamp");
        assert!(names.contains(&"stream".to_string()));
        assert_eq!(*names.last().unwrap(), "message");
    }

    // ── name ─────────────────────────────────────────────────────────

    #[test]
    fn test_name() {
        let parser = KubeCriParser;
        assert_eq!(parser.name(), "kube-cri");
    }
}
