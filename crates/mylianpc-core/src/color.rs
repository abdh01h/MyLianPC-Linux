//! An 8-bit-per-channel colour, serialized as `#RRGGBB`.
//!
//! Ported from `MyLianPC.Core/Lighting/RgbColor.cs`. Keep the rounding and
//! truncation rules identical: effects and previews depend on them.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl RgbColor {
    pub const BLACK: Self = Self::new(0, 0, 0);
    pub const WHITE: Self = Self::new(255, 255, 255);

    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn to_hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }

    /// Accepts `#RRGGBB`, `RRGGBB`, `#RGB` or `RGB` (case-insensitive).
    pub fn try_parse_hex(text: &str) -> Option<Self> {
        let s = text.trim();
        let s = s.strip_prefix('#').unwrap_or(s);
        if !s.is_ascii() {
            return None;
        }

        let expanded;
        let s = if s.len() == 3 {
            expanded = format!(
                "{0}{0}{1}{1}{2}{2}",
                &s[0..1],
                &s[1..2],
                &s[2..3]
            );
            expanded.as_str()
        } else {
            s
        };

        if s.len() != 6 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let value = u32::from_str_radix(s, 16).ok()?;
        Some(Self::new(
            (value >> 16) as u8,
            (value >> 8) as u8,
            value as u8,
        ))
    }

    pub fn parse_hex(text: &str) -> Result<Self, String> {
        Self::try_parse_hex(text).ok_or_else(|| format!("'{text}' is not a valid hex colour"))
    }

    /// Scales the colour by a brightness percentage (0-100). Truncates like the C# code.
    pub fn with_brightness(self, percent: i32) -> Self {
        let p = percent.clamp(0, 100) as u32;
        Self::new(
            ((self.r as u32 * p) / 100) as u8,
            ((self.g as u32 * p) / 100) as u8,
            ((self.b as u32 * p) / 100) as u8,
        )
    }

    /// Linear interpolation; `t` is clamped to [0, 1].
    pub fn lerp(a: Self, b: Self, t: f64) -> Self {
        let t = t.clamp(0.0, 1.0);
        Self::new(
            (a.r as f64 + (b.r as f64 - a.r as f64) * t).round() as u8,
            (a.g as f64 + (b.g as f64 - a.g as f64) * t).round() as u8,
            (a.b as f64 + (b.b as f64 - a.b as f64) * t).round() as u8,
        )
    }

    /// HSV to RGB with hue in degrees and saturation/value in [0, 1].
    pub fn from_hsv(hue: f64, saturation: f64, value: f64) -> Self {
        let hue = ((hue % 360.0) + 360.0) % 360.0;
        let saturation = saturation.clamp(0.0, 1.0);
        let value = value.clamp(0.0, 1.0);
        let c = value * saturation;
        let x = c * (1.0 - ((hue / 60.0) % 2.0 - 1.0).abs());
        let m = value - c;
        let (r, g, b) = match (hue / 60.0).floor() as i32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        Self::new(
            ((r + m) * 255.0).round() as u8,
            ((g + m) * 255.0).round() as u8,
            ((b + m) * 255.0).round() as u8,
        )
    }
}

impl fmt::Display for RgbColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl Serialize for RgbColor {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for RgbColor {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::try_parse_hex(&text)
            .ok_or_else(|| serde::de::Error::custom(format!("invalid colour '{text}'")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_supported_hex_forms() {
        let expected = RgbColor::new(0x12, 0xAB, 0x7F);
        for text in ["#12ab7f", "12AB7F"] {
            assert_eq!(RgbColor::try_parse_hex(text), Some(expected), "{text}");
        }
        // Short form expands each nibble: #1A7 -> #11AA77.
        let short = RgbColor::new(0x11, 0xAA, 0x77);
        for text in ["#1A7", "1a7"] {
            assert_eq!(RgbColor::try_parse_hex(text), Some(short), "{text}");
        }
    }

    #[test]
    fn rejects_malformed_hex() {
        for text in ["", "#12", "#GGGGGG", "#1234567", "red", "#12 45 6"] {
            assert_eq!(RgbColor::try_parse_hex(text), None, "{text}");
        }
    }

    #[test]
    fn hex_round_trips() {
        let c = RgbColor::new(1, 2, 3);
        assert_eq!(c.to_hex(), "#010203");
        assert_eq!(RgbColor::try_parse_hex(&c.to_hex()), Some(c));
    }

    #[test]
    fn brightness_truncates_and_clamps() {
        let c = RgbColor::new(200, 100, 50);
        assert_eq!(c.with_brightness(50), RgbColor::new(100, 50, 25));
        assert_eq!(c.with_brightness(0), RgbColor::BLACK);
        assert_eq!(c.with_brightness(150), c);
    }

    #[test]
    fn hsv_primaries_are_exact() {
        assert_eq!(RgbColor::from_hsv(0.0, 1.0, 1.0), RgbColor::new(255, 0, 0));
        assert_eq!(RgbColor::from_hsv(120.0, 1.0, 1.0), RgbColor::new(0, 255, 0));
        assert_eq!(RgbColor::from_hsv(240.0, 1.0, 1.0), RgbColor::new(0, 0, 255));
        assert_eq!(RgbColor::from_hsv(360.0, 1.0, 1.0), RgbColor::new(255, 0, 0));
    }

    #[test]
    fn lerp_clamps_t() {
        let a = RgbColor::new(0, 0, 0);
        let b = RgbColor::new(100, 200, 40);
        assert_eq!(RgbColor::lerp(a, b, 0.5), RgbColor::new(50, 100, 20));
        assert_eq!(RgbColor::lerp(a, b, -1.0), a);
        assert_eq!(RgbColor::lerp(a, b, 2.0), b);
    }

    #[test]
    fn serializes_as_hex() {
        let json = serde_json::to_string(&RgbColor::new(10, 11, 12)).unwrap();
        assert_eq!(json, "\"#0A0B0C\"");
        let back: RgbColor = serde_json::from_str(&json).unwrap();
        assert_eq!(back, RgbColor::new(10, 11, 12));
    }
}
