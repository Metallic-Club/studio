//! `Color3`: an RGB colour as authored.

use glam::Vec3;
use serde::{Deserialize, Serialize};

/// An RGB colour as the user authored it: sRGB-encoded channels, nominally 0..1, like Roblox's `Color3`.
///
/// Values are not clamped, so a channel above 1 can express HDR intensity. Lighting maths needs linear values:
/// convert with [`to_linear`](Self::to_linear) before blending or shading.
///
/// # Serialized form
///
/// Three numbers, `[r, g, b]`, in sRGB as stored. Scene files rely on this form.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(from = "[f32; 3]", into = "[f32; 3]")]
pub struct Color3 {
    /// Red, sRGB-encoded, nominally 0..1.
    pub r: f32,
    /// Green, sRGB-encoded, nominally 0..1.
    pub g: f32,
    /// Blue, sRGB-encoded, nominally 0..1.
    pub b: f32,
}

impl Color3 {
    /// `(1, 1, 1)`.
    pub const WHITE: Self = Self::new(1.0, 1.0, 1.0);
    /// `(0, 0, 0)`.
    pub const BLACK: Self = Self::new(0.0, 0.0, 0.0);

    /// A colour from sRGB channels, nominally 0..1.
    pub const fn new(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b }
    }

    /// A colour from 0..255 sRGB channels, as colour pickers show them.
    pub fn from_rgb_u8(r: u8, g: u8, b: u8) -> Self {
        Self::new(f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0)
    }

    /// The channels as 0..255, clamped and rounded to the nearest step.
    pub fn to_rgb_u8(self) -> [u8; 3] {
        // `as` saturates and maps NaN to 0, which is what we want after clamping.
        let to_u8 = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
        [to_u8(self.r), to_u8(self.g), to_u8(self.b)]
    }

    /// Parses `RRGGBB` or `RGB` hex digits, with or without a leading `#`, in either case. Returns `None` for
    /// anything else.
    pub fn from_hex(hex: &str) -> Option<Self> {
        let digits = hex.strip_prefix('#').unwrap_or(hex);
        if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        // The digits are ASCII, so byte slicing below cannot split a character.
        let channel = |s: &str| u8::from_str_radix(s, 16).ok();
        match digits.len() {
            6 => Some(Self::from_rgb_u8(
                channel(&digits[0..2])?,
                channel(&digits[2..4])?,
                channel(&digits[4..6])?,
            )),
            // `F80` means `FF8800`: each digit is repeated.
            3 => {
                let short = |s: &str| channel(s).map(|v| v * 17);
                Some(Self::from_rgb_u8(
                    short(&digits[0..1])?,
                    short(&digits[1..2])?,
                    short(&digits[2..3])?,
                ))
            }
            _ => None,
        }
    }

    /// The colour as six lowercase hex digits without `#` (`ff8800`), like Roblox's `Color3:ToHex()`.
    pub fn to_hex(self) -> String {
        let [r, g, b] = self.to_rgb_u8();
        format!("{r:02x}{g:02x}{b:02x}")
    }

    /// Blends towards `other` channel by channel in sRGB space (as Roblox does): `t = 0` gives `self`, `t = 1`
    /// gives `other`.
    pub fn lerp(self, other: Self, t: f32) -> Self {
        Self::new(
            self.r + (other.r - self.r) * t,
            self.g + (other.g - self.g) * t,
            self.b + (other.b - self.b) * t,
        )
    }

    /// Decodes the sRGB channels to linear light with the standard sRGB transfer function, for shading.
    pub fn to_linear(self) -> Vec3 {
        Vec3::new(srgb_to_linear(self.r), srgb_to_linear(self.g), srgb_to_linear(self.b))
    }

    /// Encodes linear light back to an sRGB colour, the inverse of [`to_linear`](Self::to_linear).
    pub fn from_linear(linear: Vec3) -> Self {
        Self::new(
            linear_to_srgb(linear.x),
            linear_to_srgb(linear.y),
            linear_to_srgb(linear.z),
        )
    }
}

impl From<[f32; 3]> for Color3 {
    fn from([r, g, b]: [f32; 3]) -> Self {
        Self::new(r, g, b)
    }
}

impl From<Color3> for [f32; 3] {
    fn from(c: Color3) -> Self {
        [c.r, c.g, c.b]
    }
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(l: f32) -> f32 {
    if l <= 0.003_130_8 {
        l * 12.92
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u8_round_trips() {
        for rgb in [[0, 0, 0], [255, 255, 255], [255, 136, 0], [1, 127, 254]] {
            assert_eq!(Color3::from_rgb_u8(rgb[0], rgb[1], rgb[2]).to_rgb_u8(), rgb);
        }
        assert_eq!(Color3::new(-1.0, 2.0, f32::NAN).to_rgb_u8(), [0, 255, 0]);
    }

    #[test]
    fn hex_round_trips() {
        let c = Color3::from_hex("#FF8800").expect("valid hex");
        assert_eq!(c.to_rgb_u8(), [255, 136, 0]);
        assert_eq!(c.to_hex(), "ff8800");
        assert_eq!(Color3::from_hex(&c.to_hex()), Some(c));
        assert_eq!(Color3::from_hex("f80"), Some(c));
        assert_eq!(Color3::WHITE.to_hex(), "ffffff");
        assert_eq!(Color3::from_hex("000000"), Some(Color3::BLACK));
    }

    #[test]
    fn hex_rejects_garbage() {
        for bad in ["", "#", "ff88", "ff88000", "gg0000", "+f+f+f", "#ff 800", "ÿÿÿ"] {
            assert_eq!(Color3::from_hex(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn linear_conversion_matches_known_values() {
        let lin = Color3::new(0.0, 0.5, 1.0).to_linear();
        assert!(lin.abs_diff_eq(Vec3::new(0.0, 0.214_041, 1.0), 1e-5), "{lin}");
        // Below the knee the curve is linear.
        assert!((Color3::new(0.04, 0.0, 0.0).to_linear().x - 0.04 / 12.92).abs() < 1e-7);
        let srgb = Color3::from_linear(Vec3::new(0.0, 0.5, 1.0));
        assert!((srgb.g - 0.735_357).abs() < 1e-5, "{srgb:?}");
    }

    #[test]
    fn linear_round_trips() {
        let c = Color3::new(0.1, 0.5, 0.9);
        let back = Color3::from_linear(c.to_linear());
        assert!((back.r - c.r).abs() < 1e-5 && (back.g - c.g).abs() < 1e-5 && (back.b - c.b).abs() < 1e-5);
    }

    #[test]
    fn lerp_blends_channels() {
        assert_eq!(Color3::BLACK.lerp(Color3::WHITE, 0.25), Color3::new(0.25, 0.25, 0.25));
        assert_eq!(Color3::BLACK.lerp(Color3::WHITE, 1.0), Color3::WHITE);
    }

    #[test]
    fn serde_round_trips_as_three_numbers() {
        let c = Color3::new(0.25, 0.5, 1.0);
        let json = serde_json::to_string(&c).expect("serialize");
        assert_eq!(json, "[0.25,0.5,1.0]");
        assert_eq!(serde_json::from_str::<Color3>(&json).expect("deserialize"), c);
    }
}
