use crate::commands::auto_complete::FieldIndex;
use crate::{
    commands::FILE_PATH_COMMANDS,
    commands::auto_complete::{
        FieldCompletion, complete_color, complete_field_name, complete_field_value,
        complete_file_path, complete_flags, extract_color_partial, extract_field_partial,
        extract_flag_partial, extract_group_partial, find_command_completions, fuzzy_match,
        shell_split,
    },
    theme::Theme,
};
use std::collections::HashSet;

#[derive(Debug, Clone, Default)]
pub struct CompletionContext {
    pub fields: FieldIndex,
    pub hidden_fields: HashSet<String>,
    pub groups: Vec<String>,
    pub source_file: Option<String>,
}

impl CompletionContext {
    pub fn for_query(input: &str, tab: &crate::shared::TabState) -> Self {
        Self {
            fields: if extract_field_partial(input.trim_start()).is_some()
                || input.trim_start().starts_with("hide-field ")
                || input.trim_start().starts_with("show-field ")
            {
                tab.build_field_index()
            } else {
                Default::default()
            },
            hidden_fields: tab.display.hidden_fields.clone(),
            groups: tab.log_manager.group_names(),
            source_file: tab.log_manager.source_file().map(str::to_owned),
        }
    }
}

fn extract_template_partial(input: &str) -> Option<&str> {
    let trimmed = input.trim();
    if !trimmed.starts_with("export") {
        return None;
    }
    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    if tokens.len() < 2 {
        return None;
    }
    let last = tokens[tokens.len() - 1];
    let second_last = tokens[tokens.len() - 2];

    if second_last == "-t" || second_last == "--template" {
        return Some(last);
    }
    if (last == "-t" || last == "--template") && input.ends_with(' ') {
        return Some("");
    }
    None
}

fn arg_partial<'a>(input: &'a str, cmd: &str) -> Option<&'a str> {
    input
        .strip_prefix(cmd)
        .and_then(|rest| rest.strip_prefix(' '))
        .map(|rest| rest.trim_start())
}

pub fn replacements(input: &str, context: &CompletionContext) -> Vec<String> {
    let trimmed = input.trim().to_string();

    // Field name/value completion for `filter --field` and `exclude --field`.
    // Use trim_start() (left-trim only) to preserve any trailing space, which
    // signals that the preceding token is complete and the next one starts.
    let input_ls = input.trim_start();
    if let Some(fc) = extract_field_partial(input_ls) {
        let field_index = context.fields.clone();
        let completions: Vec<String> = match &fc {
            FieldCompletion::Name(partial) => {
                let prefix_end = input_ls.len() - partial.len();
                let prefix = &input_ls[..prefix_end];
                complete_field_name(partial, &field_index)
                    .into_iter()
                    .map(|n| format!("{prefix}{n}="))
                    .collect()
            }
            FieldCompletion::Value { field, partial } => {
                let prefix_end = input_ls.len() - partial.len();
                let prefix = &input_ls[..prefix_end];
                complete_field_value(
                    field,
                    &shell_split(partial).into_iter().next().unwrap_or_default(),
                    &field_index,
                )
                .into_iter()
                .map(|v| format!("{prefix}{}", quote_token(&v)))
                .collect()
            }
        };
        if !completions.is_empty() {
            return completions;
        }
    }

    // Group-name completion for --group/-g arguments
    if let Some(partial) = extract_group_partial(input_ls) {
        let completions: Vec<String> = context
            .groups
            .clone()
            .into_iter()
            .filter(|g| fuzzy_match(partial, g))
            .collect();
        if !completions.is_empty() {
            let prefix = if partial.is_empty() {
                input_ls.to_string()
            } else {
                input_ls[..input_ls.len() - partial.len()].to_string()
            };
            return completions
                .into_iter()
                .map(|g| format!("{}{}", prefix, quote_token(&g)))
                .collect();
        }
    }

    // Flag/parameter completion (fires when last token starts with `-`)
    if let Some((prefix, partial)) = extract_flag_partial(input) {
        let cmd = shell_split(input).into_iter().next().unwrap_or_default();
        let completions = complete_flags(&cmd, &partial);
        if !completions.is_empty() {
            return completions
                .into_iter()
                .map(|f| format!("{}{}", prefix, f))
                .collect();
        }
    }

    // Color completion for --fg/--bg arguments
    if let Some(partial) = extract_color_partial(input_ls) {
        let completions = complete_color(partial);
        if !completions.is_empty() {
            let prefix = if partial.is_empty() {
                input_ls.to_string()
            } else {
                input_ls[..input_ls.len() - partial.len()].to_string()
            };
            return completions
                .into_iter()
                .map(|c| format!("{}{}", prefix, c))
                .collect();
        }
    }

    // Template completion for export -t/--template
    if let Some(partial) = extract_template_partial(input_ls) {
        let completions = crate::commands::complete_template(partial);
        if !completions.is_empty() {
            let prefix = if partial.is_empty() {
                input_ls.to_string()
            } else {
                input_ls[..input_ls.len() - partial.len()].to_string()
            };
            return completions
                .into_iter()
                .map(|c| format!("{}{}", prefix, c))
                .collect();
        }
    }

    // File path completion
    let input_ltrimmed = input.trim_start();
    let file_cmd = FILE_PATH_COMMANDS
        .iter()
        .find(|cmd| input_ltrimmed.starts_with(&format!("{} ", cmd)));
    if let Some(&cmd) = file_cmd {
        let partial = input_ltrimmed[cmd.len()..].trim_start();
        let default_dir: String;
        let effective_partial: &str = if partial.is_empty() {
            default_dir = context
                .source_file
                .as_deref()
                .and_then(|p| std::path::Path::new(p).parent())
                .and_then(|d| d.to_str())
                .map(|d| format!("{}/", d))
                .unwrap_or_default();
            &default_dir
        } else {
            partial
        };
        let completions = quoted_paths(effective_partial);
        if !completions.is_empty() {
            return completions
                .into_iter()
                .map(|c| format!("{} {}", cmd, c))
                .collect();
        }
        return vec![];
    }

    // hide-field: complete with all known field names
    if let Some(partial) = arg_partial(input, "hide-field") {
        let index = context.fields.clone();
        return index
            .names
            .iter()
            .filter(|n| fuzzy_match(partial, n))
            .map(|n| format!("hide-field {n}"))
            .collect();
    }

    // show-field: complete with currently hidden fields (or all if none hidden)
    if let Some(partial) = arg_partial(input, "show-field") {
        let candidates: Vec<String> = if context.hidden_fields.is_empty() {
            context.fields.clone().names
        } else {
            let mut v: Vec<String> = context.hidden_fields.iter().cloned().collect();
            v.sort();
            v
        };
        return candidates
            .iter()
            .filter(|n| fuzzy_match(partial, n))
            .map(|n| format!("show-field {n}"))
            .collect();
    }

    // toggle-group: complete with known filter group names
    if let Some(partial) = arg_partial(input, "toggle-group") {
        return context
            .groups
            .clone()
            .into_iter()
            .filter(|g| fuzzy_match(partial, g))
            .map(|g| format!("toggle-group {}", quote_token(&g)))
            .collect();
    }

    // group: complete the group-name argument with known group names
    // (both groups referenced by filters and predefined-but-filterless
    // groups — see `LogManager::group_names`).
    if let Some(partial) = arg_partial(input, "group") {
        return context
            .groups
            .clone()
            .into_iter()
            .filter(|g| fuzzy_match(partial, g))
            .map(|g| format!("group {}", quote_token(&g)))
            .collect();
    }

    // sidebar-position: complete with the two valid sides
    if let Some(partial) = arg_partial(input, "sidebar-position") {
        return ["left", "right"]
            .into_iter()
            .filter(|s| fuzzy_match(partial, s))
            .map(|s| format!("sidebar-position {s}"))
            .collect();
    }

    // schema: complete with known custom schema names and built-in formats
    if let Some(partial) = arg_partial(input, "schema") {
        let names: Vec<String> = crate::config::custom_schemas()
            .iter()
            .map(|s| s.name.clone())
            .collect();
        return crate::commands::auto_complete::complete_schema_with_builtins(partial, &names)
            .into_iter()
            .map(|(n, _)| format!("schema {n}"))
            .collect();
    }

    // default-filters: first arg is a format name, second (once present)
    // is a file path.
    if let Some(rest) = arg_partial(input, "default-filters") {
        if let Some((format, path_partial)) = rest.split_once(' ') {
            return quoted_paths(path_partial.trim_start())
                .into_iter()
                .map(|c| format!("default-filters {format} {c}"))
                .collect();
        }
        let names: Vec<String> = crate::config::custom_schemas()
            .iter()
            .map(|s| s.name.clone())
            .collect();
        return crate::commands::auto_complete::complete_schema_with_builtins(rest, &names)
            .into_iter()
            .map(|(n, _)| format!("default-filters {n}"))
            .collect();
    }

    // Theme completion
    if let Some(after_prefix) = trimmed.strip_prefix("set-theme") {
        let partial = after_prefix.trim_start();
        let mut themes = Theme::list_available_themes();
        if !partial.is_empty() {
            themes.retain(|t| fuzzy_match(partial, t));
        }
        if !themes.is_empty() {
            return themes
                .into_iter()
                .map(|t| format!("set-theme {}", t))
                .collect();
        }
        return vec![];
    }

    // Command name completion
    find_command_completions(&trimmed)
        .into_iter()
        .map(|s| s.to_string())
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionKind {
    Command,
    Field,
    Value,
    Flag,
    Color,
    Path,
    Theme,
    Schema,
    Group,
    Template,
    Argument,
}
#[derive(Debug, Clone)]
pub struct Suggestion {
    pub display: String,
    pub replacement: String,
    pub kind: CompletionKind,
    pub detail: String,
    pub help: Option<&'static crate::commands::CommandInfo>,
}
pub fn filesystem_query(input: &str) -> bool {
    FILE_PATH_COMMANDS
        .iter()
        .any(|c| input.trim_start().starts_with(&format!("{c} ")))
        || input
            .strip_prefix("default-filters ")
            .is_some_and(|s| s.trim_start().contains(' '))
}
pub fn suggestions(input: &str, context: &CompletionContext) -> Vec<Suggestion> {
    let command = shell_split(input).into_iter().next().unwrap_or_default();
    let help = crate::commands::COMMANDS.iter().find(|c| c.name == command);
    let kind = if !input.trim_start().contains(char::is_whitespace) {
        CompletionKind::Command
    } else if let Some(field) = extract_field_partial(input.trim_start()) {
        match field {
            FieldCompletion::Name(_) => CompletionKind::Field,
            _ => CompletionKind::Value,
        }
    } else if extract_group_partial(input.trim_start()).is_some() {
        CompletionKind::Group
    } else if extract_flag_partial(input).is_some() {
        CompletionKind::Flag
    } else if extract_color_partial(input.trim_start()).is_some() {
        CompletionKind::Color
    } else if extract_template_partial(input).is_some() {
        CompletionKind::Template
    } else if filesystem_query(input) {
        CompletionKind::Path
    } else if input.starts_with("set-theme") {
        CompletionKind::Theme
    } else if input.starts_with("schema ") || input.starts_with("default-filters ") {
        CompletionKind::Schema
    } else if input.starts_with("hide-field ") || input.starts_with("show-field ") {
        CompletionKind::Field
    } else if input.starts_with("group ") || input.starts_with("toggle-group ") {
        CompletionKind::Group
    } else if input.contains(' ') {
        CompletionKind::Argument
    } else {
        CompletionKind::Command
    };
    let replacements = if kind == CompletionKind::Command {
        find_command_completions(input.trim())
            .into_iter()
            .map(str::to_owned)
            .collect()
    } else {
        replacements(input, context)
    };
    replacements
        .into_iter()
        .map(|replacement| {
            let kind = if !replacement.contains(char::is_whitespace) {
                CompletionKind::Command
            } else {
                kind.clone()
            };
            let item_help = if kind == CompletionKind::Command {
                crate::commands::COMMANDS
                    .iter()
                    .find(|c| c.name == replacement)
            } else {
                help
            };
            let display = if kind == CompletionKind::Command {
                replacement.clone()
            } else {
                let value = shell_split(&replacement)
                    .pop()
                    .unwrap_or_else(|| replacement.clone());
                if kind == CompletionKind::Value {
                    value
                        .split_once('=')
                        .map(|(_, value)| value.to_owned())
                        .unwrap_or(value)
                } else if kind == CompletionKind::Field {
                    value.trim_end_matches('=').to_owned()
                } else {
                    value
                }
            };
            let detail = match kind {
                CompletionKind::Command => item_help
                    .map(|c| c.description.to_owned())
                    .unwrap_or_default(),
                CompletionKind::Schema => {
                    if crate::config::custom_schemas()
                        .iter()
                        .any(|s| s.name == display)
                    {
                        "Custom schema".into()
                    } else {
                        "Built-in schema".into()
                    }
                }
                CompletionKind::Path => {
                    if display.trim_end_matches('"').ends_with('/') {
                        "Directory".into()
                    } else {
                        "File".into()
                    }
                }
                _ => format!("{kind:?}"),
            };
            Suggestion {
                display,
                replacement,
                kind: kind.clone(),
                detail,
                help: item_help,
            }
        })
        .collect()
}

/// Match filesystem paths without quotes, then return shell-safe replacements.
fn quoted_paths(partial: &str) -> Vec<String> {
    let unquoted = if partial.starts_with('"') {
        shell_split(partial).into_iter().next().unwrap_or_default()
    } else {
        partial.to_owned()
    };
    complete_file_path(&unquoted)
        .into_iter()
        .map(|path| quote_token(&path))
        .collect()
}
fn quote_token(value: &str) -> String {
    if value.chars().any(|c| c.is_whitespace() || c == '"') {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        value.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn context() -> CompletionContext {
        let mut fields = FieldIndex {
            names: vec!["level".into(), "message".into(), "service".into()],
            ..Default::default()
        };
        fields
            .values
            .insert("level".into(), vec!["ERROR".into(), "INFO".into()]);
        fields
            .values
            .insert("message".into(), vec!["hello world".into(), "東京".into()]);
        CompletionContext {
            fields,
            groups: vec!["errors".into(), "payments team".into()],
            ..Default::default()
        }
    }
    #[test]
    fn structured_provider_covers_all_contextual_categories() {
        let cases = [
            ("filter --field lev", CompletionKind::Field),
            ("filter --field level=inf", CompletionKind::Value),
            ("filter test --f", CompletionKind::Flag),
            ("filter test --fg re", CompletionKind::Color),
            ("filter test --group er", CompletionKind::Group),
            ("group er", CompletionKind::Group),
            ("toggle-group er", CompletionKind::Group),
            ("set-theme dra", CompletionKind::Theme),
            ("schema sys", CompletionKind::Schema),
            ("default-filters sys", CompletionKind::Schema),
            ("export --template ", CompletionKind::Template),
            ("hide-field lev", CompletionKind::Field),
            ("show-field lev", CompletionKind::Field),
            ("sidebar-position le", CompletionKind::Argument),
        ];
        for (input, kind) in cases {
            let items = suggestions(input, &context());
            assert!(!items.is_empty(), "{input}");
            assert!(items.iter().all(|s| s.kind == kind), "{input}: {items:?}");
            assert!(items[0].help.is_some(), "{input}");
        }
    }
    #[test]
    fn names_use_the_tui_fuzzy_matching_order_and_case_handling() {
        for query in ["", "sf", "SF", "fIl", "lin", "notfound"] {
            let expected: Vec<_> = find_command_completions(query)
                .into_iter()
                .map(str::to_owned)
                .collect();
            assert_eq!(
                suggestions(query, &context())
                    .into_iter()
                    .map(|s| s.replacement)
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }
    #[test]
    fn quoted_and_unicode_values_roundtrip_through_the_parser() {
        let items = suggestions("filter --field message=\"hello w", &context());
        assert_eq!(items[0].display, "hello world");
        assert_eq!(
            shell_split(&items[0].replacement),
            ["filter", "--field", "message=hello world"]
        );
        assert!(
            crate::commands::CommandLine::try_parse_from(shell_split(&items[0].replacement))
                .is_ok()
        );
        let items = suggestions("filter --field message=東", &context());
        assert_eq!(items[0].display, "東京");
        assert_eq!(
            suggestions("group pay", &context())[0].replacement,
            "group \"payments team\""
        );
    }
    #[test]
    fn quoted_paths_directory_markers_and_default_directory_are_preserved() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a log 東京.txt"), "x").unwrap();
        std::fs::create_dir(dir.path().join("archive files")).unwrap();
        let mut context = context();
        context.source_file = Some(dir.path().join("source.log").display().to_string());
        let items = suggestions("open ", &context);
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|s| s.kind == CompletionKind::Path));
        assert!(
            items
                .iter()
                .any(|s| s.detail == "Directory" && s.display.ends_with('/'))
        );
        for item in &items {
            assert_eq!(shell_split(&item.replacement).len(), 2);
        }
        let query = format!("open \"{}/a log 東", dir.path().display());
        let items = suggestions(&query, &context);
        assert_eq!(items.len(), 1);
        assert!(items[0].display.ends_with("a log 東京.txt"));
        let query = format!("default-filters json {}/a log 東", dir.path().display());
        let items = suggestions(&query, &context);
        assert_eq!(items.len(), 1);
        assert_eq!(shell_split(&items[0].replacement).len(), 3);
    }
    #[test]
    fn hidden_fields_take_precedence_and_schemas_have_labels() {
        let mut context = context();
        context.hidden_fields.insert("service".into());
        assert_eq!(
            suggestions("show-field ", &context)
                .iter()
                .map(|s| s.display.as_str())
                .collect::<Vec<_>>(),
            ["service"]
        );
        assert!(
            suggestions("schema sys", &context)
                .iter()
                .all(|s| s.detail == "Built-in schema")
        );
        assert!(suggestions("filter --field unknown=", &context).is_empty());
    }
    use clap::Parser;
}
