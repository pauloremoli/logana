//! Format-agnostic log schema: key→slot mappings shared across parsers.

use super::types::FieldSemantic;

/// Describes how a specific log format maps field keys to semantic slots.
/// Shared by all parsers that differ only in their key names (JSON, logfmt, …).
#[derive(Debug)]
pub struct LogSchema {
    pub name: &'static str,
    /// All keys must be present in a line for the schema to match during detection.
    /// Empty slice means the schema matches any parseable line.
    pub detect_keys: &'static [&'static str],
    pub timestamp_keys: &'static [&'static str],
    pub level_keys: &'static [&'static str],
    pub target_keys: &'static [&'static str],
    pub message_keys: &'static [&'static str],
    /// Named extra fields that carry a non-Extra semantic (e.g. hostname → Hostname).
    pub extra_semantics: &'static [(&'static str, FieldSemantic)],
    /// Optional transformation applied to level values (e.g. syslog priority → text).
    pub level_transform: Option<fn(&str) -> Option<&'static str>>,
    /// Extra field keys to keep visible when `default_hidden_fields` is applied.
    /// Non-empty only for schemas like journalctl-json that hide most extras.
    pub keep_visible_extras: &'static [&'static str],
}

impl LogSchema {
    pub fn classify_key(&self, key: &str) -> FieldSemantic {
        if self.timestamp_keys.contains(&key) {
            return FieldSemantic::Timestamp;
        }
        if self.level_keys.contains(&key) {
            return FieldSemantic::Level;
        }
        if self.target_keys.contains(&key) {
            return FieldSemantic::Target;
        }
        if self.message_keys.contains(&key) {
            return FieldSemantic::Message;
        }
        for (k, sem) in self.extra_semantics {
            if *k == key {
                return *sem;
            }
        }
        FieldSemantic::Extra
    }

    /// Returns true when `keys` satisfies `detect_keys` (all required keys present).
    pub fn matches_detect_keys(&self, keys: &[&str]) -> bool {
        if self.detect_keys.is_empty() {
            return true;
        }
        self.detect_keys.iter().all(|dk| keys.contains(dk))
    }
}

pub fn priority_to_level(value: &str) -> Option<&'static str> {
    Some(match value {
        "0" => "EMERG",
        "1" => "ALERT",
        "2" => "CRITICAL",
        "3" => "ERROR",
        "4" => "WARNING",
        "5" => "NOTICE",
        "6" => "INFO",
        "7" => "DEBUG",
        _ => return None,
    })
}

/// journalctl --output=json / json-sse / json-seq
pub static SCHEMA_JOURNALCTL_JSON: LogSchema = LogSchema {
    name: "journalctl-json",
    detect_keys: &["MESSAGE", "PRIORITY"],
    timestamp_keys: &["__REALTIME_TIMESTAMP", "_SOURCE_REALTIME_TIMESTAMP"],
    level_keys: &["PRIORITY"],
    target_keys: &["SYSLOG_IDENTIFIER", "_COMM"],
    message_keys: &["MESSAGE"],
    extra_semantics: &[
        ("_HOSTNAME", FieldSemantic::Hostname),
        ("_PID", FieldSemantic::Pid),
    ],
    level_transform: Some(priority_to_level),
    keep_visible_extras: &["hostname", "pid"],
};

/// tracing-subscriber JSON format (fields container + span object)
pub static SCHEMA_TRACING: LogSchema = LogSchema {
    name: "tracing-json",
    detect_keys: &["target", "fields"],
    timestamp_keys: &["timestamp"],
    level_keys: &["level"],
    target_keys: &["target"],
    message_keys: &["message"],
    extra_semantics: &[
        ("traceId", FieldSemantic::TraceId),
        ("spanId", FieldSemantic::SpanId),
    ],
    level_transform: None,
    keep_visible_extras: &[],
};

/// GELF (Graylog Extended Log Format)
pub static SCHEMA_GELF: LogSchema = LogSchema {
    name: "gelf",
    detect_keys: &["short_message", "version"],
    timestamp_keys: &["timestamp"],
    level_keys: &["level"],
    target_keys: &["host", "source"],
    message_keys: &["short_message", "full_message"],
    extra_semantics: &[],
    level_transform: None,
    keep_visible_extras: &[],
};

/// Generic JSON: logrus, zap, bunyan, pino, structlog, syslog-json, …
/// Catch-all with the broadest set of well-known key aliases.
pub static SCHEMA_GENERIC_JSON: LogSchema = LogSchema {
    name: "json",
    detect_keys: &[],
    timestamp_keys: &["timestamp", "time", "ts", "t", "@timestamp", "datetime"],
    level_keys: &["level", "lvl", "severity", "log_level"],
    target_keys: &[
        "target",
        "logger",
        "module",
        "source",
        "component",
        "service",
        "name",
        "caller",
    ],
    message_keys: &["message", "msg", "log", "text"],
    extra_semantics: &[
        ("hostname", FieldSemantic::Hostname),
        ("pid", FieldSemantic::Pid),
        ("thread", FieldSemantic::Thread),
        ("traceId", FieldSemantic::TraceId),
        ("trace_id", FieldSemantic::TraceId),
        ("TraceID", FieldSemantic::TraceId),
        ("spanId", FieldSemantic::SpanId),
        ("span_id", FieldSemantic::SpanId),
        ("SpanID", FieldSemantic::SpanId),
    ],
    level_transform: None,
    keep_visible_extras: &[],
};

/// logfmt / key=value format (Go slog, Heroku, Grafana Loki, 12-factor apps)
pub static SCHEMA_LOGFMT: LogSchema = LogSchema {
    name: "logfmt",
    detect_keys: &[],
    timestamp_keys: &["time", "ts", "t", "timestamp", "@timestamp", "datetime"],
    level_keys: &["level", "lvl", "severity"],
    target_keys: &[
        "source",
        "module",
        "logger",
        "component",
        "service",
        "caller",
        "name",
        "target",
    ],
    message_keys: &["message", "msg", "log", "text"],
    extra_semantics: &[],
    level_transform: None,
    keep_visible_extras: &[],
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_key_primary_slots() {
        assert_eq!(
            SCHEMA_GENERIC_JSON.classify_key("timestamp"),
            FieldSemantic::Timestamp
        );
        assert_eq!(
            SCHEMA_GENERIC_JSON.classify_key("level"),
            FieldSemantic::Level
        );
        assert_eq!(
            SCHEMA_GENERIC_JSON.classify_key("target"),
            FieldSemantic::Target
        );
        assert_eq!(
            SCHEMA_GENERIC_JSON.classify_key("message"),
            FieldSemantic::Message
        );
    }

    #[test]
    fn test_classify_key_extra_semantics() {
        assert_eq!(
            SCHEMA_GENERIC_JSON.classify_key("hostname"),
            FieldSemantic::Hostname
        );
        assert_eq!(SCHEMA_GENERIC_JSON.classify_key("pid"), FieldSemantic::Pid);
        assert_eq!(
            SCHEMA_GENERIC_JSON.classify_key("traceId"),
            FieldSemantic::TraceId
        );
        assert_eq!(
            SCHEMA_GENERIC_JSON.classify_key("spanId"),
            FieldSemantic::SpanId
        );
    }

    #[test]
    fn test_classify_key_unknown_is_extra() {
        assert_eq!(
            SCHEMA_GENERIC_JSON.classify_key("request_id"),
            FieldSemantic::Extra
        );
        assert_eq!(SCHEMA_GENERIC_JSON.classify_key(""), FieldSemantic::Extra);
    }

    #[test]
    fn test_matches_detect_keys_empty_always_true() {
        let keys = &["foo", "bar"];
        assert!(SCHEMA_GENERIC_JSON.matches_detect_keys(keys));
        assert!(SCHEMA_LOGFMT.matches_detect_keys(keys));
    }

    #[test]
    fn test_matches_detect_keys_all_required() {
        let keys_ok = &["MESSAGE", "PRIORITY", "_HOSTNAME"];
        let keys_missing = &["MESSAGE", "_HOSTNAME"];
        assert!(SCHEMA_JOURNALCTL_JSON.matches_detect_keys(keys_ok));
        assert!(!SCHEMA_JOURNALCTL_JSON.matches_detect_keys(keys_missing));
    }

    #[test]
    fn test_journalctl_schema_level_transform() {
        let transform = SCHEMA_JOURNALCTL_JSON.level_transform.unwrap();
        assert_eq!(transform("6"), Some("INFO"));
        assert_eq!(transform("3"), Some("ERROR"));
        assert_eq!(transform("7"), Some("DEBUG"));
        assert_eq!(transform("99"), None);
    }

    #[test]
    fn test_journalctl_schema_keep_visible_extras() {
        assert!(
            SCHEMA_JOURNALCTL_JSON
                .keep_visible_extras
                .contains(&"hostname")
        );
        assert!(SCHEMA_JOURNALCTL_JSON.keep_visible_extras.contains(&"pid"));
    }

    #[test]
    fn test_tracing_schema_detect_keys() {
        let has_both = &["target", "fields", "level", "timestamp"];
        let missing_fields = &["target", "level", "timestamp"];
        assert!(SCHEMA_TRACING.matches_detect_keys(has_both));
        assert!(!SCHEMA_TRACING.matches_detect_keys(missing_fields));
    }

    #[test]
    fn test_gelf_schema_message_key() {
        assert_eq!(
            SCHEMA_GELF.classify_key("short_message"),
            FieldSemantic::Message
        );
        assert_eq!(
            SCHEMA_GELF.classify_key("full_message"),
            FieldSemantic::Message
        );
        assert_eq!(SCHEMA_GELF.classify_key("version"), FieldSemantic::Extra);
    }
}
