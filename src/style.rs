//! Frontend-independent colors and styles used by themes and filters.
use std::{fmt, str::FromStr};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Color {
    #[default]
    Reset,
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    Gray,
    DarkGray,
    LightRed,
    LightGreen,
    LightYellow,
    LightBlue,
    LightMagenta,
    LightCyan,
    White,
    Rgb(u8, u8, u8),
    Indexed(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseColorError;
impl fmt::Display for ParseColorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid color")
    }
}
impl std::error::Error for ParseColorError {}
impl FromStr for Color {
    type Err = ParseColorError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let name = value
            .to_lowercase()
            .replace([' ', '-', '_'], "")
            .replace("bright", "light")
            .replace("grey", "gray")
            .replace("silver", "gray")
            .replace("lightblack", "darkgray")
            .replace("lightwhite", "white")
            .replace("lightgray", "white");
        match name.as_str() {
            "reset" => Ok(Self::Reset),
            "black" => Ok(Self::Black),
            "red" => Ok(Self::Red),
            "green" => Ok(Self::Green),
            "yellow" => Ok(Self::Yellow),
            "blue" => Ok(Self::Blue),
            "magenta" => Ok(Self::Magenta),
            "cyan" => Ok(Self::Cyan),
            "gray" => Ok(Self::Gray),
            "darkgray" => Ok(Self::DarkGray),
            "lightred" => Ok(Self::LightRed),
            "lightgreen" => Ok(Self::LightGreen),
            "lightyellow" => Ok(Self::LightYellow),
            "lightblue" => Ok(Self::LightBlue),
            "lightmagenta" => Ok(Self::LightMagenta),
            "lightcyan" => Ok(Self::LightCyan),
            "white" => Ok(Self::White),
            _ => {
                if let Ok(index) = value.parse::<u8>() {
                    return Ok(Self::Indexed(index));
                }
                if value.starts_with('#') && value.len() == 7 {
                    let channel = |range| {
                        value
                            .get(range)
                            .and_then(|v| u8::from_str_radix(v, 16).ok())
                            .ok_or(ParseColorError)
                    };
                    return Ok(Self::Rgb(channel(1..3)?, channel(3..5)?, channel(5..7)?));
                }
                Err(ParseColorError)
            }
        }
    }
}
impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reset => f.write_str("Reset"),
            Self::Black => f.write_str("Black"),
            Self::Red => f.write_str("Red"),
            Self::Green => f.write_str("Green"),
            Self::Yellow => f.write_str("Yellow"),
            Self::Blue => f.write_str("Blue"),
            Self::Magenta => f.write_str("Magenta"),
            Self::Cyan => f.write_str("Cyan"),
            Self::Gray => f.write_str("Gray"),
            Self::DarkGray => f.write_str("DarkGray"),
            Self::LightRed => f.write_str("LightRed"),
            Self::LightGreen => f.write_str("LightGreen"),
            Self::LightYellow => f.write_str("LightYellow"),
            Self::LightBlue => f.write_str("LightBlue"),
            Self::LightMagenta => f.write_str("LightMagenta"),
            Self::LightCyan => f.write_str("LightCyan"),
            Self::White => f.write_str("White"),
            Self::Rgb(r, g, b) => write!(f, "#{r:02X}{g:02X}{b:02X}"),
            Self::Indexed(index) => write!(f, "{index}"),
        }
    }
}

bitflags::bitflags! {
    #[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct Modifier: u16 {
        const BOLD = 1;
        const DIM = 2;
        const ITALIC = 4;
        const UNDERLINED = 8;
        const SLOW_BLINK = 16;
        const RAPID_BLINK = 32;
        const REVERSED = 64;
        const HIDDEN = 128;
        const CROSSED_OUT = 256;
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub underline_color: Option<Color>,
    pub add_modifier: Modifier,
    pub sub_modifier: Modifier,
}
impl Style {
    pub fn fg(mut self, color: Color) -> Self {
        self.fg = Some(color);
        self
    }
    pub fn bg(mut self, color: Color) -> Self {
        self.bg = Some(color);
        self
    }
    pub fn add_modifier(mut self, modifier: Modifier) -> Self {
        self.sub_modifier.remove(modifier);
        self.add_modifier.insert(modifier);
        self
    }
    pub fn remove_modifier(mut self, modifier: Modifier) -> Self {
        self.add_modifier.remove(modifier);
        self.sub_modifier.insert(modifier);
        self
    }
    pub fn patch(mut self, other: Self) -> Self {
        self.fg = other.fg.or(self.fg);
        self.bg = other.bg.or(self.bg);
        self.underline_color = other.underline_color.or(self.underline_color);
        self.add_modifier.remove(other.sub_modifier);
        self.add_modifier.insert(other.add_modifier);
        self.sub_modifier.remove(other.add_modifier);
        self.sub_modifier.insert(other.sub_modifier);
        self
    }
}

#[cfg(feature = "tui")]
mod terminal {
    use super::*;
    impl From<Color> for ratatui::style::Color {
        fn from(color: Color) -> Self {
            match color {
                Color::Reset => Self::Reset,
                Color::Black => Self::Black,
                Color::Red => Self::Red,
                Color::Green => Self::Green,
                Color::Yellow => Self::Yellow,
                Color::Blue => Self::Blue,
                Color::Magenta => Self::Magenta,
                Color::Cyan => Self::Cyan,
                Color::Gray => Self::Gray,
                Color::DarkGray => Self::DarkGray,
                Color::LightRed => Self::LightRed,
                Color::LightGreen => Self::LightGreen,
                Color::LightYellow => Self::LightYellow,
                Color::LightBlue => Self::LightBlue,
                Color::LightMagenta => Self::LightMagenta,
                Color::LightCyan => Self::LightCyan,
                Color::White => Self::White,
                Color::Rgb(r, g, b) => Self::Rgb(r, g, b),
                Color::Indexed(index) => Self::Indexed(index),
            }
        }
    }
    impl From<Modifier> for ratatui::style::Modifier {
        fn from(modifier: Modifier) -> Self {
            Self::from_bits_retain(modifier.bits())
        }
    }
    impl From<Style> for ratatui::style::Style {
        fn from(style: Style) -> Self {
            Self {
                fg: style.fg.map(Into::into),
                bg: style.bg.map(Into::into),
                underline_color: style.underline_color.map(Into::into),
                add_modifier: style.add_modifier.into(),
                sub_modifier: style.sub_modifier.into(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_colors_roundtrip_without_a_frontend() {
        let named = [
            Color::Reset,
            Color::Black,
            Color::Red,
            Color::Green,
            Color::Yellow,
            Color::Blue,
            Color::Magenta,
            Color::Cyan,
            Color::Gray,
            Color::DarkGray,
            Color::LightRed,
            Color::LightGreen,
            Color::LightYellow,
            Color::LightBlue,
            Color::LightMagenta,
            Color::LightCyan,
            Color::White,
        ];
        for color in named
            .into_iter()
            .chain((0..=255).map(Color::Indexed))
            .chain([Color::Rgb(0, 255, 128), Color::Rgb(255, 0, 1)])
        {
            assert_eq!(color.to_string().parse::<Color>().unwrap(), color);
        }
        assert_eq!(Color::Rgb(10, 20, 30).to_string(), "#0A141E");
    }

    #[test]
    fn parsing_keeps_existing_aliases_and_rejects_invalid_colors() {
        for name in ["bright red", "Light-Red", "light_red"] {
            assert_eq!(name.parse::<Color>().unwrap(), Color::LightRed);
        }
        assert_eq!("grey".parse::<Color>().unwrap(), Color::Gray);
        assert_eq!("bright black".parse::<Color>().unwrap(), Color::DarkGray);
        for value in ["256", "-1", "#12345", "#GG0000", "#é1234", "invalid"] {
            assert!(value.parse::<Color>().is_err(), "{value}");
        }
    }

    #[test]
    fn patch_preserves_unspecified_colors_and_resolves_modifiers() {
        let base = Style::default()
            .fg(Color::Red)
            .bg(Color::Black)
            .add_modifier(Modifier::BOLD)
            .remove_modifier(Modifier::DIM);
        let overlay = Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::DIM)
            .remove_modifier(Modifier::BOLD);
        let merged = base.patch(overlay);
        assert_eq!(merged.fg, Some(Color::Green));
        assert_eq!(merged.bg, Some(Color::Black));
        assert_eq!(merged.add_modifier, Modifier::DIM);
        assert_eq!(merged.sub_modifier, Modifier::BOLD);
    }

    #[cfg(feature = "tui")]
    #[test]
    fn terminal_conversion_matches_existing_color_and_style_behavior() {
        use ratatui::style::{
            Color as TerminalColor, Modifier as TerminalModifier, Style as TerminalStyle,
        };
        for name in [
            "Reset",
            "Black",
            "Red",
            "Green",
            "Yellow",
            "Blue",
            "Magenta",
            "Cyan",
            "Gray",
            "DarkGray",
            "LightRed",
            "LightGreen",
            "LightYellow",
            "LightBlue",
            "LightMagenta",
            "LightCyan",
            "White",
            "bright-black",
            "#0A141E",
        ] {
            let color: TerminalColor = name.parse::<Color>().unwrap().into();
            assert_eq!(color, name.parse::<TerminalColor>().unwrap(), "{name}");
        }
        for index in 0..=255 {
            assert_eq!(
                TerminalColor::from(Color::Indexed(index)),
                TerminalColor::Indexed(index)
            );
        }
        let original = TerminalStyle::default()
            .fg(TerminalColor::Red)
            .bg(TerminalColor::Rgb(1, 2, 3))
            .add_modifier(TerminalModifier::BOLD | TerminalModifier::UNDERLINED)
            .remove_modifier(TerminalModifier::DIM | TerminalModifier::ITALIC);
        let shared = Style::default()
            .fg(Color::Red)
            .bg(Color::Rgb(1, 2, 3))
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            .remove_modifier(Modifier::DIM | Modifier::ITALIC);
        assert_eq!(TerminalStyle::from(shared), original);
        for (shared, expected) in [
            (Modifier::BOLD, TerminalModifier::BOLD),
            (Modifier::DIM, TerminalModifier::DIM),
            (Modifier::ITALIC, TerminalModifier::ITALIC),
            (Modifier::UNDERLINED, TerminalModifier::UNDERLINED),
            (Modifier::SLOW_BLINK, TerminalModifier::SLOW_BLINK),
            (Modifier::RAPID_BLINK, TerminalModifier::RAPID_BLINK),
            (Modifier::REVERSED, TerminalModifier::REVERSED),
            (Modifier::HIDDEN, TerminalModifier::HIDDEN),
            (Modifier::CROSSED_OUT, TerminalModifier::CROSSED_OUT),
        ] {
            assert_eq!(TerminalModifier::from(shared), expected);
        }
    }
}
