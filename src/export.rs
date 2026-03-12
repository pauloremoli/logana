//! Template-based export of analysis (annotations + marked lines).
//!
//! Template syntax: `{{#section}}...{{/section}}` with recognized sections:
//! `header` (once), `comment_group` (per comment/mark entry), `footer` (once,
//! optional). Placeholders: `{{filename}}`, `{{date}}`, `{{commentary}}`,
//! `{{lines}}`, `{{line_numbers}}`.
//!
//! Templates are resolved from `~/.config/logana/templates/` → `templates/`
//! (dev CWD) → bundled templates embedded via `include_str!`. Bundled
//! templates: `markdown` and `jira`.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use crate::auto_complete::fuzzy_match;

/// Templates embedded into the binary at compile time.
/// Lookup order: user config dir → local `templates/` (dev) → here.
static BUNDLED_TEMPLATES: &[(&str, &str)] = &[
    ("markdown", include_str!("../templates/markdown.txt")),
    ("jira", include_str!("../templates/jira.txt")),
];
use crate::file_reader::FileReader;
use crate::parser::LogFormatParser;
use crate::types::{Comment, FieldLayout};
use crate::ui::field_layout::apply_field_layout;

/// A parsed export template with named sections.
#[derive(Debug, Clone)]
pub struct ExportTemplate {
    pub header: String,
    pub comment_group: String,
    pub marked_lines: Option<String>,
    pub footer: Option<String>,
}

/// Data required to render an export.
pub struct ExportData<'a> {
    pub filename: &'a str,
    pub comments: &'a [Comment],
    pub marked_indices: Vec<usize>,
    pub file_reader: &'a FileReader,
    pub parser: Option<&'a dyn LogFormatParser>,
    pub field_layout: &'a FieldLayout,
    pub hidden_fields: &'a HashSet<String>,
    pub show_keys: bool,
}

/// Parse raw template text into an `ExportTemplate`.
///
/// Recognized sections: `{{#header}}...{{/header}}`, `{{#comment_group}}...{{/comment_group}}`,
/// and optionally `{{#marked_lines}}...{{/marked_lines}}` (legacy, unused by bundled templates).
pub fn parse_template(raw: &str) -> Result<ExportTemplate, String> {
    let header = extract_section(raw, "header")?;
    let comment_group = extract_section(raw, "comment_group")?;
    let marked_lines = extract_section_optional(raw, "marked_lines");
    let footer = extract_section_optional(raw, "footer");
    Ok(ExportTemplate {
        header,
        comment_group,
        marked_lines,
        footer,
    })
}

fn extract_section(raw: &str, name: &str) -> Result<String, String> {
    extract_section_optional(raw, name).ok_or_else(|| format!("Missing required section: {}", name))
}

fn extract_section_optional(raw: &str, name: &str) -> Option<String> {
    let open = format!("{{{{#{}}}}}", name);
    let close = format!("{{{{/{}}}}}", name);
    let start = raw.find(&open)?;
    let after_open = start + open.len();
    let end = raw[after_open..].find(&close)?;
    Some(raw[after_open..after_open + end].to_string())
}

/// Load a template by name, checking the user config directory first, then bundled templates.
///
/// Lookup order: `~/.config/logana/templates/` → local `templates/` (dev) → bundled binary data.
pub fn load_template(name: &str) -> Result<ExportTemplate, String> {
    let filename = format!("{}.txt", name);
    let config_path =
        dirs::config_dir().map(|d| d.join("logana").join("templates").join(&filename));
    let local_path = Path::new("templates").join(&filename);

    let data = if config_path.as_ref().is_some_and(|p| p.exists()) {
        let cp = config_path.unwrap();
        fs::read_to_string(&cp).map_err(|e| format!("Failed to read template {:?}: {}", cp, e))?
    } else if local_path.exists() {
        fs::read_to_string(&local_path)
            .map_err(|e| format!("Failed to read template {:?}: {}", local_path, e))?
    } else {
        BUNDLED_TEMPLATES
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, raw)| raw.to_string())
            .ok_or_else(|| {
                format!(
                    "Template '{}' not found in config dir, local templates/, or bundled templates",
                    name
                )
            })?
    };

    parse_template(&data)
}

/// List all available template names (without extension).
///
/// Seeded from bundled templates, then overlaid with names from local `templates/` and
/// `~/.config/logana/templates/`. User-config and local names shadow bundled ones.
pub fn list_templates() -> Vec<String> {
    let mut set: HashSet<String> = BUNDLED_TEMPLATES
        .iter()
        .map(|(name, _)| name.to_string())
        .collect();

    let mut add_from_dir = |dir: &Path| {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) == Some("txt")
                    && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
                {
                    set.insert(stem.to_string());
                }
            }
        }
    };

    add_from_dir(Path::new("templates"));
    if let Some(config_dir) = dirs::config_dir() {
        add_from_dir(&config_dir.join("logana/templates"));
    }

    let mut names: Vec<String> = set.into_iter().collect();
    names.sort();
    names
}

/// Complete a partial template name using fuzzy matching.
pub fn complete_template(partial: &str) -> Vec<String> {
    let templates = list_templates();
    if partial.is_empty() {
        templates
    } else {
        templates
            .into_iter()
            .filter(|t| fuzzy_match(partial, t))
            .collect()
    }
}

/// A render entry representing either a comment or a group of marked lines.
enum RenderEntry<'a> {
    Comment { text: &'a str, indices: Vec<usize> },
    MarkedLines { indices: Vec<usize> },
}

impl RenderEntry<'_> {
    fn first_index(&self) -> usize {
        match self {
            RenderEntry::Comment { indices, .. } | RenderEntry::MarkedLines { indices } => {
                indices.first().copied().unwrap_or(0)
            }
        }
    }
}

/// Render an export document from a template and data.
///
/// Comments and marked lines (marks not covered by any comment) are
/// interleaved in log order rather than rendered as separate sections.
pub fn render_export(template: &ExportTemplate, data: &ExportData) -> String {
    let mut output = String::new();

    // Collect all line indices that belong to comments.
    let comment_line_set: HashSet<usize> = data
        .comments
        .iter()
        .flat_map(|c| c.line_indices.iter().copied())
        .collect();

    // Standalone marked lines = marked indices not covered by any comment.
    let mut standalone_marks: Vec<usize> = data
        .marked_indices
        .iter()
        .filter(|idx| !comment_line_set.contains(idx))
        .copied()
        .collect();
    standalone_marks.sort_unstable();

    // Build unified entry list: comments + grouped marked lines.
    let mut entries: Vec<RenderEntry> = Vec::new();

    for comment in data.comments {
        let mut indices = comment.line_indices.clone();
        indices.sort_unstable();
        entries.push(RenderEntry::Comment {
            text: &comment.text,
            indices,
        });
    }

    for group in group_consecutive(&standalone_marks) {
        entries.push(RenderEntry::MarkedLines { indices: group });
    }

    // Sort all entries by their first line index.
    entries.sort_by_key(RenderEntry::first_index);

    // Render header.
    let header = template
        .header
        .replace("{{filename}}", data.filename)
        .replace("{{date}}", &format_date());
    output.push_str(&header);

    // Render entries in log order using the comment_group template.
    for entry in &entries {
        let (commentary, indices) = match entry {
            RenderEntry::Comment { text, indices } => (*text, indices.as_slice()),
            RenderEntry::MarkedLines { indices } => ("", indices.as_slice()),
        };
        let lines = format_lines(indices, data);
        let line_numbers = format_line_numbers(indices);

        let section = template
            .comment_group
            .replace("{{commentary}}", commentary)
            .replace("{{lines}}", &lines)
            .replace("{{line_numbers}}", &line_numbers);
        output.push_str(&section);
    }

    // Render footer.
    if let Some(ref footer) = template.footer {
        output.push_str(footer);
    }

    output
}

/// Group sorted indices into runs of consecutive values.
fn group_consecutive(sorted: &[usize]) -> Vec<Vec<usize>> {
    if sorted.is_empty() {
        return vec![];
    }
    let mut groups = vec![vec![sorted[0]]];
    for &idx in &sorted[1..] {
        let extends_last = groups
            .last()
            .and_then(|g| g.last())
            .is_some_and(|&last| idx == last + 1);
        if extends_last {
            if let Some(last_group) = groups.last_mut() {
                last_group.push(idx);
            }
        } else {
            groups.push(vec![idx]);
        }
    }
    groups
}

/// Format a single log line using the detected parser (if available) or raw bytes.
/// Returns the rendered string as the TUI would display it.
fn render_line_content(line_bytes: &[u8], data: &ExportData) -> String {
    if let Some(parser) = data.parser
        && let Some(parts) = parser.parse_line(line_bytes)
    {
        let cols = apply_field_layout(
            &parts,
            data.field_layout,
            data.hidden_fields,
            data.show_keys,
        );
        if !cols.is_empty() {
            return cols.join(" ");
        }
    }
    String::from_utf8_lossy(line_bytes).into_owned()
}

/// Format line indices as `N: content` strings (1-based line numbers),
/// rendering each line through the detected format parser when available.
fn format_lines(indices: &[usize], data: &ExportData) -> String {
    indices
        .iter()
        .map(|&idx| {
            let content = if idx < data.file_reader.line_count() {
                render_line_content(data.file_reader.get_line(idx), data)
            } else {
                String::new()
            };
            format!("{}: {}", idx + 1, content)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Format line indices as a comma-separated list of 1-based numbers.
fn format_line_numbers(indices: &[usize]) -> String {
    indices
        .iter()
        .map(|idx| (idx + 1).to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Returns the current date as YYYY-MM-DD.
fn format_date() -> String {
    let now = time::OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02}",
        now.year(),
        now.month() as u8,
        now.day()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_template ────────────────────────────────────────────────

    #[test]
    fn test_parse_template_all_sections() {
        let raw = "{{#header}}H{{/header}}{{#comment_group}}C{{/comment_group}}{{#marked_lines}}O{{/marked_lines}}{{#footer}}F{{/footer}}";
        let tpl = parse_template(raw).unwrap();
        assert_eq!(tpl.header, "H");
        assert_eq!(tpl.comment_group, "C");
        assert_eq!(tpl.marked_lines.as_deref(), Some("O"));
        assert_eq!(tpl.footer.as_deref(), Some("F"));
    }

    #[test]
    fn test_parse_template_without_optional_sections() {
        let raw = "{{#header}}H{{/header}}{{#comment_group}}C{{/comment_group}}";
        let tpl = parse_template(raw).unwrap();
        assert_eq!(tpl.header, "H");
        assert_eq!(tpl.comment_group, "C");
        assert!(tpl.marked_lines.is_none());
        assert!(tpl.footer.is_none());
    }

    #[test]
    fn test_parse_template_missing_header() {
        let raw = "{{#comment_group}}C{{/comment_group}}";
        let err = parse_template(raw).unwrap_err();
        assert!(err.contains("header"));
    }

    #[test]
    fn test_parse_template_missing_comment_group() {
        let raw = "{{#header}}H{{/header}}";
        let err = parse_template(raw).unwrap_err();
        assert!(err.contains("comment_group"));
    }

    #[test]
    fn test_parse_template_preserves_whitespace() {
        let raw = "{{#header}}\n  Header\n{{/header}}{{#comment_group}}\nBody\n{{/comment_group}}";
        let tpl = parse_template(raw).unwrap();
        assert_eq!(tpl.header, "\n  Header\n");
        assert_eq!(tpl.comment_group, "\nBody\n");
    }

    #[test]
    fn test_parse_template_empty_sections() {
        let raw = "{{#header}}{{/header}}{{#comment_group}}{{/comment_group}}";
        let tpl = parse_template(raw).unwrap();
        assert_eq!(tpl.header, "");
        assert_eq!(tpl.comment_group, "");
    }

    // ── render_export ─────────────────────────────────────────────────

    fn make_reader(lines: &[&str]) -> FileReader {
        let data = lines.join("\n").into_bytes();
        FileReader::from_bytes(data)
    }

    fn simple_template() -> ExportTemplate {
        ExportTemplate {
            header: "File: {{filename}} Date: {{date}}\n".to_string(),
            comment_group: "Entry: {{commentary}} Lines: {{lines}} Numbers: {{line_numbers}}\n"
                .to_string(),
            marked_lines: None,
            footer: None,
        }
    }

    static DEFAULT_LAYOUT: std::sync::LazyLock<FieldLayout> =
        std::sync::LazyLock::new(FieldLayout::default);
    static EMPTY_HIDDEN: std::sync::LazyLock<HashSet<String>> =
        std::sync::LazyLock::new(HashSet::new);

    fn make_data<'a>(
        filename: &'a str,
        comments: &'a [Comment],
        marked_indices: Vec<usize>,
        file_reader: &'a FileReader,
    ) -> ExportData<'a> {
        ExportData {
            filename,
            comments,
            marked_indices,
            file_reader,
            parser: None,
            field_layout: &DEFAULT_LAYOUT,
            hidden_fields: &EMPTY_HIDDEN,
            show_keys: false,
        }
    }

    #[test]
    fn test_render_comments_and_marks() {
        let reader = make_reader(&["alpha", "beta", "gamma", "delta"]);
        let comments = vec![Comment {
            text: "My note".to_string(),
            line_indices: vec![0, 1],
        }];
        let data = make_data("test.log", &comments, vec![0, 2], &reader);
        let output = render_export(&simple_template(), &data);
        assert!(output.contains("File: test.log"));
        assert!(output.contains("Entry: My note"));
        assert!(output.contains("1: alpha"));
        assert!(output.contains("2: beta"));
        assert!(output.contains("Numbers: 1, 2"));
        // Line 2 (index 2) is marked but not in any comment → rendered as entry with empty commentary
        assert!(output.contains("3: gamma"));
    }

    #[test]
    fn test_render_no_comments_only_marks() {
        let reader = make_reader(&["line0", "line1"]);
        let data = make_data("f.log", &[], vec![0, 1], &reader);
        let output = render_export(&simple_template(), &data);
        // Marked lines rendered via comment_group with empty commentary
        assert!(output.contains("Entry:  Lines:"));
        assert!(output.contains("1: line0"));
        assert!(output.contains("2: line1"));
    }

    #[test]
    fn test_render_no_marks() {
        let reader = make_reader(&["line0"]);
        let comments = vec![Comment {
            text: "note".to_string(),
            line_indices: vec![0],
        }];
        let data = make_data("f.log", &comments, vec![], &reader);
        let output = render_export(&simple_template(), &data);
        assert!(output.contains("Entry: note"));
    }

    #[test]
    fn test_render_empty_data() {
        let reader = make_reader(&["line0"]);
        let data = make_data("f.log", &[], vec![], &reader);
        let output = render_export(&simple_template(), &data);
        assert!(output.contains("File: f.log"));
        assert!(!output.contains("Entry:"));
    }

    #[test]
    fn test_render_overlapping_indices_go_under_comment() {
        let reader = make_reader(&["a", "b", "c"]);
        let comments = vec![Comment {
            text: "note".to_string(),
            line_indices: vec![1],
        }];
        // Mark index 1 is also in the comment → should NOT appear as a separate entry
        let data = make_data("f.log", &comments, vec![1], &reader);
        let output = render_export(&simple_template(), &data);
        assert!(output.contains("Entry: note"));
        // Only one entry (the comment), not a separate mark entry
        assert_eq!(output.matches("Entry:").count(), 1);
    }

    #[test]
    fn test_render_1based_line_numbers() {
        let reader = make_reader(&["zero", "one", "two"]);
        let comments = vec![Comment {
            text: "x".to_string(),
            line_indices: vec![0, 2],
        }];
        let data = make_data("f.log", &comments, vec![], &reader);
        let output = render_export(&simple_template(), &data);
        assert!(output.contains("1: zero"));
        assert!(output.contains("3: two"));
        assert!(output.contains("Numbers: 1, 3"));
    }

    #[test]
    fn test_render_interleaved_order() {
        let reader = make_reader(&["a", "b", "c", "d", "e"]);
        let comments = vec![Comment {
            text: "late comment".to_string(),
            line_indices: vec![3, 4],
        }];
        // Marked lines at lines 0, 1 should appear BEFORE the comment at lines 3, 4
        let data = make_data("f.log", &comments, vec![0, 1], &reader);
        let output = render_export(&simple_template(), &data);
        let mark_pos = output.find("1: a").unwrap();
        let comment_pos = output.find("late comment").unwrap();
        assert!(
            mark_pos < comment_pos,
            "Marks at earlier lines should appear before later comments"
        );
    }

    #[test]
    fn test_render_marked_lines_grouped_consecutive() {
        let reader = make_reader(&["a", "b", "c", "d", "e"]);
        // Marks at 0, 1 (consecutive) and 4 (separate) → two groups
        let data = make_data("f.log", &[], vec![0, 1, 4], &reader);
        let output = render_export(&simple_template(), &data);
        // Two entry groups
        assert_eq!(output.matches("Entry:").count(), 2);
        // First group has lines 1, 2; second group has line 5
        assert!(output.contains("Numbers: 1, 2"));
        assert!(output.contains("Numbers: 5"));
    }

    #[test]
    fn test_render_marks_between_comments() {
        let reader = make_reader(&["a", "b", "c", "d", "e"]);
        let comments = vec![
            Comment {
                text: "first".to_string(),
                line_indices: vec![0],
            },
            Comment {
                text: "second".to_string(),
                line_indices: vec![4],
            },
        ];
        // Marked line at index 2 should appear between the two comments
        let data = make_data("f.log", &comments, vec![2], &reader);
        let output = render_export(&simple_template(), &data);
        let first_pos = output.find("first").unwrap();
        let mark_pos = output.find("3: c").unwrap();
        let second_pos = output.find("second").unwrap();
        assert!(first_pos < mark_pos);
        assert!(mark_pos < second_pos);
    }

    #[test]
    fn test_render_utf8_lossy() {
        let raw = vec![0xFF, b'\n', b'o', b'k'];
        let reader = FileReader::from_bytes(raw);
        let comments = vec![Comment {
            text: "x".to_string(),
            line_indices: vec![0],
        }];
        let data = make_data("f.log", &comments, vec![], &reader);
        let output = render_export(&simple_template(), &data);
        // Should not panic; the replacement character is used
        assert!(output.contains("1: "));
    }

    #[test]
    fn test_render_marked_lines_use_comment_group_template() {
        let tpl = ExportTemplate {
            header: "H\n".to_string(),
            comment_group: "[{{commentary}}] {{lines}}\n".to_string(),
            marked_lines: None,
            footer: None,
        };
        let reader = make_reader(&["line0"]);
        let data = make_data("f.log", &[], vec![0], &reader);
        // Marked lines rendered via comment_group with empty commentary
        let output = render_export(&tpl, &data);
        assert!(output.contains("[] 1: line0"));
    }

    #[test]
    fn test_render_comment_lines_sorted() {
        let reader = make_reader(&["a", "b", "c"]);
        let comments = vec![Comment {
            text: "note".to_string(),
            line_indices: vec![2, 0],
        }];
        let data = make_data("f.log", &comments, vec![], &reader);
        let output = render_export(&simple_template(), &data);
        // Lines should be sorted: 1: a before 3: c
        let pos_a = output.find("1: a").unwrap();
        let pos_c = output.find("3: c").unwrap();
        assert!(pos_a < pos_c);
    }

    #[test]
    fn test_render_footer_appended() {
        let tpl = ExportTemplate {
            header: "H\n".to_string(),
            comment_group: "C: {{commentary}}\n".to_string(),
            marked_lines: None,
            footer: Some("\n---\nConclusion\n".to_string()),
        };
        let reader = make_reader(&["line0"]);
        let comments = vec![Comment {
            text: "note".to_string(),
            line_indices: vec![0],
        }];
        let data = make_data("f.log", &comments, vec![], &reader);
        let output = render_export(&tpl, &data);
        assert!(output.ends_with("\n---\nConclusion\n"));
    }

    #[test]
    fn test_render_no_footer_when_absent() {
        let reader = make_reader(&["line0"]);
        let data = make_data("f.log", &[], vec![], &reader);
        let output = render_export(&simple_template(), &data);
        assert!(!output.contains("Conclusion"));
    }

    // ── render with parser ────────────────────────────────────────────

    #[test]
    fn test_render_with_parser_uses_formatted_output() {
        let json_line = r#"{"timestamp":"2024-01-01T00:00:00Z","level":"INFO","msg":"hello"}"#;
        let reader = make_reader(&[json_line]);
        let parser = crate::parser::detect_format(&[json_line.as_bytes()]);
        assert!(parser.is_some(), "JSON parser should detect the line");
        let parser = parser.unwrap();

        let layout = FieldLayout::default();
        let hidden = HashSet::new();
        let comments = vec![Comment {
            text: "note".to_string(),
            line_indices: vec![0],
        }];
        let data = ExportData {
            filename: "f.log",
            comments: &comments,
            marked_indices: vec![],
            file_reader: &reader,
            parser: Some(parser.as_ref()),
            field_layout: &layout,
            hidden_fields: &hidden,
            show_keys: false,
        };
        let output = render_export(&simple_template(), &data);
        // Should contain the parsed/formatted output, not raw JSON
        assert!(output.contains("2024-01-01T00:00:00Z"));
        assert!(output.contains("INFO"));
        assert!(output.contains("hello"));
        // Should NOT contain raw JSON braces in the lines section
        assert!(!output.contains(r#""level":"INFO""#));
    }

    #[test]
    fn test_render_without_parser_uses_raw_bytes() {
        let reader = make_reader(&["raw line content"]);
        let comments = vec![Comment {
            text: "note".to_string(),
            line_indices: vec![0],
        }];
        let data = make_data("f.log", &comments, vec![], &reader);
        let output = render_export(&simple_template(), &data);
        assert!(output.contains("1: raw line content"));
    }

    #[test]
    fn test_render_with_field_layout_respects_columns() {
        let json_line = r#"{"timestamp":"2024-01-01T00:00:00Z","level":"INFO","msg":"hello"}"#;
        let reader = make_reader(&[json_line]);
        let parser = crate::parser::detect_format(&[json_line.as_bytes()]).unwrap();

        let layout = FieldLayout {
            columns: Some(vec!["level".to_string(), "message".to_string()]),
        };
        let hidden = HashSet::new();
        let comments = vec![Comment {
            text: "note".to_string(),
            line_indices: vec![0],
        }];
        let data = ExportData {
            filename: "f.log",
            comments: &comments,
            marked_indices: vec![],
            file_reader: &reader,
            parser: Some(parser.as_ref()),
            field_layout: &layout,
            hidden_fields: &hidden,
            show_keys: false,
        };
        let output = render_export(&simple_template(), &data);
        // With only level+message columns, timestamp should NOT be in the output
        assert!(!output.contains("2024-01-01"));
        assert!(output.contains("INFO"));
        assert!(output.contains("hello"));
    }

    // ── load_template ─────────────────────────────────────────────────

    #[test]
    fn test_load_template_nonexistent() {
        let result = load_template("nonexistent_xyz_template");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found in config dir"));
    }

    #[test]
    fn test_load_template_bundled_markdown() {
        let result = load_template("markdown");
        assert!(result.is_ok());
        let tpl = result.unwrap();
        assert!(!tpl.header.is_empty());
        assert!(!tpl.comment_group.is_empty());
        assert!(tpl.marked_lines.is_none());
        assert!(tpl.footer.is_some());
        assert!(tpl.footer.as_ref().unwrap().contains("Conclusion"));
        assert!(tpl.footer.as_ref().unwrap().contains("Next Steps"));
    }

    #[test]
    fn test_load_template_bundled_jira() {
        let result = load_template("jira");
        assert!(result.is_ok());
        let tpl = result.unwrap();
        assert!(tpl.header.contains("h1."));
    }

    // ── list_templates ────────────────────────────────────────────────

    #[test]
    fn test_list_templates_includes_bundled() {
        let templates = list_templates();
        assert!(templates.contains(&"markdown".to_string()));
        assert!(templates.contains(&"jira".to_string()));
    }

    #[test]
    fn test_list_templates_sorted() {
        let templates = list_templates();
        let sorted = {
            let mut s = templates.clone();
            s.sort();
            s
        };
        assert_eq!(templates, sorted);
    }

    // ── complete_template ─────────────────────────────────────────────

    #[test]
    fn test_complete_template_empty_returns_all() {
        let results = complete_template("");
        assert!(results.contains(&"markdown".to_string()));
        assert!(results.contains(&"jira".to_string()));
    }

    #[test]
    fn test_complete_template_fuzzy_match() {
        let results = complete_template("md");
        assert!(results.contains(&"markdown".to_string()));
        assert!(!results.contains(&"jira".to_string()));
    }

    #[test]
    fn test_complete_template_no_match() {
        let results = complete_template("zzznomatch");
        assert!(results.is_empty());
    }

    // ── format_date ───────────────────────────────────────────────────

    #[test]
    fn test_format_date_pattern() {
        let date = format_date();
        // Should match YYYY-MM-DD
        assert_eq!(date.len(), 10);
        assert_eq!(&date[4..5], "-");
        assert_eq!(&date[7..8], "-");
        assert!(date[..4].parse::<u32>().is_ok());
        assert!(date[5..7].parse::<u32>().is_ok());
        assert!(date[8..10].parse::<u32>().is_ok());
    }

    // ── format_lines / format_line_numbers helpers ────────────────────

    #[test]
    fn test_format_lines_basic() {
        let reader = make_reader(&["alpha", "beta"]);
        let data = make_data("f.log", &[], vec![], &reader);
        let result = format_lines(&[0, 1], &data);
        assert_eq!(result, "1: alpha\n2: beta");
    }

    #[test]
    fn test_format_line_numbers_basic() {
        let result = format_line_numbers(&[0, 2, 5]);
        assert_eq!(result, "1, 3, 6");
    }

    #[test]
    fn test_format_lines_out_of_bounds() {
        let reader = make_reader(&["only"]);
        let data = make_data("f.log", &[], vec![], &reader);
        let result = format_lines(&[0, 99], &data);
        assert!(result.contains("1: only"));
        assert!(result.contains("100: "));
    }
}
