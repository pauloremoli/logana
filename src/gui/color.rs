use crate::filters::{
    FilterDecision, FilterDef, GroupDef, MatchCollector, build_filter, effective_color_config,
    group_style,
};
use crate::style::Color;
use gpui_kit::gpui::Rgba;

/// A log line's resolved highlight: a color plus exactly which byte
/// ranges of the line it applies to. A line-mode (`-l`) filter produces
/// one span covering the whole line; the default match-only styling
/// produces one span per match — `Filter::evaluate` already reports
/// whichever shape is right (see `resolve_line_style`), so rendering
/// never needs to special-case `-l` itself.
pub struct LineStyle {
    pub fg: Option<Rgba>,
    pub bg: Option<Rgba>,
    pub spans: Vec<(usize, usize)>,
    pub whole_line: bool,
}

/// The foreground color a filter row (or group row) should render with,
/// resolved the same way the TUI resolves it: the filter's own color, else
/// its group's, if it has one.
pub fn filter_row_color(def: &FilterDef, group_defs: &[GroupDef]) -> Option<Rgba> {
    let cc = effective_color_config(def, group_defs)?;
    color_to_gpui(cc.fg?)
}

/// The background color a filter row should render with — mirrors
/// `filter_row_color`'s fallback (the filter's own color, else its
/// group's) for `bg` instead of `fg`.
pub fn filter_row_bg_color(def: &FilterDef, group_defs: &[GroupDef]) -> Option<Rgba> {
    let cc = effective_color_config(def, group_defs)?;
    color_to_gpui(cc.bg?)
}

/// The color a filter's `[group]` tag renders with: the group's own
/// predefined style specifically (unlike `filter_row_color`, this never
/// falls back to the filter's own color — mirrors the TUI sidebar, which
/// colors the tag and the pattern independently).
pub fn group_tag_color(def: &FilterDef, group_defs: &[GroupDef]) -> Option<Rgba> {
    let name = def.group.as_deref()?;
    let cc = group_style(group_defs, name)?;
    color_to_gpui(cc.fg?)
}

/// The foreground color a group row should render with, from its own
/// predefined style (groups have no fallback of their own to inherit from).
pub fn group_row_color(def: &GroupDef) -> Option<Rgba> {
    color_to_gpui(def.color_config.as_ref()?.fg?)
}

/// The background color a group row should render with, from its own
/// predefined style.
pub fn group_row_bg_color(def: &GroupDef) -> Option<Rgba> {
    color_to_gpui(def.color_config.as_ref()?.bg?)
}

/// Resolves a log line's full highlight styling — color plus exactly
/// which byte ranges it covers — from the first enabled filter (in list
/// order) that has a resolved color and whose pattern matches the raw
/// line bytes. fg, bg, and the match spans always come from the *same*
/// matched filter, not independent searches, so a filter styled with
/// only `--bg` still wins over a later filter that only sets `--fg`,
/// mirroring how the TUI's own `FilterManager`-built `Style`s combine
/// fg/bg from one filter. `None` means no styled filter matched — the
/// line renders in the default text color with no background.
pub fn resolve_line_style(
    line: &[u8],
    filter_defs: &[FilterDef],
    group_defs: &[GroupDef],
) -> Option<LineStyle> {
    filter_defs
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
            let mut collector = MatchCollector::new(line);
            if filter.evaluate(line, &mut collector) == FilterDecision::Neutral {
                return None;
            }
            if collector.spans.is_empty() {
                return None;
            }
            Some(LineStyle {
                whole_line: !cc.match_only,
                fg: cc.fg.and_then(color_to_gpui),
                bg: cc.bg.and_then(color_to_gpui),
                spans: collector.spans.iter().map(|s| (s.start, s.end)).collect(),
            })
        })
}

/// Match against the original line, preserving the filter's match-only setting.
pub fn resolve_row_style(
    line: &[u8],
    filter_defs: &[FilterDef],
    group_defs: &[GroupDef],
) -> Option<LineStyle> {
    resolve_line_style(line, filter_defs, group_defs)
}

/// Project raw-line match spans into a displayed field. Whole-line styling covers every cell.
pub fn cell_style_spans(raw: &[u8], text: &str, style: &LineStyle) -> Vec<std::ops::Range<usize>> {
    if style.whole_line {
        return if text.is_empty() {
            vec![]
        } else {
            vec![0..text.len()]
        };
    }
    let mut ranges = Vec::new();
    for &(start, end) in &style.spans {
        let Some(matched) = raw
            .get(start..end)
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
        else {
            continue;
        };
        if matched.is_empty() {
            continue;
        }
        ranges.extend(
            text.match_indices(matched)
                .map(|(offset, value)| offset..offset + value.len()),
        );
    }
    ranges.sort_by_key(|range| (range.start, range.end));
    let mut merged: Vec<std::ops::Range<usize>> = Vec::new();
    for range in ranges {
        if let Some(last) = merged.last_mut()
            && range.start <= last.end
        {
            last.end = last.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    merged
}

/// Compose every enabled filter with the same engine used by the terminal renderer.
pub fn row_styles(tab: &crate::shared::TabState, raw: &[u8]) -> Vec<LineStyle> {
    if !tab.filter.enabled {
        return Vec::new();
    }
    let (manager, styles, dates, fields) = tab.log_manager.build_filter_manager();
    let mut collector = MatchCollector::new(raw);
    collector.with_priority(500);
    manager.evaluate_into(&mut collector);
    if let Some(parts) = tab
        .display
        .format
        .as_deref()
        .and_then(|parser| parser.parse_line(raw))
    {
        for date in &dates {
            if parts
                .timestamp
                .is_some_and(|ts| date.filter.matches(ts, None))
            {
                if date.match_only {
                    if let Some(ts) = parts.timestamp {
                        push_value(&mut collector, raw, ts, date.style_id);
                    }
                } else {
                    collector.push(0, raw.len(), date.style_id);
                }
            }
        }
        for field in &fields {
            if !crate::filters::field_filter_matches(&field.field_filter, &parts, raw) {
                continue;
            }
            if !field.match_only {
                collector.push(0, raw.len(), field.style_id);
                continue;
            }
            for (name, pattern) in &field.field_filter.conditions {
                if let Some(value) =
                    crate::filters::resolve_field(name, &parts).filter(|v| v.contains(pattern))
                {
                    push_value(&mut collector, raw, value, field.style_id);
                }
            }
            if let Some(text) = &field.field_filter.text {
                push_value(&mut collector, raw, text, field.style_id);
            }
        }
    }
    crate::filters::manager::styled_spans(&collector, &styles)
        .into_iter()
        .map(|(start, end, style)| LineStyle {
            fg: style.fg.and_then(color_to_gpui),
            bg: style.bg.and_then(color_to_gpui),
            whole_line: start == 0 && end == raw.len(),
            spans: vec![(start, end)],
        })
        .collect()
}
fn push_value(
    collector: &mut MatchCollector,
    raw: &[u8],
    value: &str,
    style: crate::filters::StyleId,
) {
    if value.is_empty() {
        return;
    }
    for (start, bytes) in raw.windows(value.len()).enumerate() {
        if bytes == value.as_bytes() {
            collector.push(start, start + value.len(), style);
        }
    }
}

pub fn color_to_gpui(color: Color) -> Option<Rgba> {
    let (r, g, b) = match color {
        Color::Reset => return None,
        Color::Black => (0, 0, 0),
        Color::Red => (128, 0, 0),
        Color::Green => (0, 128, 0),
        Color::Yellow => (128, 128, 0),
        Color::Blue => (0, 0, 128),
        Color::Magenta => (128, 0, 128),
        Color::Cyan => (0, 128, 128),
        Color::Gray => (192, 192, 192),
        Color::DarkGray => (128, 128, 128),
        Color::LightRed => (255, 0, 0),
        Color::LightGreen => (0, 255, 0),
        Color::LightYellow => (255, 255, 0),
        Color::LightBlue => (0, 0, 255),
        Color::LightMagenta => (255, 0, 255),
        Color::LightCyan => (0, 255, 255),
        Color::White => (255, 255, 255),
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Indexed(i) => indexed_to_rgb(i),
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

    fn filter_def(pattern: &str, fg: Option<Color>, group: Option<&str>) -> FilterDef {
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
        let def = filter_def("ERROR", Some(Color::Red), None);
        assert_eq!(filter_row_color(&def, &[]), Some(rgba_from_u8(128, 0, 0)));
    }

    #[test]
    fn filter_row_color_falls_back_to_group_color() {
        let def = filter_def("ERROR", None, Some("errors"));
        let group = GroupDef {
            name: "errors".to_string(),
            color_config: Some(ColorConfig {
                fg: Some(Color::Blue),
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
        let def = filter_def("ERROR", Some(Color::Cyan), Some("errors"));
        let group = GroupDef {
            name: "errors".to_string(),
            color_config: Some(ColorConfig {
                fg: Some(Color::Blue),
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
    fn filter_row_bg_color_uses_own_bg() {
        let def = FilterDef {
            id: 1,
            pattern: "ERROR".to_string(),
            filter_type: FilterType::Include,
            enabled: true,
            color_config: Some(ColorConfig {
                fg: None,
                bg: Some(Color::Blue),
                match_only: true,
            }),
            use_regex: false,
            ignore_case: false,
            group: None,
        };
        assert_eq!(
            filter_row_bg_color(&def, &[]),
            Some(rgba_from_u8(0, 0, 128))
        );
    }

    #[test]
    fn filter_row_bg_color_falls_back_to_group_bg() {
        let def = filter_def("ERROR", None, Some("errors"));
        let group = GroupDef {
            name: "errors".to_string(),
            color_config: Some(ColorConfig {
                fg: None,
                bg: Some(Color::Red),
                match_only: true,
            }),
            enabled: true,
        };
        assert_eq!(
            filter_row_bg_color(&def, &[group]),
            Some(rgba_from_u8(128, 0, 0))
        );
    }

    #[test]
    fn group_row_color_uses_its_own_style() {
        let group = GroupDef {
            name: "errors".to_string(),
            color_config: Some(ColorConfig {
                fg: Some(Color::Green),
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

    #[test]
    fn group_row_bg_color_uses_its_own_style() {
        let group = GroupDef {
            name: "errors".to_string(),
            color_config: Some(ColorConfig {
                fg: None,
                bg: Some(Color::Green),
                match_only: true,
            }),
            enabled: true,
        };
        assert_eq!(group_row_bg_color(&group), Some(rgba_from_u8(0, 128, 0)));
    }

    #[test]
    fn group_row_bg_color_is_none_without_a_style() {
        let group = GroupDef::default();
        assert_eq!(group_row_bg_color(&group), None);
    }

    fn filter_def_with_colors(pattern: &str, fg: Option<Color>, bg: Option<Color>) -> FilterDef {
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
    fn row_style_matches_fields_outside_the_message_and_respects_style_scope() {
        let line = b"2026-10-04T00:54:38.686174+02:00\t\tintune-agent\tpaulo-mini\t1682074\tAttempting to shutdown the auth library";
        let defs = [filter_def_with_colors(
            "paulo",
            Some(Color::Red),
            Some(Color::Blue),
        )];
        let style = resolve_row_style(line, &defs, &[]).unwrap();
        assert!(!style.whole_line);
        assert_eq!(cell_style_spans(line, "paulo-mini", &style), vec![0..5]);
        assert!(
            cell_style_spans(line, "Attempting to shutdown the auth library", &style).is_empty()
        );
        let mut whole_defs = defs.clone();
        whole_defs[0].color_config.as_mut().unwrap().match_only = false;
        let whole = resolve_row_style(line, &whole_defs, &[]).unwrap();
        assert!(whole.whole_line);
        assert_eq!(cell_style_spans(line, "intune-agent", &whole), vec![0..12]);
        assert_eq!(whole.spans, vec![(0, line.len())]);
        assert_eq!(style.fg, Some(rgba_from_u8(128, 0, 0)));
        assert_eq!(style.bg, Some(rgba_from_u8(0, 0, 128)));
        assert!(resolve_row_style(b"unrelated", &defs, &[]).is_none());
    }

    #[test]
    fn resolve_line_style_returns_both_fg_and_bg_from_the_same_filter() {
        let defs = vec![filter_def_with_colors(
            "ERROR",
            Some(Color::Red),
            Some(Color::Blue),
        )];
        let style = resolve_line_style(b"ERROR: something broke", &defs, &[]).unwrap();
        assert_eq!(style.fg, Some(rgba_from_u8(128, 0, 0)));
        assert_eq!(style.bg, Some(rgba_from_u8(0, 0, 128)));
    }

    #[test]
    fn resolve_line_style_supports_a_background_only_filter() {
        let defs = vec![filter_def_with_colors("ERROR", None, Some(Color::Blue))];
        let style = resolve_line_style(b"ERROR: something broke", &defs, &[]).unwrap();
        assert_eq!(style.fg, None);
        assert_eq!(style.bg, Some(rgba_from_u8(0, 0, 128)));
    }

    #[test]
    fn resolve_line_style_is_none_when_nothing_matches() {
        let defs = vec![filter_def_with_colors(
            "ERROR",
            Some(Color::Red),
            Some(Color::Blue),
        )];
        assert!(resolve_line_style(b"all good here", &defs, &[]).is_none());
    }

    #[test]
    fn resolve_line_style_defaults_to_match_only_spans() {
        // Shared matching preserves match-only spans; GUI row rendering expands them.
        let defs = vec![filter_def("ERROR", Some(Color::Red), None)];
        let line = b"before ERROR after";
        let style = resolve_line_style(line, &defs, &[]).unwrap();
        assert_eq!(style.spans, vec![(7, 12)]);
        assert_eq!(&line[7..12], b"ERROR");
    }

    #[test]
    fn resolve_line_style_covers_the_whole_line_for_line_mode_filters() {
        let def = FilterDef {
            id: 1,
            pattern: "ERROR".to_string(),
            filter_type: FilterType::Include,
            enabled: true,
            color_config: Some(ColorConfig {
                fg: Some(Color::Red),
                bg: None,
                match_only: false,
            }),
            use_regex: false,
            ignore_case: false,
            group: None,
        };
        let line = b"before ERROR after";
        let style = resolve_line_style(line, &[def], &[]).unwrap();
        assert_eq!(style.spans, vec![(0, line.len())]);
    }

    #[test]
    fn resolve_line_style_matches_first_styled_enabled_filter() {
        let defs = vec![filter_def("ERROR", Some(Color::Red), None)];
        let style = resolve_line_style(b"ERROR: something broke", &defs, &[]).unwrap();
        assert_eq!(style.fg, Some(rgba_from_u8(128, 0, 0)));
    }

    #[test]
    fn resolve_line_style_is_none_when_no_filter_matches() {
        let defs = vec![filter_def("ERROR", Some(Color::Red), None)];
        assert!(resolve_line_style(b"all good here", &defs, &[]).is_none());
    }

    #[test]
    fn resolve_line_style_ignores_disabled_filters() {
        let mut def = filter_def("ERROR", Some(Color::Red), None);
        def.enabled = false;
        assert!(resolve_line_style(b"ERROR: something broke", &[def], &[]).is_none());
    }

    #[test]
    fn resolve_line_style_ignores_matching_filters_with_no_color() {
        let def = filter_def("ERROR", None, None);
        assert!(resolve_line_style(b"ERROR: something broke", &[def], &[]).is_none());
    }

    #[test]
    fn resolve_line_style_uses_groups_color_for_matching_member_filter() {
        let def = filter_def("ERROR", None, Some("errors"));
        let group = GroupDef {
            name: "errors".to_string(),
            color_config: Some(ColorConfig {
                fg: Some(Color::Magenta),
                bg: None,
                match_only: true,
            }),
            enabled: true,
        };
        let style = resolve_line_style(b"ERROR: something broke", &[def], &[group]).unwrap();
        assert_eq!(style.fg, Some(rgba_from_u8(128, 0, 128)));
    }

    #[test]
    fn reset_has_no_gpui_equivalent() {
        assert_eq!(color_to_gpui(Color::Reset), None);
    }

    #[test]
    fn named_colors_map_to_standard_xterm_rgb() {
        let cases = [
            (Color::Black, (0, 0, 0)),
            (Color::Red, (128, 0, 0)),
            (Color::Green, (0, 128, 0)),
            (Color::Yellow, (128, 128, 0)),
            (Color::Blue, (0, 0, 128)),
            (Color::Magenta, (128, 0, 128)),
            (Color::Cyan, (0, 128, 128)),
            (Color::Gray, (192, 192, 192)),
            (Color::DarkGray, (128, 128, 128)),
            (Color::LightRed, (255, 0, 0)),
            (Color::LightGreen, (0, 255, 0)),
            (Color::LightYellow, (255, 255, 0)),
            (Color::LightBlue, (0, 0, 255)),
            (Color::LightMagenta, (255, 0, 255)),
            (Color::LightCyan, (0, 255, 255)),
            (Color::White, (255, 255, 255)),
        ];
        for (input, (r, g, b)) in cases {
            assert_eq!(
                color_to_gpui(input),
                Some(rgba_from_u8(r, g, b)),
                "{input:?}"
            );
        }
    }

    #[test]
    fn rgb_passes_through_unchanged() {
        assert_eq!(
            color_to_gpui(Color::Rgb(10, 20, 30)),
            Some(rgba_from_u8(10, 20, 30))
        );
    }

    #[test]
    fn indexed_0_to_15_matches_named_ansi_colors() {
        assert_eq!(
            color_to_gpui(Color::Indexed(0)),
            color_to_gpui(Color::Black)
        );
        assert_eq!(
            color_to_gpui(Color::Indexed(9)),
            color_to_gpui(Color::LightRed)
        );
        assert_eq!(
            color_to_gpui(Color::Indexed(15)),
            color_to_gpui(Color::White)
        );
    }

    #[test]
    fn indexed_6x6x6_cube_matches_xterm_palette() {
        // Index 196 is the well-known "bright red" in the xterm 256-color cube.
        assert_eq!(
            color_to_gpui(Color::Indexed(196)),
            Some(rgba_from_u8(255, 0, 0))
        );
        // Index 21 is pure blue in the cube (r=0, g=0, b=level 5).
        assert_eq!(
            color_to_gpui(Color::Indexed(21)),
            Some(rgba_from_u8(0, 0, 255))
        );
    }

    #[test]
    fn indexed_232_to_255_is_grayscale_ramp() {
        assert_eq!(
            color_to_gpui(Color::Indexed(232)),
            Some(rgba_from_u8(8, 8, 8))
        );
        assert_eq!(
            color_to_gpui(Color::Indexed(255)),
            Some(rgba_from_u8(238, 238, 238))
        );
    }
}
