//! A solid 24-bit color and the hex spelling the CLI accepts.
//!
//! Used by `--background` (Phase 3) and `--palette` (Phase 4). Tracing works in
//! RGB only, so alpha has no place here.

use std::fmt;
use std::str::FromStr;

/// A solid color, 8 bits per channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
}

impl Rgb {
    /// Opaque white, the default background.
    pub const WHITE: Self = Self {
        r: 255,
        g: 255,
        b: 255,
    };

    /// Opaque black.
    pub const BLACK: Self = Self { r: 0, g: 0, b: 0 };

    /// A color from its three channels.
    ///
    /// # Examples
    ///
    /// ```
    /// use vectorise::color::Rgb;
    ///
    /// assert_eq!(Rgb::new(255, 255, 255), Rgb::WHITE);
    /// ```
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

impl Default for Rgb {
    fn default() -> Self {
        Self::WHITE
    }
}

/// Why a hex color could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseRgbError {
    /// The text was not 3 or 6 hex digits, with or without a leading `#`.
    #[error("expected 3 or 6 hex digits, optionally prefixed with '#', got {0:?}")]
    Shape(String),
    /// The text had the right length but a non-hex character in it.
    #[error("{0:?} is not a hex color: {1:?} is not a hex digit")]
    Digit(String, char),
}

impl FromStr for Rgb {
    type Err = ParseRgbError;

    /// Parse `#rrggbb`, `rrggbb`, `#rgb`, or `rgb`.
    ///
    /// The short form expands each digit, as in CSS: `#f0a` is `#ff00aa`.
    ///
    /// # Examples
    ///
    /// ```
    /// use vectorise::color::Rgb;
    ///
    /// assert_eq!("#ff0000".parse::<Rgb>().expect("valid"), Rgb::new(255, 0, 0));
    /// assert_eq!("f00".parse::<Rgb>().expect("valid"), Rgb::new(255, 0, 0));
    /// assert!("#gg0000".parse::<Rgb>().is_err());
    /// ```
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let digits = text.strip_prefix('#').unwrap_or(text);
        if let Some(bad) = digits.chars().find(|c| !c.is_ascii_hexdigit()) {
            return Err(ParseRgbError::Digit(text.to_owned(), bad));
        }

        let channels: [u8; 3] = match digits.len() {
            3 => {
                let mut channels = [0u8; 3];
                for (channel, digit) in channels.iter_mut().zip(digits.chars()) {
                    let value = hex_value(digit)
                        .ok_or_else(|| ParseRgbError::Digit(text.to_owned(), digit))?;
                    // CSS short form: each digit is doubled, so `f` is `ff`.
                    *channel = value * 17;
                }
                channels
            }
            6 => {
                let mut channels = [0u8; 3];
                for (index, channel) in channels.iter_mut().enumerate() {
                    let pair = digits
                        .get(index * 2..index * 2 + 2)
                        .ok_or_else(|| ParseRgbError::Shape(text.to_owned()))?;
                    *channel = u8::from_str_radix(pair, 16)
                        .map_err(|_| ParseRgbError::Shape(text.to_owned()))?;
                }
                channels
            }
            _ => return Err(ParseRgbError::Shape(text.to_owned())),
        };

        Ok(Self::new(channels[0], channels[1], channels[2]))
    }
}

impl fmt::Display for Rgb {
    /// The canonical six-digit form, lowercase, with `#`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

/// The numeric value of one hex digit.
const fn hex_value(digit: char) -> Option<u8> {
    match digit {
        '0'..='9' => Some(digit as u8 - b'0'),
        'a'..='f' => Some(digit as u8 - b'a' + 10),
        'A'..='F' => Some(digit as u8 - b'A' + 10),
        _ => None,
    }
}

/// Parse a list of colors separated by commas, whitespace, or newlines.
///
/// One spelling serves both `--palette '#fff,#000'` and a `--palette-file`
/// holding one color per line. Empty entries are skipped, so a trailing comma
/// or a blank line is not an error.
///
/// # Errors
///
/// [`ParseRgbError`] for the first entry that is not a hex color.
///
/// # Examples
///
/// ```
/// use vectorise::color::{Rgb, parse_palette};
///
/// let palette = parse_palette("#fff, 000\n#3366ff").expect("valid");
/// assert_eq!(palette, [Rgb::WHITE, Rgb::BLACK, Rgb::new(0x33, 0x66, 0xff)]);
/// assert!(parse_palette("").expect("valid").is_empty());
/// ```
pub fn parse_palette(text: &str) -> Result<Vec<Rgb>, ParseRgbError> {
    text.split([',', '\n', '\r', '\t', ' '])
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(Rgb::from_str)
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::{ParseRgbError, Rgb};

    #[test]
    fn rgb_parses_long_form_with_and_without_hash() {
        assert_eq!(
            "#3366ff".parse::<Rgb>().expect("valid"),
            Rgb::new(0x33, 0x66, 0xff)
        );
        assert_eq!(
            "3366ff".parse::<Rgb>().expect("valid"),
            Rgb::new(0x33, 0x66, 0xff)
        );
    }

    #[test]
    fn rgb_parses_short_form_by_doubling_digits() {
        assert_eq!(
            "#36f".parse::<Rgb>().expect("valid"),
            Rgb::new(0x33, 0x66, 0xff)
        );
        assert_eq!("fff".parse::<Rgb>().expect("valid"), Rgb::WHITE);
        assert_eq!("000".parse::<Rgb>().expect("valid"), Rgb::BLACK);
    }

    #[test]
    fn rgb_parsing_is_case_insensitive() {
        assert_eq!(
            "#AABBCC".parse::<Rgb>().expect("valid"),
            "#aabbcc".parse::<Rgb>().expect("valid")
        );
    }

    #[test]
    fn rgb_rejects_wrong_length() {
        for bad in ["", "#", "ff", "#ffff", "#fffff", "#fffffff"] {
            assert_eq!(
                bad.parse::<Rgb>(),
                Err(ParseRgbError::Shape(bad.to_owned())),
                "{bad:?} should be rejected for its length"
            );
        }
    }

    #[test]
    fn rgb_rejects_non_hex_digits() {
        assert_eq!(
            "#gg0000".parse::<Rgb>(),
            Err(ParseRgbError::Digit("#gg0000".to_owned(), 'g'))
        );
        assert!("#12 456".parse::<Rgb>().is_err());
    }

    #[test]
    fn rgb_display_round_trips_through_parsing() {
        let color = Rgb::new(1, 2, 3);
        assert_eq!(color.to_string(), "#010203");
        assert_eq!(color.to_string().parse::<Rgb>().expect("valid"), color);
    }

    #[test]
    fn rgb_default_is_white() {
        assert_eq!(Rgb::default(), Rgb::WHITE);
    }
}
