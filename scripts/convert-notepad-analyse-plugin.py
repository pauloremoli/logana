#!/usr/bin/env python3
"""Convert a Notepad++ highlight config into a logana filter file.

Usage:
    convert-notepad-analyse-plugin.py <input.xml> <output.json> [--filter-type {highlight,include}]

Supports two Notepad++ export formats, auto-detected from the XML:

- AnalyseDoc: every <SearchText bgColor="..." ...>pattern</SearchText> entry
  becomes a filter for that literal pattern. The XML `group` attribute is
  carried over to logana's filter `group` field, so `:toggle-group <name>`
  can toggle a whole AnalyseDoc group at once. `doSearch="false"` has no
  logana equivalent and is dropped; those entries are still emitted as
  enabled (so their highlighting is preserved).

- User Defined Language (UDL): each non-empty <Keywords name="KeywordsN">
  list (N = 1..8) becomes one filter per whitespace-separated token, colored
  using the matching <WordsStyle name="KEYWORDSN"> fgColor/bgColor, and
  grouped as "KeywordsN" so the whole set can be toggled together. Other
  keyword lists (Numbers, Operators, Delimiters, Folders, Comments) hold
  Notepad++-internal style IDs rather than searchable text and are skipped.

Either way, each entry becomes a literal, match-only filter with the given
background color and "fg": "black" (AnalyseDoc) or the style's own fgColor
(UDL), so highlighted text stays readable.

By default (--filter-type highlight), entries become logana "Highlight"
filters, which color matching lines without affecting visibility — the same
behavior these Notepad++ configs have, so no extra setup is needed.

With --filter-type include, entries become "Include" filters instead. Since
logana's "Include" filter type hides any line that doesn't match at least one
enabled include filter (unlike Notepad++'s highlighting, which never hides
lines), a leading catch-all filter (pattern "^") is emitted first to keep
every line visible while the rest of the filters only add highlighting.
"""

import argparse
import html
import json
import re

COLOR_MAP = {
    "yellow": "yellow",
    "green": "green",
    "red": "red",
    "cyan": "cyan",
    "liteGreen": "LightGreen",
    "liteRed": "LightRed",
    "veryLiteGrey": "#E8E8E8",
    "litePink": "#FFB6C1",
    "darkYellow": "#999900",
    "liteBeige": "#F5DEB3",
}

# Simple, non-recursive regex extraction — deliberately avoiding a full XML
# parser (xml.etree is vulnerable to XXE/entity-expansion attacks on
# untrusted input) since these documents' structure is trivial and known
# ahead of time.
SEARCH_TEXT_RE = re.compile(r"<SearchText([^>]*?)>(.*?)</SearchText>", re.DOTALL)
KEYWORDS_LIST_RE = re.compile(r'<Keywords name="Keywords(\d+)">(.*?)</Keywords>', re.DOTALL)
WORDS_STYLE_RE = re.compile(r'<WordsStyle name="KEYWORDS(\d+)"([^>]*?)/>')
ATTR_RE = re.compile(r'(\w+)="([^"]*)"')


def map_color(bg: str) -> str:
    if bg.startswith("#"):
        return bg
    return COLOR_MAP[bg]


def is_userlang(xml_text: str) -> bool:
    return "<UserLang" in xml_text or "<KeywordLists" in xml_text


def catch_all_filter() -> dict:
    # Notepad++'s highlighting never hides lines, but logana's "Include"
    # filter type hides any line that doesn't match at least one enabled
    # include filter, so this catch-all keeps every line visible while the
    # real entries only add highlighting. Not needed for "Highlight"
    # filters, which never affect visibility.
    return {
        "id": 0,
        "pattern": "^",
        "filter_type": "Include",
        "enabled": True,
        "color_config": None,
        "use_regex": True,
        "group": None,
    }


def convert_analysedoc(xml_text: str, filter_type: str) -> list[dict]:
    filters = [catch_all_filter()] if filter_type == "Include" else []

    for i, m in enumerate(SEARCH_TEXT_RE.finditer(xml_text), start=len(filters)):
        attrs_str, text = m.groups()
        attrs = dict(ATTR_RE.findall(attrs_str))
        pattern = html.unescape(text)
        bg = attrs["bgColor"]
        filters.append(
            {
                "id": i,
                "pattern": pattern,
                "filter_type": filter_type,
                "enabled": True,
                "color_config": {
                    "fg": "black",
                    "bg": map_color(bg),
                    "match_only": True,
                },
                "use_regex": False,
                "group": attrs.get("group"),
            }
        )

    return filters


def convert_userlang(xml_text: str, filter_type: str) -> list[dict]:
    filters = [catch_all_filter()] if filter_type == "Include" else []

    styles = {}
    for n_str, attrs_str in WORDS_STYLE_RE.findall(xml_text):
        attrs = dict(ATTR_RE.findall(attrs_str))
        if "fgColor" in attrs and "bgColor" in attrs:
            styles[int(n_str)] = (attrs["fgColor"], attrs["bgColor"])

    keyword_lists = sorted(
        ((int(n_str), html.unescape(text)) for n_str, text in KEYWORDS_LIST_RE.findall(xml_text)),
        key=lambda pair: pair[0],
    )

    next_id = len(filters)
    for n, text in keyword_lists:
        if n not in styles:
            continue
        fg, bg = styles[n]
        for token in text.split():
            filters.append(
                {
                    "id": next_id,
                    "pattern": token,
                    "filter_type": filter_type,
                    "enabled": True,
                    "color_config": {
                        "fg": f"#{fg}",
                        "bg": f"#{bg}",
                        "match_only": True,
                    },
                    "use_regex": False,
                    "group": f"Keywords{n}",
                }
            )
            next_id += 1

    return filters


def convert(xml_text: str, filter_type: str = "Highlight") -> list[dict]:
    if is_userlang(xml_text):
        return convert_userlang(xml_text, filter_type)
    return convert_analysedoc(xml_text, filter_type)


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Convert a Notepad++ highlight config (AnalyseDoc or "
        "User Defined Language) into a logana filter file."
    )
    parser.add_argument("input_xml", help="Path to the Notepad++ XML config")
    parser.add_argument("output_json", help="Path to write the logana filter file")
    parser.add_argument(
        "--filter-type",
        choices=["highlight", "include"],
        default="highlight",
        help="logana filter type to emit for each entry (default: highlight)",
    )
    args = parser.parse_args()

    with open(args.input_xml, "r", encoding="utf-8") as f:
        xml_text = f.read()

    filters = convert(xml_text, filter_type=args.filter_type.capitalize())

    with open(args.output_json, "w") as f:
        json.dump(filters, f, indent=2)
        f.write("\n")

    print(f"wrote {len(filters)} filters to {args.output_json}")


if __name__ == "__main__":
    main()
