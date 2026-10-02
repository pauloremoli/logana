use crate::filters::{
    FilterDecision, FilterDef, GroupDef, build_filter, effective_color_config, group_style,
};
use gpui_kit::gpui::Rgba;
use ratatui::style::Color as RatatuiColor;

/// The foreground color a filter row (or group row) should render with,
/// resolved the same way the TUI resolves it: the filter's own color, else
/// its group's, if it has one.
pub fn filter_row_color(def: &FilterDef, group_defs: &[GroupDef]) -> Option<Rgba> {
    let cc = effective_color_config(def, group_defs)?;
    ratatui_color_to_gpui(cc.fg?)
}

/// The color a filter's `[group]` tag renders with: the group's own
/// predefined style specifically (unlike `filter_row_color`, this never
/// falls back to the filter's own color — mirrors the TUI sidebar, which
/// colors the tag and the pattern independently).
pub fn group_tag_color(def: &FilterDef, group_defs: &[GroupDef]) -> Option<Rgba> {
    let name = def.group.as_deref()?;
    let cc = group_style(group_defs, name)?;
    ratatui_color_to_gpui(cc.fg?)
}

/// The foreground color a group row should render with, from its own
/// predefined style (groups have no fallback of their own to inherit from).
pub fn group_row_color(def: &GroupDef) -> Option<Rgba> {
    ratatui_color_to_gpui(def.color_config.as_ref()?.fg?)
}

/// Resolves both the foreground and background color a log line should
/// render with, from the first enabled filter (in list order) that has a
/// resolved color and whose pattern matches the raw line bytes — fg and
/// bg always come from the *same* matched filter's `ColorConfig`, not two
/// independent searches, so a filter styled with only `--bg` still wins
/// over a later filter that only sets `--fg`, mirroring how the TUI's own
/// `FilterManager`-built `Style`s combine both from one filter. `(None,
/// None)` means no styled filter matched — the line renders in the
/// default text color with no background.
pub fn resolve_line_colors(
    line: &[u8],
    filter_defs: &[FilterDef],
    group_defs: &[GroupDef],
) -> (Option<Rgba>, Option<Rgba>) {
    let matched = filter_defs
        .iter()
        .filter(|def| def.enabled)
        .find_map(|def| {
            let cc = effective_color_config(def, group_defs)?;
            if cc.fg.is_none() && cc.bg.is_none() {
                return None;
            }
            let filter = build_filter(
                &def.pattern,
                FilterDecision::Include,
                cc.match_only,
                0,
                def.use_regex,
                def.ignore_case,
            )?;
            if filter.matches(line) == FilterDecision::Neutral {
                return None;
            }
            Some(cc)
        });
    match matched {
        Some(cc) => (
            cc.fg.and_then(ratatui_color_to_gpui),
            cc.bg.and_then(ratatui_color_to_gpui),
        ),
        None => (None, None),
    }
}

/// The foreground-only half of [`resolve_line_colors`], for callers that
/// don't render a background.
pub fn resolve_line_color(
    line: &[u8],
    filter_defs: &[FilterDef],
    group_defs: &[GroupDef],
) -> Option<Rgba> {
    resolve_line_colors(line, filter_defs, group_defs).0
}

pub fn ratatui_color_to_gpui(color: RatatuiColor) -> Option<Rgba> {
    let (r, g, b) = match color {
        RatatuiColor::Reset => return None,
        RatatuiColor::Black => (0, 0, 0),
        RatatuiColor::Red => (128, 0, 0),
        RatatuiColor::Green => (0, 128, 0),
        RatatuiColor::Yellow => (128, 128, 0),
        RatatuiColor::Blue => (0, 0, 128),
        RatatuiColor::Magenta => (128, 0, 128),
        RatatuiColor::Cyan => (0, 128, 128),
        RatatuiColor::Gray => (192, 192, 192),
        RatatuiColor::DarkGray => (128, 128, 128),
        RatatuiColor::LightRed => (255, 0, 0),
        RatatuiColor::LightGreen => (0, 255, 0),
        RatatuiColor::LightYellow => (255, 255, 0),
        RatatuiColor::LightBlue => (0, 0, 255),
        RatatuiColor::LightMagenta => (255, 0, 255),
        RatatuiColor::LightCyan => (0, 255, 255),
        RatatuiColor::White => (255, 255, 255),
        RatatuiColor::Rgb(r, g, b) => (r, g, b),
        RatatuiColor::Indexed(i) => indexed_to_rgb(i),
    };
    Some(rgba_from_u8(r, g, b))
}

fn rgba_from_u8(r: u8, g: u8, b: u8) -> Rgba {
    Rgba {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: 1.0,
    }
}

/// Standard xterm 256-color palette: 0-15 mirror the named ANSI colors,
/// 16-231 are a 6x6x6 RGB cube, 232-255 are a grayscale ramp.
fn indexed_to_rgb(index: u8) -> (u8, u8, u8) {
    const ANSI_16: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (128, 0, 0),
        (0, 128, 0),
        (128, 128, 0),
        (0, 0, 128),
        (128, 0, 128),
        (0, 128, 128),
        (192, 192, 192),
        (128, 128, 128),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (0, 0, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    const fn cube_level(level: u8) -> u8 {
        if level == 0 { 0 } else { 55 + level * 40 }
    }

    match index {
        0..=15 => ANSI_16[index as usize],
        16..=231 => {
            let n = index - 16;
            let r = cube_level(n / 36);
            let g = cube_level((n / 6) % 6);
            let b = cube_level(n % 6);
            (r, g, b)
        }
        232..=255 => {
            let gray = 8 + (index - 232) * 10;
            (gray, gray, gray)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filters::{ColorConfig, FilterType};

    fn filter_def(pattern: &str, fg: Option<RatatuiColor>, group: Option<&str>) -> FilterDef {
        FilterDef {
            id: 1,
            pattern: pattern.to_string(),
            filter_type: FilterType::Include,
            enabled: true,
            color_config: fg.map(|fg| ColorConfig {
                fg: Some(fg),
                bg: None,
                match_only: true,
            }),
            use_regex: false,
            ignore_case: false,
            group: group.map(str::to_string),
        }
    }

    #[test]
    fn filter_row_color_uses_own_color() {
        let def = filter_def("ERROR", Some(RatatuiColor::Red), None);
        assert_eq!(filter_row_color(&def, &[]), Some(rgba_from_u8(128, 0, 0)));
    }

    #[test]
    fn filter_row_color_falls_back_to_group_color() {
        let def = filter_def("ERROR", None, Some("errors"));
        let group = GroupDef {
            name: "errors".to_string(),
            color_config: Some(ColorConfig {
                fg: Some(RatatuiColor::Blue),
                bg: None,
                match_only: true,
            }),
            enabled: true,
        };
        assert_eq!(
            filter_row_color(&def, &[group]),
            Some(rgba_from_u8(0, 0, 128))
        );
    }

    #[test]
    fn group_tag_color_uses_the_groups_own_style_not_the_filters() {
        let def = filter_def("ERROR", Some(RatatuiColor::Cyan), Some("errors"));
        let group = GroupDef {
            name: "errors".to_string(),
            color_config: Some(ColorConfig {
                fg: Some(RatatuiColor::Blue),
                bg: None,
                match_only: true,
            }),
            enabled: true,
        };
        assert_eq!(
            group_tag_color(&def, &[group]),
            Some(rgba_from_u8(0, 0, 128))
        );
    }

    #[test]
    fn group_tag_color_is_none_without_a_group() {
        let def = filter_def("ERROR", None, None);
        assert_eq!(group_tag_color(&def, &[]), None);
    }

    #[test]
    fn group_tag_color_is_none_when_group_has_no_style() {
        let def = filter_def("ERROR", None, Some("errors"));
        assert_eq!(group_tag_color(&def, &[]), None);
    }

    #[test]
    fn filter_row_color_is_none_without_any_color() {
        let def = filter_def("ERROR", None, None);
        assert_eq!(filter_row_color(&def, &[]), None);
    }

    #[test]
    fn group_row_color_uses_its_own_style() {
        let group = GroupDef {
            name: "errors".to_string(),
            color_config: Some(ColorConfig {
                fg: Some(RatatuiColor::Green),
                bg: None,
                match_only: true,
            }),
            enabled: true,
        };
        assert_eq!(group_row_color(&group), Some(rgba_from_u8(0, 128, 0)));
    }

    #[test]
    fn group_row_color_is_none_without_a_style() {
        let group = GroupDef::default();
        assert_eq!(group_row_color(&group), None);
    }

    fn filter_def_with_colors(
        pattern: &str,
        fg: Option<RatatuiColor>,
        bg: Option<RatatuiColor>,
    ) -> FilterDef {
        FilterDef {
            id: 1,
            pattern: pattern.to_string(),
            filter_type: FilterType::Include,
            enabled: true,
            color_config: Some(ColorConfig {
                fg,
                bg,
                match_only: true,
            }),
            use_regex: false,
            ignore_case: false,
            group: None,
        }
    }

    #[test]
    fn resolve_line_colors_returns_both_fg_and_bg_from_the_same_filter() {
        let defs = vec![filter_def_with_colors(
            "ERROR",
            Some(RatatuiColor::Red),
            Some(RatatuiColor::Blue),
        )];
        assert_eq!(
            resolve_line_colors(b"ERROR: something broke", &defs, &[]),
            (Some(rgba_from_u8(128, 0, 0)), Some(rgba_from_u8(0, 0, 128)))
        );
    }

    #[test]
    fn resolve_line_colors_supports_a_background_only_filter() {
        let defs = vec![filter_def_with_colors(
            "ERROR",
            None,
            Some(RatatuiColor::Blue),
        )];
        assert_eq!(
            resolve_line_colors(b"ERROR: something broke", &defs, &[]),
            (None, Some(rgba_from_u8(0, 0, 128)))
        );
    }

    #[test]
    fn resolve_line_colors_is_none_none_when_nothing_matches() {
        let defs = vec![filter_def_with_colors(
            "ERROR",
            Some(RatatuiColor::Red),
            Some(RatatuiColor::Blue),
        )];
        assert_eq!(
            resolve_line_colors(b"all good here", &defs, &[]),
            (None, None)
        );
    }

    #[test]
    fn resolve_line_color_matches_first_styled_enabled_filter() {
        let defs = vec![filter_def("ERROR", Some(RatatuiColor::Red), None)];
        assert_eq!(
            resolve_line_color(b"ERROR: something broke", &defs, &[]),
            Some(rgba_from_u8(128, 0, 0))
        );
    }

    #[test]
    fn resolve_line_color_is_none_when_no_filter_matches() {
        let defs = vec![filter_def("ERROR", Some(RatatuiColor::Red), None)];
        assert_eq!(resolve_line_color(b"all good here", &defs, &[]), None);
    }

    #[test]
    fn resolve_line_color_ignores_disabled_filters() {
        let mut def = filter_def("ERROR", Some(RatatuiColor::Red), None);
        def.enabled = false;
        assert_eq!(
            resolve_line_color(b"ERROR: something broke", &[def], &[]),
            None
        );
    }

    #[test]
    fn resolve_line_color_ignores_matching_filters_with_no_color() {
        let def = filter_def("ERROR", None, None);
        assert_eq!(
            resolve_line_color(b"ERROR: something broke", &[def], &[]),
            None
        );
    }

    #[test]
    fn resolve_line_color_uses_groups_color_for_matching_member_filter() {
        let def = filter_def("ERROR", None, Some("errors"));
        let group = GroupDef {
            name: "errors".to_string(),
            color_config: Some(ColorConfig {
                fg: Some(RatatuiColor::Magenta),
                bg: None,
                match_only: true,
            }),
            enabled: true,
        };
        assert_eq!(
            resolve_line_color(b"ERROR: something broke", &[def], &[group]),
            Some(rgba_from_u8(128, 0, 128))
        );
    }

    #[test]
    fn reset_has_no_gpui_equivalent() {
        assert_eq!(ratatui_color_to_gpui(RatatuiColor::Reset), None);
    }

    #[test]
    fn named_colors_map_to_standard_xterm_rgb() {
        let cases = [
            (RatatuiColor::Black, (0, 0, 0)),
            (RatatuiColor::Red, (128, 0, 0)),
            (RatatuiColor::Green, (0, 128, 0)),
            (RatatuiColor::Yellow, (128, 128, 0)),
            (RatatuiColor::Blue, (0, 0, 128)),
            (RatatuiColor::Magenta, (128, 0, 128)),
            (RatatuiColor::Cyan, (0, 128, 128)),
            (RatatuiColor::Gray, (192, 192, 192)),
            (RatatuiColor::DarkGray, (128, 128, 128)),
            (RatatuiColor::LightRed, (255, 0, 0)),
            (RatatuiColor::LightGreen, (0, 255, 0)),
            (RatatuiColor::LightYellow, (255, 255, 0)),
            (RatatuiColor::LightBlue, (0, 0, 255)),
            (RatatuiColor::LightMagenta, (255, 0, 255)),
            (RatatuiColor::LightCyan, (0, 255, 255)),
            (RatatuiColor::White, (255, 255, 255)),
        ];
        for (input, (r, g, b)) in cases {
            assert_eq!(
                ratatui_color_to_gpui(input),
                Some(rgba_from_u8(r, g, b)),
                "{input:?}"
            );
        }
    }

    #[test]
    fn rgb_passes_through_unchanged() {
        assert_eq!(
            ratatui_color_to_gpui(RatatuiColor::Rgb(10, 20, 30)),
            Some(rgba_from_u8(10, 20, 30))
        );
    }

    #[test]
    fn indexed_0_to_15_matches_named_ansi_colors() {
        assert_eq!(
            ratatui_color_to_gpui(RatatuiColor::Indexed(0)),
            ratatui_color_to_gpui(RatatuiColor::Black)
        );
        assert_eq!(
            ratatui_color_to_gpui(RatatuiColor::Indexed(9)),
            ratatui_color_to_gpui(RatatuiColor::LightRed)
        );
        assert_eq!(
            ratatui_color_to_gpui(RatatuiColor::Indexed(15)),
            ratatui_color_to_gpui(RatatuiColor::White)
        );
    }

    #[test]
    fn indexed_6x6x6_cube_matches_xterm_palette() {
        // Index 196 is the well-known "bright red" in the xterm 256-color cube.
        assert_eq!(
            ratatui_color_to_gpui(RatatuiColor::Indexed(196)),
            Some(rgba_from_u8(255, 0, 0))
        );
        // Index 21 is pure blue in the cube (r=0, g=0, b=level 5).
        assert_eq!(
            ratatui_color_to_gpui(RatatuiColor::Indexed(21)),
            Some(rgba_from_u8(0, 0, 255))
        );
    }

    #[test]
    fn indexed_232_to_255_is_grayscale_ramp() {
        assert_eq!(
            ratatui_color_to_gpui(RatatuiColor::Indexed(232)),
            Some(rgba_from_u8(8, 8, 8))
        );
        assert_eq!(
            ratatui_color_to_gpui(RatatuiColor::Indexed(255)),
            Some(rgba_from_u8(238, 238, 238))
        );
    }
}
