//! Converts a Notepad++ "Analyze" plugin XML config (AnalyseDoc highlight
//! export or User Defined Language export) into logana `Include` filters.
//! Regex-based extraction is used deliberately instead of a full XML parser:
//! these documents' structure is small and known ahead of time, and a real
//! parser would be exposed to XXE/entity-expansion risk on untrusted input.

use crate::filters::{ColorConfig, FilterDef, FilterType};
use ratatui::style::Color;
use regex::Regex;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::LazyLock;

static SEARCH_TEXT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<SearchText([^>]*?)>(.*?)</SearchText>").unwrap());
static KEYWORDS_LIST_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)<Keywords name="Keywords(\d+)">(.*?)</Keywords>"#).unwrap());
static WORDS_STYLE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<WordsStyle name="KEYWORDS(\d+)"([^>]*?)/>"#).unwrap());
static ATTR_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(\w+)="([^"]*)""#).unwrap());

/// Converts a Notepad++ Analyze-plugin XML config into logana `Include`
/// filters, auto-detecting AnalyseDoc vs. User Defined Language exports.
pub fn convert_npp_xml(xml_text: &str) -> Result<Vec<FilterDef>, String> {
    if is_userlang(xml_text) {
        convert_userlang(xml_text)
    } else {
        convert_analysedoc(xml_text)
    }
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

fn convert_analysedoc(xml_text: &str) -> Result<Vec<FilterDef>, String> {
    let mut filters = Vec::new();
    for caps in SEARCH_TEXT_RE.captures_iter(xml_text) {
        let attrs = parse_attrs(&caps[1]);
        let bg = attrs
            .get("bgColor")
            .ok_or("<SearchText> entry missing bgColor attribute")?;
        let bg_color = map_bg_color(bg)?;
        filters.push(FilterDef {
            id: filters.len(),
            pattern: unescape_html(&caps[2]),
            filter_type: FilterType::Include,
            enabled: true,
            color_config: Some(ColorConfig {
                fg: Some(Color::Black),
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

fn convert_userlang(xml_text: &str) -> Result<Vec<FilterDef>, String> {
    let mut filters = Vec::new();

    let mut styles: HashMap<u32, (String, String)> = HashMap::new();
    for caps in WORDS_STYLE_RE.captures_iter(xml_text) {
        let n: u32 = caps[1]
            .parse()
            .map_err(|_| "invalid WordsStyle index".to_string())?;
        let attrs = parse_attrs(&caps[2]);
        if let (Some(fg), Some(bg)) = (attrs.get("fgColor"), attrs.get("bgColor")) {
            styles.insert(n, (fg.clone(), bg.clone()));
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
        let fg_color = Color::from_str(&format!("#{fg}"))
            .map_err(|_| format!("Invalid fgColor '{fg}' for Keywords{n}"))?;
        let bg_color = Color::from_str(&format!("#{bg}"))
            .map_err(|_| format!("Invalid bgColor '{bg}' for Keywords{n}"))?;
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

/// Maps a Notepad++ `bgColor` value (named or `#RRGGBB`) to a `Color` logana
/// accepts. Named colors are mapped through the same table Notepad++'s
/// Analyze plugin uses for its highlight styles.
fn map_bg_color(raw: &str) -> Result<Color, String> {
    let mapped: String = if let Some(hex) = raw.strip_prefix('#') {
        format!("#{hex}")
    } else {
        match raw {
            "yellow" => "yellow",
            "green" => "green",
            "red" => "red",
            "cyan" => "cyan",
            "liteGreen" => "LightGreen",
            "liteRed" => "LightRed",
            "veryLiteGrey" => "#E8E8E8",
            "litePink" => "#FFB6C1",
            "darkYellow" => "#999900",
            "liteBeige" => "#F5DEB3",
            other => return Err(format!("Unknown Notepad++ color name '{other}'")),
        }
        .to_string()
    };
    Color::from_str(&mapped).map_err(|_| format!("Invalid color '{raw}'"))
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
    fn test_analysedoc_maps_named_color_and_group() {
        let filters = convert_npp_xml(ANALYSEDOC_XML).unwrap();
        let error_filter = &filters[0];
        assert_eq!(error_filter.pattern, "error");
        assert_eq!(error_filter.filter_type, FilterType::Include);
        assert_eq!(error_filter.group.as_deref(), Some("errs"));
        assert!(!error_filter.use_regex);
        let cc = error_filter.color_config.as_ref().unwrap();
        assert_eq!(cc.fg, Some(Color::Black));
        assert_eq!(cc.bg, Some(Color::Green));
        assert!(cc.match_only);
    }

    #[test]
    fn test_analysedoc_passes_through_hex_color_and_unescapes_text() {
        let filters = convert_npp_xml(ANALYSEDOC_XML).unwrap();
        let warn_filter = &filters[1];
        assert_eq!(warn_filter.pattern, "warn&ing");
        assert_eq!(warn_filter.group, None);
        let cc = warn_filter.color_config.as_ref().unwrap();
        assert_eq!(cc.bg, Some(Color::Rgb(0x11, 0x22, 0x33)));
    }

    #[test]
    fn test_udl_expands_keywords_into_one_filter_per_token() {
        let filters = convert_npp_xml(UDL_XML).unwrap();
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
        let filters = convert_npp_xml(UDL_XML).unwrap();
        assert!(filters.iter().all(|f| f.pattern != "baz"));
    }

    #[test]
    fn test_unknown_color_name_returns_error_not_panic() {
        let xml = r#"<AnalyseDoc><SearchText bgColor="mysteryColor">x</SearchText></AnalyseDoc>"#;
        let result = convert_npp_xml(xml);
        assert!(result.is_err());
    }

    #[test]
    fn test_map_bg_color_translates_notepad_names() {
        assert_eq!(map_bg_color("liteGreen").unwrap(), Color::LightGreen);
        assert_eq!(map_bg_color("liteRed").unwrap(), Color::LightRed);
        assert_eq!(
            map_bg_color("veryLiteGrey").unwrap(),
            Color::Rgb(0xE8, 0xE8, 0xE8)
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
