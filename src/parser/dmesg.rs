// ---------------------------------------------------------------------------
// dmesg (Linux kernel ring buffer) parser
// ---------------------------------------------------------------------------
//
// Format: `[ seconds.usecs] message`
//
// Example:
//   [    0.000000] Linux version 6.1.0 (gcc 12.2.0)
//   [   12.345678] usb 1-1: new high-speed USB device number 2

use std::collections::HashSet;

use super::timestamp::parse_dmesg_timestamp;
use super::types::{DisplayParts, LogFormatParser};

/// Zero-copy parser for dmesg kernel ring buffer output.
#[derive(Debug)]
pub struct DmesgParser;

fn parse_dmesg_line(s: &str) -> Option<DisplayParts<'_>> {
    let (timestamp, consumed) = parse_dmesg_timestamp(s)?;

    // Skip optional space after ']'
    let rest = &s[consumed..];
    let rest = rest.strip_prefix(' ').unwrap_or(rest);

    let mut parts = DisplayParts {
        timestamp: Some(timestamp),
        ..Default::default()
    };

    if rest.is_empty() {
        return Some(parts);
    }

    // Try to extract subsystem prefix: "subsystem: " or "subsystem 1-1: "
    // Pattern: word followed by optional identifiers then ": "
    if let Some(colon_pos) = rest.find(": ") {
        let prefix = &rest[..colon_pos];
        // Check if this looks like a subsystem prefix (relatively short, no spaces
        // or spaces that look like device identifiers)
        if prefix.len() <= 40 && !prefix.contains("  ") {
            // Extract just the first word as the subsystem target
            let target = prefix.split_whitespace().next().unwrap_or(prefix);
            if !target.is_empty()
                && target
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                parts.target = Some(target);
                parts.message = Some(&rest[colon_pos + 2..]);
                return Some(parts);
            }
        }
    }

    // No subsystem detected — whole rest is message
    parts.message = Some(rest);
    Some(parts)
}

impl LogFormatParser for DmesgParser {
    fn parse_line<'a>(&self, line: &'a [u8]) -> Option<DisplayParts<'a>> {
        let s = std::str::from_utf8(line).ok()?;
        if s.is_empty() {
            return None;
        }
        parse_dmesg_line(s)
    }

    fn collect_field_names(&self, lines: &[&[u8]]) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut has_target = false;

        for &line in lines {
            if let Some(parts) = self.parse_line(line) {
                if parts.target.is_some() {
                    has_target = true;
                }
                for (key, _) in &parts.extra_fields {
                    seen.insert(key.to_string());
                }
            }
        }

        let mut result = vec!["timestamp".to_string()];
        if has_target {
            result.push("target".to_string());
        }
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
        "dmesg"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Basic parsing ─────────────────────────────────────────────────

    #[test]
    fn test_parse_basic_kernel_line() {
        let line = b"[    0.000000] Linux version 6.1.0 (gcc 12.2.0)";
        let parser = DmesgParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("[    0.000000]"));
        // No ": " in this line, so no subsystem is extracted
        assert!(parts.target.is_none());
        assert_eq!(parts.message, Some("Linux version 6.1.0 (gcc 12.2.0)"));
    }

    #[test]
    fn test_parse_usb_device() {
        let line = b"[   12.345678] usb 1-1: new high-speed USB device number 2";
        let parser = DmesgParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("[   12.345678]"));
        assert_eq!(parts.target, Some("usb"));
        assert_eq!(parts.message, Some("new high-speed USB device number 2"));
    }

    #[test]
    fn test_parse_network_subsystem() {
        let line = b"[  100.123456] eth0: link up at 1000 Mbps";
        let parser = DmesgParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.target, Some("eth0"));
        assert_eq!(parts.message, Some("link up at 1000 Mbps"));
    }

    #[test]
    fn test_parse_no_subsystem() {
        let line = b"[    0.000000] Booting paravirtualized kernel on KVM";
        let parser = DmesgParser;
        let parts = parser.parse_line(line).unwrap();
        // "Booting" doesn't end with ": " in a subsystem-like pattern
        // but it may or may not match depending on content
        assert!(parts.timestamp.is_some());
        assert!(parts.message.is_some());
    }

    #[test]
    fn test_parse_timestamp_only() {
        let line = b"[    0.000000]";
        let parser = DmesgParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("[    0.000000]"));
        assert!(parts.message.is_none());
    }

    #[test]
    fn test_parse_large_seconds() {
        let line = b"[99999.123456] kernel: some message here";
        let parser = DmesgParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("[99999.123456]"));
        assert_eq!(parts.target, Some("kernel"));
    }

    // ── Negative cases ────────────────────────────────────────────────

    #[test]
    fn test_parse_empty() {
        let parser = DmesgParser;
        assert!(parser.parse_line(b"").is_none());
    }

    #[test]
    fn test_parse_not_dmesg() {
        let parser = DmesgParser;
        assert!(parser.parse_line(b"just plain text").is_none());
    }

    #[test]
    fn test_parse_json_not_dmesg() {
        let parser = DmesgParser;
        assert!(
            parser
                .parse_line(br#"{"level":"INFO","msg":"hello"}"#)
                .is_none()
        );
    }

    #[test]
    fn test_parse_bracket_no_dot() {
        let parser = DmesgParser;
        assert!(parser.parse_line(b"[12345] no dot").is_none());
    }

    // ── detect_score ─────────────────────────────────────────────────

    #[test]
    fn test_detect_score_all_dmesg() {
        let parser = DmesgParser;
        let lines: Vec<&[u8]> = vec![
            b"[    0.000000] kernel msg",
            b"[    1.234567] usb 1-1: device",
        ];
        let score = parser.detect_score(&lines);
        assert!((score - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_detect_score_mixed() {
        let parser = DmesgParser;
        let lines: Vec<&[u8]> = vec![b"[    0.000000] kernel msg", b"not dmesg"];
        let score = parser.detect_score(&lines);
        assert!((score - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_detect_score_none() {
        let parser = DmesgParser;
        let lines: Vec<&[u8]> = vec![b"plain text", b"more text"];
        let score = parser.detect_score(&lines);
        assert!((score - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_detect_score_empty() {
        let parser = DmesgParser;
        let lines: Vec<&[u8]> = vec![];
        let score = parser.detect_score(&lines);
        assert!((score - 0.0).abs() < 0.001);
    }

    // ── collect_field_names ──────────────────────────────────────────

    #[test]
    fn test_collect_field_names_with_target() {
        let parser = DmesgParser;
        let lines: Vec<&[u8]> = vec![b"[    0.000000] usb 1-1: new device"];
        let names = parser.collect_field_names(&lines);
        assert!(names.contains(&"timestamp".to_string()));
        assert!(names.contains(&"target".to_string()));
        assert!(names.contains(&"message".to_string()));
    }

    #[test]
    fn test_collect_field_names_without_target() {
        let parser = DmesgParser;
        let lines: Vec<&[u8]> = vec![b"[    0.000000] Booting paravirtualized kernel on KVM"];
        let names = parser.collect_field_names(&lines);
        assert!(names.contains(&"timestamp".to_string()));
        assert!(names.contains(&"message".to_string()));
    }

    // ── name ─────────────────────────────────────────────────────────

    #[test]
    fn test_name() {
        let parser = DmesgParser;
        assert_eq!(parser.name(), "dmesg");
    }
}
