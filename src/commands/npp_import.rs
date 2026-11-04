//! Converts a Notepad++ "Analyze" plugin XML config (AnalyseDoc highlight
//! export or User Defined Language export) into logana `Include` filters.
//! XML parsing rejects DTDs and never resolves external entities.

use crate::filters::{ColorConfig, FilterDef, FilterType};
use crate::style::Color;
use regex::Regex;
use std::str::FromStr;

#[derive(Debug)]
pub struct NppImport {
    pub filters: Vec<FilterDef>,
    pub warnings: Vec<String>,
}

/// Converts AnalysePlugin and UDL XML into shared filters.
/// Fragment support is retained for existing callers and exported snippets.
pub fn convert_npp_xml(xml_text: &str) -> Result<NppImport, String> {
    let wrapped = format!("<Import>{xml_text}</Import>");
    let doc = roxmltree::Document::parse(xml_text)
        .or_else(|_| roxmltree::Document::parse(&wrapped))
        .map_err(|e| format!("Invalid Notepad++ XML: {e}"))?;
    let mut warnings = Vec::new();
    let mut filters = Vec::new();
    let languages: Vec<_> = doc
        .descendants()
        .filter(|n| n.has_tag_name("UserLang"))
        .collect();
    if !languages.is_empty() {
        for lang in languages {
            convert_userlang(lang, &mut filters, &mut warnings)?;
        }
    } else if doc
        .descendants()
        .any(|n| n.has_tag_name("AnalyseDoc") || n.has_tag_name("SearchText"))
    {
        convert_analysedoc(doc.root(), &mut filters, &mut warnings)?;
    } else {
        return Err("No AnalyseDoc or UserLang configuration found".into());
    }
    Ok(NppImport { filters, warnings })
}

fn xml_text(node: roxmltree::Node<'_, '_>) -> String {
    node.children()
        .filter(|n| n.is_text())
        .filter_map(|n| n.text())
        .collect()
}

fn xml_bool(node: roxmltree::Node<'_, '_>, name: &str, default: bool) -> bool {
    node.attribute(name)
        .map(|s| matches!(s, "true" | "1" | "yes"))
        .unwrap_or(default)
}

fn convert_analysedoc(
    root: roxmltree::Node<'_, '_>,
    filters: &mut Vec<FilterDef>,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    for node in root.descendants().filter(|n| n.has_tag_name("SearchText")) {
        let bg = node
            .attribute("bgColor")
            .map(|raw| import_color(raw, "bgColor", None, warnings));
        let fg = node
            .attribute("color")
            .or_else(|| node.attribute("fgColor"));
        let mut pattern = xml_text(node);
        if pattern.is_empty() {
            warnings.push("Skipped empty SearchText".into());
            continue;
        }
        let mut use_regex = match node.attribute("searchType").unwrap_or("normal") {
            "normal" => false,
            "escaped" => {
                pattern = decode_escaped(&pattern);
                false
            }
            "regex" => true,
            other => return Err(format!("Unsupported Notepad++ searchType '{other}'")),
        };
        if xml_bool(node, "wholeWord", false) {
            if !use_regex {
                pattern = regex::escape(&pattern);
            }
            pattern = format!(r"\b(?:{pattern})\b");
            use_regex = true;
        }
        if use_regex {
            Regex::new(&pattern)
                .map_err(|e| format!("Unsupported Notepad++ regex '{pattern}': {e}"))?;
        }
        for name in ["hide", "bold", "italic", "underlined"] {
            if xml_bool(node, name, false) {
                warnings.push(format!(
                    "Notepad++ {name} is not supported; importing search and colors only"
                ));
            }
        }
        filters.push(FilterDef {
            id: filters.len(),
            pattern,
            filter_type: FilterType::Include,
            enabled: xml_bool(node, "doSearch", true),
            color_config: Some(ColorConfig {
                fg: fg
                    .map(|v| {
                        import_color(
                            v,
                            if node.attribute("color").is_some() {
                                "color"
                            } else {
                                "fgColor"
                            },
                            bg,
                            warnings,
                        )
                    })
                    .or_else(|| bg.map(readable_import_foreground)),
                bg,
                match_only: node.attribute("select") == Some("text"),
            }),
            use_regex,
            ignore_case: !xml_bool(node, "matchCase", false),
            group: node.attribute("group").map(str::to_owned),
        });
    }
    Ok(())
}

/// AnalysePlugin extended searches preserve unknown and incomplete escapes.
fn decode_escaped(text: &str) -> String {
    let mut chars = text.chars().peekable();
    let mut result = String::new();
    while let Some(c) = chars.next() {
        if c != '\\' {
            result.push(c);
            continue;
        }
        let Some(next) = chars.next() else {
            result.push(c);
            break;
        };
        match next {
            'n' => result.push('\n'),
            'r' => result.push('\r'),
            't' => result.push('\t'),
            '0' => result.push('\0'),
            '\\' => result.push('\\'),
            'b' | 'o' | 'd' | 'x' => {
                let (width, base) = match next {
                    'b' => (8, 2),
                    'o' => (3, 8),
                    'd' => (3, 10),
                    _ => (2, 16),
                };
                let digits: String = chars.clone().take(width).collect();
                if digits.len() == width
                    && let Ok(value) = u8::from_str_radix(&digits, base)
                {
                    for _ in 0..width {
                        chars.next();
                    }
                    result.push(char::from(value));
                } else {
                    result.push('\\');
                    result.push(next);
                }
            }
            _ => {
                result.push('\\');
                result.push(next);
            }
        }
    }
    result
}

fn convert_userlang(
    lang: roxmltree::Node<'_, '_>,
    filters: &mut Vec<FilterDef>,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    let ignore_case = lang
        .descendants()
        .find(|n| n.has_tag_name("Global"))
        .is_some_and(|n| xml_bool(n, "caseIgnored", false));
    let mut lists: Vec<_> = lang
        .descendants()
        .filter(|n| n.has_tag_name("Keywords"))
        .filter_map(|n| {
            n.attribute("name")?
                .strip_prefix("Keywords")?
                .parse::<u8>()
                .ok()
                .map(|i| (i, n))
        })
        .filter(|(i, _)| (1..=8).contains(i))
        .collect();
    lists.sort_by_key(|(i, _)| *i);
    for (index, list) in lists {
        let style_name = format!("KEYWORDS{index}");
        let Some(style) = lang.descendants().find(|n| {
            n.has_tag_name("WordsStyle") && n.attribute("name") == Some(style_name.as_str())
        }) else {
            warnings.push(format!("Skipped Keywords{index}: no matching style"));
            continue;
        };
        let bg = style
            .attribute("bgColor")
            .map(|raw| import_color(raw, "bgColor", None, warnings));
        let fg = style
            .attribute("fgColor")
            .map(|v| import_color(v, "fgColor", bg, warnings))
            .or_else(|| bg.map(readable_import_foreground));
        if style.attribute("fontStyle").is_some_and(|v| v != "0")
            || style.attribute("colorStyle").is_some_and(|v| v != "3")
        {
            warnings.push(format!(
                "Keywords{index}: font decorations/color inheritance are not supported"
            ));
        }
        if lang
            .descendants()
            .filter(|n| n.has_tag_name("Prefix"))
            .any(|n| xml_bool(n, &format!("Keywords{index}"), false))
        {
            warnings.push(format!(
                "Keywords{index}: prefix mode is imported as substring matching"
            ));
        }
        for token in udl_tokens(&xml_text(list))? {
            if token.chars().any(char::is_whitespace) {
                warnings.push(format!("Keywords{index}: multi-part keyword uses literal whitespace; cross-line matching is not supported"));
            }
            filters.push(FilterDef {
                id: filters.len(),
                pattern: token,
                filter_type: FilterType::Include,
                enabled: true,
                color_config: Some(ColorConfig {
                    fg,
                    bg,
                    match_only: true,
                }),
                use_regex: false,
                ignore_case,
                group: Some(format!("Keywords{index}")),
            });
        }
    }
    if lang.descendants().any(|n| {
        n.has_tag_name("Keywords")
            && !n.attribute("name").unwrap_or("").starts_with("Keywords")
            && !xml_text(n).trim().is_empty()
    }) {
        warnings.push(
            "UDL comments, numbers, operators, folding and delimiters are not imported".into(),
        );
    }
    Ok(())
}

fn udl_tokens(text: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quote = None;
    for c in text.chars() {
        if quote == Some(c) {
            quote = None;
        } else if quote.is_none() && matches!(c, '\'' | '"') {
            quote = Some(c);
        } else if quote.is_none() && c.is_whitespace() {
            if !token.is_empty() {
                tokens.push(std::mem::take(&mut token));
            }
        } else {
            token.push(c);
        }
    }
    if quote.is_some() {
        return Err("Unclosed quoted UDL keyword".into());
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    Ok(tokens)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omitted_background_stays_unset_in_both_formats() {
        for xml in [
            r#"<SearchText color="red">x</SearchText>"#,
            r#"<SearchText>x</SearchText>"#,
            r#"<UserLang><Keywords name="Keywords1">x</Keywords><WordsStyle name="KEYWORDS1" fgColor="red"/></UserLang>"#,
        ] {
            let imported = convert_npp_xml(xml).unwrap();
            assert_eq!(imported.filters.len(), 1);
            assert!(imported.warnings.is_empty());
            let colors = imported.filters[0].color_config.as_ref().unwrap();
            assert_eq!(colors.bg, None);
            if xml.contains("red") {
                assert_eq!(colors.fg, Some(Color::Rgb(255, 0, 0)));
            } else {
                assert_eq!(colors.fg, None);
            }
        }
    }

    #[test]
    fn actual_analyse_options_and_xml_syntax() {
        let imported = convert_npp_xml(
            r#"<AnalyseDoc>
          <!-- <SearchText bgColor="red">fake</SearchText> -->
          <SearchText group='a&amp;b' color='red' searchType='regex'
            matchCase='true' doSearch='false' select='text'><![CDATA[err[0-9]+]]></SearchText>
          <SearchText>default</SearchText>
        </AnalyseDoc>"#,
        )
        .unwrap();
        assert_eq!(imported.filters.len(), 2);
        let f = &imported.filters[0];
        assert_eq!(f.pattern, "err[0-9]+");
        assert!(f.use_regex);
        assert!(!f.ignore_case);
        assert!(!f.enabled);
        assert_eq!(f.group.as_deref(), Some("a&b"));
        assert_eq!(
            f.color_config.as_ref().unwrap().fg,
            Some(Color::Rgb(255, 0, 0))
        );
        assert!(f.color_config.as_ref().unwrap().match_only);
        let f = &imported.filters[1];
        assert!(f.ignore_case);
        assert!(!f.color_config.as_ref().unwrap().match_only);
        assert_eq!(f.color_config.as_ref().unwrap().bg, None);
    }

    #[test]
    fn malformed_or_unrecognized_xml_is_an_error() {
        for xml in [
            "<AnalyseDoc>",
            "<unrelated/>",
            "<!DOCTYPE x [<!ENTITY x 'boom'>]><AnalyseDoc/>",
        ] {
            assert!(convert_npp_xml(xml).is_err(), "{xml}");
        }
    }

    #[test]
    fn whole_words_escape_literals_and_validate_regex() {
        let f = convert_npp_xml(r#"<SearchText wholeWord="true">a.b</SearchText>"#)
            .unwrap()
            .filters
            .remove(0);
        assert!(f.use_regex);
        let re = Regex::new(&f.pattern).unwrap();
        assert!(re.is_match("a.b"));
        assert!(!re.is_match("aXb"));
        assert!(!re.is_match("za.b"));
        assert!(convert_npp_xml(r#"<SearchText searchType="regex">[</SearchText>"#).is_err());
    }

    #[test]
    fn udl_languages_keep_settings_and_styles_separate() {
        let imported = convert_npp_xml(
            r#"<NotepadPlus>
          <UserLang name="A"><Settings><Global caseIgnored="yes"/></Settings>
            <Keywords name='Keywords1'>"two words" alpha</Keywords>
            <WordsStyle bgColor='112233' name='KEYWORDS1'></WordsStyle>
          </UserLang>
          <UserLang name="B"><Keywords name="Keywords1">beta</Keywords>
            <WordsStyle name="KEYWORDS1" bgColor="445566"/>
          </UserLang>
        </NotepadPlus>"#,
        )
        .unwrap();
        assert_eq!(
            imported
                .filters
                .iter()
                .map(|f| f.pattern.as_str())
                .collect::<Vec<_>>(),
            ["two words", "alpha", "beta"]
        );
        assert!(imported.filters[0].ignore_case);
        assert!(!imported.filters[2].ignore_case);
        assert_eq!(
            imported.filters[0].color_config.as_ref().unwrap().bg,
            Some(Color::Rgb(17, 34, 51))
        );
    }

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
        assert!(!cc.match_only);
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
    fn xml_entities_are_decoded_once() {
        let f = convert_npp_xml("<SearchText>a&amp;b&lt;c&gt;&#65;&#x42;&amp;lt;</SearchText>")
            .unwrap()
            .filters
            .remove(0);
        assert_eq!(f.pattern, "a&b<c>AB&lt;");
    }

    #[test]
    fn escaped_search_and_unsupported_options() {
        let imported = convert_npp_xml(r#"<SearchText searchType="escaped" hide="true" bold="true" italic="true" underlined="true">a\t\x41\d066\o103\b01000100\q\</SearchText>"#).unwrap();
        assert_eq!(imported.filters[0].pattern, "a\tABCD\\q\\");
        assert!(!imported.filters[0].use_regex);
        assert_eq!(imported.warnings.len(), 4);
        assert_eq!(decode_escaped(r"\xGZ\d999\x1"), r"\xGZ\d999\x1");
        assert!(
            convert_npp_xml(r#"<SearchText searchType="rgx_multiline">x</SearchText>"#).is_err()
        );
    }

    #[test]
    fn all_eight_udl_lists_and_empty_entries() {
        let mut xml = String::from("<UserLang>");
        for i in 1..=8 {
            xml.push_str(&format!(r#"<Keywords name="Keywords{i}">word{i}</Keywords><WordsStyle name="KEYWORDS{i}" bgColor="FFFFFF"/>"#));
        }
        xml.push_str("</UserLang>");
        let imported = convert_npp_xml(&xml).unwrap();
        assert_eq!(imported.filters.len(), 8);
        for (i, f) in imported.filters.iter().enumerate() {
            assert_eq!(f.id, i);
            assert_eq!(f.pattern, format!("word{}", i + 1));
        }
        assert!(
            convert_npp_xml("<AnalyseDoc><SearchText/></AnalyseDoc>")
                .unwrap()
                .filters
                .is_empty()
        );
        assert!(udl_tokens("'unclosed").is_err());
    }
}
