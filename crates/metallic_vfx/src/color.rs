//! Colours: [`Rgb`] (sRGB, like Roblox `Color3`) written as `"#RRGGBB"` in effect files.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// An sRGB colour with components in `0..=1`, like Roblox `Color3`.
///
/// Effect files store it as a hex string (`"#FF8A2B"`) so diffs stay short and readable.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rgb {
    /// Red, `0..=1`.
    pub r: f32,
    /// Green, `0..=1`.
    pub g: f32,
    /// Blue, `0..=1`.
    pub b: f32,
}

impl Rgb {
    /// White.
    pub const WHITE: Self = Self::new(1.0, 1.0, 1.0);
    /// Black.
    pub const BLACK: Self = Self::new(0.0, 0.0, 0.0);

    /// Builds a colour from `0..=1` components.
    #[must_use]
    pub const fn new(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b }
    }

    /// Builds a colour from a `0xRRGGBB` integer.
    #[must_use]
    pub fn hex(v: u32) -> Self {
        Self::from_u8((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }

    /// Builds a colour from 8-bit components.
    #[must_use]
    pub fn from_u8(r: u8, g: u8, b: u8) -> Self {
        Self::new(
            f32::from(r) / 255.0,
            f32::from(g) / 255.0,
            f32::from(b) / 255.0,
        )
    }

    /// 8-bit components, rounded and clamped.
    #[must_use]
    pub fn to_u8(self) -> [u8; 3] {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        [q(self.r), q(self.g), q(self.b)]
    }

    /// `"#RRGGBB"`.
    #[must_use]
    pub fn to_hex_string(self) -> String {
        let [r, g, b] = self.to_u8();
        format!("#{r:02X}{g:02X}{b:02X}")
    }

    /// Parses `"#RRGGBB"` or `"RRGGBB"`.
    ///
    /// # Errors
    /// Returns a message when the text is not six hex digits.
    pub fn parse_hex(s: &str) -> Result<Self, String> {
        let t = s.trim().trim_start_matches('#');
        if t.len() != 6 || !t.is_ascii() {
            return Err(format!("colour `{s}` is not #RRGGBB"));
        }
        u32::from_str_radix(t, 16)
            .map(Self::hex)
            .map_err(|_| format!("colour `{s}` is not #RRGGBB"))
    }

    /// Component-wise linear interpolation in sRGB (what Roblox `ColorSequence` does).
    #[must_use]
    pub fn lerp(self, o: Self, t: f32) -> Self {
        Self::new(
            self.r + (o.r - self.r) * t,
            self.g + (o.g - self.g) * t,
            self.b + (o.b - self.b) * t,
        )
    }

    /// Interpolation in Oklab, which keeps hue and lightness steadier (no muddy midpoints).
    #[must_use]
    pub fn lerp_oklab(self, o: Self, t: f32) -> Self {
        let a = to_oklab(self);
        let b = to_oklab(o);
        from_oklab([
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
        ])
    }

    /// Component-wise product.
    #[must_use]
    pub fn modulate(self, o: Self) -> Self {
        Self::new(self.r * o.r, self.g * o.g, self.b * o.b)
    }

    /// Scales every component.
    #[must_use]
    pub fn scale(self, s: f32) -> Self {
        Self::new(self.r * s, self.g * s, self.b * s)
    }

    /// Relative luminance (sRGB weights, applied to the stored values).
    #[must_use]
    pub fn luma(self) -> f32 {
        0.2126 * self.r + 0.7152 * self.g + 0.0722 * self.b
    }
}

impl Serialize for Rgb {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex_string())
    }
}

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Self::parse_hex(&s).map_err(serde::de::Error::custom)
    }
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f32) -> f32 {
    let c = c.max(0.0);
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// sRGB to Oklab (Björn Ottosson's published matrices).
fn to_oklab(c: Rgb) -> [f32; 3] {
    let (r, g, b) = (
        srgb_to_linear(c.r),
        srgb_to_linear(c.g),
        srgb_to_linear(c.b),
    );
    let l = 0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b;
    let m = 0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b;
    let s = 0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b;
    let (l, m, s) = (l.cbrt(), m.cbrt(), s.cbrt());
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

fn from_oklab(c: [f32; 3]) -> Rgb {
    let l = c[0] + 0.396_337_78 * c[1] + 0.215_803_76 * c[2];
    let m = c[0] - 0.105_561_346 * c[1] - 0.063_854_17 * c[2];
    let s = c[0] - 0.089_484_18 * c[1] - 1.291_485_5 * c[2];
    let (l, m, s) = (l * l * l, m * m * m, s * s * s);
    let r = 4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s;
    let g = -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s;
    let b = -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s;
    Rgb::new(
        linear_to_srgb(r).clamp(0.0, 1.0),
        linear_to_srgb(g).clamp(0.0, 1.0),
        linear_to_srgb(b).clamp(0.0, 1.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        let c = Rgb::hex(0xFF8A2B);
        assert_eq!(c.to_hex_string(), "#FF8A2B");
        assert_eq!(Rgb::parse_hex("#ff8a2b"), Ok(c));
        assert!(Rgb::parse_hex("#ff8a2").is_err());
        assert!(Rgb::parse_hex("#gg8a2b").is_err());
    }

    #[test]
    fn oklab_round_trip_endpoints() {
        let a = Rgb::hex(0x3366CC);
        let b = Rgb::hex(0xFFAA00);
        let s = a.lerp_oklab(b, 0.0);
        let e = a.lerp_oklab(b, 1.0);
        assert_eq!(s.to_u8(), a.to_u8());
        assert_eq!(e.to_u8(), b.to_u8());
    }
}
