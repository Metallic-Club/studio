//! Colour maths for the popover: sRGB bytes, HSV, hex and CSS names, OKLab/OKLCh, harmonies.
//!
//! Formulas: HSV is the usual hexcone model. OKLab follows Björn Ottosson's published matrices
//! (public domain reference). Nothing here touches egui, so all of it is unit tested.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::css_names::CSS_COLOURS;

/// An sRGB colour with 8 bits per channel: what a Part's `Color` holds after rounding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Rgb8 {
    /// Red, 0 to 255.
    pub r: u8,
    /// Green, 0 to 255.
    pub g: u8,
    /// Blue, 0 to 255.
    pub b: u8,
}

impl Rgb8 {
    /// Creates a colour from its channels.
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Creates a colour from `0xRRGGBB`.
    pub const fn from_u32(rgb: u32) -> Self {
        Self::new((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
    }

    /// The channels as 0..=1 floats.
    pub fn to_unit(self) -> [f32; 3] {
        [self.r, self.g, self.b].map(|c| f32::from(c) / 255.0)
    }

    /// Rounds 0..=1 floats (clamped) to bytes.
    pub fn from_unit(c: [f32; 3]) -> Self {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        Self::new(q(c[0]), q(c[1]), q(c[2]))
    }

    /// `#RRGGBB`, upper case.
    pub fn to_hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }
}

impl fmt::Display for Rgb8 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

// Stored as "#RRGGBB" so palette files stay readable and diff well.
impl Serialize for Rgb8 {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Rgb8 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        parse_colour(&text).map_err(serde::de::Error::custom)
    }
}

/// Hue, saturation, value. Hue is kept even when saturation is zero, so dragging the square to
/// grey and back does not lose the hue.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Hsv {
    /// Hue in degrees, 0 to below 360.
    pub h: f32,
    /// Saturation, 0 to 1.
    pub s: f32,
    /// Value, 0 to 1.
    pub v: f32,
}

impl Hsv {
    /// Creates an HSV colour, wrapping hue into 0..360 and clamping s and v.
    pub fn new(h: f32, s: f32, v: f32) -> Self {
        Self {
            h: wrap_degrees(h),
            s: s.clamp(0.0, 1.0),
            v: v.clamp(0.0, 1.0),
        }
    }

    /// Converts to 0..=1 RGB.
    pub fn to_rgb(self) -> [f32; 3] {
        hsv_to_rgb(self)
    }

    /// Converts and rounds to bytes.
    pub fn to_rgb8(self) -> Rgb8 {
        Rgb8::from_unit(hsv_to_rgb(self))
    }

    /// Converts a byte colour; `keep_hue` is used when the colour is grey (hue undefined).
    pub fn from_rgb8(c: Rgb8, keep_hue: f32) -> Self {
        let mut hsv = rgb_to_hsv(c.to_unit());
        if hsv.s == 0.0 || hsv.v == 0.0 {
            hsv.h = wrap_degrees(keep_hue);
        }
        hsv
    }
}

/// Wraps an angle in degrees into 0..360.
pub fn wrap_degrees(h: f32) -> f32 {
    let w = h.rem_euclid(360.0);
    if w >= 360.0 { 0.0 } else { w }
}

/// HSV to 0..=1 RGB.
pub fn hsv_to_rgb(c: Hsv) -> [f32; 3] {
    let h = wrap_degrees(c.h) / 60.0;
    let (s, v) = (c.s.clamp(0.0, 1.0), c.v.clamp(0.0, 1.0));
    let sector = h.floor();
    let f = h - sector;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    match sector as i32 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

/// 0..=1 RGB to HSV. Grey gives hue 0.
pub fn rgb_to_hsv(c: [f32; 3]) -> Hsv {
    let [r, g, b] = c.map(|v| v.clamp(0.0, 1.0));
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d <= f32::EPSILON {
        0.0
    } else if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let s = if max <= f32::EPSILON { 0.0 } else { d / max };
    Hsv {
        h: wrap_degrees(h),
        s,
        v: max,
    }
}

/// Keeps each colour's saturation and value and turns its hue by `delta` degrees (Alt-drag on
/// the ring with several parts selected).
pub fn shift_hue(c: Rgb8, delta: f32) -> Rgb8 {
    let hsv = rgb_to_hsv(c.to_unit());
    Hsv::new(hsv.h + delta, hsv.s, hsv.v).to_rgb8()
}

/// Why a typed colour was not understood.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseColourError(pub String);

impl fmt::Display for ParseColourError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not a colour: \"{}\"", self.0)
    }
}

impl std::error::Error for ParseColourError {}

/// Reads `#RGB`, `#RRGGBB`, `RRGGBB` or a CSS colour name ("teal", "steel blue"; case and spaces
/// ignored).
pub fn parse_colour(text: &str) -> Result<Rgb8, ParseColourError> {
    let t = text.trim();
    let err = || ParseColourError(t.to_owned());
    if let Some(hex) = t.strip_prefix('#') {
        return parse_hex_digits(hex, true).ok_or_else(err);
    }
    if let Some(c) = parse_hex_digits(t, false) {
        return Ok(c);
    }
    css_colour(t).ok_or_else(err)
}

fn parse_hex_digits(hex: &str, allow_short: bool) -> Option<Rgb8> {
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let digit = |i: usize| u8::from_str_radix(&hex[i..=i], 16).ok();
    match hex.len() {
        6 => {
            let v = u32::from_str_radix(hex, 16).ok()?;
            Some(Rgb8::from_u32(v))
        }
        3 if allow_short => {
            let (r, g, b) = (digit(0)?, digit(1)?, digit(2)?);
            Some(Rgb8::new(r * 17, g * 17, b * 17))
        }
        _ => None,
    }
}

/// The colour of a CSS name, ignoring case and spaces.
pub fn css_colour(name: &str) -> Option<Rgb8> {
    let key: String = name
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    CSS_COLOURS
        .iter()
        .find(|(n, _)| n.chars().filter(|c| *c != ' ').eq(key.chars()))
        .map(|&(_, rgb)| Rgb8::from_u32(rgb))
}

/// The CSS colour name (with spaces, "steel blue") nearest to `c` by OKLab distance.
pub fn nearest_css_name(c: Rgb8) -> &'static str {
    let target = Oklab::from_rgb8(c);
    CSS_COLOURS
        .iter()
        .map(|&(name, rgb)| {
            (
                name,
                target.distance2(Oklab::from_rgb8(Rgb8::from_u32(rgb))),
            )
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(name, _)| name)
        .unwrap_or("black")
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// A colour in OKLab (L 0 to 1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Oklab {
    /// Perceived lightness, 0 to 1.
    pub l: f32,
    /// Green-red axis.
    pub a: f32,
    /// Blue-yellow axis.
    pub b: f32,
}

impl Oklab {
    /// From 8-bit sRGB.
    pub fn from_rgb8(c: Rgb8) -> Self {
        Self::from_linear(c.to_unit().map(srgb_to_linear))
    }

    /// From linear-light sRGB.
    pub fn from_linear([r, g, b]: [f32; 3]) -> Self {
        let l = 0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b;
        let m = 0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b;
        let s = 0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b;
        let (l, m, s) = (l.cbrt(), m.cbrt(), s.cbrt());
        Self {
            l: 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
            a: 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
            b: 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
        }
    }

    /// To linear-light sRGB (may fall outside 0..=1 when out of gamut).
    pub fn to_linear(self) -> [f32; 3] {
        let l = self.l + 0.396_337_78 * self.a + 0.215_803_76 * self.b;
        let m = self.l - 0.105_561_346 * self.a - 0.063_854_17 * self.b;
        let s = self.l - 0.089_484_18 * self.a - 1.291_485_5 * self.b;
        let (l, m, s) = (l * l * l, m * m * m, s * s * s);
        [
            4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
            -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
            -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
        ]
    }

    fn distance2(self, o: Self) -> f32 {
        (self.l - o.l).powi(2) + (self.a - o.a).powi(2) + (self.b - o.b).powi(2)
    }
}

/// OKLab in polar form: lightness, chroma, hue in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Oklch {
    /// Lightness, 0 to 1.
    pub l: f32,
    /// Chroma, 0 and up (sRGB stays under about 0.33).
    pub c: f32,
    /// Hue in degrees.
    pub h: f32,
}

impl Oklch {
    /// From 8-bit sRGB.
    pub fn from_rgb8(c: Rgb8) -> Self {
        let lab = Oklab::from_rgb8(c);
        Self {
            l: lab.l,
            c: lab.a.hypot(lab.b),
            h: wrap_degrees(lab.b.atan2(lab.a).to_degrees()),
        }
    }

    /// To OKLab.
    pub fn to_oklab(self) -> Oklab {
        let (sin, cos) = self.h.to_radians().sin_cos();
        Oklab {
            l: self.l,
            a: self.c * cos,
            b: self.c * sin,
        }
    }

    /// True when the colour fits sRGB (within a rounding tolerance).
    pub fn in_srgb_gamut(self) -> bool {
        const EPS: f32 = 1e-4;
        self.to_oklab()
            .to_linear()
            .iter()
            .all(|&v| (-EPS..=1.0 + EPS).contains(&v))
    }

    /// Clamps lightness to 0..=1 and lowers chroma until the colour fits sRGB, keeping hue and
    /// lightness (the CSS Color 4 approach, without the deltaE shortcut).
    pub fn fit_srgb(self) -> Self {
        let mut out = Self {
            l: self.l.clamp(0.0, 1.0),
            c: self.c.max(0.0),
            h: wrap_degrees(self.h),
        };
        if out.in_srgb_gamut() {
            return out;
        }
        let (mut lo, mut hi) = (0.0f32, out.c);
        for _ in 0..24 {
            let mid = 0.5 * (lo + hi);
            if (Self { c: mid, ..out }).in_srgb_gamut() {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        out.c = lo;
        out
    }

    /// Fits into sRGB and rounds to bytes.
    pub fn to_rgb8(self) -> Rgb8 {
        let lin = self.fit_srgb().to_oklab().to_linear();
        Rgb8::from_unit(lin.map(|v| linear_to_srgb(v.clamp(0.0, 1.0))))
    }
}

/// The harmony rules offered in the popover.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Harmony {
    /// -30, -15, 0, +15, +30 degrees.
    #[default]
    Analogous,
    /// Darker base, base, complement, lighter complement.
    Complementary,
    /// 0, 150, 210 degrees.
    SplitComplementary,
    /// 0, 120, 240 degrees.
    Triadic,
    /// OKLab L at 0.25, 0.4, 0.55, 0.7, 0.85.
    TintsAndShades,
}

impl Harmony {
    /// Every rule, in menu order.
    pub const ALL: [Harmony; 5] = [
        Harmony::Analogous,
        Harmony::Complementary,
        Harmony::SplitComplementary,
        Harmony::Triadic,
        Harmony::TintsAndShades,
    ];

    /// Menu label.
    pub fn label(self) -> &'static str {
        match self {
            Harmony::Analogous => "Analogous",
            Harmony::Complementary => "Complementary",
            Harmony::SplitComplementary => "Split compl.",
            Harmony::Triadic => "Triadic",
            Harmony::TintsAndShades => "Tints & shades",
        }
    }

    /// The colours this rule makes from `base`, in OKLCh (keep lightness and chroma, rotate hue),
    /// each already fitted to sRGB.
    pub fn generate_oklch(self, base: Rgb8) -> Vec<Oklch> {
        /// Lightness step for the lighter and darker complementary chips.
        const STEP: f32 = 0.15;
        let o = Oklch::from_rgb8(base);
        let at = |dh: f32, dl: f32| Oklch {
            l: o.l + dl,
            c: o.c,
            h: o.h + dh,
        };
        let raw: Vec<Oklch> = match self {
            Harmony::Analogous => [-30.0, -15.0, 0.0, 15.0, 30.0].map(|d| at(d, 0.0)).to_vec(),
            Harmony::Complementary => vec![
                at(0.0, -STEP),
                at(0.0, 0.0),
                at(180.0, 0.0),
                at(180.0, STEP),
            ],
            Harmony::SplitComplementary => [0.0, 150.0, 210.0].map(|d| at(d, 0.0)).to_vec(),
            Harmony::Triadic => [0.0, 120.0, 240.0].map(|d| at(d, 0.0)).to_vec(),
            Harmony::TintsAndShades => [0.25, 0.4, 0.55, 0.7, 0.85]
                .map(|l| Oklch { l, ..o })
                .to_vec(),
        };
        raw.into_iter().map(Oklch::fit_srgb).collect()
    }

    /// The colours as bytes.
    pub fn generate(self, base: Rgb8) -> Vec<Rgb8> {
        self.generate_oklch(base)
            .into_iter()
            .map(Oklch::to_rgb8)
            .collect()
    }
}

/// The undo step name: "Colour: #5280CC (3 parts)".
pub fn undo_name(c: Rgb8, parts: usize) -> String {
    let noun = if parts == 1 { "part" } else { "parts" };
    format!("Colour: {} ({parts} {noun})", c.to_hex())
}

/// Evaluates a number typed into a field: `+ - * /`, parentheses and unary minus ("128+16",
/// "255/2"). Returns `None` for anything else.
pub fn eval_number(text: &str) -> Option<f64> {
    struct P<'a> {
        s: &'a [u8],
        i: usize,
    }
    impl P<'_> {
        fn ws(&mut self) {
            while self.s.get(self.i).is_some_and(u8::is_ascii_whitespace) {
                self.i += 1;
            }
        }
        fn eat(&mut self, c: u8) -> bool {
            self.ws();
            if self.s.get(self.i) == Some(&c) {
                self.i += 1;
                true
            } else {
                false
            }
        }
        fn expr(&mut self, depth: u32) -> Option<f64> {
            let mut v = self.term(depth)?;
            loop {
                if self.eat(b'+') {
                    v += self.term(depth)?;
                } else if self.eat(b'-') {
                    v -= self.term(depth)?;
                } else {
                    return Some(v);
                }
            }
        }
        fn term(&mut self, depth: u32) -> Option<f64> {
            let mut v = self.unary(depth)?;
            loop {
                if self.eat(b'*') {
                    v *= self.unary(depth)?;
                } else if self.eat(b'/') {
                    let d = self.unary(depth)?;
                    if d == 0.0 {
                        return None;
                    }
                    v /= d;
                } else {
                    return Some(v);
                }
            }
        }
        fn unary(&mut self, depth: u32) -> Option<f64> {
            if depth > 32 {
                return None;
            }
            if self.eat(b'-') {
                return self.unary(depth + 1).map(|v| -v);
            }
            if self.eat(b'(') {
                let v = self.expr(depth + 1)?;
                return self.eat(b')').then_some(v);
            }
            self.ws();
            let start = self.i;
            while self
                .s
                .get(self.i)
                .is_some_and(|c| c.is_ascii_digit() || *c == b'.')
            {
                self.i += 1;
            }
            std::str::from_utf8(&self.s[start..self.i])
                .ok()?
                .parse()
                .ok()
        }
    }
    let mut p = P {
        s: text.as_bytes(),
        i: 0,
    };
    let v = p.expr(0)?;
    p.ws();
    (p.i == p.s.len() && v.is_finite()).then_some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Small deterministic generator so the tests need no extra crate.
    struct XorShift(u64);
    impl XorShift {
        fn next(&mut self) -> u32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 32) as u32
        }
        fn rgb(&mut self) -> Rgb8 {
            Rgb8::from_u32(self.next() & 0xFF_FFFF)
        }
    }

    #[test]
    fn every_byte_colour_survives_rgb_hsv_rgb() {
        // A 6-bit grid per channel plus the extremes: 274,625 colours.
        let steps: Vec<u8> = (0..=255).step_by(4).chain([255]).collect();
        for &r in &steps {
            for &g in &steps {
                for &b in &steps {
                    let c = Rgb8::new(r, g, b);
                    assert_eq!(Hsv::from_rgb8(c, 0.0).to_rgb8(), c);
                }
            }
        }
    }

    #[test]
    fn hsv_rgb_hsv_round_trip_keeps_hue_saturation_value() {
        let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
        for _ in 0..10_000 {
            let h = (rng.next() % 3600) as f32 / 10.0;
            let s = (rng.next() % 1001) as f32 / 1000.0;
            let v = (rng.next() % 1001) as f32 / 1000.0;
            let back = rgb_to_hsv(hsv_to_rgb(Hsv::new(h, s, v)));
            assert!((back.v - v).abs() < 1e-5, "v {v} -> {}", back.v);
            if v > 1e-3 {
                assert!((back.s - s).abs() < 1e-4, "s {s} -> {}", back.s);
            }
            if v > 1e-3 && s > 1e-3 {
                let dh = (back.h - h).abs();
                assert!(dh.min(360.0 - dh) < 0.05, "h {h} -> {}", back.h);
            }
        }
    }

    #[test]
    fn the_spec_example_matches_its_fields() {
        let c = parse_colour("#5280CC").unwrap();
        assert_eq!(c, Rgb8::new(82, 128, 204));
        let hsv = Hsv::from_rgb8(c, 0.0);
        assert_eq!((hsv.s * 100.0).round(), 60.0);
        assert_eq!((hsv.v * 100.0).round(), 80.0);
        assert!((hsv.h - 217.38).abs() < 0.01, "hue {}", hsv.h);
    }

    #[test]
    fn primaries_have_the_expected_hues() {
        for (hex, hue) in [
            ("#FF0000", 0.0),
            ("#FFFF00", 60.0),
            ("#00FF00", 120.0),
            ("#00FFFF", 180.0),
            ("#0000FF", 240.0),
            ("#FF00FF", 300.0),
        ] {
            let hsv = rgb_to_hsv(parse_colour(hex).unwrap().to_unit());
            assert!((hsv.h - hue).abs() < 1e-3, "{hex}: {}", hsv.h);
            assert_eq!(Hsv::new(hue, 1.0, 1.0).to_rgb8().to_hex(), hex);
        }
    }

    #[test]
    fn grey_keeps_the_hue_it_is_given() {
        let hsv = Hsv::from_rgb8(Rgb8::new(90, 90, 90), 218.0);
        assert_eq!(hsv.h, 218.0);
        assert_eq!(hsv.s, 0.0);
    }

    #[test]
    fn hue_wraps() {
        assert_eq!(wrap_degrees(360.0), 0.0);
        assert_eq!(wrap_degrees(-30.0), 330.0);
        assert_eq!(wrap_degrees(725.0), 5.0);
        assert_eq!(Hsv::new(-1.0, 2.0, -1.0), Hsv::new(359.0, 1.0, 0.0));
    }

    #[test]
    fn hex_forms_are_accepted() {
        let want = Rgb8::new(0x52, 0x80, 0xCC);
        for text in ["#5280CC", "#5280cc", "5280CC", "  #5280CC  "] {
            assert_eq!(parse_colour(text), Ok(want), "{text}");
        }
        assert_eq!(parse_colour("#f0a"), Ok(Rgb8::new(0xFF, 0x00, 0xAA)));
        assert_eq!(want.to_hex(), "#5280CC");
    }

    #[test]
    fn hex_rejects_bad_input() {
        for text in [
            "",
            "#",
            "#12",
            "#1234",
            "#12345G",
            "12345",
            "1234567",
            "#+12345",
            "f0a",
            "notacolour",
        ] {
            assert!(parse_colour(text).is_err(), "{text:?} should fail");
        }
    }

    #[test]
    fn css_names_are_accepted() {
        assert_eq!(parse_colour("teal"), Ok(Rgb8::new(0, 128, 128)));
        assert_eq!(parse_colour("SteelBlue"), Ok(Rgb8::new(70, 130, 180)));
        assert_eq!(parse_colour("steel blue"), Ok(Rgb8::new(70, 130, 180)));
        assert_eq!(
            parse_colour("rebeccapurple"),
            Ok(Rgb8::new(0x66, 0x33, 0x99))
        );
        // "bad" and "fab" are valid hex only with '#'; "add" is neither hex without '#' nor a name.
        assert!(parse_colour("add").is_err());
        assert_eq!(parse_colour("#add"), Ok(Rgb8::new(0xAA, 0xDD, 0xDD)));
    }

    #[test]
    fn css_table_has_148_unique_names() {
        assert_eq!(CSS_COLOURS.len(), 148);
        let mut keys: Vec<String> = CSS_COLOURS
            .iter()
            .map(|(n, _)| n.replace(' ', ""))
            .collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), 148);
    }

    #[test]
    fn nearest_name_finds_exact_and_close_colours() {
        assert_eq!(nearest_css_name(Rgb8::new(70, 130, 180)), "steel blue");
        assert_eq!(nearest_css_name(Rgb8::new(254, 1, 0)), "red");
        assert_eq!(nearest_css_name(Rgb8::new(3, 3, 3)), "black");
    }

    #[test]
    fn oklab_round_trips_and_has_known_white() {
        let white = Oklab::from_rgb8(Rgb8::new(255, 255, 255));
        assert!((white.l - 1.0).abs() < 1e-3 && white.a.abs() < 1e-3 && white.b.abs() < 1e-3);
        let mut rng = XorShift(7);
        for _ in 0..2_000 {
            let c = rng.rgb();
            assert_eq!(Oklch::from_rgb8(c).to_rgb8(), c);
        }
    }

    #[test]
    fn harmony_output_stays_in_srgb_gamut_for_1000_random_inputs() {
        let mut rng = XorShift(0xC0FFEE);
        for _ in 0..1_000 {
            let base = rng.rgb();
            let base_lch = Oklch::from_rgb8(base);
            for rule in Harmony::ALL {
                let out = rule.generate_oklch(base);
                assert!(!out.is_empty());
                for c in &out {
                    assert!(c.in_srgb_gamut(), "{rule:?} from {base}: {c:?}");
                    assert!((0.0..=1.0).contains(&c.l));
                    assert!(c.c <= base_lch.c + 1e-6, "chroma only ever drops");
                }
                if matches!(
                    rule,
                    Harmony::Analogous | Harmony::SplitComplementary | Harmony::Triadic
                ) {
                    for c in &out {
                        assert!((c.l - base_lch.l).abs() < 1e-5, "lightness is kept");
                    }
                }
                // The byte versions decode back to nearly the same OKLCh lightness (8-bit
                // rounding moves L by up to about 0.03 near black, where L changes fastest).
                for (c, rgb) in out.iter().zip(rule.generate(base)) {
                    assert!((Oklch::from_rgb8(rgb).l - c.l).abs() < 0.04);
                }
            }
        }
    }

    #[test]
    fn harmony_offsets_follow_the_spec() {
        let base = Rgb8::new(82, 128, 204);
        let h0 = Oklch::from_rgb8(base).h;
        let hue_of = |rule: Harmony| -> Vec<f32> {
            rule.generate_oklch(base)
                .iter()
                .map(|c| wrap_degrees(c.h - h0).round())
                .collect()
        };
        assert_eq!(hue_of(Harmony::Analogous), [330.0, 345.0, 0.0, 15.0, 30.0]);
        assert_eq!(hue_of(Harmony::SplitComplementary), [0.0, 150.0, 210.0]);
        assert_eq!(hue_of(Harmony::Triadic), [0.0, 120.0, 240.0]);
        assert_eq!(hue_of(Harmony::Complementary), [0.0, 0.0, 180.0, 180.0]);
        let ls: Vec<f32> = Harmony::TintsAndShades
            .generate_oklch(base)
            .iter()
            .map(|c| (c.l * 100.0).round())
            .collect();
        assert_eq!(ls, [25.0, 40.0, 55.0, 70.0, 85.0]);
        // The middle analogous chip is the base colour itself.
        assert_eq!(Harmony::Analogous.generate(base)[2], base);
    }

    #[test]
    fn out_of_gamut_colours_lose_chroma_not_hue() {
        let wild = Oklch {
            l: 0.6,
            c: 0.9,
            h: 140.0,
        };
        assert!(!wild.in_srgb_gamut());
        let fit = wild.fit_srgb();
        assert!(fit.in_srgb_gamut());
        assert_eq!(fit.h, 140.0);
        assert_eq!(fit.l, 0.6);
        assert!(fit.c > 0.05 && fit.c < 0.9);
    }

    #[test]
    fn alt_drag_hue_shift_keeps_shade() {
        let c = Rgb8::new(200, 40, 40);
        let shifted = shift_hue(c, 120.0);
        let (a, b) = (rgb_to_hsv(c.to_unit()), rgb_to_hsv(shifted.to_unit()));
        assert!((a.s - b.s).abs() < 0.01 && (a.v - b.v).abs() < 0.01);
        assert!((b.h - 120.0).abs() < 1.0);
    }

    #[test]
    fn undo_names_follow_the_spec() {
        assert_eq!(
            undo_name(Rgb8::new(82, 128, 204), 3),
            "Colour: #5280CC (3 parts)"
        );
        assert_eq!(undo_name(Rgb8::new(0, 0, 0), 1), "Colour: #000000 (1 part)");
    }

    #[test]
    fn number_fields_take_expressions() {
        assert_eq!(eval_number("128"), Some(128.0));
        assert_eq!(eval_number(" 128 + 16 "), Some(144.0));
        assert_eq!(eval_number("255/2"), Some(127.5));
        assert_eq!(eval_number("-(2*3)+10"), Some(4.0));
        assert_eq!(eval_number("1/0"), None);
        assert_eq!(eval_number("12a"), None);
        assert_eq!(eval_number(""), None);
        assert_eq!(eval_number(&"(".repeat(100)), None);
    }

    #[test]
    fn rgb8_serialises_as_hex_text() {
        let text = ron::to_string(&Rgb8::new(82, 128, 204)).unwrap();
        assert_eq!(text, "\"#5280CC\"");
        let back: Rgb8 = ron::from_str("\"teal\"").unwrap();
        assert_eq!(back, Rgb8::new(0, 128, 128));
    }
}
