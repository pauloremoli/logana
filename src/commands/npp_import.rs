//! Converts a Notepad++ "Analyze" plugin XML config (AnalyseDoc highlight
//! export or User Defined Language export) into logana `Include` filters.
//! Regex-based extraction is used deliberately instead of a full XML parser:
//! these documents' structure is small and known ahead of time, and a real
//! parser would be exposed to XXE/entity-expansion risk on untrusted input.

use crate::filters::{ColorConfig, FilterDef, FilterType};
use crate::style::Color;
use regex::Regex;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::LazyLock;

#[derive(Debug)]
pub struct NppImport {
    pub filters: Vec<FilterDef>,
    pub warnings: Vec<String>,
}

static SEARCH_TEXT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<SearchText([^>]*?)>(.*?)</SearchText>").unwrap());
static KEYWORDS_LIST_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)<Keywords name="Keywords(\d+)">(.*?)</Keywords>"#).unwrap());
static WORDS_STYLE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<WordsStyle name="KEYWORDS(\d+)"([^>]*?)/>"#).unwrap());
static ATTR_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(\w+)="([^"]*)""#).unwrap());

/// Converts a Notepad++ Analyze-plugin XML config into logana `Include`
/// filters, auto-detecting AnalyseDoc vs. User Defined Language exports.
pub fn convert_npp_xml(xml_text: &str) -> Result<NppImport, String> {
    let mut warnings = Vec::new();
    let filters = if is_userlang(xml_text) {
        convert_userlang(xml_text, &mut warnings)?
    } else {
        convert_analysedoc(xml_text, &mut warnings)?
    };
    Ok(NppImport { filters, warnings })
}

fn is_userlang(xml_text: &str) -> bool {
    xml_text.contains("<UserLang") || xml_text.contains("<KeywordLists")
}

fn parse_attrs(attrs_str: &str) -> HashMap<String, String> {
    ATTR_RE
        .captures_iter(attrs_str)
        .map(|c| (c[1].to_string(), c[2].to_string()))
        .collect()
}

fn convert_analysedoc(
    xml_text: &str,
    warnings: &mut Vec<String>,
) -> Result<Vec<FilterDef>, String> {
    let mut filters = Vec::new();
    for caps in SEARCH_TEXT_RE.captures_iter(xml_text) {
        let attrs = parse_attrs(&caps[1]);
        let bg = attrs
            .get("bgColor")
            .ok_or("<SearchText> entry missing bgColor attribute")?;
        let bg_color = import_color(bg, "bgColor", None, warnings);
        filters.push(FilterDef {
            id: filters.len(),
            pattern: unescape_html(&caps[2]),
            filter_type: FilterType::Include,
            enabled: true,
            color_config: Some(ColorConfig {
                fg: Some(
                    attrs
                        .get("fgColor")
                        .map(|fg| import_color(fg, "fgColor", Some(bg_color), warnings))
                        .unwrap_or_else(|| readable_import_foreground(bg_color)),
                ),
                bg: Some(bg_color),
                match_only: true,
            }),
            use_regex: false,
            ignore_case: false,
            group: attrs.get("group").cloned(),
        });
    }
    Ok(filters)
}

fn convert_userlang(xml_text: &str, warnings: &mut Vec<String>) -> Result<Vec<FilterDef>, String> {
    let mut filters = Vec::new();

    let mut styles: HashMap<u32, (Option<String>, String)> = HashMap::new();
    for caps in WORDS_STYLE_RE.captures_iter(xml_text) {
        let n: u32 = caps[1]
            .parse()
            .map_err(|_| "invalid WordsStyle index".to_string())?;
        let attrs = parse_attrs(&caps[2]);
        if let Some(bg) = attrs.get("bgColor") {
            styles.insert(n, (attrs.get("fgColor").cloned(), bg.clone()));
        }
    }

    let mut keyword_lists: Vec<(u32, String)> = KEYWORDS_LIST_RE
        .captures_iter(xml_text)
        .map(|caps| (caps[1].parse().unwrap_or(0), unescape_html(&caps[2])))
        .collect();
    keyword_lists.sort_by_key(|(n, _)| *n);

    for (n, text) in keyword_lists {
        let Some((fg, bg)) = styles.get(&n) else {
            continue;
        };
        // UDL exports use bare RRGGBB, while Analyze exports also use names.
        let bg_color = import_color(bg, "bgColor", None, warnings);
        let fg_color = fg
            .as_ref()
            .map(|fg| import_color(fg, "fgColor", Some(bg_color), warnings))
            .unwrap_or_else(|| readable_import_foreground(bg_color));
        for token in text.split_whitespace() {
            filters.push(FilterDef {
                id: filters.len(),
                pattern: token.to_string(),
                filter_type: FilterType::Include,
                enabled: true,
                color_config: Some(ColorConfig {
                    fg: Some(fg_color),
                    bg: Some(bg_color),
                    match_only: true,
                }),
                use_regex: false,
                ignore_case: false,
                group: Some(format!("Keywords{n}")),
            });
        }
    }
    Ok(filters)
}

/// Choose black or white using the shared contrast rule. Named backgrounds
/// use the same RGB values as the GUI's ANSI palette.
fn readable_import_foreground(bg: Color) -> Color {
    let rgb = match bg {
        Color::Green => (0, 128, 0),
        Color::Red => (128, 0, 0),
        Color::Yellow => (128, 128, 0),
        Color::Cyan => (0, 128, 128),
        Color::LightGreen => (0, 255, 0),
        Color::LightRed => (255, 0, 0),
        other => crate::theme::color_to_rgb(other),
    };
    let (r, g, b) = crate::theme::readable_foreground_for(rgb);
    Color::Rgb(r, g, b)
}

/// Resolve names from Notepad++'s PowerEditor/src/ScintillaComponent/colors.h
/// to exact RGB values, retaining Analyze aliases and hex color support.
fn map_bg_color(raw: &str) -> Result<Color, String> {
    let raw = raw.trim();
    let normalized = raw.to_ascii_lowercase().replace("grey", "gray");
    let mapped = match normalized.as_str() {
        "red" => Color::Rgb(255, 0, 0),
        "darkred" => Color::Rgb(128, 0, 0),
        "offwhite" => Color::Rgb(255, 251, 240),
        "darkgreen" => Color::Rgb(0, 128, 0),
        "litegreen" => Color::Rgb(0, 255, 0),
        "bluegreen" => Color::Rgb(0, 128, 128),
        "litered" => Color::Rgb(255, 170, 170),
        "litebluegreen" => Color::Rgb(170, 255, 200),
        "liteblue" => Color::Rgb(166, 202, 240),
        "veryliteblue" => Color::Rgb(196, 249, 253),
        "extremeliteblue" => Color::Rgb(242, 244, 255),
        "darkblue" => Color::Rgb(0, 0, 128),
        "blue" => Color::Rgb(0, 0, 255),
        "black" => Color::Rgb(0, 0, 0),
        "white" => Color::Rgb(255, 255, 255),
        "darkgray" => Color::Rgb(64, 64, 64),
        "gray" => Color::Rgb(128, 128, 128),
        "litegray" => Color::Rgb(192, 192, 192),
        "verylitegray" => Color::Rgb(224, 224, 224),
        "brown" => Color::Rgb(128, 64, 0),
        "darkyellow" => Color::Rgb(255, 192, 0),
        "yellow" => Color::Rgb(255, 255, 0),
        "lightyellow" => Color::Rgb(255, 255, 213),
        "cyan" => Color::Rgb(0, 255, 255),
        "orange" => Color::Rgb(255, 128, 0),
        "purple" => Color::Rgb(128, 0, 255),
        "deeppurple" => Color::Rgb(135, 19, 151),
        "extremelitepurple" => Color::Rgb(248, 232, 255),
        "verylitepurple" => Color::Rgb(231, 216, 233),
        "liteberge" => Color::Rgb(254, 252, 245),
        "berge" => Color::Rgb(253, 248, 227),
        "midgreen" => Color::Rgb(0, 160, 0),
        "darkcyan" => Color::Rgb(64, 160, 191),
        "yellowgreen" => Color::Rgb(160, 192, 0),
        // Older Analyze exports also use these names.
        "green" => Color::Rgb(0, 128, 0),
        "litepink" => Color::Rgb(0xFF, 0xB6, 0xC1),
        "litebeige" => Color::Rgb(0xF5, 0xDE, 0xB3),
        _ => {
            if raw.len() == 6 && raw.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Color::from_str(&format!("#{raw}"))
                    .map_err(|_| format!("Invalid color '{raw}'"));
            }
            return crate::theme::parse_color(raw)
                .ok_or_else(|| format!("Unknown Notepad++ color '{raw}'"));
        }
    };
    Ok(mapped)
}

fn import_color(
    raw: &str,
    attribute: &str,
    background: Option<Color>,
    warnings: &mut Vec<String>,
) -> Color {
    map_bg_color(raw).unwrap_or_else(|_| {
        let replacement = if let Some(bg) = background {
            readable_import_foreground(bg)
        } else {
            let (_, (r, g, b)) = crate::theme::random_readable_color_pair();
            Color::Rgb(r, g, b)
        };
        warnings.push(format!(
            "Warning: unrecognized Notepad++ {attribute} '{raw}'; using {replacement}"
        ));
        replacement
    })
}

/// Unescapes the handful of HTML/XML entities Notepad++ emits in
/// `<SearchText>`/`<Keywords>` content: the five named entities plus decimal
/// and hex numeric character references.
fn unescape_html(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s.as_bytes()[i] == b'&'
            && let Some(rel_end) = s[i..].find(';')
        {
            let end = i + rel_end;
            let entity = &s[i + 1..end];
            let replacement = match entity {
                "lt" => Some('<'),
                "gt" => Some('>'),
                "amp" => Some('&'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => entity.strip_prefix('#').and_then(|num| {
                    let code = if let Some(hex) = num.strip_prefix('x').or(num.strip_prefix('X')) {
                        u32::from_str_radix(hex, 16).ok()
                    } else {
                        num.parse::<u32>().ok()
                    };
                    code.and_then(char::from_u32)
                }),
            };
            if let Some(c) = replacement {
                result.push(c);
                i = end + 1;
                continue;
            }
        }
        let ch = s[i..].chars().next().unwrap();
        result.push(ch);
        i += ch.len_utf8();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANALYSEDOC_XML: &str = r##"
        <AnalyseDoc>
            <SearchText bgColor="green" group="errs">error</SearchText>
            <SearchText bgColor="#112233">warn&amp;ing</SearchText>
        </AnalyseDoc>
    "##;

    const UDL_XML: &str = r#"
        <UserLang name="Test">
            <KeywordLists>
                <Keywords name="Keywords1">foo bar</Keywords>
                <Keywords name="Keywords2">baz</Keywords>
            </KeywordLists>
            <WordsStyle name="KEYWORDS1" fgColor="FFFFFF" bgColor="FF0000" />
        </UserLang>
    "#;

    #[test]
    fn background_only_imports_generate_readable_foregrounds() {
        for bg in ["000000", "FFFFFF", "112233", "777777", "FF0000", "00FF00"] {
            for xml in [
                format!(r##"<AnalyseDoc><SearchText bgColor="#{bg}">x</SearchText></AnalyseDoc>"##),
                format!(
                    r#"<UserLang><Keywords name="Keywords1">x</Keywords><WordsStyle name="KEYWORDS1" bgColor="{bg}" /></UserLang>"#
                ),
            ] {
                let filters = convert_npp_xml(&xml).unwrap().filters;
                assert_eq!(filters.len(), 1, "{xml}");
                let colors = filters[0].color_config.as_ref().unwrap();
                let fg = crate::theme::color_to_rgb(colors.fg.unwrap());
                let bg = crate::theme::color_to_rgb(colors.bg.unwrap());
                assert!(
                    crate::theme::contrast_ratio(fg, bg) >= crate::theme::MIN_READABLE_CONTRAST,
                    "{xml}"
                );
            }
        }
    }

    #[test]
    fn analysedoc_preserves_explicit_foreground() {
        let filters =
            convert_npp_xml(r##"<SearchText bgColor="#000000" fgColor="#123456">x</SearchText>"##)
                .unwrap()
                .filters;
        assert_eq!(
            filters[0].color_config.as_ref().unwrap().fg,
            Some(Color::Rgb(0x12, 0x34, 0x56))
        );
    }

    #[test]
    fn test_analysedoc_maps_named_color_and_group() {
        let filters = convert_npp_xml(ANALYSEDOC_XML).unwrap().filters;
        let error_filter = &filters[0];
        assert_eq!(error_filter.pattern, "error");
        assert_eq!(error_filter.filter_type, FilterType::Include);
        assert_eq!(error_filter.group.as_deref(), Some("errs"));
        assert!(!error_filter.use_regex);
        let cc = error_filter.color_config.as_ref().unwrap();
        assert_eq!(cc.fg, Some(Color::Rgb(255, 255, 255)));
        assert_eq!(cc.bg, Some(Color::Rgb(0, 128, 0)));
        assert!(cc.match_only);
    }

    #[test]
    fn test_analysedoc_passes_through_hex_color_and_unescapes_text() {
        let filters = convert_npp_xml(ANALYSEDOC_XML).unwrap().filters;
        let warn_filter = &filters[1];
        assert_eq!(warn_filter.pattern, "warn&ing");
        assert_eq!(warn_filter.group, None);
        let cc = warn_filter.color_config.as_ref().unwrap();
        assert_eq!(cc.bg, Some(Color::Rgb(0x11, 0x22, 0x33)));
    }

    #[test]
    fn test_udl_expands_keywords_into_one_filter_per_token() {
        let filters = convert_npp_xml(UDL_XML).unwrap().filters;
        // "foo" + "bar" ("baz" is skipped: no matching WordsStyle)
        assert_eq!(filters.len(), 2);
        assert_eq!(filters[0].pattern, "foo");
        assert_eq!(filters[1].pattern, "bar");
        for f in &filters {
            assert_eq!(f.group.as_deref(), Some("Keywords1"));
            assert_eq!(f.filter_type, FilterType::Include);
            let cc = f.color_config.as_ref().unwrap();
            assert_eq!(cc.fg, Some(Color::Rgb(0xFF, 0xFF, 0xFF)));
            assert_eq!(cc.bg, Some(Color::Rgb(0xFF, 0x00, 0x00)));
        }
    }

    #[test]
    fn test_udl_skips_keyword_lists_without_matching_style() {
        let filters = convert_npp_xml(UDL_XML).unwrap().filters;
        assert!(filters.iter().all(|f| f.pattern != "baz"));
    }

    #[test]
    fn test_unknown_color_name_does_not_abort_import() {
        let xml = r#"<AnalyseDoc><SearchText bgColor="mysteryColor">x</SearchText></AnalyseDoc>"#;
        let result = convert_npp_xml(xml);
        let result = result.unwrap();
        assert_eq!(result.filters.len(), 1);
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].contains("mysteryColor"));
        let colors = result.filters[0].color_config.as_ref().unwrap();
        assert!(matches!(colors.bg, Some(Color::Rgb(..))));
        let fg = crate::theme::color_to_rgb(colors.fg.unwrap());
        let bg = crate::theme::color_to_rgb(colors.bg.unwrap());
        assert!(crate::theme::contrast_ratio(fg, bg) >= crate::theme::MIN_READABLE_CONTRAST);
    }

    #[test]
    fn unknown_foregrounds_warn_and_preserve_valid_backgrounds_in_both_formats() {
        for xml in [
            r##"<AnalyseDoc><SearchText bgColor="#112233" fgColor="unknown">x</SearchText><SearchText bgColor="liteGray">y</SearchText></AnalyseDoc>"##,
            r#"<UserLang><Keywords name="Keywords1">x y</Keywords><WordsStyle name="KEYWORDS1" bgColor="112233" fgColor="unknown" /></UserLang>"#,
        ] {
            let imported = convert_npp_xml(xml).unwrap();
            assert_eq!(imported.filters.len(), 2);
            assert_eq!(imported.warnings.len(), 1);
            assert!(imported.warnings[0].contains("fgColor 'unknown'"));
            let colors = imported.filters[0].color_config.as_ref().unwrap();
            assert_eq!(colors.bg, Some(Color::Rgb(0x11, 0x22, 0x33)));
            assert_eq!(
                colors.fg,
                Some(readable_import_foreground(colors.bg.unwrap()))
            );
        }
    }

    #[test]
    fn invalid_udl_background_warns_once_per_style_and_imports_every_token() {
        let imported = convert_npp_xml(
            r#"<UserLang><Keywords name="Keywords1">x y</Keywords><WordsStyle name="KEYWORDS1" bgColor="unknown" /></UserLang>"#
        ).unwrap();
        assert_eq!(imported.filters.len(), 2);
        assert_eq!(imported.warnings.len(), 1);
        assert_eq!(
            imported.filters[0].color_config,
            imported.filters[1].color_config
        );
    }

    #[test]
    fn all_notepad_palette_colors_import_with_exact_rgb() {
        // Expected values from Notepad++ PowerEditor/src/ScintillaComponent/colors.h.
        for (name, expected) in [
            ("red", Color::Rgb(255, 0, 0)),
            ("darkRed", Color::Rgb(128, 0, 0)),
            ("offWhite", Color::Rgb(255, 251, 240)),
            ("darkGreen", Color::Rgb(0, 128, 0)),
            ("liteGreen", Color::Rgb(0, 255, 0)),
            ("blueGreen", Color::Rgb(0, 128, 128)),
            ("liteRed", Color::Rgb(255, 170, 170)),
            ("liteBlueGreen", Color::Rgb(170, 255, 200)),
            ("liteBlue", Color::Rgb(166, 202, 240)),
            ("veryLiteBlue", Color::Rgb(196, 249, 253)),
            ("extremeLiteBlue", Color::Rgb(242, 244, 255)),
            ("darkBlue", Color::Rgb(0, 0, 128)),
            ("blue", Color::Rgb(0, 0, 255)),
            ("black", Color::Rgb(0, 0, 0)),
            ("white", Color::Rgb(255, 255, 255)),
            ("darkGrey", Color::Rgb(64, 64, 64)),
            ("grey", Color::Rgb(128, 128, 128)),
            ("liteGrey", Color::Rgb(192, 192, 192)),
            ("veryLiteGrey", Color::Rgb(224, 224, 224)),
            ("brown", Color::Rgb(128, 64, 0)),
            ("darkYellow", Color::Rgb(255, 192, 0)),
            ("yellow", Color::Rgb(255, 255, 0)),
            ("lightYellow", Color::Rgb(255, 255, 213)),
            ("cyan", Color::Rgb(0, 255, 255)),
            ("orange", Color::Rgb(255, 128, 0)),
            ("purple", Color::Rgb(128, 0, 255)),
            ("deepPurple", Color::Rgb(135, 19, 151)),
            ("extremeLitePurple", Color::Rgb(248, 232, 255)),
            ("veryLitePurple", Color::Rgb(231, 216, 233)),
            ("liteBerge", Color::Rgb(254, 252, 245)),
            ("berge", Color::Rgb(253, 248, 227)),
            ("midGreen", Color::Rgb(0, 160, 0)),
            ("darkCyan", Color::Rgb(64, 160, 191)),
            ("yellowGreen", Color::Rgb(160, 192, 0)),
        ] {
            for alias in [
                name.to_string(),
                name.to_ascii_uppercase(),
                name.replace("Grey", "Gray").replace("grey", "gray"),
            ] {
                assert_eq!(map_bg_color(&alias).unwrap(), expected, "{alias}");
                for xml in [
                    format!(r#"<SearchText bgColor="{alias}" fgColor="{alias}">x</SearchText>"#),
                    format!(
                        r#"<UserLang><Keywords name="Keywords1">x</Keywords><WordsStyle name="KEYWORDS1" bgColor="{alias}" fgColor="{alias}" /></UserLang>"#
                    ),
                ] {
                    let imported = convert_npp_xml(&xml).unwrap();
                    assert!(
                        imported.warnings.is_empty(),
                        "{alias}: {:?}",
                        imported.warnings
                    );
                    let colors = imported.filters[0].color_config.as_ref().unwrap();
                    assert_eq!(colors.bg, Some(expected), "{alias}");
                    assert_eq!(colors.fg, Some(expected), "{alias}");
                }
            }
            let imported =
                convert_npp_xml(&format!(r#"<SearchText bgColor="{name}">x</SearchText>"#))
                    .unwrap();
            let colors = imported.filters[0].color_config.as_ref().unwrap();
            assert!(
                crate::theme::contrast_ratio(
                    crate::theme::color_to_rgb(colors.fg.unwrap()),
                    crate::theme::color_to_rgb(expected),
                ) >= crate::theme::MIN_READABLE_CONTRAST,
                "{name}"
            );
        }
    }

    #[test]
    fn lite_gray_aliases_are_recognized() {
        for name in ["liteGray", "liteGrey", "LITEGRAY"] {
            assert_eq!(map_bg_color(name).unwrap(), Color::Rgb(192, 192, 192));
        }
    }

    #[test]
    fn test_map_bg_color_translates_notepad_names() {
        assert_eq!(map_bg_color("liteGreen").unwrap(), Color::Rgb(0, 255, 0));
        assert_eq!(map_bg_color("liteRed").unwrap(), Color::Rgb(255, 170, 170));
        assert_eq!(
            map_bg_color("veryLiteGrey").unwrap(),
            Color::Rgb(224, 224, 224)
        );
        assert_eq!(
            map_bg_color("#abcdef").unwrap(),
            Color::Rgb(0xAB, 0xCD, 0xEF)
        );
    }

    #[test]
    fn test_unescape_html_handles_named_and_numeric_entities() {
        assert_eq!(unescape_html("a&amp;b&lt;c&gt;d"), "a&b<c>d");
        assert_eq!(unescape_html("&#65;&#x42;"), "AB");
    }
}
