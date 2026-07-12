use std::collections::{HashMap, HashSet};

use regex::Regex;

use crate::config::CustomSchemaConfig;
use crate::parser::types::{
    DisplayParts, FieldSemantic, LogFormatParser, LogLevel, push_extra_field, push_field_as,
};

enum FieldRole {
    Semantic(FieldSemantic),
    Extra,
    Ignored,
}

/// One piece of a compiled `template`: literal text to reproduce verbatim, or
/// a placeholder to fill with that field's captured value at parse time.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TemplateSegment {
    Literal(String),
    Field(String),
}

/// Raw `level` values (lowercased) a schema declares as Error/Warning, built
/// from `CustomSchemaConfig::levels`. Empty sets are the common case (no
/// override — fall back to `LogLevel::parse_level`'s built-in keywords).
#[derive(Debug, Default)]
struct LevelOverrides {
    error: HashSet<String>,
    warning: HashSet<String>,
}

#[derive(Debug)]
pub struct CustomParser {
    name: String,
    regex: Regex,
    field_map: Vec<(usize, &'static str, FieldRoleStored)>,
    /// `Some` only when built from a `template` (not a raw `pattern`), since
    /// only a template has a literal skeleton to reconstruct the line from.
    template_segments: Option<Vec<TemplateSegment>>,
    level_overrides: LevelOverrides,
}

#[derive(Debug)]
enum FieldRoleStored {
    Semantic(FieldSemantic),
    Extra,
    Ignored,
}

impl From<FieldRole> for FieldRoleStored {
    fn from(r: FieldRole) -> Self {
        match r {
            FieldRole::Semantic(s) => FieldRoleStored::Semantic(s),
            FieldRole::Extra => FieldRoleStored::Extra,
            FieldRole::Ignored => FieldRoleStored::Ignored,
        }
    }
}

impl FieldRole {
    fn from_canonical(name: &str) -> Option<FieldRole> {
        match name {
            "timestamp" => Some(FieldRole::Semantic(FieldSemantic::Timestamp)),
            "level" => Some(FieldRole::Semantic(FieldSemantic::Level)),
            "message" => Some(FieldRole::Semantic(FieldSemantic::Message)),
            "target" => Some(FieldRole::Semantic(FieldSemantic::Target)),
            "hostname" => Some(FieldRole::Semantic(FieldSemantic::Hostname)),
            "pid" => Some(FieldRole::Semantic(FieldSemantic::Pid)),
            "thread" => Some(FieldRole::Semantic(FieldSemantic::Thread)),
            "facility" => Some(FieldRole::Semantic(FieldSemantic::Facility)),
            "component" => Some(FieldRole::Semantic(FieldSemantic::Component)),
            "feature" => Some(FieldRole::Semantic(FieldSemantic::Feature)),
            _ => None,
        }
    }

    fn from_str(s: &str) -> Result<FieldRole, String> {
        match s {
            "timestamp" => Ok(FieldRole::Semantic(FieldSemantic::Timestamp)),
            "level" => Ok(FieldRole::Semantic(FieldSemantic::Level)),
            "message" => Ok(FieldRole::Semantic(FieldSemantic::Message)),
            "target" => Ok(FieldRole::Semantic(FieldSemantic::Target)),
            "hostname" => Ok(FieldRole::Semantic(FieldSemantic::Hostname)),
            "pid" => Ok(FieldRole::Semantic(FieldSemantic::Pid)),
            "thread" => Ok(FieldRole::Semantic(FieldSemantic::Thread)),
            "facility" => Ok(FieldRole::Semantic(FieldSemantic::Facility)),
            "component" => Ok(FieldRole::Semantic(FieldSemantic::Component)),
            "feature" => Ok(FieldRole::Semantic(FieldSemantic::Feature)),
            "extra" => Ok(FieldRole::Extra),
            "ignored" => Ok(FieldRole::Ignored),
            other => Err(format!("unknown field role: '{other}'")),
        }
    }
}

pub fn compile_template(template: &str) -> Result<String, String> {
    let placeholder_names: Vec<&str> = collect_placeholder_names(template);
    let last_name = placeholder_names.last().copied();

    let mut pattern = String::new();
    let mut remaining = template;

    while let Some(open) = remaining.find('{') {
        let literal = &remaining[..open];
        pattern.push_str(&compile_literal(literal));

        let rest = &remaining[open + 1..];
        let close = rest
            .find('}')
            .ok_or_else(|| "unclosed '{' in template".to_string())?;
        let name = &rest[..close];

        let after_close = &rest[close + 1..];
        let is_last = Some(name) == last_name && after_close.trim().is_empty();

        let group_pat = if is_last {
            format!("(?P<{name}>.*)")
        } else {
            let next_char = after_close.chars().next();
            match next_char {
                Some(c) if !c.is_whitespace() && c != '{' => {
                    let escaped = regex_escape_char(c);
                    format!("(?P<{name}>[^{escaped}]+)")
                }
                _ => format!("(?P<{name}>\\S+)"),
            }
        };

        pattern.push_str(&group_pat);
        remaining = &rest[close + 1..];
    }

    pattern.push_str(&compile_literal(remaining));

    Ok(format!("^{pattern}$"))
}

/// Splits `template` into literal runs and `{field}` placeholders, in order,
/// preserving the template's own separators (see [`TemplateSegment`]).
fn parse_template_segments(template: &str) -> Vec<TemplateSegment> {
    let mut segments = Vec::new();
    let mut remaining = template;
    while let Some(open) = remaining.find('{') {
        let literal = &remaining[..open];
        if !literal.is_empty() {
            segments.push(TemplateSegment::Literal(literal.to_string()));
        }
        let rest = &remaining[open + 1..];
        let Some(close) = rest.find('}') else {
            break;
        };
        segments.push(TemplateSegment::Field(rest[..close].to_string()));
        remaining = &rest[close + 1..];
    }
    if !remaining.is_empty() {
        segments.push(TemplateSegment::Literal(remaining.to_string()));
    }
    segments
}

fn collect_placeholder_names(template: &str) -> Vec<&str> {
    let mut names = Vec::new();
    let mut remaining = template;
    while let Some(open) = remaining.find('{') {
        let rest = &remaining[open + 1..];
        if let Some(close) = rest.find('}') {
            names.push(&rest[..close]);
            remaining = &rest[close + 1..];
        } else {
            break;
        }
    }
    names
}

fn compile_literal(literal: &str) -> String {
    if literal.is_empty() {
        return String::new();
    }
    let mut result = String::new();
    let mut i = 0;
    let chars: Vec<char> = literal.chars().collect();
    while i < chars.len() {
        if chars[i].is_whitespace() {
            result.push_str("\\s+");
            while i + 1 < chars.len() && chars[i + 1].is_whitespace() {
                i += 1;
            }
        } else {
            result.push_str(&regex::escape(&chars[i].to_string()));
        }
        i += 1;
    }
    result
}

fn regex_escape_char(c: char) -> String {
    let s = c.to_string();
    regex::escape(&s)
}

/// Builds the lowercased error/warning value sets from `cfg.levels`,
/// rejecting a value declared as both (ambiguous — which would win at
/// classification time is not something a user should have to guess).
fn build_level_overrides(cfg: &CustomSchemaConfig) -> Result<LevelOverrides, String> {
    let error: HashSet<String> = cfg.levels.error.iter().map(|v| v.to_lowercase()).collect();
    let warning: HashSet<String> = cfg
        .levels
        .warning
        .iter()
        .map(|v| v.to_lowercase())
        .collect();
    if let Some(both) = error.intersection(&warning).next() {
        return Err(format!(
            "schema '{}': level value '{both}' is declared as both error and warning",
            cfg.name
        ));
    }
    Ok(LevelOverrides { error, warning })
}

impl CustomParser {
    pub fn from_config(cfg: &CustomSchemaConfig) -> Result<Self, String> {
        let mut template_segments = None;
        let pattern_str = match (&cfg.template, &cfg.pattern) {
            (Some(_), Some(_)) => {
                return Err(format!(
                    "schema '{}': cannot specify both 'template' and 'pattern'",
                    cfg.name
                ));
            }
            (None, None) => {
                return Err(format!(
                    "schema '{}': must specify either 'template' or 'pattern'",
                    cfg.name
                ));
            }
            (Some(tmpl), None) => {
                template_segments = Some(parse_template_segments(tmpl));
                compile_template(tmpl)?
            }
            (None, Some(raw)) => raw.clone(),
        };

        let regex = Regex::new(&pattern_str)
            .map_err(|e| format!("schema '{}': invalid regex: {e}", cfg.name))?;

        let mut field_map = Vec::new();
        for (capture_idx, group_name) in named_capture_groups(&regex) {
            let role = if let Some(explicit) = cfg.fields.get(group_name) {
                FieldRole::from_str(explicit)?
            } else if let Some(implicit) = FieldRole::from_canonical(group_name) {
                implicit
            } else {
                FieldRole::Extra
            };
            let static_name: &'static str = Box::leak(group_name.to_string().into_boxed_str());
            field_map.push((capture_idx, static_name, role.into()));
        }

        let level_overrides = build_level_overrides(cfg)?;

        Ok(CustomParser {
            name: cfg.name.clone(),
            regex,
            field_map,
            template_segments,
            level_overrides,
        })
    }

    /// Rebuilds the line from `template_segments`, substituting each field's
    /// captured raw value and reproducing the template's literal separators
    /// verbatim. `Ignored` fields contribute no value (matching that role's
    /// "captured but never displayed" contract) but their surrounding literal
    /// text is still emitted as-is. Returns `None` when this parser was built
    /// from a raw `pattern` rather than a `template`.
    fn reconstruct_line(&self, field_values: &HashMap<&str, &str>) -> Option<String> {
        let segments = self.template_segments.as_ref()?;
        let mut out = String::new();
        for seg in segments {
            match seg {
                TemplateSegment::Literal(text) => out.push_str(text),
                TemplateSegment::Field(name) => {
                    if let Some(val) = field_values.get(name.as_str()) {
                        out.push_str(val);
                    }
                }
            }
        }
        Some(out)
    }
}

fn named_capture_groups(regex: &Regex) -> Vec<(usize, &str)> {
    regex
        .capture_names()
        .enumerate()
        .filter_map(|(idx, name)| name.map(|n| (idx, n)))
        .collect()
}

impl LogFormatParser for CustomParser {
    fn name(&self) -> &str {
        &self.name
    }

    fn classify_level(&self, raw: &str) -> LogLevel {
        let lower = raw.to_lowercase();
        if self.level_overrides.error.contains(&lower) {
            LogLevel::Error
        } else if self.level_overrides.warning.contains(&lower) {
            LogLevel::Warning
        } else {
            LogLevel::parse_level(raw)
        }
    }

    fn parse_line<'a>(&self, line: &'a [u8]) -> Option<DisplayParts<'a>> {
        let s = std::str::from_utf8(line).ok()?;
        let byte_ranges: Vec<Option<std::ops::Range<usize>>> = {
            let caps = self.regex.captures(s)?;
            self.field_map
                .iter()
                .map(|(idx, _, _)| caps.get(*idx).map(|m| m.range()))
                .collect()
        };

        let mut parts = DisplayParts::default();
        let mut field_values: HashMap<&str, &str> = HashMap::new();
        for ((_, group_name, role), range_opt) in self.field_map.iter().zip(byte_ranges.iter()) {
            let val = match range_opt {
                Some(range) => &s[range.clone()],
                None => continue,
            };
            match role {
                FieldRoleStored::Semantic(FieldSemantic::Timestamp) => {
                    parts.timestamp = Some(val);
                }
                FieldRoleStored::Semantic(FieldSemantic::Level) => {
                    parts.level = Some(val);
                }
                FieldRoleStored::Semantic(FieldSemantic::Message) => {
                    parts.message = Some(val);
                }
                FieldRoleStored::Semantic(FieldSemantic::Target) => {
                    parts.target = Some(val);
                }
                FieldRoleStored::Semantic(other) => {
                    push_field_as(&mut parts.extra_fields, *other, val);
                }
                FieldRoleStored::Extra => {
                    push_extra_field(&mut parts.extra_fields, group_name, val);
                }
                FieldRoleStored::Ignored => {}
            }
            if !matches!(role, FieldRoleStored::Ignored) {
                field_values.insert(group_name, val);
            }
        }
        parts.reconstructed_line = self.reconstruct_line(&field_values);

        Some(parts)
    }

    fn collect_field_names(&self, _lines: &[&[u8]]) -> Vec<String> {
        let mut has_timestamp = false;
        let mut has_level = false;
        let mut has_target = false;
        let mut has_message = false;
        let mut seen = HashSet::new();
        let mut extras: Vec<String> = Vec::new();

        for (_, group_name, role) in &self.field_map {
            match role {
                FieldRoleStored::Semantic(FieldSemantic::Timestamp) => has_timestamp = true,
                FieldRoleStored::Semantic(FieldSemantic::Level) => has_level = true,
                FieldRoleStored::Semantic(FieldSemantic::Target) => has_target = true,
                FieldRoleStored::Semantic(FieldSemantic::Message) => has_message = true,
                // Other semantic roles are rendered under their canonical name
                // (see push_field_as), not the raw capture group name.
                FieldRoleStored::Semantic(other) => {
                    let name = other.canonical_name().to_string();
                    if seen.insert(name.clone()) {
                        extras.push(name);
                    }
                }
                FieldRoleStored::Extra => {
                    if seen.insert(group_name.to_string()) {
                        extras.push(group_name.to_string());
                    }
                }
                FieldRoleStored::Ignored => {}
            }
        }

        // Matches the column order rendered by the default field layout:
        // timestamp, level, target, sorted extras, message.
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
        result.extend(extras);
        if has_message {
            result.push("message".to_string());
        }
        result
    }

    fn matches_for_detection(&self, line: &[u8]) -> bool {
        std::str::from_utf8(line)
            .map(|s| self.regex.is_match(s))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_telecom_parser() -> CustomParser {
        CustomParser::from_config(&CustomSchemaConfig {
            name: "telecom".to_string(),
            description: None,
            template: Some(
                "{id} {service} <{timestamp}> {pid} {level}/{component}/{feature}, {message}"
                    .to_string(),
            ),
            pattern: None,
            fields: [
                ("id".to_string(), "extra".to_string()),
                ("service".to_string(), "target".to_string()),
            ]
            .into_iter()
            .collect(),
            levels: Default::default(),
        })
        .unwrap()
    }

    #[test]
    fn test_compile_template_telecom() {
        let template =
            "{id} {service} <{timestamp}> {pid} {level}/{component}/{feature}, {message}";
        let result = compile_template(template).unwrap();
        assert_eq!(
            result,
            r"^(?P<id>\S+)\s+(?P<service>\S+)\s+<(?P<timestamp>[^>]+)>\s+(?P<pid>\S+)\s+(?P<level>[^/]+)/(?P<component>[^/]+)/(?P<feature>[^,]+),\s+(?P<message>.*)$"
        );
    }

    #[test]
    fn test_last_placeholder_is_rest() {
        let result = compile_template("{ts} {msg}").unwrap();
        assert!(
            result.contains("(?P<msg>.*)"),
            "last field should be .*: {result}"
        );
    }

    #[test]
    fn test_delimiter_adjacent_field() {
        let result = compile_template("{ts}>rest").unwrap();
        assert!(
            result.contains("(?P<ts>[^>]+)"),
            "expected [^>]+ pattern, got: {result}"
        );
    }

    #[test]
    fn test_parse_telecom_line() {
        let parser = make_telecom_parser();
        let line = b"04 LINUX-0-syscon <2035-04-04T21:54:53.283856Z> 62A INF/Syscon/StartupMgr, StateChange: dirtyrfservice::instance1 state=CONNECTED";
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.timestamp, Some("2035-04-04T21:54:53.283856Z"));
        assert_eq!(parts.level, Some("INF"));
        assert_eq!(parts.target, Some("LINUX-0-syscon"));
        assert_eq!(
            parts.message,
            Some("StateChange: dirtyrfservice::instance1 state=CONNECTED")
        );
        let component = parts
            .extra_fields
            .iter()
            .find(|(_, k, _)| *k == "component");
        assert_eq!(component.map(|(_, _, v)| *v), Some("Syscon"));
        let feature = parts.extra_fields.iter().find(|(_, k, _)| *k == "feature");
        assert_eq!(feature.map(|(_, _, v)| *v), Some("StartupMgr"));
    }

    #[test]
    fn test_parse_line_with_comma_in_message() {
        let parser = make_telecom_parser();
        let line = b"05 LINUX-0-syscon <2035-04-04T21:54:53.283979Z> 62A INF/Syscon/StartupMgr, dirtyrfservice::instance1 started in 878 ms, and achieved CONNECTED";
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(
            parts.message,
            Some("dirtyrfservice::instance1 started in 878 ms, and achieved CONNECTED")
        );
    }

    #[test]
    fn test_parse_template_segments_telecom() {
        let template =
            "{id} {service} <{timestamp}> {pid} {level}/{component}/{feature}, {message}";
        let segments = parse_template_segments(template);
        assert_eq!(
            segments,
            vec![
                TemplateSegment::Field("id".to_string()),
                TemplateSegment::Literal(" ".to_string()),
                TemplateSegment::Field("service".to_string()),
                TemplateSegment::Literal(" <".to_string()),
                TemplateSegment::Field("timestamp".to_string()),
                TemplateSegment::Literal("> ".to_string()),
                TemplateSegment::Field("pid".to_string()),
                TemplateSegment::Literal(" ".to_string()),
                TemplateSegment::Field("level".to_string()),
                TemplateSegment::Literal("/".to_string()),
                TemplateSegment::Field("component".to_string()),
                TemplateSegment::Literal("/".to_string()),
                TemplateSegment::Field("feature".to_string()),
                TemplateSegment::Literal(", ".to_string()),
                TemplateSegment::Field("message".to_string()),
            ]
        );
    }

    #[test]
    fn test_parse_template_segments_leading_and_trailing_literal() {
        let segments = parse_template_segments("[{level}] done");
        assert_eq!(
            segments,
            vec![
                TemplateSegment::Literal("[".to_string()),
                TemplateSegment::Field("level".to_string()),
                TemplateSegment::Literal("] done".to_string()),
            ]
        );
    }

    #[test]
    fn test_reconstructed_line_preserves_template_separators() {
        // The schema's own "/" and ", " separators must survive in the
        // reconstructed line instead of being collapsed to plain spaces.
        let parser = make_telecom_parser();
        let line = b"04 LINUX-0-syscon <2035-04-04T21:54:53.283856Z> 62A INF/Syscon/StartupMgr, StateChange: dirtyrfservice::instance1 state=CONNECTED";
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(
            parts.reconstructed_line.as_deref(),
            Some(std::str::from_utf8(line).unwrap())
        );
    }

    #[test]
    fn test_reconstructed_line_omits_ignored_field_value() {
        let parser = CustomParser::from_config(&CustomSchemaConfig {
            name: "test".to_string(),
            description: None,
            template: Some("{pid} {level} {message}".to_string()),
            pattern: None,
            fields: [("pid".to_string(), "ignored".to_string())]
                .into_iter()
                .collect(),
            levels: Default::default(),
        })
        .unwrap();
        let line = b"1234 INFO started";
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.reconstructed_line.as_deref(), Some(" INFO started"));
    }

    #[test]
    fn test_reconstructed_line_none_for_raw_pattern_schema() {
        let parser = CustomParser::from_config(&CustomSchemaConfig {
            name: "test".to_string(),
            description: None,
            template: None,
            pattern: Some("^(?P<level>\\w+) (?P<message>.*)$".to_string()),
            fields: Default::default(),
            levels: Default::default(),
        })
        .unwrap();
        let parts = parser.parse_line(b"INFO started").unwrap();
        assert!(parts.reconstructed_line.is_none());
    }

    #[test]
    fn test_parse_line_no_match_returns_none() {
        let parser = make_telecom_parser();
        let line = b"this does not match the telecom format at all";
        assert!(parser.parse_line(line).is_none());
    }

    #[test]
    fn test_from_config_neither_template_nor_pattern_is_err() {
        let result = CustomParser::from_config(&CustomSchemaConfig {
            name: "bad".to_string(),
            description: None,
            template: None,
            pattern: None,
            fields: Default::default(),
            levels: Default::default(),
        });
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must specify either"));
    }

    #[test]
    fn test_from_config_both_template_and_pattern_is_err() {
        let result = CustomParser::from_config(&CustomSchemaConfig {
            name: "bad".to_string(),
            description: None,
            template: Some("{foo}".to_string()),
            pattern: Some("(?P<foo>.*)".to_string()),
            fields: Default::default(),
            levels: Default::default(),
        });
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("cannot specify both"));
    }

    #[test]
    fn test_from_config_invalid_regex_is_err() {
        let result = CustomParser::from_config(&CustomSchemaConfig {
            name: "bad".to_string(),
            description: None,
            template: None,
            pattern: Some("(?P<foo>[invalid".to_string()),
            fields: Default::default(),
            levels: Default::default(),
        });
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid regex"));
    }

    #[test]
    fn test_implicit_field_resolution() {
        let parser = CustomParser::from_config(&CustomSchemaConfig {
            name: "test".to_string(),
            description: None,
            template: Some("{level} {component} {unknown_field} {message}".to_string()),
            pattern: None,
            fields: Default::default(),
            levels: Default::default(),
        })
        .unwrap();

        let level_entry = parser
            .field_map
            .iter()
            .find(|(_, n, _)| *n == "level")
            .unwrap();
        assert!(
            matches!(
                level_entry.2,
                FieldRoleStored::Semantic(FieldSemantic::Level)
            ),
            "level should map to Level semantic"
        );

        let component_entry = parser
            .field_map
            .iter()
            .find(|(_, n, _)| *n == "component")
            .unwrap();
        assert!(
            matches!(
                component_entry.2,
                FieldRoleStored::Semantic(FieldSemantic::Component)
            ),
            "component should map to Component semantic"
        );

        let unknown_entry = parser
            .field_map
            .iter()
            .find(|(_, n, _)| *n == "unknown_field")
            .unwrap();
        assert!(
            matches!(unknown_entry.2, FieldRoleStored::Extra),
            "unknown field should map to Extra"
        );
    }

    #[test]
    fn test_unknown_explicit_role_is_err() {
        let result = CustomParser::from_config(&CustomSchemaConfig {
            name: "bad".to_string(),
            description: None,
            template: Some("{foo}".to_string()),
            pattern: None,
            fields: [("foo".to_string(), "not_a_role".to_string())]
                .into_iter()
                .collect(),
            levels: Default::default(),
        });
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown field role"));
    }

    #[test]
    fn test_matches_for_detection() {
        let parser = make_telecom_parser();
        let matching =
            b"04 LINUX-0-syscon <2035-04-04T21:54:53.283856Z> 62A INF/Syscon/StartupMgr, msg";
        let not_matching = b"not a telecom line";
        assert!(parser.matches_for_detection(matching));
        assert!(!parser.matches_for_detection(not_matching));
    }

    fn parser_with_level_overrides(error: &[&str], warning: &[&str]) -> CustomParser {
        CustomParser::from_config(&CustomSchemaConfig {
            name: "sev".to_string(),
            description: None,
            template: Some("{level} {message}".to_string()),
            pattern: None,
            fields: Default::default(),
            levels: crate::config::CustomLevelValues {
                error: error.iter().map(|s| s.to_string()).collect(),
                warning: warning.iter().map(|s| s.to_string()).collect(),
            },
        })
        .unwrap()
    }

    #[test]
    fn test_classify_level_uses_error_override() {
        let parser = parser_with_level_overrides(&["SEV1"], &["SEV2"]);
        assert_eq!(parser.classify_level("SEV1"), LogLevel::Error);
    }

    #[test]
    fn test_classify_level_uses_warning_override() {
        let parser = parser_with_level_overrides(&["SEV1"], &["SEV2"]);
        assert_eq!(parser.classify_level("SEV2"), LogLevel::Warning);
    }

    #[test]
    fn test_classify_level_override_is_case_insensitive() {
        let parser = parser_with_level_overrides(&["SEV1"], &[]);
        assert_eq!(parser.classify_level("sev1"), LogLevel::Error);
    }

    #[test]
    fn test_classify_level_falls_back_to_builtin_keywords() {
        // No override declared for "ERR" — still classified via the
        // built-in LogLevel::parse_level keyword table.
        let parser = parser_with_level_overrides(&["SEV1"], &["SEV2"]);
        assert_eq!(parser.classify_level("ERR"), LogLevel::Error);
        assert_eq!(parser.classify_level("WARN"), LogLevel::Warning);
    }

    #[test]
    fn test_classify_level_unrecognized_value_without_override_is_unknown() {
        let parser = parser_with_level_overrides(&[], &[]);
        assert_eq!(parser.classify_level("SEV1"), LogLevel::Unknown);
    }

    #[test]
    fn test_from_config_conflicting_level_override_is_err() {
        let result = CustomParser::from_config(&CustomSchemaConfig {
            name: "bad".to_string(),
            description: None,
            template: Some("{level} {message}".to_string()),
            pattern: None,
            fields: Default::default(),
            levels: crate::config::CustomLevelValues {
                error: vec!["SEV1".to_string()],
                warning: vec!["sev1".to_string()],
            },
        });
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("both error and warning"));
    }

    #[test]
    fn test_ignored_role() {
        let parser = CustomParser::from_config(&CustomSchemaConfig {
            name: "test".to_string(),
            description: None,
            template: Some("{pid} {message}".to_string()),
            pattern: None,
            fields: [("pid".to_string(), "ignored".to_string())]
                .into_iter()
                .collect(),
            levels: Default::default(),
        })
        .unwrap();

        let line = b"1234 hello world";
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(parts.message, Some("hello world"));
        assert!(
            parts.extra_fields.is_empty(),
            "ignored field should not appear in extra_fields"
        );
    }

    #[test]
    fn test_collect_field_names_matches_render_order() {
        // Matches the column order rendered by the default field layout
        // (timestamp, level, target, sorted extras, message), so the Select
        // Fields popup shows fields in the same order they're first displayed.
        let parser = CustomParser::from_config(&CustomSchemaConfig {
            name: "test".to_string(),
            description: None,
            template: Some("{timestamp} {level} {target} {zebra} {alpha} {message}".to_string()),
            pattern: None,
            fields: Default::default(),
            levels: Default::default(),
        })
        .unwrap();

        let names = parser.collect_field_names(&[]);
        assert_eq!(
            names,
            vec![
                "timestamp".to_string(),
                "level".to_string(),
                "target".to_string(),
                "alpha".to_string(),
                "zebra".to_string(),
                "message".to_string(),
            ]
        );
    }

    #[test]
    fn test_collect_field_names_uses_canonical_name_for_semantic_alias() {
        // A capture group named "host" with an explicit "hostname" role must
        // surface as "hostname" — the key actually used in extra_fields at
        // render time (see push_field_as) — not the raw group name "host".
        let parser = CustomParser::from_config(&CustomSchemaConfig {
            name: "test".to_string(),
            description: None,
            template: Some("{host} {message}".to_string()),
            pattern: None,
            fields: [("host".to_string(), "hostname".to_string())]
                .into_iter()
                .collect(),
            levels: Default::default(),
        })
        .unwrap();

        let names = parser.collect_field_names(&[]);
        assert!(names.contains(&"hostname".to_string()));
        assert!(!names.contains(&"host".to_string()));
    }

    #[test]
    fn test_collect_field_names_excludes_ignored() {
        let parser = CustomParser::from_config(&CustomSchemaConfig {
            name: "test".to_string(),
            description: None,
            template: Some("{pid} {message}".to_string()),
            pattern: None,
            fields: [("pid".to_string(), "ignored".to_string())]
                .into_iter()
                .collect(),
            levels: Default::default(),
        })
        .unwrap();

        let names = parser.collect_field_names(&[]);
        assert!(!names.contains(&"pid".to_string()));
        assert!(names.contains(&"message".to_string()));
    }
}
