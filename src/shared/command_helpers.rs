pub fn parse_key_value(pattern: &str) -> Result<(&str, &str), String> {
    let eq = pattern
        .find('=')
        .ok_or_else(|| format!("--field pattern must be 'key=value', got: {pattern}"))?;
    let key = &pattern[..eq];
    let value = &pattern[eq + 1..];
    if key.is_empty() {
        return Err("field name must not be empty".to_string());
    }
    if value.is_empty() {
        return Err("field value must not be empty".to_string());
    }
    Ok((key, value))
}

/// Resolves `fg`/`bg` for `--auto`/`--fga`.
///
/// `--auto` generates a random, readable fg/bg pair (see
/// `theme::random_readable_color_pair`). `--fga` sets only `fg`, to a color
/// readable against `theme_bg` (the app's current theme background,
/// typically its `root_bg`) — unlike `--auto`, `bg` is left as given. Both
/// are formatted as `[r,g,b]` strings, the shape `FilterOptions::fg`/`bg`
/// already accept from `--fg`/`--bg`.
///
/// Errors if `--auto`/`--fga` are combined with each other, or with an
/// explicit `--fg` (and, for `--auto`, `--bg` too) — silently overriding one
/// or the other would be more surprising than just asking the user to pick
/// one.
pub fn resolve_colors(
    auto: bool,
    fga: bool,
    fg: Option<String>,
    bg: Option<String>,
    theme_bg: (u8, u8, u8),
) -> Result<(Option<String>, Option<String>), String> {
    if auto && fga {
        return Err("--auto cannot be combined with --fga".to_string());
    }
    if fga {
        if fg.is_some() {
            return Err("--fga cannot be combined with --fg".to_string());
        }
        let (r, g, b) = crate::theme::random_readable_foreground_for(theme_bg);
        return Ok((Some(format!("[{r},{g},{b}]")), bg));
    }
    if !auto {
        return Ok((fg, bg));
    }
    if fg.is_some() || bg.is_some() {
        return Err("--auto cannot be combined with --fg/--bg".to_string());
    }
    let (fg_rgb, bg_rgb) = crate::theme::random_readable_color_pair();
    Ok((
        Some(format!("[{},{},{}]", fg_rgb.0, fg_rgb.1, fg_rgb.2)),
        Some(format!("[{},{},{}]", bg_rgb.0, bg_rgb.1, bg_rgb.2)),
    ))
}

/// Builds the stored pattern for `:filter`/`:exclude`/`:highlight` from
/// repeatable `--field key=value` values and the trailing free-text
/// `pattern`. Every field entry becomes an AND'd condition; a non-empty
/// trailing pattern becomes an additional AND'd free-text condition
/// (matched against the full line, like a plain non-field filter).
pub fn build_field_filter_pattern(field: &[String], pattern: &str) -> Result<String, String> {
    let conditions = field
        .iter()
        .map(|kv| {
            let (k, v) = parse_key_value(kv)?;
            Ok((k.to_string(), v.to_string()))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let text = if pattern.trim().is_empty() {
        None
    } else {
        Some(pattern)
    };
    Ok(crate::filters::encode_field_filter(&conditions, text))
}
