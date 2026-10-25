use crate::db::{AppSettingsStore, Database, SettingsKey};
use crate::gui::color::color_to_gpui;
use crate::style::Color;
use crate::theme::Theme as TuiTheme;
use gpui_kit::component::theme::{Theme as GpuiTheme, ThemeConfig, ThemeSet};
use gpui_kit::gpui::App;
use serde_json::json;
use std::rc::Rc;

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

/// Apply directly: the registry intentionally ignores duplicate names,
/// which otherwise leaves every component on the first selected theme.
pub fn install(theme: &TuiTheme, cx: &mut App) {
    let config = gui_config(theme);
    GpuiTheme::update(cx, |active| active.apply_config(&config));
}

fn gui_config(theme: &TuiTheme) -> Rc<ThemeConfig> {
    let set: ThemeSet = serde_json::from_str(&theme_set_json(theme)).expect("valid GUI palette");
    Rc::new(set.themes.into_iter().next().expect("GUI theme"))
}

type Rgb = (u8, u8, u8);

/// Translate terminal colors into distinct desktop surfaces and readable
/// interaction states. The original syntax palette stays on log content.
#[derive(Clone, Copy)]
pub(crate) struct GuiPalette {
    pub background: Rgb,
    pub foreground: Rgb,
    pub surface: Rgb,
    pub elevated: Rgb,
    pub control: Rgb,
    pub hover: Rgb,
    pub selected: Rgb,
    pub border: Rgb,
    pub muted: Rgb,
    pub primary: Rgb,
    pub primary_foreground: Rgb,
}

impl GuiPalette {
    pub fn from_theme(theme: &TuiTheme) -> Self {
        let background = to_rgb(theme.root_bg);
        let mut foreground = readable(to_rgb(theme.text), background, 4.5);
        let gui = theme.gui.as_ref();
        let surface = gui
            .map(|g| to_rgb(g.surface))
            .unwrap_or_else(|| blend(background, foreground, 0.035));
        let elevated = gui
            .map(|g| to_rgb(g.elevated))
            .unwrap_or_else(|| blend(background, foreground, 0.075));
        let control = gui
            .map(|g| to_rgb(g.control))
            .unwrap_or_else(|| blend(background, foreground, 0.06));
        let hover = blend(control, foreground, 0.08);
        // A TUI border title is often plain text, not an action accent.
        let accent = [
            theme.text_highlight_fg,
            theme.debug_fg,
            theme.comment_fg,
            theme.border_title,
        ]
        .into_iter()
        .map(to_rgb)
        .find(|&(r, g, b)| r.max(g).max(b) - r.min(g).min(b) > 30)
        .unwrap_or_else(|| to_rgb(theme.border_title));
        let mut primary = gui.map(|g| to_rgb(g.primary)).unwrap_or(accent);
        for background in [surface, elevated, control] {
            primary = readable(primary, background, 3.0);
        }
        let selected = blend(background, primary, 0.23);
        for background in [surface, elevated, control, hover, selected] {
            foreground = readable(foreground, background, 4.5);
        }
        let mut muted = gui
            .map(|g| to_rgb(g.muted))
            .unwrap_or_else(|| to_rgb(theme.inactive_tab_fg));
        for background in [surface, elevated, control] {
            muted = readable(muted, background, 4.5);
        }
        let primary_foreground =
            if contrast((255, 255, 255), primary) >= contrast((0, 0, 0), primary) {
                (255, 255, 255)
            } else {
                (0, 0, 0)
            };
        Self {
            background,
            foreground,
            surface,
            elevated,
            control,
            hover,
            selected,
            border: gui
                .map(|g| to_rgb(g.border))
                .unwrap_or_else(|| blend(background, foreground, 0.18)),
            muted,
            primary,
            primary_foreground,
        }
    }
}

fn to_rgb(color: Color) -> Rgb {
    let color = color_to_gpui(color).unwrap_or_default();
    (
        (color.r * 255.).round() as u8,
        (color.g * 255.).round() as u8,
        (color.b * 255.).round() as u8,
    )
}

fn blend(base: Rgb, tint: Rgb, amount: f32) -> Rgb {
    let channel = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * amount).round() as u8;
    (
        channel(base.0, tint.0),
        channel(base.1, tint.1),
        channel(base.2, tint.2),
    )
}
fn luminance(color: Rgb) -> f32 {
    let linear = |channel: u8| {
        let x = channel as f32 / 255.;
        if x <= 0.04045 {
            x / 12.92
        } else {
            ((x + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.0) + 0.7152 * linear(color.1) + 0.0722 * linear(color.2)
}
fn contrast(a: Rgb, b: Rgb) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}
fn readable(color: Rgb, background: Rgb, minimum: f32) -> Rgb {
    if contrast(color, background) >= minimum {
        return color;
    }
    let target = if contrast((255, 255, 255), background) >= contrast((0, 0, 0), background) {
        (255, 255, 255)
    } else {
        (0, 0, 0)
    };
    for step in 1..=100 {
        let candidate = blend(color, target, step as f32 / 100.);
        if contrast(candidate, background) >= minimum {
            return candidate;
        }
    }
    target
}

fn theme_set_json(theme: &TuiTheme) -> String {
    let palette = GuiPalette::from_theme(theme);
    let mut colors = serde_json::Map::new();
    let mut set = |keys: &[&str], color: Rgb| {
        let value = format!("#{:02x}{:02x}{:02x}", color.0, color.1, color.2);
        for key in keys {
            colors.insert((*key).to_string(), json!(value));
        }
    };
    set(&["background", "table.background"], palette.background);
    set(
        &[
            "sidebar.background",
            "tab.background",
            "tab_bar.background",
            "table.foot.background",
            "title_bar.background",
            "status_bar.background",
            "list.background",
            "table.head.background",
            "list.head.background",
            "accordion.background",
        ],
        palette.surface,
    );
    set(
        &["popover.background", "group_box.background"],
        palette.elevated,
    );
    set(
        &[
            "foreground",
            "popover.foreground",
            "sidebar.foreground",
            "tab.foreground",
            "button.foreground",
            "secondary.foreground",
            "group_box.foreground",
            "tab.active.foreground",
            "accent.foreground",
            "sidebar.accent.foreground",
        ],
        palette.foreground,
    );
    set(
        &[
            "border",
            "input.border",
            "sidebar.border",
            "table.row.border",
            "window.border",
            "title_bar.border",
            "status_bar.border",
            "scrollbar.thumb.background",
        ],
        palette.border,
    );
    set(
        &[
            "muted.foreground",
            "table.head.foreground",
            "table.foot.foreground",
            "group_box.title.foreground",
        ],
        palette.muted,
    );
    set(
        &[
            "muted.background",
            "button.background",
            "secondary.background",
            "switch.background",
            "slider.background",
            "tab_bar.segmented.background",
            "skeleton.background",
        ],
        palette.control,
    );
    set(
        &[
            "button.hover.background",
            "secondary.hover.background",
            "list.hover.background",
            "table.hover.background",
        ],
        palette.hover,
    );
    set(
        &[
            "accent.background",
            "sidebar.accent.background",
            "tab.active.background",
            "button.active.background",
            "secondary.active.background",
            "list.active.background",
            "table.active.background",
        ],
        palette.selected,
    );
    set(
        &[
            "primary.background",
            "sidebar.primary.background",
            "slider.thumb.background",
            "progress.bar.background",
            "ring",
            "caret",
            "link",
            "link.hover",
            "link.active",
            "list.active.border",
            "table.active.border",
            "drag.border",
            "scrollbar.thumb.hover.background",
        ],
        palette.primary,
    );
    set(
        &["primary.foreground", "sidebar.primary.foreground"],
        palette.primary_foreground,
    );
    // Move toward the opposite of button text to retain contrast in every state.
    let state_tint = if palette.primary_foreground == (0, 0, 0) {
        (255, 255, 255)
    } else {
        (0, 0, 0)
    };
    set(
        &["primary.hover.background"],
        blend(palette.primary, state_tint, 0.10),
    );
    set(
        &["primary.active.background"],
        blend(palette.primary, state_tint, 0.20),
    );
    // Selected primary buttons keep a solid accent instead of the pressed tint,
    // which can lose its distinction from neutral controls in light themes.
    set(&["button.primary.active.background"], palette.primary);
    set(&["selection.background"], to_rgb(theme.visual_select_bg));
    // One thumb color is used in both positions. Choose it against both tracks,
    // then lift/darken the neutral track only when needed for a visible thumb.
    let thumb = palette.primary_foreground;
    set(&["switch.thumb.background"], thumb);
    set(
        &["switch.background"],
        readable(palette.control, thumb, 3.0),
    );
    for (name, color) in [
        ("red", theme.error_fg),
        ("yellow", theme.warning_fg),
        ("green", theme.notice_fg),
        ("cyan", theme.comment_fg),
    ] {
        set(
            &[&format!("base.{name}")],
            readable(to_rgb(color), palette.background, 3.),
        );
    }
    set(&["base.blue"], palette.primary);
    set(
        &["base.magenta"],
        theme
            .process_colors
            .get(2)
            .copied()
            .map(to_rgb)
            .unwrap_or(palette.primary),
    );
    for (name, color) in [
        ("success", theme.notice_fg),
        ("warning", theme.warning_fg),
        ("danger", theme.error_fg),
        ("info", theme.debug_fg),
    ] {
        let status = readable(to_rgb(color), palette.background, 3.0);
        set(&[&format!("{name}.background")], status);
        let fg = if contrast((255, 255, 255), status) >= contrast((0, 0, 0), status) {
            (255, 255, 255)
        } else {
            (0, 0, 0)
        };
        set(&[&format!("{name}.foreground")], fg);
    }
    json!({ "name": THEME_NAME, "themes": [{ "name": THEME_NAME, "mode": if is_light(theme) { "light" } else { "dark" }, "font.size": 13, "mono_font.size": 12, "radius": 6, "colors": colors }] }).to_string()
}

fn is_light(theme: &TuiTheme) -> bool {
    let (r, g, b) = to_rgb(theme.root_bg);
    0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32 > 127.5
}

#[cfg(test)]
fn hex(color: Color) -> Option<String> {
    let rgba = color_to_gpui(color)?;
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

    fn theme_with(root_bg: Color, text: Color) -> TuiTheme {
        TuiTheme {
            root_bg,
            text,
            gui: None,
            ..TuiTheme::default()
        }
    }

    #[test]
    fn hex_renders_lowercase_six_digit_rgb() {
        assert_eq!(hex(Color::Rgb(10, 20, 30)), Some("#0a141e".to_string()));
    }

    #[test]
    fn hex_is_none_for_colors_with_no_rgb_equivalent() {
        assert_eq!(hex(Color::Reset), None);
    }

    #[test]
    fn theme_set_json_includes_background_and_foreground() {
        let theme = theme_with(Color::Black, Color::White);
        let json = theme_set_json(&theme);
        assert!(json.contains("\"background\":\"#000000\""));
        assert!(json.contains("\"foreground\":\"#ffffff\""));
    }

    #[test]
    fn dracula_and_alucard_use_semantic_selection_tokens() {
        for (name, background, selection, comment) in [
            ("dracula", (40, 42, 54), (68, 71, 90), (98, 114, 164)),
            ("alucard", (255, 251, 235), (207, 207, 222), (108, 102, 75)),
        ] {
            assert!(TuiTheme::list_available_themes().contains(&name.to_string()));
            let theme = TuiTheme::from_file(format!("{name}.json")).unwrap();
            assert_eq!(to_rgb(theme.root_bg), background);
            assert_eq!(to_rgb(theme.cursor_bg), comment);
            assert_eq!(to_rgb(theme.comment_fg), comment);
            let tokens: serde_json::Value = serde_json::from_str(&theme_set_json(&theme)).unwrap();
            assert!(tokens.to_string().contains(&format!(
                "\"selection.background\":\"#{:02x}{:02x}{:02x}\"",
                selection.0, selection.1, selection.2
            )));
        }
    }

    #[test]
    fn bundled_palettes_have_distinct_surfaces_and_readable_text() {
        for name in TuiTheme::list_available_themes() {
            let theme = TuiTheme::from_file(format!("{name}.json")).unwrap();
            let palette = GuiPalette::from_theme(&theme);
            assert_ne!(palette.background, palette.surface, "{name}");
            assert_ne!(palette.surface, palette.elevated, "{name}");
            for surface in [
                palette.background,
                palette.surface,
                palette.elevated,
                palette.control,
                palette.hover,
                palette.selected,
            ] {
                assert!(
                    contrast(palette.foreground, surface) >= 4.5,
                    "{name}: text contrast"
                );
            }
            assert!(
                contrast(palette.muted, palette.surface) >= 4.5,
                "{name}: muted contrast"
            );
            assert!(
                contrast(palette.primary, palette.surface) >= 3.,
                "{name}: accent contrast"
            );
            assert!(
                contrast(palette.primary_foreground, palette.primary) >= 4.5,
                "{name}: button contrast"
            );
        }
    }

    #[test]
    fn bundled_control_states_have_clear_selection_and_thumb_contrast() {
        let rgb = |color: gpui_kit::gpui::Hsla| {
            let color = gpui_kit::gpui::Rgba::from(color);
            (
                (color.r * 255.).round() as u8,
                (color.g * 255.).round() as u8,
                (color.b * 255.).round() as u8,
            )
        };
        for name in TuiTheme::list_available_themes() {
            let source = TuiTheme::from_file(format!("{name}.json")).unwrap();
            let palette = GuiPalette::from_theme(&source);
            let mut active = GpuiTheme::default();
            active.apply_config(&gui_config(&source));
            let selected = rgb(active.button_primary_active);
            for neutral in [palette.control, palette.elevated, palette.surface] {
                assert!(
                    contrast(selected, neutral) >= 3.,
                    "{name}: selected control"
                );
                assert!(
                    contrast(palette.primary, neutral) >= 3.,
                    "{name}: switch on"
                );
            }
            assert!(
                contrast(rgb(active.button_primary_foreground), selected) >= 4.5,
                "{name}: selected label"
            );
            for track in [rgb(active.switch), palette.primary] {
                assert!(
                    contrast(rgb(active.switch_thumb), track) >= 3.,
                    "{name}: switch thumb"
                );
            }
        }
    }

    #[test]
    fn gui_overrides_roundtrip_and_legacy_themes_keep_fallbacks() {
        let theme = TuiTheme::from_file("dracula.json").unwrap();
        let encoded = serde_json::to_string(&theme).unwrap();
        let decoded: TuiTheme = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.gui, theme.gui);
        let palette = GuiPalette::from_theme(&decoded);
        assert_eq!(palette.primary, (189, 147, 249));
        assert_eq!(palette.surface, (33, 34, 44));
        let mut legacy: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        legacy.as_object_mut().unwrap().remove("gui");
        let legacy: TuiTheme = serde_json::from_value(legacy).unwrap();
        assert!(legacy.gui.is_none());
        assert!(
            contrast(
                GuiPalette::from_theme(&legacy).foreground,
                to_rgb(legacy.root_bg)
            ) >= 4.5
        );
    }

    #[test]
    fn component_tokens_and_button_states_use_the_selected_palette() {
        for name in TuiTheme::list_available_themes() {
            let source = TuiTheme::from_file(format!("{name}.json")).unwrap();
            let palette = GuiPalette::from_theme(&source);
            let config = gui_config(&source);
            // Check parsed fields: misspelled JSON keys are silently ignored by GPUI.
            assert!(config.colors.table_row_border.is_some(), "{name}");
            assert!(config.colors.table_hover.is_some(), "{name}");
            let parsed_colors = serde_json::to_value(&config.colors).unwrap();
            assert!(parsed_colors["base.blue"].is_string(), "{name}");
            let mut active = GpuiTheme::default();
            active.apply_config(&config);
            assert_ne!(active.primary, active.primary_hover, "{name}");
            assert_ne!(active.primary_hover, active.primary_active, "{name}");
            for amount in [0.10, 0.20] {
                let tint = if palette.primary_foreground == (0, 0, 0) {
                    (255, 255, 255)
                } else {
                    (0, 0, 0)
                };
                assert!(
                    contrast(
                        palette.primary_foreground,
                        blend(palette.primary, tint, amount)
                    ) >= 4.5,
                    "{name}"
                );
            }
        }
    }

    #[test]
    fn switching_same_named_configs_refreshes_components_and_semantic_tokens() {
        let dark = TuiTheme::from_file("tokyonight.json").unwrap();
        let light = TuiTheme::from_file("github-light.json").unwrap();
        let mut active = GpuiTheme::default();
        for source in [&dark, &light, &dark] {
            let config = gui_config(source);
            active.apply_config(&config);
            let palette = GuiPalette::from_theme(source);
            let expected: gpui_kit::gpui::Hsla = gpui_kit::gpui::rgb(
                (palette.primary.0 as u32) << 16
                    | (palette.primary.1 as u32) << 8
                    | palette.primary.2 as u32,
            )
            .into();
            let color = |(r, g, b): Rgb| -> gpui_kit::gpui::Hsla {
                gpui_kit::gpui::rgb((r as u32) << 16 | (g as u32) << 8 | b as u32).into()
            };
            assert_eq!(active.background, color(palette.background));
            for surface in [
                active.sidebar,
                active.tab,
                active.title_bar,
                active.status_bar,
            ] {
                assert_eq!(surface, color(palette.surface));
            }
            assert_eq!(active.popover, color(palette.elevated));
            assert_eq!(active.secondary, color(palette.control));
            assert_eq!(active.accent, color(palette.selected));
            assert_eq!(active.foreground, color(palette.foreground));
            assert_eq!(
                active.semantic_tokens().colors.surface,
                color(palette.elevated)
            );
            assert_eq!(active.primary, expected);
            assert_eq!(active.tokens.primary.color, expected);
            assert_eq!(active.semantic_tokens().colors.primary, expected);
            assert_eq!(!active.mode.is_dark(), is_light(source));
        }
    }

    #[test]
    fn theme_set_json_names_the_theme_logana() {
        let json = theme_set_json(&TuiTheme::default());
        assert!(json.contains(&format!("\"name\":\"{THEME_NAME}\"")));
    }
}
