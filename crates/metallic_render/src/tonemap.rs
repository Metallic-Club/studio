//! The finish pass settings: exposure, a scene-linear grade, the tone curve, vignette, output
//! encoding and dither.
//!
//! The scene renders into a linear HDR target; the finish pass (`shaders/finish.wgsl`) turns it
//! into display values. [`ToneCurve::map`] mirrors the shader on the CPU so tests and tools can
//! check curve maths without a GPU.
//!
//! Every curve is pre-scaled so scene grey 0.18 lands on the same display value as
//! [`ToneCurve::Filmic`], the curve the renderer has always used: switching curves changes
//! contrast and hue, not overall brightness, and the default image is unchanged. (The pre-scales
//! in `docs/design/look-presets.md` 3.3 normalise to AgX instead and do not all agree with these
//! curve implementations; the ones here are solved against them.)

/// Tone curve of the finish pass. Names follow `ColorGradingEffect.TonemapperPreset` where Roblox
/// has one; the others are Metallic additions (`docs/research/2026-10-30-lighting.md` 5.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum ToneCurve {
    /// The Narkowicz fit of the ACES curve. The renderer's original curve and the default.
    #[default]
    Filmic,
    /// AgX (Troy Sobotka) as a polynomial fit; gentle, hue-stable highlights.
    Agx,
    /// AgX with the "punchy" look: more contrast and saturation. The Realistic Look's curve.
    AgxPunchy,
    /// Khronos PBR Neutral: keeps base colours close to their sRGB values. For stylised looks.
    Neutral,
    /// Stephen Hill's fit of the ACES RRT and ODT.
    Aces,
    /// No curve: a plain clamp ("Retro").
    Clamp,
}

impl ToneCurve {
    /// Every curve, in shader index order.
    pub const ALL: [ToneCurve; 6] = [
        ToneCurve::Filmic,
        ToneCurve::Agx,
        ToneCurve::AgxPunchy,
        ToneCurve::Neutral,
        ToneCurve::Aces,
        ToneCurve::Clamp,
    ];

    /// The index `finish.wgsl` switches on.
    pub fn index(self) -> u32 {
        match self {
            ToneCurve::Filmic => 0,
            ToneCurve::Agx => 1,
            ToneCurve::AgxPunchy => 2,
            ToneCurve::Neutral => 3,
            ToneCurve::Aces => 4,
            ToneCurve::Clamp => 5,
        }
    }

    /// Factor applied to scene light before the curve so 0.18 maps to Filmic's display value.
    /// Solved numerically against [`ToneCurve::map`] (the test
    /// `grey_lands_on_one_display_value_under_every_curve` checks them).
    pub fn prescale(self) -> f32 {
        match self {
            ToneCurve::Filmic => 1.0,
            ToneCurve::Agx => 1.3331,
            ToneCurve::AgxPunchy => 2.2407,
            ToneCurve::Neutral => 1.7050,
            ToneCurve::Aces => 2.0102,
            ToneCurve::Clamp => 1.4828,
        }
    }

    /// Maps linear scene light to linear display light (0..1), pre-scale included. CPU mirror
    /// of `apply_curve` in `finish.wgsl`.
    pub fn map(self, c: [f32; 3]) -> [f32; 3] {
        let s = self.prescale();
        let x = [c[0] * s, c[1] * s, c[2] * s];
        let out = match self {
            ToneCurve::Filmic => x.map(narkowicz),
            ToneCurve::Agx => agx(x, false),
            ToneCurve::AgxPunchy => agx(x, true),
            ToneCurve::Neutral => pbr_neutral(x),
            ToneCurve::Aces => aces_hill(x),
            ToneCurve::Clamp => x,
        };
        out.map(|v| v.clamp(0.0, 1.0))
    }
}

/// Settings of the finish pass. The defaults change nothing but the curve: they reproduce the
/// renderer's original output.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FinishSettings {
    /// Tone curve.
    pub curve: ToneCurve,
    /// Multiplier on scene light before the grade (an exposure in linear units, 1 = none).
    pub exposure: f32,
    /// Multiplier per channel in scene-linear light (white = none), like
    /// `ColorCorrectionEffect.TintColor`.
    pub tint: [f32; 3],
    /// Saturation change, -1 (grey) .. 1 (double); 0 = none.
    pub saturation: f32,
    /// Contrast change around scene grey 0.18, -1 .. 1; 0 = none.
    pub contrast: f32,
    /// Added to the encoded display value after the curve, -1 .. 1; 0 = none.
    pub brightness: f32,
    /// Corner darkening, 0 (none) .. 1.
    pub vignette: f32,
    /// Adds +-0.5/255 of noise before 8-bit quantisation against banding.
    pub dither: bool,
}

impl Default for FinishSettings {
    fn default() -> Self {
        FinishSettings {
            curve: ToneCurve::Filmic,
            exposure: 1.0,
            tint: [1.0; 3],
            saturation: 0.0,
            contrast: 0.0,
            brightness: 0.0,
            vignette: 0.0,
            dither: true,
        }
    }
}

/// Uniform block of the finish pass; mirrors `Finish` in `finish.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct FinishUniform {
    tint_exposure: [f32; 4],
    grade: [f32; 4],
    mode: [f32; 4],
}

impl FinishUniform {
    /// `exposure` is the scene exposure (the sky's), multiplied with the settings' own.
    pub(crate) fn new(s: &FinishSettings, exposure: f32, output_srgb: bool) -> FinishUniform {
        let e = exposure * s.exposure * s.curve.prescale();
        FinishUniform {
            tint_exposure: [s.tint[0], s.tint[1], s.tint[2], e],
            grade: [s.saturation, s.contrast, s.brightness, s.vignette],
            mode: [
                s.curve.index() as f32,
                if output_srgb { 1.0 } else { 0.0 },
                if s.dither { 1.0 } else { 0.0 },
                0.0,
            ],
        }
    }
}

fn narkowicz(x: f32) -> f32 {
    let x = x.max(0.0);
    (x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14)
}

/// `m` as three columns, like a WGSL `mat3x3` constructor.
fn mul_cols(m: [[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| m[0][i] * v[0] + m[1][i] * v[1] + m[2][i] * v[2])
}

/// `m` as three rows, as published for the Hill fit.
fn mul_rows(m: [[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    m.map(|r| r[0] * v[0] + r[1] * v[1] + r[2] * v[2])
}

const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

fn luma(c: [f32; 3]) -> f32 {
    c[0] * LUMA[0] + c[1] * LUMA[1] + c[2] * LUMA[2]
}

const AGX_INSET: [[f32; 3]; 3] = [
    [0.842_479, 0.042_328_24, 0.042_375_65],
    [0.078_433_6, 0.878_468_6, 0.078_433_6],
    [0.079_223_745, 0.079_166_13, 0.879_143],
];
const AGX_OUTSET: [[f32; 3]; 3] = [
    [1.196_879, -0.052_896_85, -0.052_971_635],
    [-0.098_020_88, 1.151_903_1, -0.098_043_45],
    [-0.099_029_74, -0.098_961_18, 1.151_073_7],
];
const AGX_MIN_EV: f32 = -12.47393;
const AGX_MAX_EV: f32 = 4.026069;

fn agx_contrast(x: f32) -> f32 {
    let x2 = x * x;
    let x4 = x2 * x2;
    15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x
        - 0.00232
}

fn agx(c: [f32; 3], punchy: bool) -> [f32; 3] {
    let c = mul_cols(AGX_INSET, c.map(|v| v.max(1e-10)));
    let mut c = c.map(|v| {
        let ev = v.max(1e-10).log2().clamp(AGX_MIN_EV, AGX_MAX_EV);
        agx_contrast((ev - AGX_MIN_EV) / (AGX_MAX_EV - AGX_MIN_EV))
    });
    if punchy {
        c = c.map(|v| v.max(0.0).powf(1.35));
        let l = luma(c);
        c = c.map(|v| l + 1.4 * (v - l));
    }
    mul_cols(AGX_OUTSET, c).map(|v| v.clamp(0.0, 1.0).powf(2.2))
}

fn pbr_neutral(c: [f32; 3]) -> [f32; 3] {
    const START: f32 = 0.8 - 0.04;
    const DESAT: f32 = 0.15;
    let x = c[0].min(c[1]).min(c[2]);
    let offset = if x < 0.08 { x - 6.25 * x * x } else { 0.04 };
    let c = c.map(|v| v - offset);
    let peak = c[0].max(c[1]).max(c[2]);
    if peak < START {
        return c;
    }
    let d = 1.0 - START;
    let new_peak = 1.0 - d * d / (peak + d - START);
    let g = 1.0 - 1.0 / (DESAT * (peak - new_peak) + 1.0);
    c.map(|v| {
        let s = v * new_peak / peak;
        s + (new_peak - s) * g
    })
}

const ACES_IN: [[f32; 3]; 3] = [
    [0.59719, 0.35458, 0.04823],
    [0.07600, 0.90834, 0.01566],
    [0.02840, 0.13383, 0.83777],
];
const ACES_OUT: [[f32; 3]; 3] = [
    [1.60475, -0.53108, -0.07367],
    [-0.10208, 1.10813, -0.00605],
    [-0.00327, -0.07276, 1.07602],
];

fn aces_hill(c: [f32; 3]) -> [f32; 3] {
    let v = mul_rows(ACES_IN, c.map(|x| x.max(0.0)));
    let v = v.map(|v| {
        let a = v * (v + 0.024_578_6) - 0.000_090_537;
        let b = v * (0.983_729 * v + 0.432_951) + 0.238_081;
        a / b
    });
    mul_rows(ACES_OUT, v)
}

/// Linear to sRGB-encoded, one channel.
pub fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grey_lands_on_one_display_value_under_every_curve() {
        let reference = linear_to_srgb(ToneCurve::Filmic.map([0.18; 3])[1]);
        for curve in ToneCurve::ALL {
            let out = curve.map([0.18; 3]);
            for ch in out {
                let diff = (linear_to_srgb(ch) - reference).abs() * 255.0;
                assert!(diff <= 1.0, "{curve:?} maps grey {diff:.2}/255 away");
            }
        }
    }

    #[test]
    fn curves_are_monotonic_and_bounded() {
        for curve in ToneCurve::ALL {
            let mut last = -1.0;
            for i in 0..200 {
                let x = (i as f32 / 20.0).exp2() / 1024.0;
                let y = curve.map([x; 3])[1];
                assert!((0.0..=1.0).contains(&y), "{curve:?} out of range at {x}");
                assert!(y + 1e-4 >= last, "{curve:?} decreases at {x}");
                last = y;
            }
            assert_eq!(curve.map([0.0; 3])[1].min(0.01), curve.map([0.0; 3])[1]);
        }
    }

    #[test]
    fn indices_follow_the_list() {
        for (i, c) in ToneCurve::ALL.iter().enumerate() {
            assert_eq!(c.index() as usize, i);
        }
    }
}
