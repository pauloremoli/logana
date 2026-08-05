use std::collections::HashSet;

use regex::Regex;

use crate::config::{ContinuationConfig, CustomSchemaConfig};
use crate::parser::json::parse_json_line;
use crate::parser::types::{
    DisplayParts, FieldSemantic, LogFormatParser, LogLevel, TemplateSegment, push_extra_field,
    push_field_as,
};

enum FieldRole {
    Semantic(FieldSemantic),
    Extra,
    Ignored,
}

/// One piece of a template as written, before field roles are known: literal
/// text to reproduce verbatim, or a placeholder holding the *raw* `{name}`
/// from the template. Resolved into `TemplateSegment` (canonical field
/// names) once `field_map` is available — see `resolve_segments`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RawSegment {
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

/// One compiled `continuation.fields[]` entry: a matcher tried against each
/// continuation line, extracting either plain fields via `field_map` or (when
/// `json_capture` is `Some`) unpacking a single capture's text as JSON.
#[derive(Debug)]
struct ContinuationMatcher {
    regex: Regex,
    field_map: Vec<(usize, &'static str, FieldRoleStored)>,
    json_capture: Option<usize>,
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
    multiline: bool,
    continuation_matchers: Vec<ContinuationMatcher>,
    end_pattern: Option<Regex>,
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
/// preserving the template's own separators (see [`RawSegment`]).
fn parse_raw_segments(template: &str) -> Vec<RawSegment> {
    let mut segments = Vec::new();
    let mut remaining = template;
    while let Some(open) = remaining.find('{') {
        let literal = &remaining[..open];
        if !literal.is_empty() {
            segments.push(RawSegment::Literal(literal.to_string()));
        }
        let rest = &remaining[open + 1..];
        let Some(close) = rest.find('}') else {
            break;
        };
        segments.push(RawSegment::Field(rest[..close].to_string()));
        remaining = &rest[close + 1..];
    }
    if !remaining.is_empty() {
        segments.push(RawSegment::Literal(remaining.to_string()));
    }
    segments
}

/// Resolves `raw` template segments into `TemplateSegment`s carrying each
/// field's canonical name (the name `resolve_field`/`hidden_fields` use),
/// using the same role information as `field_map`. A raw field name with no
/// matching capture group (shouldn't happen — every `{name}` in the template
/// compiles to a capture group of the same name) is dropped.
fn resolve_segments(
    raw: Vec<RawSegment>,
    field_map: &[(usize, &'static str, FieldRoleStored)],
) -> Vec<TemplateSegment> {
    raw.into_iter()
        .filter_map(|seg| match seg {
            RawSegment::Literal(text) => Some(TemplateSegment::Literal(text)),
            RawSegment::Field(raw_name) => {
                let (_, group_name, role) =
                    field_map.iter().find(|(_, name, _)| *name == raw_name)?;
                let ignored = matches!(role, FieldRoleStored::Ignored);
                let canonical_name = match role {
                    FieldRoleStored::Semantic(sem) => sem.canonical_name().to_string(),
                    FieldRoleStored::Extra | FieldRoleStored::Ignored => group_name.to_string(),
                };
                Some(TemplateSegment::Field {
                    canonical_name,
                    ignored,
                })
            }
        })
        .collect()
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

/// Result of compiling one `template`/`pattern` pair (the header, or one
/// `continuation.fields[]` entry) into a regex and its resolved field roles.
struct CompiledMatcher {
    regex: Regex,
    field_map: Vec<(usize, &'static str, FieldRoleStored)>,
    /// `Some` only when compiled from a `template`, not a raw `pattern`.
    raw_segments: Option<Vec<RawSegment>>,
}

/// Compiles a `template`/`pattern` pair (mutually exclusive, exactly one
/// required) into a regex and resolves each named capture group's field
/// role. Shared by the header (`allow_slot_roles: true`) and each
/// `continuation.fields[]` entry (`allow_slot_roles: false` — a continuation
/// field can't claim the header's singular timestamp/level/target/message
/// slots).
fn compile_matcher(
    schema_name: &str,
    template: &Option<String>,
    pattern: &Option<String>,
    fields: &std::collections::HashMap<String, String>,
    allow_slot_roles: bool,
) -> Result<CompiledMatcher, String> {
    let mut raw_segments = None;
    let pattern_str = match (template, pattern) {
        (Some(_), Some(_)) => {
            return Err(format!(
                "schema '{schema_name}': cannot specify both 'template' and 'pattern'"
            ));
        }
        (None, None) => {
            return Err(format!(
                "schema '{schema_name}': must specify either 'template' or 'pattern'"
            ));
        }
        (Some(tmpl), None) => {
            raw_segments = Some(parse_raw_segments(tmpl));
            compile_template(tmpl)?
        }
        (None, Some(raw)) => raw.clone(),
    };

    let regex = Regex::new(&pattern_str)
        .map_err(|e| format!("schema '{schema_name}': invalid regex: {e}"))?;

    let mut field_map = Vec::new();
    for (capture_idx, group_name) in named_capture_groups(&regex) {
        let role = if let Some(explicit) = fields.get(group_name) {
            FieldRole::from_str(explicit)?
        } else if let Some(implicit) = FieldRole::from_canonical(group_name) {
            implicit
        } else {
            FieldRole::Extra
        };
        if !allow_slot_roles
            && let FieldRole::Semantic(sem) = &role
            && matches!(
                sem,
                FieldSemantic::Timestamp
                    | FieldSemantic::Level
                    | FieldSemantic::Target
                    | FieldSemantic::Message
            )
        {
            return Err(format!(
                "schema '{schema_name}': continuation field '{group_name}' cannot use role '{}' \
                 — timestamp/level/target/message are only valid on the header template/pattern",
                sem.canonical_name()
            ));
        }
        let static_name: &'static str = Box::leak(group_name.to_string().into_boxed_str());
        field_map.push((capture_idx, static_name, role.into()));
    }

    Ok(CompiledMatcher {
        regex,
        field_map,
        raw_segments,
    })
}

/// Compiles a schema's `continuation` block (if any) into its runtime form.
fn compile_continuation(
    cfg: &CustomSchemaConfig,
) -> Result<(Vec<ContinuationMatcher>, Option<Regex>), String> {
    let Some(ContinuationConfig {
        end_pattern,
        fields,
    }) = &cfg.continuation
    else {
        return Ok((Vec::new(), None));
    };

    let end_pattern = end_pattern
        .as_ref()
        .map(|p| -> Result<Regex, String> {
            let pattern_str = compile_template(p)?;
            Regex::new(&pattern_str).map_err(|e| {
                format!(
                    "schema '{}': invalid continuation end_pattern: {e}",
                    cfg.name
                )
            })
        })
        .transpose()?;

    let mut matchers = Vec::with_capacity(fields.len());
    for spec in fields {
        let compiled = compile_matcher(
            &cfg.name,
            &spec.template,
            &spec.pattern,
            &spec.fields,
            false,
        )?;
        let json_capture = if spec.json {
            if compiled.field_map.len() != 1 {
                return Err(format!(
                    "schema '{}': a 'json' continuation field must have exactly one placeholder",
                    cfg.name
                ));
            }
            Some(compiled.field_map[0].0)
        } else {
            None
        };
        matchers.push(ContinuationMatcher {
            regex: compiled.regex,
            field_map: compiled.field_map,
            json_capture,
        });
    }

    Ok((matchers, end_pattern))
}

impl CustomParser {
    pub fn from_config(cfg: &CustomSchemaConfig) -> Result<Self, String> {
        let header = compile_matcher(&cfg.name, &cfg.template, &cfg.pattern, &cfg.fields, true)?;
        let template_segments = header
            .raw_segments
            .map(|raw| resolve_segments(raw, &header.field_map));
        let level_overrides = build_level_overrides(cfg)?;
        let (continuation_matchers, end_pattern) = compile_continuation(cfg)?;

        Ok(CustomParser {
            name: cfg.name.clone(),
            regex: header.regex,
            field_map: header.field_map,
            template_segments,
            level_overrides,
            multiline: cfg.multiline,
            continuation_matchers,
            end_pattern,
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

    fn merges_continuation_into_message(&self) -> bool {
        self.multiline
    }

    fn wants_continuation_walk(&self) -> bool {
        self.multiline || !self.continuation_matchers.is_empty() || self.end_pattern.is_some()
    }

    fn is_continuation_end(&self, line: &[u8]) -> bool {
        let Some(end) = &self.end_pattern else {
            return false;
        };
        std::str::from_utf8(line)
            .map(|s| end.is_match(s))
            .unwrap_or(false)
    }

    fn extract_continuation_fields<'a>(
        &self,
        line: &'a [u8],
    ) -> Vec<(FieldSemantic, &'a str, &'a str)> {
        let Ok(s) = std::str::from_utf8(line) else {
            return Vec::new();
        };
        for matcher in &self.continuation_matchers {
            let Some(caps) = matcher.regex.captures(s) else {
                continue;
            };
            let mut out = Vec::new();
            if let Some(json_idx) = matcher.json_capture {
                let Some(text) = caps.get(json_idx).map(|m| m.as_str().trim()) else {
                    return out;
                };
                let Some(json_fields) = parse_json_line(text.as_bytes()) else {
                    return out;
                };
                for f in &json_fields {
                    push_extra_field(&mut out, f.key, f.value);
                }
                return out;
            }
            for (idx, group_name, role) in &matcher.field_map {
                let Some(val) = caps.get(*idx).map(|m| m.as_str()) else {
                    continue;
                };
                match role {
                    FieldRoleStored::Semantic(sem) => push_field_as(&mut out, *sem, val),
                    FieldRoleStored::Extra => push_extra_field(&mut out, group_name, val),
                    FieldRoleStored::Ignored => {}
                }
            }
            return out;
        }
        Vec::new()
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

    fn template_segments(&self) -> Option<&[TemplateSegment]> {
        self.template_segments.as_deref()
    }

    fn collect_field_names(&self, lines: &[&[u8]]) -> Vec<String> {
        // A `template`-based schema has its own declared field order (see
        // `TemplateSegment`/`resolve_segments`) — surface fields in that
        // order so the Select Fields popup (and its "reset" default) matches
        // what the schema describes, consistent with how the log panel
        // already renders the line in template order (see log_panel.rs).
        // `pattern`-based schemas have no such order to draw from, so they
        // fall back to the canonical-slot order below. Either way, any
        // `continuation.fields`/embedded-JSON names actually observed in
        // `lines` are appended last (dynamic — a sample-dependent set, like
        // JsonParser's extras), so they show up in the Select Fields popup.
        if let Some(segments) = &self.template_segments {
            let mut seen = HashSet::new();
            let mut result: Vec<String> = segments
                .iter()
                .filter_map(|seg| match seg {
                    TemplateSegment::Field {
                        canonical_name,
                        ignored: false,
                    } if seen.insert(canonical_name.clone()) => Some(canonical_name.clone()),
                    _ => None,
                })
                .collect();
            result.extend(self.continuation_field_names(lines, &mut seen));
            return result;
        }

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
        result.extend(self.continuation_field_names(lines, &mut seen));
        result
    }

    fn matches_for_detection(&self, line: &[u8]) -> bool {
        std::str::from_utf8(line)
            .map(|s| self.regex.is_match(s))
            .unwrap_or(false)
    }
}

impl CustomParser {
    /// Scans `lines` with `extract_continuation_fields`, returning names not
    /// already in `seen` (in first-seen order), and inserting them into
    /// `seen`. Shared by both `collect_field_names` branches.
    fn continuation_field_names(&self, lines: &[&[u8]], seen: &mut HashSet<String>) -> Vec<String> {
        let mut names = Vec::new();
        for &line in lines {
            for (sem, key, _) in self.extract_continuation_fields(line) {
                let name = match sem {
                    FieldSemantic::Extra => key.to_string(),
                    other => other.canonical_name().to_string(),
                };
                if seen.insert(name.clone()) {
                    names.push(name);
                }
            }
        }
        names
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_acme_parser() -> CustomParser {
        CustomParser::from_config(&CustomSchemaConfig {
            name: "acme".to_string(),
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
            multiline: false,
            continuation: None,
            ..Default::default()
        })
        .unwrap()
    }

    #[test]
    fn test_compile_template_acme() {
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
    fn test_parse_acme_line() {
        let parser = make_acme_parser();
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
        let parser = make_acme_parser();
        let line = b"05 LINUX-0-syscon <2035-04-04T21:54:53.283979Z> 62A INF/Syscon/StartupMgr, dirtyrfservice::instance1 started in 878 ms, and achieved CONNECTED";
        let parts = parser.parse_line(line).unwrap();
        assert_eq!(
            parts.message,
            Some("dirtyrfservice::instance1 started in 878 ms, and achieved CONNECTED")
        );
    }

    #[test]
    fn test_parse_raw_segments_acme() {
        let template =
            "{id} {service} <{timestamp}> {pid} {level}/{component}/{feature}, {message}";
        let segments = parse_raw_segments(template);
        assert_eq!(
            segments,
            vec![
                RawSegment::Field("id".to_string()),
                RawSegment::Literal(" ".to_string()),
                RawSegment::Field("service".to_string()),
                RawSegment::Literal(" <".to_string()),
                RawSegment::Field("timestamp".to_string()),
                RawSegment::Literal("> ".to_string()),
                RawSegment::Field("pid".to_string()),
                RawSegment::Literal(" ".to_string()),
                RawSegment::Field("level".to_string()),
                RawSegment::Literal("/".to_string()),
                RawSegment::Field("component".to_string()),
                RawSegment::Literal("/".to_string()),
                RawSegment::Field("feature".to_string()),
                RawSegment::Literal(", ".to_string()),
                RawSegment::Field("message".to_string()),
            ]
        );
    }

    #[test]
    fn test_parse_raw_segments_leading_and_trailing_literal() {
        let segments = parse_raw_segments("[{level}] done");
        assert_eq!(
            segments,
            vec![
                RawSegment::Literal("[".to_string()),
                RawSegment::Field("level".to_string()),
                RawSegment::Literal("] done".to_string()),
            ]
        );
    }

    #[test]
    fn test_template_segments_uses_canonical_names() {
        // "service" is mapped to the Target role — the resolved segment must
        // carry the canonical name "target" (what `resolve_field`/
        // `hidden_fields` use), not the raw placeholder name "service".
        let parser = make_acme_parser();
        let segments = parser.template_segments().unwrap();
        assert_eq!(
            segments[0],
            TemplateSegment::Field {
                canonical_name: "id".to_string(),
                ignored: false,
            }
        );
        assert_eq!(
            segments[2],
            TemplateSegment::Field {
                canonical_name: "target".to_string(),
                ignored: false,
            }
        );
    }

    #[test]
    fn test_template_segments_marks_ignored_field() {
        let parser = CustomParser::from_config(&CustomSchemaConfig {
            name: "test".to_string(),
            description: None,
            template: Some("{pid} {level} {message}".to_string()),
            pattern: None,
            fields: [("pid".to_string(), "ignored".to_string())]
                .into_iter()
                .collect(),
            levels: Default::default(),
            multiline: false,
            continuation: None,
            ..Default::default()
        })
        .unwrap();
        let segments = parser.template_segments().unwrap();
        assert_eq!(
            segments[0],
            TemplateSegment::Field {
                canonical_name: "pid".to_string(),
                ignored: true,
            }
        );
    }

    #[test]
    fn test_template_segments_preserves_literal_separators() {
        let parser = make_acme_parser();
        let segments = parser.template_segments().unwrap();
        assert!(segments.contains(&TemplateSegment::Literal("/".to_string())));
        assert!(segments.contains(&TemplateSegment::Literal(", ".to_string())));
    }

    #[test]
    fn test_template_segments_none_for_raw_pattern_schema() {
        let parser = CustomParser::from_config(&CustomSchemaConfig {
            name: "test".to_string(),
            description: None,
            template: None,
            pattern: Some("^(?P<level>\\w+) (?P<message>.*)$".to_string()),
            fields: Default::default(),
            levels: Default::default(),
            multiline: false,
            continuation: None,
            ..Default::default()
        })
        .unwrap();
        assert!(parser.template_segments().is_none());
    }

    #[test]
    fn test_parse_line_no_match_returns_none() {
        let parser = make_acme_parser();
        let line = b"this does not match the acme format at all";
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
            multiline: false,
            continuation: None,
            ..Default::default()
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
            multiline: false,
            continuation: None,
            ..Default::default()
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
            multiline: false,
            continuation: None,
            ..Default::default()
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
            multiline: false,
            continuation: None,
            ..Default::default()
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
            multiline: false,
            continuation: None,
            ..Default::default()
        });
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown field role"));
    }

    #[test]
    fn test_matches_for_detection() {
        let parser = make_acme_parser();
        let matching =
            b"04 LINUX-0-syscon <2035-04-04T21:54:53.283856Z> 62A INF/Syscon/StartupMgr, msg";
        let not_matching = b"not a acme line";
        assert!(parser.matches_for_detection(matching));
        assert!(!parser.matches_for_detection(not_matching));
    }

    #[test]
    fn test_merges_continuation_into_message_defaults_false() {
        let parser = make_acme_parser();
        assert!(!parser.merges_continuation_into_message());
    }

    #[test]
    fn test_merges_continuation_into_message_reflects_config() {
        let parser = CustomParser::from_config(&CustomSchemaConfig {
            name: "test".to_string(),
            description: None,
            template: Some("{level} {message}".to_string()),
            pattern: None,
            fields: Default::default(),
            levels: Default::default(),
            multiline: true,
            continuation: None,
            ..Default::default()
        })
        .unwrap();
        assert!(parser.merges_continuation_into_message());
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
            multiline: false,
            continuation: None,
            ..Default::default()
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
            multiline: false,
            continuation: None,
            ..Default::default()
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
            multiline: false,
            continuation: None,
            ..Default::default()
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
    fn test_collect_field_names_matches_template_order() {
        // A template-based schema's own field order (not the canonical
        // timestamp/level/target/sorted-extras/message slot order) drives
        // the Select Fields popup, matching how the log panel already
        // renders the line in template order (see log_panel.rs). "zebra"
        // before "alpha" here — the opposite of alphabetical — proves it's
        // not falling back to sorting extras.
        let parser = CustomParser::from_config(&CustomSchemaConfig {
            name: "test".to_string(),
            description: None,
            template: Some("{timestamp} {level} {target} {zebra} {alpha} {message}".to_string()),
            pattern: None,
            fields: Default::default(),
            levels: Default::default(),
            multiline: false,
            continuation: None,
            ..Default::default()
        })
        .unwrap();

        let names = parser.collect_field_names(&[]);
        assert_eq!(
            names,
            vec![
                "timestamp".to_string(),
                "level".to_string(),
                "target".to_string(),
                "zebra".to_string(),
                "alpha".to_string(),
                "message".to_string(),
            ]
        );
    }

    #[test]
    fn test_collect_field_names_matches_acme_template_order() {
        // The message field isn't last in field-role terms here (component
        // and feature come between level and message in the template) —
        // the popup order must follow the template exactly, not group
        // "extras" together the way the canonical-slot fallback would.
        let parser = make_acme_parser();
        let names = parser.collect_field_names(&[]);
        assert_eq!(
            names,
            vec![
                "id".to_string(),
                "target".to_string(),
                "timestamp".to_string(),
                "pid".to_string(),
                "level".to_string(),
                "component".to_string(),
                "feature".to_string(),
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
            multiline: false,
            continuation: None,
            ..Default::default()
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
            multiline: false,
            continuation: None,
            ..Default::default()
        })
        .unwrap();

        let names = parser.collect_field_names(&[]);
        assert!(!names.contains(&"pid".to_string()));
        assert!(names.contains(&"message".to_string()));
    }

    // ── continuation field extraction ────────────────────────────────────

    use crate::config::ContinuationFieldSpec;

    fn transaction_schema_config(fields: Vec<ContinuationFieldSpec>) -> CustomSchemaConfig {
        CustomSchemaConfig {
            name: "transaction".to_string(),
            description: None,
            template: Some("### Start transaction {id}".to_string()),
            pattern: None,
            fields: [("id".to_string(), "extra".to_string())]
                .into_iter()
                .collect(),
            levels: Default::default(),
            multiline: false,
            continuation: Some(crate::config::ContinuationConfig {
                end_pattern: Some("### End transaction".to_string()),
                fields,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn test_continuation_field_rejects_slot_role() {
        let result =
            CustomParser::from_config(&transaction_schema_config(vec![ContinuationFieldSpec {
                template: Some("ts: {when}".to_string()),
                pattern: None,
                fields: [("when".to_string(), "timestamp".to_string())]
                    .into_iter()
                    .collect(),
                json: false,
            }]));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("timestamp"), "{err}");
        assert!(err.contains("continuation field"), "{err}");
    }

    #[test]
    fn test_continuation_json_field_requires_exactly_one_placeholder() {
        let no_placeholder =
            CustomParser::from_config(&transaction_schema_config(vec![ContinuationFieldSpec {
                template: Some("Object literal".to_string()),
                pattern: None,
                fields: Default::default(),
                json: true,
            }]));
        assert!(no_placeholder.is_err());
        assert!(
            no_placeholder
                .unwrap_err()
                .contains("exactly one placeholder")
        );

        let two_placeholders =
            CustomParser::from_config(&transaction_schema_config(vec![ContinuationFieldSpec {
                template: Some("Object {a} {b}".to_string()),
                pattern: None,
                fields: Default::default(),
                json: true,
            }]));
        assert!(two_placeholders.is_err());
        assert!(
            two_placeholders
                .unwrap_err()
                .contains("exactly one placeholder")
        );
    }

    #[test]
    fn test_extract_continuation_fields_plain_template() {
        let parser =
            CustomParser::from_config(&transaction_schema_config(vec![ContinuationFieldSpec {
                template: Some("field1: {field1}".to_string()),
                pattern: None,
                fields: Default::default(),
                json: false,
            }]))
            .unwrap();

        let fields = parser.extract_continuation_fields(b"field1: 10");
        assert_eq!(fields, vec![(FieldSemantic::Extra, "field1", "10")]);
    }

    #[test]
    fn test_extract_continuation_fields_unpacks_json() {
        let parser =
            CustomParser::from_config(&transaction_schema_config(vec![ContinuationFieldSpec {
                template: Some("Object {payload}".to_string()),
                pattern: None,
                fields: Default::default(),
                json: true,
            }]))
            .unwrap();

        let fields = parser.extract_continuation_fields(br#"Object {"user":"alice","amount":99}"#);
        assert!(fields.contains(&(FieldSemantic::Extra, "user", "alice")));
        assert!(fields.contains(&(FieldSemantic::Extra, "amount", "99")));
    }

    #[test]
    fn test_extract_continuation_fields_tries_specs_in_order_first_match_wins() {
        let parser = CustomParser::from_config(&transaction_schema_config(vec![
            ContinuationFieldSpec {
                template: Some("field1: {field1}".to_string()),
                pattern: None,
                fields: Default::default(),
                json: false,
            },
            ContinuationFieldSpec {
                template: Some("field2: {field2}".to_string()),
                pattern: None,
                fields: Default::default(),
                json: false,
            },
        ]))
        .unwrap();

        assert_eq!(
            parser.extract_continuation_fields(b"field1: 10"),
            vec![(FieldSemantic::Extra, "field1", "10")]
        );
        assert_eq!(
            parser.extract_continuation_fields(b"field2: 3"),
            vec![(FieldSemantic::Extra, "field2", "3")]
        );
    }

    #[test]
    fn test_extract_continuation_fields_no_match_returns_empty() {
        let parser =
            CustomParser::from_config(&transaction_schema_config(vec![ContinuationFieldSpec {
                template: Some("field1: {field1}".to_string()),
                pattern: None,
                fields: Default::default(),
                json: false,
            }]))
            .unwrap();

        assert!(
            parser
                .extract_continuation_fields(b"nothing to see here")
                .is_empty()
        );
    }

    #[test]
    fn test_is_continuation_end_matches_configured_end_pattern() {
        let parser = CustomParser::from_config(&transaction_schema_config(vec![])).unwrap();
        assert!(parser.is_continuation_end(b"### End transaction"));
        assert!(!parser.is_continuation_end(b"field1: 10"));
    }

    #[test]
    fn test_is_continuation_end_false_without_end_pattern() {
        let parser = make_acme_parser();
        assert!(!parser.is_continuation_end(b"anything"));
    }

    #[test]
    fn test_wants_continuation_walk_true_for_continuation_block_without_multiline() {
        let parser =
            CustomParser::from_config(&transaction_schema_config(vec![ContinuationFieldSpec {
                template: Some("field1: {field1}".to_string()),
                pattern: None,
                fields: Default::default(),
                json: false,
            }]))
            .unwrap();
        assert!(!parser.merges_continuation_into_message());
        assert!(parser.wants_continuation_walk());
    }

    #[test]
    fn test_wants_continuation_walk_false_without_multiline_or_continuation() {
        let parser = make_acme_parser();
        assert!(!parser.wants_continuation_walk());
    }

    #[test]
    fn test_collect_field_names_includes_continuation_fields_from_sample() {
        let parser =
            CustomParser::from_config(&transaction_schema_config(vec![ContinuationFieldSpec {
                template: Some("field1: {field1}".to_string()),
                pattern: None,
                fields: Default::default(),
                json: false,
            }]))
            .unwrap();

        let sample: Vec<&[u8]> = vec![b"### Start transaction 42", b"field1: 10"];
        let names = parser.collect_field_names(&sample);
        assert_eq!(names, vec!["id".to_string(), "field1".to_string()]);
    }
}
