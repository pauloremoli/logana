// ---------------------------------------------------------------------------
// Common log format parser: TIMESTAMP + LEVEL + TARGET + MESSAGE family
// ---------------------------------------------------------------------------
//
// Handles the broad family of log formats that share a common structure:
// a timestamp, followed by a log level, optionally a target/logger name,
// and a message. This covers env_logger, tracing fmt, log4rs, logback,
// log4j2, Spring Boot, Python logging, loguru, structlog, and more.
//
// Key rule: Requires a recognizable level keyword to match — prevents
// claiming journalctl/syslog lines that lack level info.

use std::collections::HashSet;

use super::timestamp::{
    is_level_keyword, normalize_level, parse_datetime_timestamp, parse_iso_timestamp,
};
use super::types::{DisplayParts, LogFormatParser};

/// Zero-copy parser for the TIMESTAMP + LEVEL + TARGET + MESSAGE family.
#[derive(Debug)]
pub struct CommonLogParser;

// ---------------------------------------------------------------------------
// Sub-strategy parsers
// ---------------------------------------------------------------------------

/// env_logger: `[ISO LEVEL  target] msg`
/// or `[DATETIME LEVEL target] msg`
fn try_env_logger(s: &str) -> Option<DisplayParts<'_>> {
    if !s.starts_with('[') {
        return None;
    }
    let close = s.find(']')?;
    let bracket_content = &s[1..close];
    let rest = &s[close + 1..];
    let rest = rest.strip_prefix(' ').unwrap_or(rest);

    // Split bracket content: timestamp, level, target
    let mut tokens = bracket_content.split_whitespace();

    // First token(s): try to find a timestamp + level
    let first = tokens.next()?;

    // Check if first is a timestamp
    if let Some((_ts, ts_end)) = parse_iso_timestamp(bracket_content) {
        let after_ts = bracket_content[ts_end..].trim_start();
        let mut after_tokens = after_ts.split_whitespace();
        let level_token = after_tokens.next()?;
        let level = normalize_level(level_token)?;
        let target = after_tokens.next();

        let mut parts = DisplayParts {
            timestamp: Some(&bracket_content[..ts_end]),
            level: Some(level),
            ..Default::default()
        };
        if let Some(t) = target.filter(|t| !t.is_empty()) {
            parts.target = Some(t);
        }
        if !rest.is_empty() {
            parts.message = Some(rest);
        }
        return Some(parts);
    }

    if let Some((_ts, ts_end)) = parse_datetime_timestamp(bracket_content) {
        let after_ts = bracket_content[ts_end..].trim_start();
        let mut after_tokens = after_ts.split_whitespace();
        let level_token = after_tokens.next()?;
        let level = normalize_level(level_token)?;
        let target = after_tokens.next();

        let mut parts = DisplayParts {
            timestamp: Some(&bracket_content[..ts_end]),
            level: Some(level),
            ..Default::default()
        };
        if let Some(t) = target.filter(|t| !t.is_empty()) {
            parts.target = Some(t);
        }
        if !rest.is_empty() {
            parts.message = Some(rest);
        }
        return Some(parts);
    }

    // Maybe just LEVEL target (no timestamp inside brackets)
    let level = normalize_level(first)?;
    let target = tokens.next();
    let remaining: String = tokens.collect::<Vec<_>>().join(" ");

    let mut parts = DisplayParts {
        level: Some(level),
        ..Default::default()
    };
    if let Some(t) = target.filter(|t| !t.is_empty()) {
        parts.target = Some(t);
    }
    let msg = if remaining.is_empty() {
        rest
    } else if rest.is_empty() {
        // Shouldn't happen but be safe
        return Some(parts);
    } else {
        rest
    };
    if !msg.is_empty() {
        parts.message = Some(msg);
    }
    Some(parts)
}

/// logback/log4j2: `DATETIME [thread] LEVEL target - msg`
fn try_logback(s: &str) -> Option<DisplayParts<'_>> {
    let (timestamp, consumed) = parse_datetime_timestamp(s)?;

    let rest = s.get(consumed..)?.trim_start();
    if rest.is_empty() {
        return None;
    }

    // [thread]
    if !rest.starts_with('[') {
        return None;
    }
    let close = rest.find(']')?;
    let thread = &rest[1..close];
    let rest = &rest[close + 1..];
    let rest = rest.trim_start();

    // LEVEL
    let space = rest.find(' ')?;
    let level_token = &rest[..space];
    let level = normalize_level(level_token)?;
    let rest = &rest[space + 1..];
    let rest = rest.trim_start();

    // target - message  or  target : message
    let mut parts = DisplayParts {
        timestamp: Some(timestamp),
        level: Some(level),
        ..Default::default()
    };
    parts.extra_fields.push(("thread", thread));

    if let Some(sep_pos) = rest.find(" - ") {
        let target = &rest[..sep_pos];
        if !target.is_empty() {
            parts.target = Some(target.trim_end());
        }
        let msg = &rest[sep_pos + 3..];
        if !msg.is_empty() {
            parts.message = Some(msg);
        }
    } else if let Some(sep_pos) = rest.find(" : ") {
        let target = &rest[..sep_pos];
        if !target.is_empty() {
            parts.target = Some(target.trim_end());
        }
        let msg = &rest[sep_pos + 3..];
        if !msg.is_empty() {
            parts.message = Some(msg);
        }
    } else {
        parts.message = Some(rest);
    }

    Some(parts)
}

/// Spring Boot: `DATETIME  LEVEL PID --- [thread] target : msg`
fn try_spring_boot(s: &str) -> Option<DisplayParts<'_>> {
    let (timestamp, consumed) = parse_datetime_timestamp(s)?;

    let rest = s.get(consumed..)?.trim_start();
    if rest.is_empty() {
        return None;
    }

    // LEVEL
    let space = rest.find(' ')?;
    let level_token = &rest[..space];
    let level = normalize_level(level_token)?;
    let rest = &rest[space + 1..];
    let rest = rest.trim_start();

    // PID (numeric)
    let space = rest.find(' ')?;
    let pid = &rest[..space];
    if !pid.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let rest = &rest[space + 1..];
    let rest = rest.trim_start();

    // --- separator
    if !rest.starts_with("--- ") {
        return None;
    }
    let rest = &rest[4..];

    // [thread]
    if !rest.starts_with('[') {
        return None;
    }
    let close = rest.find(']')?;
    let thread = &rest[1..close];
    let rest = &rest[close + 1..];
    let rest = rest.trim_start();

    let mut parts = DisplayParts {
        timestamp: Some(timestamp),
        level: Some(level),
        ..Default::default()
    };
    parts.extra_fields.push(("pid", pid));
    parts.extra_fields.push(("thread", thread.trim()));

    // target : message
    if let Some(sep_pos) = rest.find(" : ") {
        let target = &rest[..sep_pos];
        if !target.is_empty() {
            parts.target = Some(target.trim_end());
        }
        let msg = &rest[sep_pos + 3..];
        if !msg.is_empty() {
            parts.message = Some(msg);
        }
    } else if !rest.is_empty() {
        parts.message = Some(rest);
    }

    Some(parts)
}

/// Python basic: `LEVEL:target:msg` (no timestamp)
fn try_python_basic(s: &str) -> Option<DisplayParts<'_>> {
    // Must start with a level keyword followed by ':'
    let colon1 = s.find(':')?;
    let level_token = &s[..colon1];
    let level = normalize_level(level_token)?;
    let rest = &s[colon1 + 1..];

    // target:msg
    if let Some(colon2) = rest.find(':') {
        let target = &rest[..colon2];
        let msg = &rest[colon2 + 1..];

        let mut parts = DisplayParts {
            level: Some(level),
            ..Default::default()
        };
        if !target.is_empty() {
            parts.target = Some(target);
        }
        if !msg.is_empty() {
            parts.message = Some(msg);
        }
        Some(parts)
    } else {
        // Just LEVEL:msg
        let mut parts = DisplayParts {
            level: Some(level),
            ..Default::default()
        };
        if !rest.is_empty() {
            parts.message = Some(rest);
        }
        Some(parts)
    }
}

/// Python prod: `DATETIME - target - LEVEL - msg`
fn try_python_prod(s: &str) -> Option<DisplayParts<'_>> {
    let (timestamp, consumed) = parse_datetime_timestamp(s)?;

    let rest = s.get(consumed..)?;
    let rest = rest.strip_prefix(" - ")?;

    // Split by " - "
    let segments: Vec<&str> = rest.splitn(4, " - ").collect();
    if segments.len() < 3 {
        return None;
    }

    let target = segments[0];
    let level_token = segments[1];
    let level = normalize_level(level_token)?;

    let mut parts = DisplayParts {
        timestamp: Some(timestamp),
        level: Some(level),
        ..Default::default()
    };

    if !target.is_empty() {
        parts.target = Some(target);
    }

    // Remaining segments form the message
    let msg = if segments.len() > 3 {
        segments[2..].join(" - ")
    } else {
        segments[2].to_string()
    };
    // We need to return a reference, but the joined string is owned.
    // Re-find the message position in the original string.
    let msg_start = s.len() - rest[rest.len() - segments[segments.len() - 1].len()..].len();
    if segments.len() == 3 {
        let msg_slice = segments[2];
        if !msg_slice.is_empty() {
            parts.message = Some(msg_slice);
        }
    } else {
        // For messages containing " - ", find the start of the third segment
        let mut offset = 0;
        for (i, seg) in segments.iter().enumerate() {
            if i == 2 {
                let remaining = &rest[offset..];
                if !remaining.is_empty() {
                    parts.message = Some(remaining);
                }
                break;
            }
            offset += seg.len() + 3; // " - "
        }
    }
    // Silence unused variable warning
    let _ = msg_start;
    let _ = msg;

    Some(parts)
}

/// loguru: `DATETIME | LEVEL    | location - msg`
fn try_loguru(s: &str) -> Option<DisplayParts<'_>> {
    let (timestamp, consumed) = parse_datetime_timestamp(s)?;

    let rest = s.get(consumed..)?;
    let rest = rest.strip_prefix(" | ")?;

    // LEVEL (possibly padded with spaces)
    let pipe = rest.find(" | ")?;
    let level_token = rest[..pipe].trim();
    let level = normalize_level(level_token)?;
    let rest = &rest[pipe + 3..];

    // location - message
    let mut parts = DisplayParts {
        timestamp: Some(timestamp),
        level: Some(level),
        ..Default::default()
    };

    if let Some(sep) = rest.find(" - ") {
        let target = &rest[..sep];
        if !target.is_empty() {
            parts.target = Some(target.trim());
        }
        let msg = &rest[sep + 3..];
        if !msg.is_empty() {
            parts.message = Some(msg);
        }
    } else if !rest.is_empty() {
        parts.message = Some(rest);
    }

    Some(parts)
}

/// structlog: `DATETIME [level    ] msg key=val...`
fn try_structlog(s: &str) -> Option<DisplayParts<'_>> {
    let (timestamp, consumed) = parse_datetime_timestamp(s)?;

    let rest = s.get(consumed..)?.trim_start();

    // [level]
    if !rest.starts_with('[') {
        return None;
    }
    let close = rest.find(']')?;
    let level_token = rest[1..close].trim();
    let level = normalize_level(level_token)?;
    let rest = &rest[close + 1..].trim_start();

    let mut parts = DisplayParts {
        timestamp: Some(timestamp),
        level: Some(level),
        ..Default::default()
    };

    if !rest.is_empty() {
        parts.message = Some(rest);
    }

    Some(parts)
}

/// Generic fallback: `TIMESTAMP LEVEL rest-as-message`
/// Also handles: `TIMESTAMP LEVEL target: message` and `TIMESTAMP LEVEL target - message`
fn try_generic(s: &str) -> Option<DisplayParts<'_>> {
    // Try ISO timestamp first, then datetime
    let (timestamp, consumed) = parse_iso_timestamp(s).or_else(|| parse_datetime_timestamp(s))?;

    let rest = s.get(consumed..)?.trim_start();
    if rest.is_empty() {
        return None;
    }

    // Next token should be a level keyword
    let space = rest.find(' ').unwrap_or(rest.len());
    let level_token = &rest[..space];
    let level = normalize_level(level_token)?;

    let rest = if space < rest.len() {
        rest[space + 1..].trim_start()
    } else {
        ""
    };

    let mut parts = DisplayParts {
        timestamp: Some(timestamp),
        level: Some(level),
        ..Default::default()
    };

    if rest.is_empty() {
        return Some(parts);
    }

    // Try to extract target: "target:: msg", "target: msg", "target - msg"
    // Check for Rust module path (contains ::)
    if let Some(double_colon) = rest.find(":: ") {
        let target = &rest[..double_colon];
        // Validate it looks like a module path
        if !target.is_empty()
            && target
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':' || c == '-')
        {
            parts.target = Some(target);
            let msg = &rest[double_colon + 3..];
            if !msg.is_empty() {
                parts.message = Some(msg);
            }
            return Some(parts);
        }
    }

    // target: msg (single colon with space)
    if let Some(colon_space) = rest.find(": ") {
        let target_candidate = &rest[..colon_space];
        // Target should be a reasonable identifier (no spaces before colon for the
        // first word; allow module paths and simple names)
        let first_space = target_candidate.find(' ');
        if first_space.is_none()
            || target_candidate
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':' || c == '-' || c == '.')
        {
            if let Some(sp) = first_space {
                // If there's a space, only take the first word as target
                let t = &target_candidate[..sp];
                if !t.is_empty() && !is_level_keyword(t) {
                    parts.target = Some(t);
                    let msg = &rest[sp + 1..];
                    if !msg.is_empty() {
                        parts.message = Some(msg);
                    }
                    return Some(parts);
                }
            } else if !target_candidate.is_empty() && !is_level_keyword(target_candidate) {
                parts.target = Some(target_candidate);
                let msg = &rest[colon_space + 2..];
                if !msg.is_empty() {
                    parts.message = Some(msg);
                }
                return Some(parts);
            }
        }
    }

    // target - msg (dash separator)
    if let Some(dash_pos) = rest.find(" - ") {
        let target_candidate = &rest[..dash_pos];
        if !target_candidate.contains(' ')
            && !target_candidate.is_empty()
            && !is_level_keyword(target_candidate)
        {
            parts.target = Some(target_candidate);
            let msg = &rest[dash_pos + 3..];
            if !msg.is_empty() {
                parts.message = Some(msg);
            }
            return Some(parts);
        }
    }

    // No target detected — entire rest is message
    parts.message = Some(rest);
    Some(parts)
}

impl LogFormatParser for CommonLogParser {
    fn parse_line<'a>(&self, line: &'a [u8]) -> Option<DisplayParts<'a>> {
        let s = std::str::from_utf8(line).ok()?;
        if s.is_empty() {
            return None;
        }

        // Try sub-strategies in order from most specific to least
        if let Some(parts) = try_spring_boot(s) {
            return Some(parts);
        }
        if let Some(parts) = try_logback(s) {
            return Some(parts);
        }
        if let Some(parts) = try_loguru(s) {
            return Some(parts);
        }
        if let Some(parts) = try_structlog(s) {
            return Some(parts);
        }
        if let Some(parts) = try_env_logger(s) {
            return Some(parts);
        }
        if let Some(parts) = try_python_prod(s) {
            return Some(parts);
        }
        if let Some(parts) = try_generic(s) {
            return Some(parts);
        }
        if let Some(parts) = try_python_basic(s) {
            return Some(parts);
        }

        None
    }

    fn collect_field_names(&self, lines: &[&[u8]]) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut extras = Vec::new();
        let mut has_timestamp = false;
        let mut has_level = false;
        let mut has_target = false;
        let mut has_message = false;

        for &line in lines {
            if let Some(parts) = self.parse_line(line) {
                if parts.timestamp.is_some() {
                    has_timestamp = true;
                }
                if parts.level.is_some() {
                    has_level = true;
                }
                if parts.target.is_some() {
                    has_target = true;
                }
                if parts.message.is_some() {
                    has_message = true;
                }
                for (key, _) in &parts.extra_fields {
                    let k = key.to_string();
                    if seen.insert(k.clone()) {
                        extras.push(k);
                    }
                }
            }
        }

        let mut result = Vec::new();
        if has_timestamp {
            result.push("timestamp".to_string());
        }
        if has_level {
            result.push("level".to_string());
        }
        if has_target {
            result.push("target".to_string());
        }
        extras.sort();
        extras.dedup();
        result.extend(extras);
        if has_message {
            result.push("message".to_string());
        }
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
        // 0.95 penalty: yield to more specific parsers on ties
        (parsed as f64 / sample.len() as f64) * 0.95
    }

    fn name(&self) -> &str {
        "common-log"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── env_logger ────────────────────────────────────────────────────

    #[test]
    fn test_env_logger_iso() {
        let line = b"[2024-07-24T10:00:00Z INFO  myapp] Starting server";
        let parser = CommonLogParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024-07-24T10:00:00Z"));
        assert_eq!(parts.level, Some("INFO"));
        assert_eq!(parts.target, Some("myapp"));
        assert_eq!(parts.message, Some("Starting server"));
    }

    #[test]
    fn test_env_logger_no_timestamp() {
        let line = b"[WARN myapp::server] Connection timeout";
        let parser = CommonLogParser;
        let parts = parser.parse_line(line).unwrap();
        assert!(parts.timestamp.is_none());
        assert_eq!(parts.level, Some("WARN"));
        assert_eq!(parts.target, Some("myapp::server"));
        assert_eq!(parts.message, Some("Connection timeout"));
    }

    #[test]
    fn test_env_logger_level_only() {
        let line = b"[ERROR] Something failed";
        let parser = CommonLogParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("ERROR"));
        assert_eq!(parts.message, Some("Something failed"));
    }

    // ── tracing fmt / generic ─────────────────────────────────────────

    #[test]
    fn test_tracing_fmt_with_module_path() {
        let line = b"2024-07-24T10:00:00Z INFO myapp::server:: listening on 0.0.0.0:3000";
        let parser = CommonLogParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024-07-24T10:00:00Z"));
        assert_eq!(parts.level, Some("INFO"));
        assert_eq!(parts.target, Some("myapp::server"));
        assert_eq!(parts.message, Some("listening on 0.0.0.0:3000"));
    }

    #[test]
    fn test_generic_iso_level_message() {
        let line = b"2024-07-24T10:00:00Z ERROR database connection failed";
        let parser = CommonLogParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024-07-24T10:00:00Z"));
        assert_eq!(parts.level, Some("ERROR"));
        assert_eq!(parts.message, Some("database connection failed"));
    }

    #[test]
    fn test_generic_datetime_level_message() {
        let line = b"2024-07-24 10:00:00 INFO request processed";
        let parser = CommonLogParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024-07-24 10:00:00"));
        assert_eq!(parts.level, Some("INFO"));
        assert_eq!(parts.message, Some("request processed"));
    }

    #[test]
    fn test_generic_with_target_colon() {
        let line = b"2024-07-24T10:00:00Z WARN myapp: disk space low";
        let parser = CommonLogParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("WARN"));
        assert_eq!(parts.target, Some("myapp"));
        assert_eq!(parts.message, Some("disk space low"));
    }

    #[test]
    fn test_generic_with_target_dash() {
        let line = b"2024-07-24T10:00:00Z INFO myapp - starting up";
        let parser = CommonLogParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.target, Some("myapp"));
        assert_eq!(parts.message, Some("starting up"));
    }

    // ── logback/log4j2 ───────────────────────────────────────────────

    #[test]
    fn test_logback_basic() {
        let line = b"2024-07-24 10:00:00.123 [main] INFO  com.example.App - Application started";
        let parser = CommonLogParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024-07-24 10:00:00.123"));
        assert_eq!(parts.level, Some("INFO"));
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "thread" && *v == "main")
        );
        assert_eq!(parts.target, Some("com.example.App"));
        assert_eq!(parts.message, Some("Application started"));
    }

    #[test]
    fn test_logback_warn() {
        let line =
            b"2024-07-24 10:00:00,456 [http-nio-8080-exec-1] WARN  c.e.security.AuthFilter - Unauthorized access attempt";
        let parser = CommonLogParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024-07-24 10:00:00,456"));
        assert_eq!(parts.level, Some("WARN"));
        assert_eq!(parts.target, Some("c.e.security.AuthFilter"));
    }

    // ── Spring Boot ──────────────────────────────────────────────────

    #[test]
    fn test_spring_boot_basic() {
        let line = b"2024-07-24 10:00:00.123  INFO 12345 --- [           main] c.e.MyApp : Started MyApp in 2.5 seconds";
        let parser = CommonLogParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024-07-24 10:00:00.123"));
        assert_eq!(parts.level, Some("INFO"));
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "pid" && *v == "12345")
        );
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(k, v)| *k == "thread" && *v == "main")
        );
        assert_eq!(parts.target, Some("c.e.MyApp"));
        assert_eq!(parts.message, Some("Started MyApp in 2.5 seconds"));
    }

    #[test]
    fn test_spring_boot_warn() {
        let line = b"2024-07-24 10:00:00.123  WARN 99 --- [pool-1-thread-3] c.e.CacheService : Cache miss for key=abc";
        let parser = CommonLogParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("WARN"));
    }

    // ── Python formats ───────────────────────────────────────────────

    #[test]
    fn test_python_basic_level_target_msg() {
        let line = b"WARNING:django.server:Not Found: /favicon.ico";
        let parser = CommonLogParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("WARN"));
        assert_eq!(parts.target, Some("django.server"));
        assert_eq!(parts.message, Some("Not Found: /favicon.ico"));
    }

    #[test]
    fn test_python_basic_info() {
        let line = b"INFO:root:Application started";
        let parser = CommonLogParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("INFO"));
        assert_eq!(parts.target, Some("root"));
        assert_eq!(parts.message, Some("Application started"));
    }

    #[test]
    fn test_python_prod() {
        let line = b"2024-07-24 10:00:00,123 - myapp.views - INFO - Request handled successfully";
        let parser = CommonLogParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024-07-24 10:00:00,123"));
        assert_eq!(parts.level, Some("INFO"));
        assert_eq!(parts.target, Some("myapp.views"));
        assert_eq!(parts.message, Some("Request handled successfully"));
    }

    #[test]
    fn test_python_prod_error() {
        let line = b"2024-07-24 10:00:00,123 - myapp.db - ERROR - Connection refused to database";
        let parser = CommonLogParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("ERROR"));
        assert_eq!(parts.target, Some("myapp.db"));
    }

    // ── loguru ───────────────────────────────────────────────────────

    #[test]
    fn test_loguru_basic() {
        let line = b"2024-07-24 10:00:00.123 | INFO     | myapp.main - Starting application";
        let parser = CommonLogParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024-07-24 10:00:00.123"));
        assert_eq!(parts.level, Some("INFO"));
        assert_eq!(parts.target, Some("myapp.main"));
        assert_eq!(parts.message, Some("Starting application"));
    }

    #[test]
    fn test_loguru_debug() {
        let line = b"2024-07-24 10:00:00.123 | DEBUG    | module:func:42 - Processing item";
        let parser = CommonLogParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("DEBUG"));
    }

    // ── structlog ────────────────────────────────────────────────────

    #[test]
    fn test_structlog_basic() {
        let line = b"2024-07-24 10:00:00 [info     ] request handled              key=val";
        let parser = CommonLogParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024-07-24 10:00:00"));
        assert_eq!(parts.level, Some("INFO"));
        assert!(parts.message.is_some());
    }

    #[test]
    fn test_structlog_warning() {
        let line = b"2024-07-24 10:00:00 [warning  ] cache miss                   key=abc";
        let parser = CommonLogParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("WARN"));
    }

    // ── Extended levels ──────────────────────────────────────────────

    #[test]
    fn test_trace_level() {
        let line = b"2024-07-24T10:00:00Z TRACE entering function";
        let parser = CommonLogParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("TRACE"));
    }

    #[test]
    fn test_fatal_level() {
        let line = b"2024-07-24T10:00:00Z FATAL system crash";
        let parser = CommonLogParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("FATAL"));
    }

    #[test]
    fn test_critical_level() {
        let line = b"2024-07-24T10:00:00Z CRITICAL out of memory";
        let parser = CommonLogParser;
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("FATAL"));
    }

    // ── Negative cases ───────────────────────────────────────────────

    #[test]
    fn test_parse_empty() {
        let parser = CommonLogParser;
        assert!(parser.parse_line(b"").is_none());
    }

    #[test]
    fn test_parse_plain_text() {
        let parser = CommonLogParser;
        assert!(parser.parse_line(b"just some random text").is_none());
    }

    #[test]
    fn test_parse_json_not_common_log() {
        let parser = CommonLogParser;
        assert!(
            parser
                .parse_line(br#"{"level":"INFO","msg":"hello"}"#)
                .is_none()
        );
    }

    #[test]
    fn test_parse_no_level_keyword() {
        // Timestamp but no level keyword — should NOT match
        let parser = CommonLogParser;
        assert!(
            parser
                .parse_line(b"2024-07-24T10:00:00Z myhost sshd: accepted")
                .is_none()
        );
    }

    // ── detect_score ─────────────────────────────────────────────────

    #[test]
    fn test_detect_score_all_common() {
        let parser = CommonLogParser;
        let lines: Vec<&[u8]> = vec![
            b"2024-07-24T10:00:00Z INFO msg1",
            b"2024-07-24T10:00:01Z WARN msg2",
        ];
        let score = parser.detect_score(&lines);
        // Should be 0.95 (penalty factor)
        assert!((score - 0.95).abs() < 0.001, "Got {}", score);
    }

    #[test]
    fn test_detect_score_mixed() {
        let parser = CommonLogParser;
        let lines: Vec<&[u8]> = vec![b"2024-07-24T10:00:00Z INFO msg1", b"plain text"];
        let score = parser.detect_score(&lines);
        assert!((score - 0.475).abs() < 0.001, "Got {}", score);
    }

    #[test]
    fn test_detect_score_none() {
        let parser = CommonLogParser;
        let lines: Vec<&[u8]> = vec![b"plain text", b"more text"];
        let score = parser.detect_score(&lines);
        assert!((score - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_detect_score_empty() {
        let parser = CommonLogParser;
        let lines: Vec<&[u8]> = vec![];
        let score = parser.detect_score(&lines);
        assert!((score - 0.0).abs() < 0.001);
    }

    // ── collect_field_names ──────────────────────────────────────────

    #[test]
    fn test_collect_field_names() {
        let parser = CommonLogParser;
        let lines: Vec<&[u8]> = vec![
            b"2024-07-24T10:00:00Z INFO myapp: hello",
            b"2024-07-24T10:00:01Z WARN myapp: world",
        ];
        let names = parser.collect_field_names(&lines);
        assert!(names.contains(&"timestamp".to_string()));
        assert!(names.contains(&"level".to_string()));
        assert!(names.contains(&"target".to_string()));
        assert!(names.contains(&"message".to_string()));
    }

    #[test]
    fn test_collect_field_names_logback() {
        let parser = CommonLogParser;
        let lines: Vec<&[u8]> =
            vec![b"2024-07-24 10:00:00.123 [main] INFO com.example.App - started"];
        let names = parser.collect_field_names(&lines);
        assert!(names.contains(&"thread".to_string()));
    }

    // ── name ─────────────────────────────────────────────────────────

    #[test]
    fn test_name() {
        let parser = CommonLogParser;
        assert_eq!(parser.name(), "common-log");
    }

    // ── Priority: CommonLogParser should NOT match journalctl lines ──

    #[test]
    fn test_journalctl_line_no_level_rejected() {
        let parser = CommonLogParser;
        // Journalctl: timestamp + hostname + unit — no level keyword
        assert!(
            parser
                .parse_line(b"2024-02-22T10:15:30+0000 myhost sshd[1234]: Accepted password")
                .is_none()
        );
    }

    #[test]
    fn test_syslog_bsd_rejected() {
        let parser = CommonLogParser;
        // BSD syslog without level
        assert!(
            parser
                .parse_line(b"Oct 11 22:14:15 myhost sshd[1234]: msg")
                .is_none()
        );
    }
}
