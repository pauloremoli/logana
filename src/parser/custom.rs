use std::collections::HashSet;

use regex::Regex;

use crate::config::CustomSchemaConfig;
use crate::parser::types::{
    DisplayParts, FieldSemantic, LogFormatParser, push_extra_field, push_field_as,
};

enum FieldRole {
    Semantic(FieldSemantic),
    Extra,
    Ignored,
}

#[derive(Debug)]
pub struct CustomParser {
    name: String,
    regex: Regex,
    field_map: Vec<(usize, &'static str, FieldRoleStored)>,
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

impl CustomParser {
    pub fn from_config(cfg: &CustomSchemaConfig) -> Result<Self, String> {
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
            (Some(tmpl), None) => compile_template(tmpl)?,
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

        Ok(CustomParser {
            name: cfg.name.clone(),
            regex,
            field_map,
        })
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
        }

        Some(parts)
    }

    fn collect_field_names(&self, _lines: &[&[u8]]) -> Vec<String> {
        self.field_map
            .iter()
            .filter(|(_, _, role)| !matches!(role, FieldRoleStored::Ignored))
            .map(|(_, name, _)| name.to_string())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect()
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
    fn test_collect_field_names_excludes_ignored() {
        let parser = CustomParser::from_config(&CustomSchemaConfig {
            name: "test".to_string(),
            description: None,
            template: Some("{pid} {message}".to_string()),
            pattern: None,
            fields: [("pid".to_string(), "ignored".to_string())]
                .into_iter()
                .collect(),
        })
        .unwrap();

        let names = parser.collect_field_names(&[]);
        assert!(!names.contains(&"pid".to_string()));
        assert!(names.contains(&"message".to_string()));
    }
}
