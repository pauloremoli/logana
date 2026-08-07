pub mod dlt;
pub mod keybindings;

pub use dlt::*;
pub use keybindings::*;

use serde::{Deserialize, Serialize};

pub const DEFAULT_PREVIEW_BYTES: u64 = 16 * 1024 * 1024;

/// Controls whether logana asks before restoring a previous session.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
    strum::Display,
    strum::EnumString,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum RestoreSessionPolicy {
    /// Ask before restoring.
    Ask,
    /// Restore without asking (default).
    #[default]
    Always,
    /// Never restore (skip without asking).
    Never,
}

/// Raw `level` field values (case-insensitive) that a custom schema wants
/// classified as Error/Warning, for schemas whose level values (e.g. `SEV1`)
/// don't match any of the built-in keywords (`error`, `warn`, `fatal`, …)
/// that `LogLevel::parse_level` recognizes. Drives both level coloring and
/// the `e`/`w` error/warning navigation keys.
#[derive(Debug, Default, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CustomLevelValues {
    #[serde(default)]
    pub error: Vec<String>,
    #[serde(default)]
    pub warning: Vec<String>,
}

/// One matcher tried against each continuation line following a matched
/// header, in the order declared in `ContinuationConfig::fields`. Same
/// `template`/`pattern` syntax as the top-level schema, but a continuation
/// field can only resolve to `extra` or a non-slot semantic role (see
/// `CustomParser::compile_matcher`'s `allow_slot_roles`) — `timestamp`,
/// `level`, `target` and `message` belong to the header alone.
#[derive(Debug, Default, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContinuationFieldSpec {
    #[serde(default)]
    pub template: Option<String>,
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(default)]
    pub fields: std::collections::HashMap<String, String>,
    /// When `true`, this spec must have exactly one placeholder; its
    /// captured text is parsed as a JSON object and each key becomes its
    /// own `extra` field (the `fields` role map is unused in this case).
    #[serde(default)]
    pub json: bool,
}

/// Structured extraction for a multiline custom-schema record: fields
/// carried on lines *after* the header, and where that record's
/// continuation block ends. See [`CustomSchemaConfig::continuation`].
#[derive(Debug, Default, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContinuationConfig {
    /// Template/literal matched against a continuation line to mark the end
    /// of this record's continuation block (e.g. `"### End transaction"`).
    /// Compiled the same way as a header `template`. Optional — when
    /// absent, the block still ends at the next line matching the header
    /// (today's continuation-grouping behavior, unchanged).
    #[serde(default)]
    pub end_pattern: Option<String>,
    /// Matchers tried in declared order against each continuation line; the
    /// first one that matches extracts that line's fields.
    #[serde(default)]
    pub fields: Vec<ContinuationFieldSpec>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CustomSchemaConfig {
    /// JSON Schema URL — accepted and ignored so editors can include a `$schema` line.
    #[serde(rename = "$schema", default, skip_serializing)]
    #[schemars(skip)]
    pub schema_url: Option<String>,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub template: Option<String>,
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(default)]
    pub fields: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub levels: CustomLevelValues,
    /// When `true`, continuation lines that follow a matched record (lines
    /// that don't match this schema's `template`/`pattern`) are merged into
    /// the record's `message` field, so field filters and the structured
    /// fields panel can see their content. See `merges_continuation_into_message`.
    #[serde(default)]
    pub multiline: bool,
    /// Structured field extraction from continuation lines (as opposed to
    /// `multiline`'s raw-text folding into `message`). See
    /// [`ContinuationConfig`].
    #[serde(default)]
    pub continuation: Option<ContinuationConfig>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[derive(Default)]
pub struct Config {
    /// JSON Schema URL — accepted and ignored so editors can include a `$schema` line.
    #[serde(rename = "$schema", default, skip_serializing)]
    #[schemars(skip)]
    pub schema_url: Option<String>,
    /// Theme name (without `.json` extension) to load on startup.
    pub theme: Option<String>,
    #[serde(default)]
    pub keybindings: Keybindings,
    /// When `Some`, overrides the DB-stored show_mode_bar runtime setting.
    #[serde(default)]
    pub show_mode_bar: Option<bool>,
    /// When `Some`, overrides the DB-stored show_borders runtime setting.
    #[serde(default)]
    pub show_borders: Option<bool>,
    /// Number of bytes to read for the instant preview shown while the full
    /// file index is being built in the background (default: 16 MiB).
    #[serde(default)]
    pub preview_bytes: Option<u64>,
    /// When `Some`, overrides the DB-stored restore_session runtime setting.
    #[serde(default)]
    pub restore_session: Option<RestoreSessionPolicy>,
    /// When `Some`, overrides the DB-stored restore_file_context runtime setting.
    #[serde(default)]
    pub restore_file_context: Option<RestoreSessionPolicy>,
    /// When `Some`, overrides the DB-stored show_sidebar runtime setting.
    #[serde(default)]
    pub show_sidebar: Option<bool>,
    /// When `Some`, overrides the DB-stored show_line_numbers runtime setting.
    #[serde(default)]
    pub show_line_numbers: Option<bool>,
    /// When `Some`, overrides the DB-stored relative_line_numbers runtime setting.
    #[serde(default)]
    pub relative_line_numbers: Option<bool>,
    /// When `Some`, overrides the DB-stored wrap runtime setting.
    #[serde(default)]
    pub wrap: Option<bool>,
    /// When `Some`, pins the filter sidebar to the given side, overriding the DB-stored value.
    #[serde(default)]
    pub sidebar_side: Option<crate::ui::SidebarSide>,
    #[serde(default)]
    pub dlt_devices: Vec<DltDevice>,
    /// Default port for the embedded MCP server. Used when `:enable-mcp` is invoked without
    /// an explicit `--port` argument. When `None`, falls back to the built-in default (9876).
    #[serde(default)]
    pub mcp_port: Option<u16>,
    /// Filter file to auto-load when a tab's format (built-in or custom
    /// schema name) is assigned and it has no filters yet. Entries here take
    /// priority per-key over the DB-persisted map that `:default-filters`/its
    /// popup write to.
    #[serde(default)]
    pub default_filter_files: Option<std::collections::HashMap<String, String>>,
}

impl Config {
    fn load_from_path(config_path: &std::path::Path) -> Result<Self, String> {
        let contents = match std::fs::read_to_string(config_path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Config::default());
            }
            Err(e) => {
                return Err(format!(
                    "Could not read config file '{}': {}",
                    config_path.display(),
                    e
                ));
            }
        };
        serde_json::from_str(&contents).map_err(|e| {
            format!(
                "Could not parse config file '{}': {}",
                config_path.display(),
                e
            )
        })
    }

    pub fn load() -> Result<Self, String> {
        let Some(config_path) = dirs::config_dir().map(|d| d.join("logana").join("config.json"))
        else {
            return Ok(Config::default());
        };
        Self::load_from_path(&config_path)
    }
}

/// Merges `config.json`'s `default_filter_files` with the DB-persisted map,
/// per-key: an entry in `config.json` always wins for that format name; the
/// DB-persisted map fills in/overrides any format `config.json` doesn't
/// mention. Unlike the `theme`-style precedent (a scalar, all-or-nothing
/// override), this is a map, so a per-key merge is the only way editing
/// `config.json` for one format doesn't blow away popup-set mappings for
/// every other format.
pub async fn resolve_default_filter_files(
    config: &Config,
    db: &dyn crate::db::AppSettingsStore,
) -> std::collections::HashMap<String, String> {
    let mut merged: std::collections::HashMap<String, String> = db
        .load_app_setting(crate::db::SettingsKey::DefaultFilterFiles)
        .await
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default();
    if let Some(from_config) = &config.default_filter_files {
        merged.extend(from_config.clone());
    }
    merged
}

static CUSTOM_SCHEMAS: std::sync::OnceLock<Vec<CustomSchemaConfig>> = std::sync::OnceLock::new();

/// Loads user-provided schemas from disk and returns any errors encountered
/// (e.g. malformed JSON) so the caller can surface them to the user — schema
/// files are meant to be hand-edited, so a typo shouldn't fail silently.
pub fn init_schemas() -> Vec<String> {
    let (schemas, errors) = schemas_dir().map(|d| load_schemas(&d)).unwrap_or_default();
    let _ = CUSTOM_SCHEMAS.set(schemas);
    errors
}

pub fn custom_schemas() -> &'static [CustomSchemaConfig] {
    CUSTOM_SCHEMAS.get().map(Vec::as_slice).unwrap_or_default()
}

/// Reads every `*.json` file in `schemas_dir` as a [`CustomSchemaConfig`].
/// Returns the successfully parsed schemas alongside a human-readable error
/// for each file that failed to parse — this only catches malformed JSON;
/// a syntactically valid schema with an invalid template/regex is caught
/// later by `parser::validate_custom_schemas`.
pub fn load_schemas(schemas_dir: &std::path::Path) -> (Vec<CustomSchemaConfig>, Vec<String>) {
    let entries = match std::fs::read_dir(schemas_dir) {
        Ok(e) => e,
        Err(_) => return (vec![], vec![]),
    };
    let mut schemas = Vec::new();
    let mut errors = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        match std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|s| serde_json::from_str::<CustomSchemaConfig>(&s).map_err(|e| e.to_string()))
        {
            Ok(schema) => schemas.push(schema),
            Err(e) => {
                errors.push(format!("could not load schema '{}': {e}", path.display()));
            }
        }
    }
    (schemas, errors)
}

pub fn schemas_dir() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|d| d.join("logana").join("schema"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::AppSettingsStore;
    use crossterm::event::{KeyCode, KeyModifiers};

    #[test]
    fn test_parse_single_char() {
        let kb = KeyBinding::parse("j").unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE));
    }

    #[test]
    fn test_parse_uppercase_char() {
        let kb = KeyBinding::parse("G").unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::Char('G'), KeyModifiers::NONE));
    }

    #[test]
    fn test_parse_tab() {
        let kb = KeyBinding::parse("Tab").unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::Tab, KeyModifiers::NONE));
    }

    #[test]
    fn test_parse_page_down() {
        let kb = KeyBinding::parse("PageDown").unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::PageDown, KeyModifiers::NONE));
    }

    #[test]
    fn test_parse_page_up() {
        let kb = KeyBinding::parse("PageUp").unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::PageUp, KeyModifiers::NONE));
    }

    #[test]
    fn test_parse_space() {
        let kb = KeyBinding::parse("Space").unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::Char(' '), KeyModifiers::NONE));
    }

    #[test]
    fn test_parse_esc() {
        let kb = KeyBinding::parse("Esc").unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::Esc, KeyModifiers::NONE));
    }

    #[test]
    fn test_parse_arrow_keys() {
        assert_eq!(
            KeyBinding::parse("Up").unwrap(),
            KeyBinding(KeyCode::Up, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("Down").unwrap(),
            KeyBinding(KeyCode::Down, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("Left").unwrap(),
            KeyBinding(KeyCode::Left, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("Right").unwrap(),
            KeyBinding(KeyCode::Right, KeyModifiers::NONE)
        );
    }

    #[test]
    fn test_parse_enter_backspace_delete() {
        assert_eq!(
            KeyBinding::parse("Enter").unwrap(),
            KeyBinding(KeyCode::Enter, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("Backspace").unwrap(),
            KeyBinding(KeyCode::Backspace, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("Delete").unwrap(),
            KeyBinding(KeyCode::Delete, KeyModifiers::NONE)
        );
    }

    #[test]
    fn test_parse_home_end_insert() {
        assert_eq!(
            KeyBinding::parse("Home").unwrap(),
            KeyBinding(KeyCode::Home, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("End").unwrap(),
            KeyBinding(KeyCode::End, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("Insert").unwrap(),
            KeyBinding(KeyCode::Insert, KeyModifiers::NONE)
        );
    }

    #[test]
    fn test_parse_f_keys() {
        assert_eq!(
            KeyBinding::parse("F1").unwrap(),
            KeyBinding(KeyCode::F(1), KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("F12").unwrap(),
            KeyBinding(KeyCode::F(12), KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("f5").unwrap(),
            KeyBinding(KeyCode::F(5), KeyModifiers::NONE)
        );
    }

    #[test]
    fn test_parse_invalid_f_key() {
        assert!(KeyBinding::parse("Fxx").is_err());
    }

    #[test]
    fn test_parse_lowercase_key_names() {
        assert_eq!(
            KeyBinding::parse("tab").unwrap(),
            KeyBinding(KeyCode::Tab, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("pagedown").unwrap(),
            KeyBinding(KeyCode::PageDown, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("pageup").unwrap(),
            KeyBinding(KeyCode::PageUp, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("space").unwrap(),
            KeyBinding(KeyCode::Char(' '), KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("esc").unwrap(),
            KeyBinding(KeyCode::Esc, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("up").unwrap(),
            KeyBinding(KeyCode::Up, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("down").unwrap(),
            KeyBinding(KeyCode::Down, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("left").unwrap(),
            KeyBinding(KeyCode::Left, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("right").unwrap(),
            KeyBinding(KeyCode::Right, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("enter").unwrap(),
            KeyBinding(KeyCode::Enter, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("backspace").unwrap(),
            KeyBinding(KeyCode::Backspace, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("delete").unwrap(),
            KeyBinding(KeyCode::Delete, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("home").unwrap(),
            KeyBinding(KeyCode::Home, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("end").unwrap(),
            KeyBinding(KeyCode::End, KeyModifiers::NONE)
        );
        assert_eq!(
            KeyBinding::parse("insert").unwrap(),
            KeyBinding(KeyCode::Insert, KeyModifiers::NONE)
        );
    }

    #[test]
    fn test_parse_ctrl_prefix() {
        let kb = KeyBinding::parse("Ctrl+d").unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::Char('d'), KeyModifiers::CONTROL));
    }

    #[test]
    fn test_parse_ctrl_lowercase_prefix() {
        let kb = KeyBinding::parse("ctrl+d").unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::Char('d'), KeyModifiers::CONTROL));
    }

    #[test]
    fn test_parse_alt_prefix() {
        let kb = KeyBinding::parse("Alt+x").unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::Char('x'), KeyModifiers::ALT));
    }

    #[test]
    fn test_parse_alt_lowercase_prefix() {
        let kb = KeyBinding::parse("alt+x").unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::Char('x'), KeyModifiers::ALT));
    }

    #[test]
    fn test_parse_shift_prefix() {
        let kb = KeyBinding::parse("Shift+Enter").unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::Enter, KeyModifiers::SHIFT));
    }

    #[test]
    fn test_parse_shift_lowercase_prefix() {
        let kb = KeyBinding::parse("shift+Enter").unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::Enter, KeyModifiers::SHIFT));
    }

    #[test]
    fn test_parse_shift_tab_special_alias() {
        let kb = KeyBinding::parse("Shift+Tab").unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::BackTab, KeyModifiers::NONE));
    }

    #[test]
    fn test_parse_shift_tab_case_insensitive() {
        let kb = KeyBinding::parse("shift+tab").unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::BackTab, KeyModifiers::NONE));
    }

    #[test]
    fn test_backtab_matches_with_shift_modifier() {
        let kb = KeyBinding(KeyCode::BackTab, KeyModifiers::NONE);
        assert!(kb.matches(KeyCode::BackTab, KeyModifiers::NONE));
        assert!(kb.matches(KeyCode::BackTab, KeyModifiers::SHIFT));
    }

    #[test]
    fn test_backtab_does_not_match_with_ctrl() {
        let kb = KeyBinding(KeyCode::BackTab, KeyModifiers::NONE);
        assert!(!kb.matches(KeyCode::BackTab, KeyModifiers::CONTROL));
        assert!(!kb.matches(
            KeyCode::BackTab,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        ));
    }

    #[test]
    fn test_parse_combined_ctrl_alt() {
        let kb = KeyBinding::parse("Ctrl+Alt+x").unwrap();
        assert_eq!(
            kb,
            KeyBinding(
                KeyCode::Char('x'),
                KeyModifiers::CONTROL | KeyModifiers::ALT
            )
        );
    }

    #[test]
    fn test_parse_ctrl_shift() {
        let kb = KeyBinding::parse("Ctrl+Shift+Enter").unwrap();
        assert_eq!(
            kb,
            KeyBinding(KeyCode::Enter, KeyModifiers::CONTROL | KeyModifiers::SHIFT)
        );
    }

    #[test]
    fn test_parse_invalid_returns_err() {
        assert!(KeyBinding::parse("NotAKey").is_err());
    }

    #[test]
    fn test_display_backtab() {
        let kb = KeyBinding(KeyCode::BackTab, KeyModifiers::NONE);
        assert_eq!(kb.display(), "Shift+Tab");
    }

    #[test]
    fn test_display_ctrl_modifier() {
        let kb = KeyBinding(KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert_eq!(kb.display(), "Ctrl+d");
    }

    #[test]
    fn test_display_alt_modifier() {
        let kb = KeyBinding(KeyCode::Char('x'), KeyModifiers::ALT);
        assert_eq!(kb.display(), "Alt+x");
    }

    #[test]
    fn test_display_shift_modifier() {
        let kb = KeyBinding(KeyCode::Enter, KeyModifiers::SHIFT);
        assert_eq!(kb.display(), "Shift+Enter");
    }

    #[test]
    fn test_display_combined_modifiers() {
        let kb = KeyBinding(
            KeyCode::Char('x'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        );
        assert_eq!(kb.display(), "Ctrl+Alt+x");
    }

    #[test]
    fn test_display_named_keys() {
        assert_eq!(
            KeyBinding(KeyCode::Tab, KeyModifiers::NONE).display(),
            "Tab"
        );
        assert_eq!(
            KeyBinding(KeyCode::PageDown, KeyModifiers::NONE).display(),
            "PageDown"
        );
        assert_eq!(
            KeyBinding(KeyCode::PageUp, KeyModifiers::NONE).display(),
            "PageUp"
        );
        assert_eq!(
            KeyBinding(KeyCode::Char(' '), KeyModifiers::NONE).display(),
            "Space"
        );
        assert_eq!(
            KeyBinding(KeyCode::Esc, KeyModifiers::NONE).display(),
            "Esc"
        );
        assert_eq!(KeyBinding(KeyCode::Up, KeyModifiers::NONE).display(), "Up");
        assert_eq!(
            KeyBinding(KeyCode::Down, KeyModifiers::NONE).display(),
            "Down"
        );
        assert_eq!(
            KeyBinding(KeyCode::Left, KeyModifiers::NONE).display(),
            "Left"
        );
        assert_eq!(
            KeyBinding(KeyCode::Right, KeyModifiers::NONE).display(),
            "Right"
        );
        assert_eq!(
            KeyBinding(KeyCode::Enter, KeyModifiers::NONE).display(),
            "Enter"
        );
        assert_eq!(
            KeyBinding(KeyCode::Backspace, KeyModifiers::NONE).display(),
            "Backspace"
        );
        assert_eq!(
            KeyBinding(KeyCode::Delete, KeyModifiers::NONE).display(),
            "Delete"
        );
        assert_eq!(
            KeyBinding(KeyCode::Home, KeyModifiers::NONE).display(),
            "Home"
        );
        assert_eq!(
            KeyBinding(KeyCode::End, KeyModifiers::NONE).display(),
            "End"
        );
        assert_eq!(
            KeyBinding(KeyCode::Insert, KeyModifiers::NONE).display(),
            "Insert"
        );
    }

    #[test]
    fn test_display_f_key() {
        assert_eq!(
            KeyBinding(KeyCode::F(1), KeyModifiers::NONE).display(),
            "F1"
        );
        assert_eq!(
            KeyBinding(KeyCode::F(12), KeyModifiers::NONE).display(),
            "F12"
        );
    }

    #[test]
    fn test_display_char() {
        assert_eq!(
            KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE).display(),
            "j"
        );
        assert_eq!(
            KeyBinding(KeyCode::Char('G'), KeyModifiers::NONE).display(),
            "G"
        );
    }

    #[test]
    fn test_display_roundtrip() {
        let cases = vec![
            "j",
            "G",
            "Tab",
            "PageDown",
            "PageUp",
            "Space",
            "Esc",
            "Up",
            "Down",
            "Left",
            "Right",
            "Enter",
            "Backspace",
            "Delete",
            "Home",
            "End",
            "Insert",
            "F1",
            "Ctrl+d",
            "Alt+x",
            "Shift+Enter",
            "Shift+Tab",
        ];
        for s in cases {
            let kb = KeyBinding::parse(s).unwrap();
            let displayed = kb.display();
            let reparsed = KeyBinding::parse(&displayed).unwrap();
            assert_eq!(kb, reparsed, "Roundtrip failed for {:?}", s);
        }
    }

    #[test]
    fn test_matches_exact() {
        let kb = KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE);
        assert!(kb.matches(KeyCode::Char('j'), KeyModifiers::NONE));
    }

    #[test]
    fn test_matches_accepts_shift_when_no_ctrl_alt() {
        let kb = KeyBinding(KeyCode::Char('G'), KeyModifiers::NONE);
        assert!(kb.matches(KeyCode::Char('G'), KeyModifiers::SHIFT));
    }

    #[test]
    fn test_matches_rejects_ctrl_when_not_in_binding() {
        let kb = KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE);
        assert!(!kb.matches(KeyCode::Char('j'), KeyModifiers::CONTROL));
    }

    #[test]
    fn test_matches_rejects_alt_when_not_in_binding() {
        let kb = KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE);
        assert!(!kb.matches(KeyCode::Char('j'), KeyModifiers::ALT));
    }

    #[test]
    fn test_matches_ctrl_binding_requires_ctrl() {
        let kb = KeyBinding(KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert!(kb.matches(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert!(!kb.matches(KeyCode::Char('d'), KeyModifiers::NONE));
    }

    #[test]
    fn test_matches_alt_binding_requires_alt() {
        let kb = KeyBinding(KeyCode::Char('x'), KeyModifiers::ALT);
        assert!(kb.matches(KeyCode::Char('x'), KeyModifiers::ALT));
        assert!(!kb.matches(KeyCode::Char('x'), KeyModifiers::NONE));
    }

    #[test]
    fn test_matches_wrong_key() {
        let kb = KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE);
        assert!(!kb.matches(KeyCode::Char('k'), KeyModifiers::NONE));
    }

    #[test]
    fn test_matches_non_char_shift_exact() {
        let kb = KeyBinding(KeyCode::Enter, KeyModifiers::SHIFT);
        assert!(kb.matches(KeyCode::Enter, KeyModifiers::SHIFT));
        assert!(!kb.matches(KeyCode::Enter, KeyModifiers::NONE));
    }

    #[test]
    fn test_matches_non_char_no_shift_rejects_shift() {
        let kb = KeyBinding(KeyCode::Enter, KeyModifiers::NONE);
        assert!(kb.matches(KeyCode::Enter, KeyModifiers::NONE));
        assert!(!kb.matches(KeyCode::Enter, KeyModifiers::SHIFT));
    }

    #[test]
    fn test_matches_non_char_rejects_ctrl() {
        let kb = KeyBinding(KeyCode::Enter, KeyModifiers::NONE);
        assert!(!kb.matches(KeyCode::Enter, KeyModifiers::CONTROL));
    }

    #[test]
    fn test_matches_ctrl_alt_combined() {
        let kb = KeyBinding(
            KeyCode::Char('x'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        );
        assert!(kb.matches(
            KeyCode::Char('x'),
            KeyModifiers::CONTROL | KeyModifiers::ALT
        ));
        assert!(!kb.matches(KeyCode::Char('x'), KeyModifiers::CONTROL));
        assert!(!kb.matches(KeyCode::Char('x'), KeyModifiers::ALT));
        assert!(!kb.matches(KeyCode::Char('x'), KeyModifiers::NONE));
    }

    #[test]
    fn test_keybinding_serialize() {
        let kb = KeyBinding(KeyCode::Char('d'), KeyModifiers::CONTROL);
        let json = serde_json::to_string(&kb).unwrap();
        assert_eq!(json, r#""Ctrl+d""#);
    }

    #[test]
    fn test_keybinding_deserialize() {
        let kb: KeyBinding = serde_json::from_str(r#""Alt+x""#).unwrap();
        assert_eq!(kb, KeyBinding(KeyCode::Char('x'), KeyModifiers::ALT));
    }

    #[test]
    fn test_keybinding_deserialize_invalid() {
        let result: Result<KeyBinding, _> = serde_json::from_str(r#""NotAKey""#);
        assert!(result.is_err());
    }

    #[test]
    fn test_keybindings_matches_any() {
        let kbs = KeyBindings(vec![
            KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE),
            KeyBinding(KeyCode::Down, KeyModifiers::NONE),
        ]);
        assert!(kbs.matches(KeyCode::Char('j'), KeyModifiers::NONE));
        assert!(kbs.matches(KeyCode::Down, KeyModifiers::NONE));
        assert!(!kbs.matches(KeyCode::Char('k'), KeyModifiers::NONE));
    }

    #[test]
    fn test_keybindings_matches_empty() {
        let kbs = KeyBindings(vec![]);
        assert!(!kbs.matches(KeyCode::Char('j'), KeyModifiers::NONE));
    }

    #[test]
    fn test_keybindings_display_single() {
        let kbs = KeyBindings(vec![KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE)]);
        assert_eq!(kbs.display(), "j");
    }

    #[test]
    fn test_keybindings_display_multi() {
        let kbs = KeyBindings(vec![
            KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE),
            KeyBinding(KeyCode::Down, KeyModifiers::NONE),
        ]);
        assert_eq!(kbs.display(), "j/Down");
    }

    #[test]
    fn test_keybindings_display_empty() {
        let kbs = KeyBindings(vec![]);
        assert_eq!(kbs.display(), "");
    }

    #[test]
    fn test_keybindings_deserialize_string() {
        let kbs: KeyBindings = serde_json::from_str(r#""j""#).unwrap();
        assert_eq!(kbs.0.len(), 1);
        assert!(kbs.matches(KeyCode::Char('j'), KeyModifiers::NONE));
    }

    #[test]
    fn test_keybindings_deserialize_array() {
        let kbs: KeyBindings = serde_json::from_str(r#"["j", "Down"]"#).unwrap();
        assert_eq!(kbs.0.len(), 2);
        assert!(kbs.matches(KeyCode::Char('j'), KeyModifiers::NONE));
        assert!(kbs.matches(KeyCode::Down, KeyModifiers::NONE));
    }

    #[test]
    fn test_keybindings_serialize_single_as_string() {
        let kbs = KeyBindings(vec![KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE)]);
        let json = serde_json::to_string(&kbs).unwrap();
        assert_eq!(json, r#""j""#);
    }

    #[test]
    fn test_keybindings_serialize_multi_as_array() {
        let kbs = KeyBindings(vec![
            KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE),
            KeyBinding(KeyCode::Down, KeyModifiers::NONE),
        ]);
        let json = serde_json::to_string(&kbs).unwrap();
        assert_eq!(json, r#"["j","Down"]"#);
    }

    #[test]
    fn test_keybindings_serde_roundtrip() {
        let original = KeyBindings(vec![
            KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE),
            KeyBinding(KeyCode::Down, KeyModifiers::NONE),
        ]);
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: KeyBindings = serde_json::from_str(&json).unwrap();
        assert_eq!(original, deserialized);
    }

    #[test]
    fn test_has_overlap_true() {
        let a = KeyBindings(vec![
            KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE),
            KeyBinding(KeyCode::Down, KeyModifiers::NONE),
        ]);
        let b = KeyBindings(vec![KeyBinding(KeyCode::Down, KeyModifiers::NONE)]);
        assert!(a.has_overlap(&b));
    }

    #[test]
    fn test_has_overlap_false() {
        let a = KeyBindings(vec![KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE)]);
        let b = KeyBindings(vec![KeyBinding(KeyCode::Char('k'), KeyModifiers::NONE)]);
        assert!(!a.has_overlap(&b));
    }

    #[test]
    fn test_has_overlap_same_key_different_modifiers() {
        let a = KeyBindings(vec![KeyBinding(KeyCode::Char('d'), KeyModifiers::NONE)]);
        let b = KeyBindings(vec![KeyBinding(KeyCode::Char('d'), KeyModifiers::CONTROL)]);
        assert!(!a.has_overlap(&b));
    }

    #[test]
    fn test_has_overlap_empty() {
        let a = KeyBindings(vec![]);
        let b = KeyBindings(vec![KeyBinding(KeyCode::Char('j'), KeyModifiers::NONE)]);
        assert!(!a.has_overlap(&b));
    }

    #[test]
    fn test_validate_default_no_conflicts() {
        let kb = Keybindings::default();
        let conflicts = kb.validate();
        assert!(
            conflicts.is_empty(),
            "Default keybindings should have no conflicts, got: {:?}",
            conflicts
        );
    }

    #[test]
    fn test_validate_detects_normal_conflict() {
        let mut kb = Keybindings::default();
        kb.navigation.scroll_down =
            KeyBindings(vec![KeyBinding(KeyCode::Char('k'), KeyModifiers::NONE)]);
        let conflicts = kb.validate();
        assert!(
            !conflicts.is_empty(),
            "Should detect conflict between scroll_down and scroll_up"
        );
        assert!(conflicts[0].contains("scroll_down"));
        assert!(conflicts[0].contains("scroll_up"));
    }

    #[test]
    fn test_validate_detects_normal_global_conflict() {
        let mut kb = Keybindings::default();
        kb.navigation.scroll_down =
            KeyBindings(vec![KeyBinding(KeyCode::Char('q'), KeyModifiers::NONE)]);
        let conflicts = kb.validate();
        assert!(
            !conflicts.is_empty(),
            "Should detect conflict between scroll_down and global quit"
        );
        let joined = conflicts.join(" ");
        assert!(joined.contains("scroll_down"));
        assert!(joined.contains("quit"));
    }

    #[test]
    fn test_validate_detects_filter_conflict() {
        let mut kb = Keybindings::default();
        kb.navigation.scroll_up =
            KeyBindings(vec![KeyBinding(KeyCode::Char(' '), KeyModifiers::NONE)]);
        let conflicts = kb.validate();
        assert!(
            !conflicts.is_empty(),
            "Should detect conflict between navigation.scroll_up and toggle_filter"
        );
    }

    #[test]
    fn test_navigation_keybindings_default() {
        let kb = NavigationKeybindings::default();
        assert!(
            kb.scroll_down
                .matches(KeyCode::Char('j'), KeyModifiers::NONE)
        );
        assert!(kb.scroll_down.matches(KeyCode::Down, KeyModifiers::NONE));
        assert!(kb.scroll_up.matches(KeyCode::Char('k'), KeyModifiers::NONE));
        assert!(kb.scroll_up.matches(KeyCode::Up, KeyModifiers::NONE));
        assert!(
            kb.half_page_down
                .matches(KeyCode::Char('d'), KeyModifiers::CONTROL)
        );
        assert!(
            kb.half_page_up
                .matches(KeyCode::Char('u'), KeyModifiers::CONTROL)
        );
        assert!(kb.page_down.matches(KeyCode::PageDown, KeyModifiers::NONE));
        assert!(kb.page_up.matches(KeyCode::PageUp, KeyModifiers::NONE));
    }

    #[test]
    fn test_normal_keybindings_default() {
        let kb = NormalKeybindings::default();
        assert!(
            kb.scroll_left
                .matches(KeyCode::Char('h'), KeyModifiers::NONE)
        );
        assert!(kb.scroll_left.matches(KeyCode::Left, KeyModifiers::NONE));
        assert!(
            kb.scroll_right
                .matches(KeyCode::Char('l'), KeyModifiers::NONE)
        );
        assert!(kb.scroll_right.matches(KeyCode::Right, KeyModifiers::NONE));
        assert!(
            kb.command_mode
                .matches(KeyCode::Char(':'), KeyModifiers::NONE)
        );
        assert!(
            kb.filter_mode
                .matches(KeyCode::Char('f'), KeyModifiers::NONE)
        );
        assert!(
            kb.toggle_filtering
                .matches(KeyCode::Char('F'), KeyModifiers::NONE)
        );
        assert!(
            kb.filter_include
                .matches(KeyCode::Char('i'), KeyModifiers::NONE)
        );
        assert!(
            kb.filter_exclude
                .matches(KeyCode::Char('o'), KeyModifiers::NONE)
        );
        assert!(
            kb.enter_ui_mode
                .matches(KeyCode::Char('u'), KeyModifiers::NONE)
        );
        assert!(
            kb.go_to_top_chord
                .matches(KeyCode::Char('g'), KeyModifiers::NONE)
        );
        assert!(
            kb.go_to_bottom
                .matches(KeyCode::Char('G'), KeyModifiers::NONE)
        );
        assert!(kb.mark_line.matches(KeyCode::Char('m'), KeyModifiers::NONE));
        assert!(
            kb.search_forward
                .matches(KeyCode::Char('/'), KeyModifiers::NONE)
        );
        assert!(
            kb.search_backward
                .matches(KeyCode::Char('?'), KeyModifiers::NONE)
        );
        assert!(
            kb.next_match
                .matches(KeyCode::Char('n'), KeyModifiers::NONE)
        );
        assert!(
            kb.prev_match
                .matches(KeyCode::Char('N'), KeyModifiers::NONE)
        );
        assert!(
            kb.visual_mode
                .matches(KeyCode::Char('V'), KeyModifiers::NONE)
        );
        assert!(
            kb.toggle_marks_only
                .matches(KeyCode::Char('M'), KeyModifiers::NONE)
        );
        assert!(kb.yank_line.matches(KeyCode::Char('y'), KeyModifiers::NONE));
        assert!(
            kb.yank_marked
                .matches(KeyCode::Char('Y'), KeyModifiers::NONE)
        );
        assert!(
            kb.show_keybindings
                .matches(KeyCode::F(1), KeyModifiers::NONE)
        );
        assert!(kb.clear_all.matches(KeyCode::Char('C'), KeyModifiers::NONE));
        assert!(
            kb.edit_comment
                .matches(KeyCode::Char('r'), KeyModifiers::NONE)
        );
        assert!(
            kb.delete_comment
                .matches(KeyCode::Char('d'), KeyModifiers::NONE)
        );
    }

    #[test]
    fn test_filter_keybindings_default() {
        let kb = FilterKeybindings::default();
        assert!(
            kb.toggle_filter
                .matches(KeyCode::Char(' '), KeyModifiers::NONE)
        );
        assert!(
            kb.delete_filter
                .matches(KeyCode::Char('d'), KeyModifiers::NONE)
        );
        assert!(
            kb.move_filter_up
                .matches(KeyCode::Char('K'), KeyModifiers::NONE)
        );
        assert!(
            kb.move_filter_down
                .matches(KeyCode::Char('J'), KeyModifiers::NONE)
        );
        assert!(
            kb.edit_filter
                .matches(KeyCode::Char('e'), KeyModifiers::NONE)
        );
        assert!(kb.set_color.matches(KeyCode::Char('c'), KeyModifiers::NONE));
        assert!(
            kb.toggle_all_filters
                .matches(KeyCode::Char('A'), KeyModifiers::NONE)
        );
        assert!(
            kb.clear_all_filters
                .matches(KeyCode::Char('C'), KeyModifiers::NONE)
        );
        assert!(kb.exit_mode.matches(KeyCode::Esc, KeyModifiers::NONE));
    }

    #[test]
    fn test_global_keybindings_default() {
        let kb = GlobalKeybindings::default();
        assert!(kb.quit.matches(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(kb.next_tab.matches(KeyCode::Tab, KeyModifiers::NONE));
        assert!(kb.prev_tab.matches(KeyCode::BackTab, KeyModifiers::NONE));
        assert!(
            kb.close_tab
                .matches(KeyCode::Char('w'), KeyModifiers::CONTROL)
        );
        assert!(
            kb.new_tab
                .matches(KeyCode::Char('t'), KeyModifiers::CONTROL)
        );
        assert!(
            kb.file_switcher
                .matches(KeyCode::Char('p'), KeyModifiers::CONTROL)
        );
    }

    #[test]
    fn test_comment_keybindings_default() {
        let kb = CommentKeybindings::default();
        assert!(kb.save.matches(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(kb.newline.matches(KeyCode::Enter, KeyModifiers::NONE));
        assert!(kb.cancel.matches(KeyCode::Esc, KeyModifiers::NONE));
        assert!(kb.delete.matches(KeyCode::Char('d'), KeyModifiers::CONTROL));
    }

    #[test]
    fn test_search_keybindings_default() {
        let kb = SearchKeybindings::default();
        assert!(kb.cancel.matches(KeyCode::Esc, KeyModifiers::NONE));
        assert!(kb.confirm.matches(KeyCode::Enter, KeyModifiers::NONE));
    }

    #[test]
    fn test_filter_edit_keybindings_default() {
        let kb = FilterEditKeybindings::default();
        assert!(kb.cancel.matches(KeyCode::Esc, KeyModifiers::NONE));
        assert!(kb.confirm.matches(KeyCode::Enter, KeyModifiers::NONE));
    }

    #[test]
    fn test_command_keybindings_default() {
        let kb = CommandModeKeybindings::default();
        assert!(kb.cancel.matches(KeyCode::Esc, KeyModifiers::NONE));
        assert!(kb.confirm.matches(KeyCode::Enter, KeyModifiers::NONE));
    }

    #[test]
    fn test_docker_select_keybindings_default() {
        let kb = DockerSelectKeybindings::default();
        assert!(kb.confirm.matches(KeyCode::Enter, KeyModifiers::NONE));
        assert!(kb.cancel.matches(KeyCode::Esc, KeyModifiers::NONE));
    }

    #[test]
    fn test_value_colors_keybindings_default() {
        let kb = ValueColorsKeybindings::default();
        assert!(kb.toggle.matches(KeyCode::Char(' '), KeyModifiers::NONE));
        assert!(kb.all.matches(KeyCode::Char('a'), KeyModifiers::NONE));
        assert!(kb.none.matches(KeyCode::Char('n'), KeyModifiers::NONE));
        assert!(kb.apply.matches(KeyCode::Enter, KeyModifiers::NONE));
        assert!(kb.cancel.matches(KeyCode::Esc, KeyModifiers::NONE));
    }

    #[test]
    fn test_select_fields_keybindings_default() {
        let kb = SelectFieldsKeybindings::default();
        assert!(kb.toggle.matches(KeyCode::Char(' '), KeyModifiers::NONE));
        assert!(kb.move_up.matches(KeyCode::Char('K'), KeyModifiers::NONE));
        assert!(kb.move_down.matches(KeyCode::Char('J'), KeyModifiers::NONE));
        assert!(kb.all.matches(KeyCode::Char('a'), KeyModifiers::NONE));
        assert!(kb.none.matches(KeyCode::Char('n'), KeyModifiers::NONE));
        assert!(kb.reset.matches(KeyCode::Char('r'), KeyModifiers::NONE));
        assert!(kb.apply.matches(KeyCode::Enter, KeyModifiers::NONE));
        assert!(kb.cancel.matches(KeyCode::Esc, KeyModifiers::NONE));
    }

    #[test]
    fn test_help_keybindings_default() {
        let kb = HelpKeybindings::default();
        assert!(kb.close.matches(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(kb.close.matches(KeyCode::Esc, KeyModifiers::NONE));
    }

    #[test]
    fn test_confirm_keybindings_default() {
        let kb = ConfirmKeybindings::default();
        assert!(kb.yes.matches(KeyCode::Char('y'), KeyModifiers::NONE));
        assert!(kb.yes.matches(KeyCode::Enter, KeyModifiers::NONE));
        assert!(kb.no.matches(KeyCode::Char('n'), KeyModifiers::NONE));
        assert!(kb.no.matches(KeyCode::Esc, KeyModifiers::NONE));
        assert!(kb.always.matches(KeyCode::Char('Y'), KeyModifiers::SHIFT));
        assert!(kb.never.matches(KeyCode::Char('N'), KeyModifiers::SHIFT));
    }

    #[test]
    fn test_restore_session_policy_default_is_always() {
        assert_eq!(
            RestoreSessionPolicy::default(),
            RestoreSessionPolicy::Always
        );
    }

    #[test]
    fn test_restore_session_policy_serializes() {
        let ask = serde_json::to_string(&RestoreSessionPolicy::Ask).unwrap();
        let always = serde_json::to_string(&RestoreSessionPolicy::Always).unwrap();
        let never = serde_json::to_string(&RestoreSessionPolicy::Never).unwrap();
        assert_eq!(ask, "\"ask\"");
        assert_eq!(always, "\"always\"");
        assert_eq!(never, "\"never\"");
    }

    #[test]
    fn test_restore_session_policy_deserializes() {
        let ask: RestoreSessionPolicy = serde_json::from_str("\"ask\"").unwrap();
        let always: RestoreSessionPolicy = serde_json::from_str("\"always\"").unwrap();
        let never: RestoreSessionPolicy = serde_json::from_str("\"never\"").unwrap();
        assert_eq!(ask, RestoreSessionPolicy::Ask);
        assert_eq!(always, RestoreSessionPolicy::Always);
        assert_eq!(never, RestoreSessionPolicy::Never);
    }

    #[test]
    fn test_config_restore_session_defaults_to_ask() {
        let config = Config::default();
        assert!(config.restore_session.is_none());
    }

    #[test]
    fn test_config_restore_session_deserializes_from_json() {
        let json = r#"{"restore_session": "always"}"#;
        let config: Config = serde_json::from_str(json).unwrap();
        assert_eq!(config.restore_session, Some(RestoreSessionPolicy::Always));
    }

    #[test]
    fn test_config_load_fallback_on_missing_file() {
        let config = Config::default();
        assert!(config.theme.is_none());
        assert!(
            config
                .keybindings
                .global
                .quit
                .matches(KeyCode::Char('q'), KeyModifiers::NONE)
        );
    }

    #[test]
    fn test_config_deserialize_theme_and_keybinding() {
        let json = r#"{"theme":"dracula","keybindings":{"navigation":{"scroll_down":"e"}}}"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.theme.as_deref(), Some("dracula"));
        assert!(
            cfg.keybindings
                .navigation
                .scroll_down
                .matches(KeyCode::Char('e'), KeyModifiers::NONE)
        );
        assert!(
            cfg.keybindings
                .global
                .quit
                .matches(KeyCode::Char('q'), KeyModifiers::NONE)
        );
    }

    #[test]
    fn test_config_deserialize_empty_object() {
        let cfg: Config = serde_json::from_str("{}").unwrap();
        assert!(cfg.theme.is_none());
        assert!(
            cfg.keybindings
                .navigation
                .scroll_down
                .matches(KeyCode::Char('j'), KeyModifiers::NONE)
        );
    }

    #[test]
    fn test_config_deserialize_comment_keybindings() {
        let json = r#"{"keybindings":{"comment":{"save":"Shift+Enter"}}}"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert!(
            cfg.keybindings
                .comment
                .save
                .matches(KeyCode::Enter, KeyModifiers::SHIFT)
        );
    }

    #[test]
    fn test_config_deserialize_filter_keybindings() {
        let json = r#"{"keybindings":{"filter":{"delete_filter":"x"}}}"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert!(
            cfg.keybindings
                .filter
                .delete_filter
                .matches(KeyCode::Char('x'), KeyModifiers::NONE)
        );
        assert!(
            cfg.keybindings
                .filter
                .toggle_filter
                .matches(KeyCode::Char(' '), KeyModifiers::NONE)
        );
    }

    #[test]
    fn test_config_serialize_roundtrip() {
        let original = Config::default();
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(original.theme, deserialized.theme);
        assert!(
            deserialized
                .keybindings
                .navigation
                .scroll_down
                .matches(KeyCode::Char('j'), KeyModifiers::NONE)
        );
        assert!(
            deserialized
                .keybindings
                .global
                .quit
                .matches(KeyCode::Char('q'), KeyModifiers::NONE)
        );
    }

    #[test]
    fn test_check_conflicts_no_overlap() {
        let a = KeyBindings(vec![KeyBinding(KeyCode::Char('a'), KeyModifiers::NONE)]);
        let b = KeyBindings(vec![KeyBinding(KeyCode::Char('b'), KeyModifiers::NONE)]);
        let actions: &[(&str, &KeyBindings)] = &[("action_a", &a), ("action_b", &b)];
        let mut conflicts = Vec::new();
        keybindings::check_conflicts(actions, &mut conflicts);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn test_check_conflicts_overlap_reports_key() {
        let a = KeyBindings(vec![KeyBinding(KeyCode::Char('x'), KeyModifiers::NONE)]);
        let b = KeyBindings(vec![KeyBinding(KeyCode::Char('x'), KeyModifiers::NONE)]);
        let actions: &[(&str, &KeyBindings)] = &[("alpha", &a), ("beta", &b)];
        let mut conflicts = Vec::new();
        keybindings::check_conflicts(actions, &mut conflicts);
        assert_eq!(conflicts.len(), 1);
        assert!(conflicts[0].contains("alpha"));
        assert!(conflicts[0].contains("beta"));
        assert!(conflicts[0].contains("x"));
    }

    #[test]
    fn test_config_show_mode_bar_defaults_true() {
        let cfg: Config = serde_json::from_str("{}").unwrap();
        assert!(cfg.show_mode_bar.is_none());
    }

    #[test]
    fn test_config_show_borders_defaults_true() {
        let cfg: Config = serde_json::from_str("{}").unwrap();
        assert!(cfg.show_borders.is_none());
    }

    #[test]
    fn test_config_show_mode_bar_false() {
        let cfg: Config = serde_json::from_str(r#"{"show_mode_bar": false}"#).unwrap();
        assert_eq!(cfg.show_mode_bar, Some(false));
    }

    #[test]
    fn test_config_show_mode_bar_true() {
        let cfg: Config = serde_json::from_str(r#"{"show_mode_bar": true}"#).unwrap();
        assert_eq!(cfg.show_mode_bar, Some(true));
    }

    #[test]
    fn test_config_show_borders_false() {
        let cfg: Config = serde_json::from_str(r#"{"show_borders": false}"#).unwrap();
        assert_eq!(cfg.show_borders, Some(false));
    }

    #[test]
    fn test_config_default_show_mode_bar_true() {
        let cfg = Config::default();
        assert!(cfg.show_mode_bar.is_none());
    }

    #[test]
    fn test_config_default_show_borders_true() {
        let cfg = Config::default();
        assert!(cfg.show_borders.is_none());
    }

    #[test]
    fn test_config_default_preview_bytes() {
        assert_eq!(Config::default().preview_bytes, None);
    }

    #[test]
    fn test_config_preview_bytes_from_json() {
        let cfg: Config = serde_json::from_str(r#"{"preview_bytes": 4194304}"#).unwrap();
        assert_eq!(cfg.preview_bytes, Some(4 * 1024 * 1024));
    }

    #[test]
    fn test_config_preview_bytes_absent_is_none() {
        let cfg: Config = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(cfg.preview_bytes, None);
        assert_eq!(
            cfg.preview_bytes.unwrap_or(DEFAULT_PREVIEW_BYTES),
            DEFAULT_PREVIEW_BYTES
        );
    }

    #[test]
    fn test_toggle_mode_bar_keybinding_default() {
        let kb = UiKeybindings::default();
        assert!(
            kb.toggle_mode_bar
                .matches(KeyCode::Char('b'), KeyModifiers::NONE)
        );
    }

    #[test]
    fn test_toggle_borders_keybinding_default() {
        let kb = UiKeybindings::default();
        assert!(
            kb.toggle_borders
                .matches(KeyCode::Char('B'), KeyModifiers::NONE)
        );
    }

    #[test]
    fn test_filter_include_keybinding_default() {
        let kb = NormalKeybindings::default();
        assert!(
            kb.filter_include
                .matches(KeyCode::Char('i'), KeyModifiers::NONE)
        );
    }

    #[test]
    fn test_filter_exclude_keybinding_default() {
        let kb = NormalKeybindings::default();
        assert!(
            kb.filter_exclude
                .matches(KeyCode::Char('o'), KeyModifiers::NONE)
        );
    }

    #[test]
    fn test_enter_ui_mode_keybinding_default() {
        let kb = NormalKeybindings::default();
        assert!(
            kb.enter_ui_mode
                .matches(KeyCode::Char('u'), KeyModifiers::NONE)
        );
    }

    #[test]
    fn test_ui_keybindings_defaults() {
        let kb = UiKeybindings::default();
        assert!(
            kb.toggle_sidebar
                .matches(KeyCode::Char('s'), KeyModifiers::NONE)
        );
        assert!(
            kb.toggle_wrap
                .matches(KeyCode::Char('w'), KeyModifiers::NONE)
        );
        assert!(kb.exit.matches(KeyCode::Esc, KeyModifiers::NONE));
    }

    #[test]
    fn test_config_restore_file_context_defaults_to_none() {
        let config = Config::default();
        assert!(config.restore_file_context.is_none());
    }

    #[test]
    fn test_config_restore_session_defaults_to_none() {
        let config = Config::default();
        assert!(config.restore_session.is_none());
    }

    #[test]
    fn test_config_restore_file_context_deserializes_from_json() {
        let json = r#"{"restore_file_context": "never"}"#;
        let config: Config = serde_json::from_str(json).unwrap();
        assert_eq!(
            config.restore_file_context,
            Some(RestoreSessionPolicy::Never)
        );
    }

    #[test]
    fn test_config_restore_file_context_independent_of_session() {
        let json = r#"{"restore_session": "always", "restore_file_context": "never"}"#;
        let config: Config = serde_json::from_str(json).unwrap();
        assert_eq!(config.restore_session, Some(RestoreSessionPolicy::Always));
        assert_eq!(
            config.restore_file_context,
            Some(RestoreSessionPolicy::Never)
        );
    }

    #[test]
    fn test_deserialize_config_with_dlt_devices() {
        let json = r#"{
            "dlt_devices": [
                {"name": "ecu1", "host": "192.168.1.10", "port": 3490},
                {"name": "ecu2", "host": "10.0.0.1"}
            ]
        }"#;
        let config: Config = serde_json::from_str(json).unwrap();
        assert_eq!(config.dlt_devices.len(), 2);
        assert_eq!(config.dlt_devices[0].name, "ecu1");
        assert_eq!(config.dlt_devices[0].port, Some(3490));
        assert_eq!(config.dlt_devices[1].name, "ecu2");
        assert_eq!(config.dlt_devices[1].port, None);
    }

    #[test]
    fn test_default_config_has_empty_dlt_devices() {
        let config = Config::default();
        assert!(config.dlt_devices.is_empty());
    }

    #[test]
    fn test_deserialize_config_with_default_filter_files() {
        let json = r#"{
            "default_filter_files": {"acme": "~/logs/filters/acme.json"}
        }"#;
        let config: Config = serde_json::from_str(json).unwrap();
        assert_eq!(
            config.default_filter_files.unwrap().get("acme"),
            Some(&"~/logs/filters/acme.json".to_string())
        );
    }

    #[test]
    fn test_default_config_has_no_default_filter_files() {
        let config = Config::default();
        assert!(config.default_filter_files.is_none());
    }

    #[test]
    fn test_dlt_device_default_port() {
        let json = r#"{"name": "test", "host": "localhost"}"#;
        let device: DltDevice = serde_json::from_str(json).unwrap();
        assert_eq!(device.port, None);
        assert_eq!(device.port.unwrap_or(DEFAULT_DLT_PORT), DEFAULT_DLT_PORT);
    }

    #[test]
    fn test_save_to_and_load_from_path_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.json");
        std::fs::write(&config_path, r#"{"theme": "dark"}"#).unwrap();

        let device = DltDevice {
            name: "test".to_string(),
            host: "localhost".to_string(),
            port: Some(3490),
        };
        DltDevice::save_to(&device, &config_path).unwrap();

        let config = Config::load_from_path(&config_path).unwrap();
        assert_eq!(config.dlt_devices.len(), 1);
        assert_eq!(config.dlt_devices[0].name, "test");
    }

    #[test]
    fn test_remove_from_and_load_from_path_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.json");
        let a = DltDevice {
            name: "a".to_string(),
            host: "h1".to_string(),
            port: Some(3490),
        };
        let b = DltDevice {
            name: "b".to_string(),
            host: "h2".to_string(),
            port: Some(3491),
        };
        DltDevice::save_to(&a, &config_path).unwrap();
        DltDevice::save_to(&b, &config_path).unwrap();

        DltDevice::remove_from("a", &config_path).unwrap();

        let config = Config::load_from_path(&config_path).unwrap();
        assert_eq!(config.dlt_devices.len(), 1);
        assert_eq!(config.dlt_devices[0].name, "b");
    }

    #[test]
    fn test_dlt_select_keybindings_default() {
        let kb = DltSelectKeybindings::default();
        assert!(kb.confirm.matches(KeyCode::Enter, KeyModifiers::NONE));
        assert!(kb.cancel.matches(KeyCode::Esc, KeyModifiers::NONE));
        assert!(kb.delete.matches(KeyCode::Char('d'), KeyModifiers::NONE));
        assert!(kb.next_field.matches(KeyCode::Tab, KeyModifiers::NONE));
        assert!(kb.prev_field.matches(KeyCode::BackTab, KeyModifiers::NONE));
    }

    #[test]
    fn test_config_mcp_port_default_none() {
        let config = Config::default();
        assert!(config.mcp_port.is_none());
    }

    #[test]
    fn test_config_mcp_port_from_json() {
        let json = r#"{"mcp_port": 8765}"#;
        let config: Config = serde_json::from_str(json).unwrap();
        assert_eq!(config.mcp_port, Some(8765));
    }

    #[test]
    fn test_load_from_path_missing_file_returns_default() {
        let result = Config::load_from_path(std::path::Path::new("/nonexistent/config.json"));
        assert!(result.is_ok());
        assert!(result.unwrap().mcp_port.is_none());
    }

    #[test]
    fn test_load_from_path_invalid_json_returns_err() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"{invalid}").unwrap();
        let result = Config::load_from_path(tmp.path());
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("Could not parse config file"));
    }

    #[test]
    fn test_load_from_path_valid_json_returns_config() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), br#"{"mcp_port": 1234}"#).unwrap();
        let result = Config::load_from_path(tmp.path());
        assert!(result.is_ok());
        assert_eq!(result.unwrap().mcp_port, Some(1234));
    }

    #[test]
    fn test_load_from_path_unknown_key_returns_err() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), br#"{"unknown_key": true}"#).unwrap();
        let result = Config::load_from_path(tmp.path());
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("Could not parse config file"));
    }

    #[test]
    fn test_load_reads_from_config_dir() {
        let dir = tempfile::tempdir().unwrap();
        let logana_dir = dir.path().join("logana");
        std::fs::create_dir_all(&logana_dir).unwrap();
        std::fs::write(logana_dir.join("config.json"), r#"{"mcp_port": 9999}"#).unwrap();

        // SAFETY: test binary is single-threaded at this point; no other thread reads env.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", dir.path()) };
        let result = Config::load();
        unsafe { std::env::remove_var("XDG_CONFIG_HOME") };

        assert!(result.is_ok());
        assert_eq!(result.unwrap().mcp_port, Some(9999));
    }

    #[test]
    fn example_config_validates_against_schema() {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../../schema/config.schema.json")).unwrap();
        let example: serde_json::Value =
            serde_json::from_str(include_str!("../../examples/config.example.json")).unwrap();

        let validator = jsonschema::validator_for(&schema).expect("invalid schema");
        let errors: Vec<String> = validator
            .iter_errors(&example)
            .map(|e| format!("{} (path: {})", e, e.instance_path))
            .collect();

        assert!(
            errors.is_empty(),
            "example config failed schema validation:\n{}",
            errors.join("\n")
        );
    }

    #[test]
    fn test_load_schemas_empty_on_missing_dir() {
        let dir = tempfile::tempdir().unwrap();
        let (schemas, errors) = load_schemas(&dir.path().join("nonexistent"));
        assert!(schemas.is_empty());
        assert!(errors.is_empty());
    }

    #[test]
    fn test_load_schemas_reads_json_files() {
        let dir = tempfile::tempdir().unwrap();
        let schema_json = r#"{
            "name": "acme",
            "template": "{id} {service} <{timestamp}> {pid} {level}/{component}/{feature}, {message}",
            "fields": {"id": "extra", "service": "target"}
        }"#;
        std::fs::write(dir.path().join("acme.json"), schema_json).unwrap();
        let (schemas, errors) = load_schemas(dir.path());
        assert_eq!(schemas.len(), 1);
        assert_eq!(schemas[0].name, "acme");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_load_schemas_ignores_non_json_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("readme.txt"), "not json").unwrap();
        let (schemas, errors) = load_schemas(dir.path());
        assert!(schemas.is_empty());
        assert!(errors.is_empty());
    }

    #[test]
    fn test_load_schemas_skips_invalid_json() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("bad.json"), "{invalid}").unwrap();
        let (schemas, _) = load_schemas(dir.path());
        assert!(schemas.is_empty());
    }

    #[test]
    fn test_load_schemas_reports_error_for_invalid_json() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("bad.json"), "{invalid}").unwrap();
        let (_, errors) = load_schemas(dir.path());
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("bad.json"));
    }

    #[test]
    fn test_load_schemas_reports_one_error_per_bad_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("bad1.json"), "{invalid}").unwrap();
        std::fs::write(dir.path().join("bad2.json"), "not json at all").unwrap();
        std::fs::write(
            dir.path().join("good.json"),
            r#"{"name": "ok", "fields": {}}"#,
        )
        .unwrap();
        let (schemas, errors) = load_schemas(dir.path());
        assert_eq!(schemas.len(), 1);
        assert_eq!(errors.len(), 2);
    }

    async fn setup_db() -> crate::db::Database {
        crate::db::Database::in_memory().await.unwrap()
    }

    #[tokio::test]
    async fn test_resolve_default_filter_files_config_only() {
        let db = setup_db().await;
        let config = Config {
            default_filter_files: Some(
                [("acme".to_string(), "a.json".to_string())]
                    .into_iter()
                    .collect(),
            ),
            ..Default::default()
        };
        let resolved = resolve_default_filter_files(&config, &db).await;
        assert_eq!(resolved.get("acme"), Some(&"a.json".to_string()));
    }

    #[tokio::test]
    async fn test_resolve_default_filter_files_db_only() {
        let db = setup_db().await;
        db.save_app_setting(
            crate::db::SettingsKey::DefaultFilterFiles,
            r#"{"syslog":"s.json"}"#,
        )
        .await
        .unwrap();
        let config = Config::default();
        let resolved = resolve_default_filter_files(&config, &db).await;
        assert_eq!(resolved.get("syslog"), Some(&"s.json".to_string()));
    }

    #[tokio::test]
    async fn test_resolve_default_filter_files_per_key_merge() {
        let db = setup_db().await;
        db.save_app_setting(
            crate::db::SettingsKey::DefaultFilterFiles,
            r#"{"acme":"old.json","syslog":"s.json"}"#,
        )
        .await
        .unwrap();
        let config = Config {
            default_filter_files: Some(
                [("acme".to_string(), "a.json".to_string())]
                    .into_iter()
                    .collect(),
            ),
            ..Default::default()
        };
        let resolved = resolve_default_filter_files(&config, &db).await;
        assert_eq!(resolved.get("acme"), Some(&"a.json".to_string()));
        assert_eq!(resolved.get("syslog"), Some(&"s.json".to_string()));
    }

    #[tokio::test]
    async fn test_resolve_default_filter_files_malformed_db_json_falls_back_to_empty() {
        let db = setup_db().await;
        db.save_app_setting(crate::db::SettingsKey::DefaultFilterFiles, "not json")
            .await
            .unwrap();
        let config = Config {
            default_filter_files: Some(
                [("acme".to_string(), "a.json".to_string())]
                    .into_iter()
                    .collect(),
            ),
            ..Default::default()
        };
        let resolved = resolve_default_filter_files(&config, &db).await;
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved.get("acme"), Some(&"a.json".to_string()));
    }

    #[tokio::test]
    async fn test_resolve_default_filter_files_both_empty() {
        let db = setup_db().await;
        let config = Config::default();
        let resolved = resolve_default_filter_files(&config, &db).await;
        assert!(resolved.is_empty());
    }

    #[test]
    fn test_custom_schema_config_multiline_defaults_false() {
        let cfg: CustomSchemaConfig = serde_json::from_str(r#"{"name": "test"}"#).unwrap();
        assert!(!cfg.multiline);
    }

    #[test]
    fn test_custom_schema_config_multiline_deserializes_true() {
        let cfg: CustomSchemaConfig =
            serde_json::from_str(r#"{"name": "test", "multiline": true}"#).unwrap();
        assert!(cfg.multiline);
    }

    #[test]
    fn test_custom_schema_config_multiline_serde_roundtrip() {
        let cfg = CustomSchemaConfig {
            name: "test".to_string(),
            multiline: true,
            ..Default::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: CustomSchemaConfig = serde_json::from_str(&json).unwrap();
        assert!(deserialized.multiline);
    }

    #[test]
    fn test_custom_schema_config_continuation_defaults_none() {
        let cfg: CustomSchemaConfig = serde_json::from_str(r#"{"name": "test"}"#).unwrap();
        assert!(cfg.continuation.is_none());
    }

    #[test]
    fn test_custom_schema_config_continuation_deserializes_from_docs_example() {
        // The exact JSON shape documented in docs/src/custom-schemas.md's
        // "Multiline field extraction" section.
        let json = "{
            \"name\": \"transaction\",
            \"template\": \"### Start transaction {id}\",
            \"fields\": { \"id\": \"extra\" },
            \"continuation\": {
                \"end_pattern\": \"### End transaction\",
                \"fields\": [
                    { \"template\": \"field1: {field1}\" },
                    { \"template\": \"field2: {field2}\" },
                    { \"template\": \"Object {payload}\", \"json\": true }
                ]
            }
        }";
        let cfg: CustomSchemaConfig = serde_json::from_str(json).unwrap();

        // Compiles successfully via the same path startup validation uses.
        assert!(crate::parser::CustomParser::from_config(&cfg).is_ok());

        let continuation = cfg.continuation.expect("continuation block should parse");
        assert_eq!(
            continuation.end_pattern.as_deref(),
            Some("### End transaction")
        );
        assert_eq!(continuation.fields.len(), 3);
        assert_eq!(
            continuation.fields[0].template.as_deref(),
            Some("field1: {field1}")
        );
        assert!(!continuation.fields[0].json);
        assert_eq!(
            continuation.fields[2].template.as_deref(),
            Some("Object {payload}")
        );
        assert!(continuation.fields[2].json);
    }

    #[test]
    fn test_continuation_config_end_pattern_optional() {
        let json = r#"{"name": "test", "continuation": {"fields": []}}"#;
        let cfg: CustomSchemaConfig = serde_json::from_str(json).unwrap();
        assert!(cfg.continuation.unwrap().end_pattern.is_none());
    }

    #[test]
    fn test_custom_schema_config_rejects_unknown_field() {
        // A typo'd key (e.g. "templte" instead of "template") should be
        // reported immediately, not silently ignored only to surface later
        // as a confusing "must specify either 'template' or 'pattern'" error.
        let json = r#"{"name": "test", "templte": "{message}"}"#;
        let err = serde_json::from_str::<CustomSchemaConfig>(json).unwrap_err();
        assert!(err.to_string().contains("templte"), "{err}");
    }

    #[test]
    fn test_custom_schema_config_accepts_schema_url_key() {
        let json = r#"{"$schema": "./custom-schema.schema.json", "name": "test"}"#;
        let cfg: CustomSchemaConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.name, "test");
    }

    #[test]
    fn test_custom_schema_config_schema_url_not_serialized() {
        let cfg = CustomSchemaConfig {
            name: "test".to_string(),
            ..Default::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(!json.contains("$schema"), "{json}");
    }

    #[test]
    fn test_continuation_config_rejects_unknown_field() {
        let json = r#"{"name": "test", "continuation": {"fields": [], "bogus": true}}"#;
        let err = serde_json::from_str::<CustomSchemaConfig>(json).unwrap_err();
        assert!(err.to_string().contains("bogus"), "{err}");
    }

    #[test]
    fn test_continuation_field_spec_rejects_unknown_field() {
        let json =
            r#"{"name": "test", "continuation": {"fields": [{"template": "{a}", "bogus": true}]}}"#;
        let err = serde_json::from_str::<CustomSchemaConfig>(json).unwrap_err();
        assert!(err.to_string().contains("bogus"), "{err}");
    }

    #[test]
    fn test_custom_level_values_rejects_unknown_field() {
        let json = r#"{"name": "test", "levels": {"bogus": []}}"#;
        let err = serde_json::from_str::<CustomSchemaConfig>(json).unwrap_err();
        assert!(err.to_string().contains("bogus"), "{err}");
    }

    #[test]
    fn example_custom_schema_validates_against_schema() {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../../schema/custom-schema.schema.json")).unwrap();
        let example: serde_json::Value =
            serde_json::from_str(include_str!("../../examples/custom-schema.example.json"))
                .unwrap();

        let validator = jsonschema::validator_for(&schema).expect("invalid schema");
        let errors: Vec<String> = validator
            .iter_errors(&example)
            .map(|e| format!("{} (path: {})", e, e.instance_path))
            .collect();

        assert!(
            errors.is_empty(),
            "example custom schema failed schema validation:\n{}",
            errors.join("\n")
        );
    }
}
