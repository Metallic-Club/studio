//! 3D colour lookup tables for the finish pass's grade (#65).
//!
//! A LUT maps a display-encoded colour (sRGB, after the tone curve and `Brightness`) to another
//! display-encoded colour, so LUTs authored on display images in any grading tool apply as
//! they were made. The finish pass samples it with trilinear filtering and mixes the result in
//! by a strength (0 = off, the default). See [`crate::Renderer::set_lut`].

/// A cube of `size`^3 colours, red changing fastest, then green, then blue (the `.cube` order).
#[derive(Debug, Clone, PartialEq)]
pub struct Lut3d {
    size: u32,
    data: Vec<[f32; 3]>,
}

/// Largest edge the renderer accepts (65^3 is the largest common `.cube` size).
pub const MAX_LUT_SIZE: u32 = 65;

/// Why a `.cube` file could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LutError {
    /// No `LUT_3D_SIZE` line, or a size outside 2..=[`MAX_LUT_SIZE`].
    Size,
    /// A line that is neither a keyword nor three numbers (1-based line number).
    Line(usize),
    /// An entry that is NaN or infinite (1-based line number).
    NotFinite(usize),
    /// Fewer or more entries than `size`^3.
    Count {
        /// Entries the size asks for.
        expected: usize,
        /// Entries found.
        found: usize,
    },
}

impl std::fmt::Display for LutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LutError::Size => write!(f, "missing or unsupported LUT_3D_SIZE"),
            LutError::Line(n) => write!(f, "line {n} is not three numbers"),
            LutError::NotFinite(n) => write!(f, "line {n} has a NaN or infinite value"),
            LutError::Count { expected, found } => {
                write!(f, "expected {expected} entries, found {found}")
            }
        }
    }
}

impl std::error::Error for LutError {}

impl Lut3d {
    /// A LUT of edge `size` (2..=65) from a function of the input colour (each channel 0..1).
    pub fn from_fn(size: u32, f: impl Fn([f32; 3]) -> [f32; 3]) -> Lut3d {
        let size = size.clamp(2, MAX_LUT_SIZE);
        let n = size as usize;
        let s = (size - 1) as f32;
        let mut data = Vec::with_capacity(n * n * n);
        for b in 0..n {
            for g in 0..n {
                for r in 0..n {
                    data.push(f([r as f32 / s, g as f32 / s, b as f32 / s]));
                }
            }
        }
        Lut3d { size, data }
    }

    /// The identity LUT (output = input).
    pub fn identity(size: u32) -> Lut3d {
        Lut3d::from_fn(size, |c| c)
    }

    /// Reads a 3D `.cube` LUT (the text format most grading tools export). `DOMAIN_MIN` and
    /// `DOMAIN_MAX` other than 0 and 1 are not supported and are ignored; 1D LUTs are refused.
    pub fn parse_cube(text: &str) -> Result<Lut3d, LutError> {
        let mut size = None;
        let mut data = Vec::new();
        for (i, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut words = line.split_whitespace();
            let first = words.next().unwrap_or_default();
            if first.starts_with(|c: char| c.is_ascii_alphabetic()) {
                if first == "LUT_3D_SIZE" {
                    let n: u32 = words
                        .next()
                        .and_then(|w| w.parse().ok())
                        .ok_or(LutError::Size)?;
                    if !(2..=MAX_LUT_SIZE).contains(&n) {
                        return Err(LutError::Size);
                    }
                    size = Some(n);
                } else if first == "LUT_1D_SIZE" {
                    return Err(LutError::Size);
                }
                // TITLE, DOMAIN_MIN, DOMAIN_MAX and unknown keywords are skipped.
                continue;
            }
            let v: Vec<f32> = line
                .split_whitespace()
                .map(str::parse)
                .collect::<Result<_, _>>()
                .map_err(|_| LutError::Line(i + 1))?;
            let [r, g, b] = v[..] else {
                return Err(LutError::Line(i + 1));
            };
            if !(r.is_finite() && g.is_finite() && b.is_finite()) {
                return Err(LutError::NotFinite(i + 1));
            }
            data.push([r, g, b]);
        }
        let size = size.ok_or(LutError::Size)?;
        let expected = (size as usize).pow(3);
        if data.len() != expected {
            return Err(LutError::Count {
                expected,
                found: data.len(),
            });
        }
        Ok(Lut3d { size, data })
    }

    /// Edge length.
    pub fn size(&self) -> u32 {
        self.size
    }

    /// The colour at grid point (r, g, b).
    pub fn at(&self, r: u32, g: u32, b: u32) -> [f32; 3] {
        let n = self.size as usize;
        self.data[(b as usize * n + g as usize) * n + r as usize]
    }

    /// Texel data for an `Rgba16Float` 3D texture, in upload order. Entries are clamped to 0..=1;
    /// NaN (possible from [`Lut3d::from_fn`]) becomes 0.
    pub(crate) fn texels(&self) -> Vec<[u16; 4]> {
        // Positive half bits order like their values, so this clamps to 1.0 (0x3c00).
        let h = |v: f32| half_bits(v).min(0x3c00);
        self.data
            .iter()
            .map(|c| [h(c[0]), h(c[1]), h(c[2]), h(1.0)])
            .collect()
    }
}

/// IEEE half-float bits of `v`, rounded to nearest. Total over every `f32`: NaN and values at or
/// below 0 give 0, values past the half range give the largest finite half, and values below
/// the smallest half subnormal flush to 0.
fn half_bits(v: f32) -> u16 {
    if v.is_nan() || v <= 0.0 {
        return 0;
    }
    if v >= 65504.0 {
        return 0x7bff;
    }
    if v < 2f32.powi(-14) {
        // Subnormal half: v = m * 2^-24. The product is exact in f32 and m <= 1024 (1024 is the
        // smallest normal half, 0x0400, so the rounding carry is still correct).
        return (v * 16_777_216.0).round() as u16;
    }
    let bits = v.to_bits();
    let exp = ((bits >> 23) & 0xff) + 15 - 127;
    let mant = bits & 0x7f_ffff;
    let h = (exp << 10) | (mant >> 13);
    // Round to nearest on the dropped bits (a carry into the exponent is still correct, and
    // v < 65504 keeps it finite).
    (h + ((mant >> 12) & 1)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn half_to_f32(h: u16) -> f32 {
        let exp = (h >> 10) & 0x1f;
        let mant = (h & 0x3ff) as f32;
        if exp == 0 {
            mant * 2f32.powi(-24)
        } else {
            (1.0 + mant / 1024.0) * 2f32.powi(exp as i32 - 15)
        }
    }

    #[test]
    fn half_conversion_is_close() {
        for i in 0..=1000 {
            let v = i as f32 / 1000.0;
            let back = half_to_f32(half_bits(v));
            assert!((back - v).abs() <= v * 1e-3 + 1e-7, "{v} -> {back}");
        }
        assert_eq!(half_bits(1.0), 0x3c00);
        assert_eq!(half_bits(0.5), 0x3800);
        // Subnormal halves round to nearest; anything below half the smallest one is 0.
        assert_eq!(half_bits(2f32.powi(-24)), 1);
        assert_eq!(half_bits(3.0 * 2f32.powi(-24)), 3);
        assert_eq!(half_bits(2f32.powi(-14)), 0x0400);
        assert_eq!(half_bits(2f32.powi(-14) * 0.9999), 0x0400);
        for tiny in [1e-12, 1.2e-10, 1e-38, f32::MIN_POSITIVE, 1e-45, 2.9e-8] {
            assert_eq!(half_bits(tiny), 0, "{tiny}");
        }
        for v in [1e-7, 1e-6, 3e-5, 6e-5] {
            let back = half_to_f32(half_bits(v));
            assert!((back - v).abs() <= 2f32.powi(-25), "{v} -> {back}");
        }
        // Non-finite, negative and huge inputs.
        assert_eq!(half_bits(f32::NAN), 0);
        assert_eq!(half_bits(-f32::NAN), 0);
        assert_eq!(half_bits(f32::NEG_INFINITY), 0);
        assert_eq!(half_bits(-0.5), 0);
        assert_eq!(half_bits(-0.0), 0);
        assert_eq!(half_bits(f32::INFINITY), 0x7bff);
        assert_eq!(half_bits(1e9), 0x7bff);
        assert_eq!(half_bits(65504.0), 0x7bff);
    }

    #[test]
    fn texels_are_clamped_and_sanitised() {
        let l = Lut3d::from_fn(2, |c| match (c[0] > 0.5, c[1] > 0.5) {
            (false, false) => [1e-12, -0.25, f32::NAN],
            (true, false) => [f32::INFINITY, 2.0, f32::NEG_INFINITY],
            _ => c,
        });
        let t = l.texels();
        assert_eq!(t[0], [0, 0, 0, 0x3c00]);
        assert_eq!(t[1], [0x3c00, 0x3c00, 0, 0x3c00]);
    }

    #[test]
    fn identity_and_order() {
        let l = Lut3d::identity(3);
        assert_eq!(l.at(2, 0, 0), [1.0, 0.0, 0.0]);
        assert_eq!(l.at(0, 1, 2), [0.0, 0.5, 1.0]);
    }

    #[test]
    fn cube_files_parse_and_errors_are_reported() {
        let text = "TITLE \"warm\"\n# comment\nLUT_3D_SIZE 2\nDOMAIN_MIN 0 0 0\n\
                    0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
        let l = Lut3d::parse_cube(text).expect("parses");
        assert_eq!(l, Lut3d::identity(2));
        assert_eq!(
            Lut3d::parse_cube("LUT_3D_SIZE 2\n0 0 0\n"),
            Err(LutError::Count {
                expected: 8,
                found: 1
            })
        );
        assert_eq!(Lut3d::parse_cube("0 0 0\n"), Err(LutError::Size));
        assert_eq!(
            Lut3d::parse_cube("LUT_3D_SIZE 2\n0 0\n"),
            Err(LutError::Line(2))
        );
        assert_eq!(Lut3d::parse_cube("LUT_1D_SIZE 16\n"), Err(LutError::Size));
        assert_eq!(Lut3d::parse_cube("LUT_3D_SIZE 99\n"), Err(LutError::Size));
    }
}
