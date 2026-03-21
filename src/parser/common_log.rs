//! Catch-all parser for the TIMESTAMP + LEVEL + TARGET + MESSAGE log family.
//! Covers env_logger, tracing fmt, logback, Spring Boot, Python, loguru, structlog.

use std::collections::HashSet;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

use super::timestamp::{
    is_level_keyword, normalize_level, parse_datetime_timestamp, parse_iso_timestamp,
    parse_nano_timestamp,
};
use super::types::{DisplayParts, FieldSemantic, LogFormatParser, SpanInfo, push_field_as};

#[derive(Debug, Clone, Copy)]
enum LineParser {
    SpringBoot,
    Logback,
    Loguru,
    Structlog,
    EnvLogger,
    PythonProd,
    TracingFmt,
    Generic,
    PythonBasic,
}

impl LineParser {
    fn index(self) -> usize {
        self as usize
    }

    fn from_index(i: usize) -> Self {
        match i {
            0 => Self::SpringBoot,
            1 => Self::Logback,
            2 => Self::Loguru,
            3 => Self::Structlog,
            4 => Self::EnvLogger,
            5 => Self::PythonProd,
            6 => Self::TracingFmt,
            7 => Self::Generic,
            _ => Self::PythonBasic,
        }
    }
}

const MIN_SAMPLES: u32 = 50;

#[derive(Debug, Clone, Copy)]
enum CommonFormat {
    BracketIso,
    BracketDatetime,
    Iso,
    Datetime,
    Nano,
}

fn detect_common_timestamp(s: &str) -> Option<(CommonFormat, &str)> {
    if let Some(inner) = s.strip_prefix('[') {
        if let Some((ts, _)) = parse_iso_timestamp(inner) {
            return Some((CommonFormat::BracketIso, ts));
        }
        if let Some((ts, _)) = parse_datetime_timestamp(inner) {
            return Some((CommonFormat::BracketDatetime, ts));
        }
        return None;
    }
    if let Some((ts, _)) = parse_iso_timestamp(s) {
        return Some((CommonFormat::Iso, ts));
    }
    if let Some((ts, _)) = parse_datetime_timestamp(s) {
        return Some((CommonFormat::Datetime, ts));
    }
    parse_nano_timestamp(s).map(|(ts, _)| (CommonFormat::Nano, ts))
}

fn extract_common_timestamp(s: &str, fmt: CommonFormat) -> Option<&str> {
    match fmt {
        CommonFormat::BracketIso => {
            let inner = s.strip_prefix('[')?;
            parse_iso_timestamp(inner).map(|(ts, _)| ts)
        }
        CommonFormat::BracketDatetime => {
            let inner = s.strip_prefix('[')?;
            parse_datetime_timestamp(inner).map(|(ts, _)| ts)
        }
        CommonFormat::Iso => parse_iso_timestamp(s).map(|(ts, _)| ts),
        CommonFormat::Datetime => parse_datetime_timestamp(s).map(|(ts, _)| ts),
        CommonFormat::Nano => parse_nano_timestamp(s).map(|(ts, _)| ts),
    }
}

#[derive(Debug, Default)]
pub struct CommonLogParser {
    format: OnceLock<CommonFormat>,
    line_parser: OnceLock<LineParser>,
    lp_counts: [AtomicU32; 9],
    lp_total: AtomicU32,
}
fn try_env_logger(s: &str) -> Option<DisplayParts<'_>> {
    if !s.starts_with('[') {
        return None;
    }
    let close = s.find(']')?;
    let bracket_content = &s[1..close];
    let rest = &s[close + 1..];
    let rest = rest.strip_prefix(' ').unwrap_or(rest);

    let mut tokens = bracket_content.split_whitespace();
    let first = tokens.next()?;

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
    let msg = if rest.is_empty() && !remaining.is_empty() {
        return Some(parts);
    } else {
        rest
    };
    if !msg.is_empty() {
        parts.message = Some(msg);
    }
    Some(parts)
}

fn try_parse_span_prefix(s: &str) -> (Option<SpanInfo<'_>>, &str) {
    let brace = match s.find('{') {
        Some(p) => p,
        None => return (None, s),
    };
    let name = &s[..brace];
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return (None, s);
    }
    let after_open = brace + 1;
    let close = match s[after_open..].find('}') {
        Some(p) => after_open + p,
        None => return (None, s),
    };
    let after = &s[close + 1..];
    if !after.starts_with(": ") {
        return (None, s);
    }
    let fields = parse_tracing_span_fields(&s[after_open..close]);
    (Some(SpanInfo { name, fields }), &after[2..])
}

fn parse_tracing_span_fields(s: &str) -> Vec<(&str, &str)> {
    let mut fields = Vec::new();
    let mut rest = s.trim();
    while !rest.is_empty() {
        let eq = match rest.find('=') {
            Some(p) => p,
            None => break,
        };
        let key = rest[..eq].trim();
        if key.is_empty() {
            break;
        }
        let val_src = &rest[eq + 1..];
        let (val, consumed) = if let Some(inner) = val_src.strip_prefix('"') {
            match inner.find('"') {
                Some(close) => (&val_src[1..1 + close], 1 + close + 1),
                None => (inner, val_src.len()),
            }
        } else {
            match val_src.find(' ') {
                Some(sp) => (&val_src[..sp], sp),
                None => (val_src, val_src.len()),
            }
        };
        fields.push((key, val));
        rest = rest[eq + 1 + consumed..].trim_start();
    }
    fields
}

fn try_tracing_fmt(s: &str) -> Option<DisplayParts<'_>> {
    let (timestamp, consumed) = parse_iso_timestamp(s)?;
    let rest = s.get(consumed..)?.trim_start();
    let space = rest.find(' ')?;
    let level = normalize_level(&rest[..space])?;
    let rest = rest[space..].trim_start();

    let (span, rest) = try_parse_span_prefix(rest);
    let span = span?;

    let mut parts = DisplayParts {
        timestamp: Some(timestamp),
        level: Some(level),
        span: Some(span),
        ..Default::default()
    };

    if let Some(colon) = rest.find(": ") {
        let target = &rest[..colon];
        if !target.is_empty() {
            parts.target = Some(target);
        }
        let msg = &rest[colon + 2..];
        if !msg.is_empty() {
            parts.message = Some(msg);
        }
    } else if !rest.is_empty() {
        parts.message = Some(rest);
    }

    Some(parts)
}

fn try_logback(s: &str) -> Option<DisplayParts<'_>> {
    let (timestamp, consumed) = parse_datetime_timestamp(s)?;

    let rest = s.get(consumed..)?.trim_start();
    if rest.is_empty() {
        return None;
    }

    if !rest.starts_with('[') {
        return None;
    }
    let close = rest.find(']')?;
    let thread = &rest[1..close];
    let rest = &rest[close + 1..].trim_start();

    let space = rest.find(' ')?;
    let level = normalize_level(&rest[..space])?;
    let rest = rest[space + 1..].trim_start();

    let mut parts = DisplayParts {
        timestamp: Some(timestamp),
        level: Some(level),
        ..Default::default()
    };
    push_field_as(&mut parts.extra_fields, FieldSemantic::Thread, thread);

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

fn try_spring_boot(s: &str) -> Option<DisplayParts<'_>> {
    let (timestamp, consumed) = parse_datetime_timestamp(s)?;

    let rest = s.get(consumed..)?.trim_start();
    if rest.is_empty() {
        return None;
    }

    let space = rest.find(' ')?;
    let level = normalize_level(&rest[..space])?;
    let rest = rest[space + 1..].trim_start();

    let space = rest.find(' ')?;
    let pid = &rest[..space];
    if !pid.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let rest = rest[space + 1..].trim_start();

    if !rest.starts_with("--- ") {
        return None;
    }
    let rest = &rest[4..];

    if !rest.starts_with('[') {
        return None;
    }
    let close = rest.find(']')?;
    let thread = &rest[1..close];
    let rest = rest[close + 1..].trim_start();

    let mut parts = DisplayParts {
        timestamp: Some(timestamp),
        level: Some(level),
        ..Default::default()
    };
    push_field_as(&mut parts.extra_fields, FieldSemantic::Pid, pid);
    push_field_as(
        &mut parts.extra_fields,
        FieldSemantic::Thread,
        thread.trim(),
    );

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

fn try_python_basic(s: &str) -> Option<DisplayParts<'_>> {
    let colon1 = s.find(':')?;
    let level_token = &s[..colon1];
    let level = normalize_level(level_token)?;
    let rest = &s[colon1 + 1..];

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

    let segments: Vec<&str> = rest.splitn(4, " - ").collect();
    if segments.len() < 3 {
        return None;
    }

    let level = normalize_level(segments[1])?;

    let mut parts = DisplayParts {
        timestamp: Some(timestamp),
        level: Some(level),
        ..Default::default()
    };

    if !segments[0].is_empty() {
        parts.target = Some(segments[0]);
    }

    // Find the message start as a slice into `rest` to stay zero-copy
    let msg_offset = segments[0].len() + 3 + segments[1].len() + 3;
    let msg = &rest[msg_offset..];
    if !msg.is_empty() {
        parts.message = Some(msg);
    }

    Some(parts)
}

fn try_loguru(s: &str) -> Option<DisplayParts<'_>> {
    let (timestamp, consumed) = parse_datetime_timestamp(s)?;

    let rest = s.get(consumed..)?;
    let rest = rest.strip_prefix(" | ")?;

    let pipe = rest.find(" | ")?;
    let level_token = rest[..pipe].trim();
    let level = normalize_level(level_token)?;
    let rest = &rest[pipe + 3..];

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

fn try_structlog(s: &str) -> Option<DisplayParts<'_>> {
    let (timestamp, consumed) = parse_datetime_timestamp(s)?;

    let rest = s.get(consumed..)?.trim_start();

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

fn try_generic(s: &str) -> Option<DisplayParts<'_>> {
    let (timestamp, consumed) = parse_iso_timestamp(s)
        .or_else(|| parse_datetime_timestamp(s))
        .or_else(|| parse_nano_timestamp(s))?;

    let rest = s.get(consumed..)?.trim_start();
    if rest.is_empty() {
        return None;
    }

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

    if let Some(double_colon) = rest.find(":: ") {
        let target = &rest[..double_colon];
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

    if let Some(colon_space) = rest.find(": ") {
        let target_candidate = &rest[..colon_space];
        let first_space = target_candidate.find(' ');
        if first_space.is_none()
            || target_candidate
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':' || c == '-' || c == '.')
        {
            if let Some(sp) = first_space {
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

    parts.message = Some(rest);
    Some(parts)
}

impl CommonLogParser {
    fn record_line_parser(&self, lp: LineParser) {
        self.lp_counts[lp.index()].fetch_add(1, Ordering::Relaxed);
        let total = self.lp_total.fetch_add(1, Ordering::Relaxed) + 1;
        if total >= MIN_SAMPLES && self.line_parser.get().is_none() {
            let winner = (0..9)
                .max_by_key(|&i| self.lp_counts[i].load(Ordering::Relaxed))
                .unwrap_or(0);
            let _ = self.line_parser.set(LineParser::from_index(winner));
        }
    }
}

impl LogFormatParser for CommonLogParser {
    fn parse_timestamp<'a>(&self, line: &'a [u8]) -> Option<&'a str> {
        let s = std::str::from_utf8(line).ok()?;
        if s.is_empty() {
            return None;
        }
        if let Some(&fmt) = self.format.get() {
            return extract_common_timestamp(s, fmt);
        }
        let (fmt, ts) = detect_common_timestamp(s)?;
        let _ = self.format.set(fmt);
        Some(ts)
    }

    fn parse_line<'a>(&self, line: &'a [u8]) -> Option<DisplayParts<'a>> {
        let s = std::str::from_utf8(line).ok()?;
        if s.is_empty() {
            return None;
        }

        if let Some(&winner) = self.line_parser.get() {
            let result = match winner {
                LineParser::SpringBoot => try_spring_boot(s),
                LineParser::Logback => try_logback(s),
                LineParser::Loguru => try_loguru(s),
                LineParser::Structlog => try_structlog(s),
                LineParser::EnvLogger => try_env_logger(s),
                LineParser::PythonProd => try_python_prod(s),
                LineParser::TracingFmt => try_tracing_fmt(s),
                LineParser::Generic => try_generic(s),
                LineParser::PythonBasic => try_python_basic(s),
            };
            if result.is_some() {
                return result;
            }
        }

        type ParserEntry = (
            &'static dyn for<'s> Fn(&'s str) -> Option<DisplayParts<'s>>,
            LineParser,
        );
        let parsers: &[ParserEntry] = &[
            (&try_spring_boot, LineParser::SpringBoot),
            (&try_logback, LineParser::Logback),
            (&try_loguru, LineParser::Loguru),
            (&try_structlog, LineParser::Structlog),
            (&try_env_logger, LineParser::EnvLogger),
            (&try_python_prod, LineParser::PythonProd),
            (&try_tracing_fmt, LineParser::TracingFmt),
            (&try_generic, LineParser::Generic),
            (&try_python_basic, LineParser::PythonBasic),
        ];

        for (f, variant) in parsers {
            if let Some(parts) = f(s) {
                self.record_line_parser(*variant);
                return Some(parts);
            }
        }

        None
    }

    fn collect_field_names(&self, lines: &[&[u8]]) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut extras = Vec::new();
        let mut has_timestamp = false;
        let mut has_level = false;
        let mut has_target = false;
        let mut has_span = false;
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
                if let Some(ref span) = parts.span {
                    has_span = true;
                    for (key, _) in &span.fields {
                        let dotted = format!("span.{key}");
                        if seen.insert(dotted.clone()) {
                            extras.push(dotted);
                        }
                    }
                }
                if parts.message.is_some() {
                    has_message = true;
                }
                for (_, key, _) in &parts.extra_fields {
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
        if has_span {
            result.push("span".to_string());
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
        (parsed as f64 / sample.len() as f64) * 0.95
    }

    fn detection_weight(&self) -> f64 {
        0.95
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
        let parser = CommonLogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024-07-24T10:00:00Z"));
        assert_eq!(parts.level, Some("INFO"));
        assert_eq!(parts.target, Some("myapp"));
        assert_eq!(parts.message, Some("Starting server"));
    }

    #[test]
    fn test_env_logger_no_timestamp() {
        let line = b"[WARN myapp::server] Connection timeout";
        let parser = CommonLogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert!(parts.timestamp.is_none());
        assert_eq!(parts.level, Some("WARN"));
        assert_eq!(parts.target, Some("myapp::server"));
        assert_eq!(parts.message, Some("Connection timeout"));
    }

    #[test]
    fn test_env_logger_level_only() {
        let line = b"[ERROR] Something failed";
        let parser = CommonLogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("ERROR"));
        assert_eq!(parts.message, Some("Something failed"));
    }

    // ── tracing fmt / generic ─────────────────────────────────────────

    #[test]
    fn test_tracing_fmt_with_module_path() {
        let line = b"2024-07-24T10:00:00Z INFO myapp::server:: listening on 0.0.0.0:3000";
        let parser = CommonLogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024-07-24T10:00:00Z"));
        assert_eq!(parts.level, Some("INFO"));
        assert_eq!(parts.target, Some("myapp::server"));
        assert_eq!(parts.message, Some("listening on 0.0.0.0:3000"));
    }

    #[test]
    fn test_generic_iso_level_message() {
        let line = b"2024-07-24T10:00:00Z ERROR database connection failed";
        let parser = CommonLogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024-07-24T10:00:00Z"));
        assert_eq!(parts.level, Some("ERROR"));
        assert_eq!(parts.message, Some("database connection failed"));
    }

    #[test]
    fn test_generic_datetime_level_message() {
        let line = b"2024-07-24 10:00:00 INFO request processed";
        let parser = CommonLogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024-07-24 10:00:00"));
        assert_eq!(parts.level, Some("INFO"));
        assert_eq!(parts.message, Some("request processed"));
    }

    #[test]
    fn test_generic_with_target_colon() {
        let line = b"2024-07-24T10:00:00Z WARN myapp: disk space low";
        let parser = CommonLogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("WARN"));
        assert_eq!(parts.target, Some("myapp"));
        assert_eq!(parts.message, Some("disk space low"));
    }

    #[test]
    fn test_generic_with_target_dash() {
        let line = b"2024-07-24T10:00:00Z INFO myapp - starting up";
        let parser = CommonLogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.target, Some("myapp"));
        assert_eq!(parts.message, Some("starting up"));
    }

    // ── logback/log4j2 ───────────────────────────────────────────────

    #[test]
    fn test_logback_basic() {
        let line = b"2024-07-24 10:00:00.123 [main] INFO  com.example.App - Application started";
        let parser = CommonLogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024-07-24 10:00:00.123"));
        assert_eq!(parts.level, Some("INFO"));
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(_, k, v)| *k == "thread" && *v == "main")
        );
        assert_eq!(parts.target, Some("com.example.App"));
        assert_eq!(parts.message, Some("Application started"));
    }

    #[test]
    fn test_logback_warn() {
        let line =
            b"2024-07-24 10:00:00,456 [http-nio-8080-exec-1] WARN  c.e.security.AuthFilter - Unauthorized access attempt";
        let parser = CommonLogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024-07-24 10:00:00,456"));
        assert_eq!(parts.level, Some("WARN"));
        assert_eq!(parts.target, Some("c.e.security.AuthFilter"));
    }

    // ── Spring Boot ──────────────────────────────────────────────────

    #[test]
    fn test_spring_boot_basic() {
        let line = b"2024-07-24 10:00:00.123  INFO 12345 --- [           main] c.e.MyApp : Started MyApp in 2.5 seconds";
        let parser = CommonLogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024-07-24 10:00:00.123"));
        assert_eq!(parts.level, Some("INFO"));
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(_, k, v)| *k == "pid" && *v == "12345")
        );
        assert!(
            parts
                .extra_fields
                .iter()
                .any(|(_, k, v)| *k == "thread" && *v == "main")
        );
        assert_eq!(parts.target, Some("c.e.MyApp"));
        assert_eq!(parts.message, Some("Started MyApp in 2.5 seconds"));
    }

    #[test]
    fn test_spring_boot_warn() {
        let line = b"2024-07-24 10:00:00.123  WARN 99 --- [pool-1-thread-3] c.e.CacheService : Cache miss for key=abc";
        let parser = CommonLogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("WARN"));
    }

    // ── Python formats ───────────────────────────────────────────────

    #[test]
    fn test_python_basic_level_target_msg() {
        let line = b"WARNING:django.server:Not Found: /favicon.ico";
        let parser = CommonLogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("WARN"));
        assert_eq!(parts.target, Some("django.server"));
        assert_eq!(parts.message, Some("Not Found: /favicon.ico"));
    }

    #[test]
    fn test_python_basic_info() {
        let line = b"INFO:root:Application started";
        let parser = CommonLogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("INFO"));
        assert_eq!(parts.target, Some("root"));
        assert_eq!(parts.message, Some("Application started"));
    }

    #[test]
    fn test_python_prod() {
        let line = b"2024-07-24 10:00:00,123 - myapp.views - INFO - Request handled successfully";
        let parser = CommonLogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024-07-24 10:00:00,123"));
        assert_eq!(parts.level, Some("INFO"));
        assert_eq!(parts.target, Some("myapp.views"));
        assert_eq!(parts.message, Some("Request handled successfully"));
    }

    #[test]
    fn test_python_prod_error() {
        let line = b"2024-07-24 10:00:00,123 - myapp.db - ERROR - Connection refused to database";
        let parser = CommonLogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("ERROR"));
        assert_eq!(parts.target, Some("myapp.db"));
    }

    // ── loguru ───────────────────────────────────────────────────────

    #[test]
    fn test_loguru_basic() {
        let line = b"2024-07-24 10:00:00.123 | INFO     | myapp.main - Starting application";
        let parser = CommonLogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024-07-24 10:00:00.123"));
        assert_eq!(parts.level, Some("INFO"));
        assert_eq!(parts.target, Some("myapp.main"));
        assert_eq!(parts.message, Some("Starting application"));
    }

    #[test]
    fn test_loguru_debug() {
        let line = b"2024-07-24 10:00:00.123 | DEBUG    | module:func:42 - Processing item";
        let parser = CommonLogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("DEBUG"));
    }

    // ── structlog ────────────────────────────────────────────────────

    #[test]
    fn test_structlog_basic() {
        let line = b"2024-07-24 10:00:00 [info     ] request handled              key=val";
        let parser = CommonLogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2024-07-24 10:00:00"));
        assert_eq!(parts.level, Some("INFO"));
        assert!(parts.message.is_some());
    }

    #[test]
    fn test_structlog_warning() {
        let line = b"2024-07-24 10:00:00 [warning  ] cache miss                   key=abc";
        let parser = CommonLogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("WARN"));
    }

    // ── Extended levels ──────────────────────────────────────────────

    #[test]
    fn test_trace_level() {
        let line = b"2024-07-24T10:00:00Z TRACE entering function";
        let parser = CommonLogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("TRACE"));
    }

    #[test]
    fn test_fatal_level() {
        let line = b"2024-07-24T10:00:00Z FATAL system crash";
        let parser = CommonLogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("FATAL"));
    }

    #[test]
    fn test_critical_level() {
        let line = b"2024-07-24T10:00:00Z CRITICAL out of memory";
        let parser = CommonLogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("FATAL"));
    }

    // ── Negative cases ───────────────────────────────────────────────

    #[test]
    fn test_parse_empty() {
        let parser = CommonLogParser::default();
        assert!(parser.parse_line(b"").is_none());
    }

    #[test]
    fn test_parse_plain_text() {
        let parser = CommonLogParser::default();
        assert!(parser.parse_line(b"just some random text").is_none());
    }

    #[test]
    fn test_parse_json_not_common_log() {
        let parser = CommonLogParser::default();
        assert!(
            parser
                .parse_line(br#"{"level":"INFO","msg":"hello"}"#)
                .is_none()
        );
    }

    #[test]
    fn test_parse_no_level_keyword() {
        // Timestamp but no level keyword — should NOT match
        let parser = CommonLogParser::default();
        assert!(
            parser
                .parse_line(b"2024-07-24T10:00:00Z myhost sshd: accepted")
                .is_none()
        );
    }

    // ── detect_score ─────────────────────────────────────────────────

    #[test]
    fn test_detect_score_all_common() {
        let parser = CommonLogParser::default();
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
        let parser = CommonLogParser::default();
        let lines: Vec<&[u8]> = vec![b"2024-07-24T10:00:00Z INFO msg1", b"plain text"];
        let score = parser.detect_score(&lines);
        assert!((score - 0.475).abs() < 0.001, "Got {}", score);
    }

    #[test]
    fn test_detect_score_none() {
        let parser = CommonLogParser::default();
        let lines: Vec<&[u8]> = vec![b"plain text", b"more text"];
        let score = parser.detect_score(&lines);
        assert!((score - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_detect_score_empty() {
        let parser = CommonLogParser::default();
        let lines: Vec<&[u8]> = vec![];
        let score = parser.detect_score(&lines);
        assert!((score - 0.0).abs() < 0.001);
    }

    // ── collect_field_names ──────────────────────────────────────────

    #[test]
    fn test_collect_field_names() {
        let parser = CommonLogParser::default();
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
        let parser = CommonLogParser::default();
        let lines: Vec<&[u8]> =
            vec![b"2024-07-24 10:00:00.123 [main] INFO com.example.App - started"];
        let names = parser.collect_field_names(&lines);
        assert!(names.contains(&"thread".to_string()));
    }

    // ── nano timestamp ───────────────────────────────────────────────

    #[test]
    fn test_nano_timestamp_level_target_message() {
        let line = b"1700046000000000000 INFO  api-gateway server started on 0.0.0.0:8080";
        let parser = CommonLogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("1700046000000000000"));
        assert_eq!(parts.level, Some("INFO"));
        assert!(parts.message.is_some());
    }

    #[test]
    fn test_nano_timestamp_with_service_and_host() {
        let line = b"1700046001123000000 INFO  api-gateway prod-host-01 request received";
        let parser = CommonLogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("1700046001123000000"));
        assert_eq!(parts.level, Some("INFO"));
    }

    // ── name ─────────────────────────────────────────────────────────

    #[test]
    fn test_name() {
        let parser = CommonLogParser::default();
        assert_eq!(parser.name(), "common-log");
    }

    // ── tracing-subscriber span helpers ──────────────────────────────

    #[test]
    fn test_parse_tracing_span_fields_unquoted() {
        let fields =
            parse_tracing_span_fields("method=GET uri=/api/store-settings version=HTTP/1.1");
        assert_eq!(
            fields,
            vec![
                ("method", "GET"),
                ("uri", "/api/store-settings"),
                ("version", "HTTP/1.1"),
            ]
        );
    }

    #[test]
    fn test_parse_tracing_span_fields_quoted() {
        let fields = parse_tracing_span_fields(r#"id="0.5" name="payments""#);
        assert_eq!(fields, vec![("id", "0.5"), ("name", "payments")]);
    }

    #[test]
    fn test_parse_tracing_span_fields_empty() {
        let fields = parse_tracing_span_fields("");
        assert!(fields.is_empty());
    }

    #[test]
    fn test_try_parse_span_prefix_unquoted() {
        let s = r#"request{method=GET uri=/api/items version=HTTP/1.1}: tower_http: started"#;
        let (span, rest) = try_parse_span_prefix(s);
        let span = span.unwrap();
        assert_eq!(span.name, "request");
        assert_eq!(
            span.fields,
            vec![
                ("method", "GET"),
                ("uri", "/api/items"),
                ("version", "HTTP/1.1"),
            ]
        );
        assert_eq!(rest, "tower_http: started");
    }

    #[test]
    fn test_try_parse_span_prefix_quoted() {
        let s = r#"Actor{id="0.5" name="payments"}: api_server::actors: msg"#;
        let (span, rest) = try_parse_span_prefix(s);
        let span = span.unwrap();
        assert_eq!(span.name, "Actor");
        assert_eq!(span.fields, vec![("id", "0.5"), ("name", "payments")]);
        assert_eq!(rest, "api_server::actors: msg");
    }

    #[test]
    fn test_try_parse_span_prefix_no_brace() {
        let (span, rest) = try_parse_span_prefix("tower_http::trace: started");
        assert!(span.is_none());
        assert_eq!(rest, "tower_http::trace: started");
    }

    #[test]
    fn test_try_parse_span_prefix_no_colon_space_after_brace() {
        // `}` not followed by `: ` — not a span prefix
        let (span, rest) = try_parse_span_prefix("something{foo=bar}message");
        assert!(span.is_none());
        assert_eq!(rest, "something{foo=bar}message");
    }

    // ── try_tracing_fmt ───────────────────────────────────────────────

    #[test]
    fn test_tracing_fmt_request_span() {
        let line = b"2026-03-05T10:55:16.661990Z DEBUG request{method=GET uri=/api/store-settings version=HTTP/1.1}: tower_http::trace::on_request: started processing request";
        let parser = CommonLogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2026-03-05T10:55:16.661990Z"));
        assert_eq!(parts.level, Some("DEBUG"));
        assert_eq!(parts.target, Some("tower_http::trace::on_request"));
        assert_eq!(parts.message, Some("started processing request"));
        let span = parts.span.unwrap();
        assert_eq!(span.name, "request");
        assert_eq!(
            span.fields,
            vec![
                ("method", "GET"),
                ("uri", "/api/store-settings"),
                ("version", "HTTP/1.1"),
            ]
        );
    }

    #[test]
    fn test_tracing_fmt_actor_span_quoted_fields() {
        let line = br#"2026-03-05T10:44:59.381757Z DEBUG Actor{id="0.5" name="payments"}: api_server::actors::payment: PaymentMsg::CheckExpiredPix"#;
        let parser = CommonLogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.level, Some("DEBUG"));
        assert_eq!(parts.target, Some("api_server::actors::payment"));
        assert_eq!(parts.message, Some("PaymentMsg::CheckExpiredPix"));
        let span = parts.span.unwrap();
        assert_eq!(span.name, "Actor");
        assert_eq!(span.fields, vec![("id", "0.5"), ("name", "payments")]);
    }

    #[test]
    fn test_tracing_fmt_no_span_still_parses() {
        // Startup lines without a span still parse via try_generic
        let line =
            b"2026-03-05T10:44:59.378731Z INFO  api_server API server listening on 0.0.0.0:3001";
        let parser = CommonLogParser::default();
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2026-03-05T10:44:59.378731Z"));
        assert_eq!(parts.level, Some("INFO"));
        assert!(parts.span.is_none());
    }

    #[test]
    fn test_collect_field_names_includes_span_dotted() {
        let parser = CommonLogParser::default();
        let lines: Vec<&[u8]> = vec![
            b"2026-03-05T10:55:16.661990Z DEBUG request{method=GET uri=/api/store-settings version=HTTP/1.1}: tower_http::trace::on_request: started processing request",
            b"2026-03-05T10:55:16.662071Z DEBUG request{method=GET uri=/api/store-settings version=HTTP/1.1}: api_server::routes::catalog: get_store_settings",
        ];
        let names = parser.collect_field_names(&lines);
        assert!(names.contains(&"span".to_string()));
        assert!(names.contains(&"span.method".to_string()));
        assert!(names.contains(&"span.uri".to_string()));
        assert!(names.contains(&"span.version".to_string()));
        assert!(names.contains(&"target".to_string()));
        assert!(names.contains(&"message".to_string()));
    }

    #[test]
    fn test_detect_score_mixed_startup_and_span_lines() {
        // Format detection from startup-only sample should still yield common-log,
        // and span lines should parse once the parser is applied to the full log.
        let parser = CommonLogParser::default();
        let startup: Vec<&[u8]> = vec![
            b"2026-03-05T10:44:59.378731Z INFO  api_server SMTP_HOST not set",
            b"2026-03-05T10:44:59.382274Z INFO  api_server API server listening on 0.0.0.0:3001",
        ];
        let score = parser.detect_score(&startup);
        assert!(score > 0.0, "startup lines should yield positive score");

        // Span lines also parse correctly
        let span_line = b"2026-03-05T10:55:16.661990Z DEBUG request{method=GET uri=/api/store-settings version=HTTP/1.1}: tower_http::trace::on_request: started processing request";
        assert!(parser.parse_line(span_line).is_some());
    }

    // ── Priority: CommonLogParser should NOT match journalctl lines ──

    #[test]
    fn test_journalctl_line_no_level_rejected() {
        let parser = CommonLogParser::default();
        // Journalctl: timestamp + hostname + unit — no level keyword
        assert!(
            parser
                .parse_line(b"2024-02-22T10:15:30+0000 myhost sshd[1234]: Accepted password")
                .is_none()
        );
    }

    #[test]
    fn test_syslog_bsd_rejected() {
        let parser = CommonLogParser::default();
        // BSD syslog without level
        assert!(
            parser
                .parse_line(b"Oct 11 22:14:15 myhost sshd[1234]: msg")
                .is_none()
        );
    }

    // ── parse_timestamp ────────────────────────────────────────────────

    #[test]
    fn test_parse_timestamp_iso_prefix() {
        let line = b"2024-02-22T10:15:30Z INFO app: message";
        let parser = CommonLogParser::default();
        assert_eq!(parser.parse_timestamp(line), Some("2024-02-22T10:15:30Z"));
    }

    #[test]
    fn test_parse_timestamp_datetime_prefix() {
        let line = b"2024-01-15 10:30:00.123 [thread] INFO target - msg";
        let parser = CommonLogParser::default();
        assert_eq!(
            parser.parse_timestamp(line),
            parser.parse_line(line).and_then(|p| p.timestamp)
        );
    }

    #[test]
    fn test_parse_timestamp_bracket_iso() {
        let line = b"[2024-02-22T10:15:30Z INFO target] message";
        let parser = CommonLogParser::default();
        assert_eq!(
            parser.parse_timestamp(line),
            parser.parse_line(line).and_then(|p| p.timestamp)
        );
    }

    #[test]
    fn test_parse_timestamp_nano() {
        let line = b"1700046010234000000 INFO target: msg";
        let parser = CommonLogParser::default();
        assert_eq!(parser.parse_timestamp(line), Some("1700046010234000000"));
    }

    #[test]
    fn test_parse_timestamp_no_timestamp_returns_none() {
        let parser = CommonLogParser::default();
        assert!(parser.parse_timestamp(b"ERROR:root:msg").is_none());
    }

    #[test]
    fn test_line_parser_winner_not_set_before_min_samples() {
        let parser = CommonLogParser::default();
        let line = b"2024-07-24T10:00:00Z INFO myapp::server:: listening on 0.0.0.0:3000";
        for _ in 0..MIN_SAMPLES - 1 {
            parser.parse_line(line).unwrap();
        }
        assert!(parser.line_parser.get().is_none());
    }

    #[test]
    fn test_line_parser_winner_set_after_min_samples() {
        let parser = CommonLogParser::default();
        let line = b"2024-07-24T10:00:00Z INFO myapp::server:: listening on 0.0.0.0:3000";
        for _ in 0..MIN_SAMPLES {
            parser.parse_line(line).unwrap();
        }
        assert!(parser.line_parser.get().is_some());
    }

    #[test]
    fn test_line_parser_majority_wins() {
        let parser = CommonLogParser::default();
        let tracing = b"2024-07-24T10:00:00Z INFO myapp::server:: listening on 0.0.0.0:3000";
        let generic = b"2024-07-24T10:00:00Z ERROR something happened";
        for _ in 0..MIN_SAMPLES - 1 {
            parser.parse_line(tracing).unwrap();
        }
        parser.parse_line(generic).unwrap();
        assert!(parser.line_parser.get().is_some());
    }

    #[test]
    fn test_line_parser_consistent_output_before_and_after_lock() {
        let parser = CommonLogParser::default();
        let line = b"2024-07-24T10:00:00Z INFO myapp::server:: listening on 0.0.0.0:3000";
        let before = parser.parse_line(line).unwrap();
        for _ in 0..MIN_SAMPLES {
            parser.parse_line(line).unwrap();
        }
        let after = parser.parse_line(line).unwrap();
        assert_eq!(before.timestamp, after.timestamp);
        assert_eq!(before.level, after.level);
        assert_eq!(before.target, after.target);
        assert_eq!(before.message, after.message);
    }
}
