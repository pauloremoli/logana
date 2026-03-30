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
