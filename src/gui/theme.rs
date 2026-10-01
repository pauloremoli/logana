use crate::db::{AppSettingsStore, Database, SettingsKey};
use crate::gui::color::ratatui_color_to_gpui;
use crate::theme::Theme as TuiTheme;
use gpui_kit::component::theme::{Theme as GpuiTheme, ThemeMode, ThemeRegistry};
use gpui_kit::gpui::App;
use ratatui::style::Color as RatatuiColor;
use serde_json::json;

const THEME_NAME: &str = "logana";

/// Loads the theme the TUI is currently configured to use — same DB
/// setting, same bundled/user theme files — so both frontends match.
pub async fn load_current(db: &Database) -> TuiTheme {
    let name = db.load_app_setting(SettingsKey::Theme).await.ok().flatten();
    name.as_deref()
        .and_then(|name| TuiTheme::from_file(format!("{name}.json")).ok())
        .unwrap_or_default()
}

/// Persists `name` as the current theme, same DB setting `load_current`
/// reads — mirrors the TUI's `:set-theme`/`:theme`-picker confirm. Errors
/// are silently dropped: a failed setting write shouldn't block the theme
/// switch the user already sees applied.
pub async fn save(db: &Database, name: &str) {
    let _ = db.save_app_setting(SettingsKey::Theme, name).await;
}

/// Registers a gpui-component theme mirroring the TUI theme's key colors
/// and activates it, so buttons/checkboxes/backgrounds/text all pick up
/// the same look by default. Set once at startup.
pub fn install(theme: &TuiTheme, cx: &mut App) {
    ThemeRegistry::global_mut(cx)
        .load_themes_from_str(&theme_set_json(theme))
        .expect("logana theme JSON must be valid");
    let config = ThemeRegistry::global(cx)
        .themes()
        .get(THEME_NAME)
        .cloned()
        .expect("logana theme must be registered after load_themes_from_str");
    let active = GpuiTheme::global_mut(cx);
    active.light_theme = config.clone();
    active.dark_theme = config;
    GpuiTheme::change(ThemeMode::Dark, None, cx);
}

/// The gpui-component `ThemeSet` JSON (one theme, named [`THEME_NAME`])
/// mirroring the TUI theme's background/text/accent/border/success/
/// warning/danger colors. Unconvertible colors (see `color::tests`) are
/// simply omitted, so gpui-component's own defaults fill the gap.
fn theme_set_json(theme: &TuiTheme) -> String {
    let mut colors = serde_json::Map::new();
    let mut set = |key: &str, color: RatatuiColor| {
        if let Some(hex) = hex(color) {
            colors.insert(key.to_string(), json!(hex));
        }
    };
    set("background", theme.root_bg);
    set("foreground", theme.text);
    set("border", theme.border);
    // Popovers/menus (e.g. the context menu) use the same flat
    // background as the rest of the app — the TUI has no distinct
    // "elevated surface" shade, panels are differentiated by border only.
    set("popover.background", theme.root_bg);
    set("popover.foreground", theme.text);
    set("primary.background", theme.text_highlight_fg);
    set("selection.background", theme.text_highlight_bg);
    set("success.background", theme.info_fg);
    set("warning.background", theme.warning_fg);
    set("danger.background", theme.error_fg);

    json!({
        "name": THEME_NAME,
        "themes": [{
            "name": THEME_NAME,
            "mode": "dark",
            "colors": colors,
        }],
    })
    .to_string()
}

fn hex(color: RatatuiColor) -> Option<String> {
    let rgba = ratatui_color_to_gpui(color)?;
    Some(format!(
        "#{:02x}{:02x}{:02x}",
        (rgba.r * 255.0).round() as u8,
        (rgba.g * 255.0).round() as u8,
        (rgba.b * 255.0).round() as u8,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme_with(root_bg: RatatuiColor, text: RatatuiColor) -> TuiTheme {
        TuiTheme {
            root_bg,
            text,
            ..TuiTheme::default()
        }
    }

    #[test]
    fn hex_renders_lowercase_six_digit_rgb() {
        assert_eq!(
            hex(RatatuiColor::Rgb(10, 20, 30)),
            Some("#0a141e".to_string())
        );
    }

    #[test]
    fn hex_is_none_for_colors_with_no_rgb_equivalent() {
        assert_eq!(hex(RatatuiColor::Reset), None);
    }

    #[test]
    fn theme_set_json_includes_background_and_foreground() {
        let theme = theme_with(RatatuiColor::Black, RatatuiColor::White);
        let json = theme_set_json(&theme);
        assert!(json.contains("\"background\":\"#000000\""));
        assert!(json.contains("\"foreground\":\"#ffffff\""));
    }

    #[test]
    fn theme_set_json_omits_unconvertible_colors() {
        let theme = theme_with(RatatuiColor::Reset, RatatuiColor::White);
        let json = theme_set_json(&theme);
        assert!(!json.contains("\"background\""));
        assert!(json.contains("\"foreground\":\"#ffffff\""));
    }

    #[test]
    fn theme_set_json_names_the_theme_logana() {
        let json = theme_set_json(&TuiTheme::default());
        assert!(json.contains(&format!("\"name\":\"{THEME_NAME}\"")));
    }
}
