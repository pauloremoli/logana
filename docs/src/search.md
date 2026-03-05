# Search

Search operates on visible lines only — it respects active filters and only scans lines that are currently shown.

## Keybindings

| Key | Action |
|---|---|
| `/` | Search forward |
| `?` | Search backward |
| `n` | Jump to next match |
| `N` | Jump to previous match |

## Usage

Press `/` or `?` to open the search bar at the bottom of the screen. Type your query and press `Enter`. logana highlights all matches on visible lines and scrolls to the first match.

- `n` wraps around to the first match after the last line.
- `N` wraps around to the last match before the first line.

## Pattern Syntax

Search uses full regex syntax (via the `regex` crate). Examples:

```
/ERROR                  plain literal
/connection.*refused    regex
/\d{3} \d+              HTTP status + bytes
/^2024-03               lines starting with date prefix
```

## Case Sensitivity

By default, search is case-sensitive. Case sensitivity can be toggled programmatically via the `Search` API (no UI toggle yet — the default behavior is case-sensitive matching).

## Match Highlighting

Matched byte spans are highlighted with the search style (distinct from filter highlight colors). Search highlights take priority over filter highlights — if a search match overlaps a filter-colored span, the search color wins.

## Search vs. Filters

| | Search | Filter |
|---|---|---|
| Persisted | No | Yes |
| Affects visible lines | No | Yes |
| Highlighted | Yes | Yes |
| Navigation (n/N) | Yes | No |
| Regex support | Yes | Yes |

Use **filters** to permanently narrow the view. Use **search** to navigate through specific patterns within the already-filtered view.
