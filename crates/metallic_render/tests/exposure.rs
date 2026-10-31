//! Auto exposure and tone curve tests (#151): the histogram meter converges on bright and dark
//! scenes and adapts faster toward light, NaN and infinite light cannot poison it or the finish
//! pass, the curves match their published references on the CPU and the GPU, and Looks pick
//! their curve and exposure mode.
//!
//! References, written from the public descriptions (no code copied):
//! - AgX: Troy Sobotka's AgX, in the minimal polynomial form Benjamin Wrensch published for the
//!   iolite engine ("Minimal AgX Implementation", 2023): inset matrix, log2 encoding over
//!   -12.47393..4.026069 EV, a 6th-order polynomial, outset matrix, 2.2 power. The Punchy look
//!   is an ASC CDL power 1.35 and saturation 1.4 (ASC CDL v1.2: Rec. 709 luma of the
//!   slope-offset-power result).
//! - Khronos PBR Neutral: the specification in the KhronosGroup/ToneMapping repository
//!   (start of compression 0.76, desaturation 0.15, toe offset 0.04 below 0.08).
//! - Clamp: identity, clipped to 0..1.
mod support;

use metallic_core::lighting::{PRESETS, apply_preset};
use metallic_render::exposure::{METER_KEY_LOG2, REFERENCE_EV100};
use metallic_render::lighting::evaluate;
use metallic_render::{
    AutoExposure, Camera, ExposureMode, FinishSettings, FullscreenPass, LightingInputs, LookTone,
    PostContext, PostPass, PostSlot, Quality, RenderContext, RenderSettings, Renderer, ToneCurve,
};
use support::reference_scene::{reference_camera, reference_world};

const SIZE: (u32, u32) = (160, 90);
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

fn context() -> Option<RenderContext> {
    match RenderContext::new_headless() {
        Ok(c) => Some(c),
        Err(_) => {
            eprintln!("no GPU adapter; skipping");
            None
        }
    }
}

fn renderer_on(ctx: &RenderContext) -> Renderer {
    Renderer::with_settings(
        ctx,
        FORMAT,
        RenderSettings {
            quality: Quality::Low,
        },
    )
}

fn camera() -> Camera {
    reference_camera()
}

// ---- A test HDR source ---------------------------------------------------------------------

/// What [`Source`] writes into the HDR image (replacing the scene).
#[derive(Clone, Copy, PartialEq)]
enum Pattern {
    /// Linear grey `l` everywhere.
    Flat(f32),
    /// Horizontal ramp of exp2(lo..hi) in three bands: grey, orange, blue.
    Ramp(f32, f32),
    /// Grey `l`, except NaN on the left quarter, then thin strips of +inf and -inf.
    Poisoned(f32),
    /// NaN everywhere.
    AllNan,
}

struct Source {
    pass: FullscreenPass,
    pattern: Pattern,
}

const SOURCE_WGSL: &str = "struct P { a: vec4<f32>, b: vec4<f32>, bits: vec4<u32> }
@group(0) @binding(3) var<uniform> params: P;
@fragment fn fs_main(in: PostIn) -> @location(0) vec4<f32> {
    let keep = textureSampleLevel(post_input, post_sampler, in.uv, 0.0).rgb * 0.0;
    let mode = u32(params.a.w + 0.5);
    // Built from bits at run time: constant NaN or infinity is a WGSL error.
    let nan = bitcast<f32>(params.bits.x);
    let inf = bitcast<f32>(params.bits.y);
    var c = params.a.rgb;
    if (mode == 1u) {
        let v = exp2(params.b.x + (params.b.y - params.b.x) * in.uv.x);
        let band = min(u32(in.uv.y * 3.0), 2u);
        var tint = vec3<f32>(1.0);
        if (band == 1u) { tint = vec3<f32>(1.0, 0.3, 0.05); }
        if (band == 2u) { tint = vec3<f32>(0.05, 0.2, 1.0); }
        c = tint * v;
    } else if (mode == 2u) {
        if (in.uv.x < 0.25) { c = vec3<f32>(nan); }
        else if (in.uv.x < 0.27) { c = vec3<f32>(inf); }
        else if (in.uv.x < 0.29) { c = vec3<f32>(-inf); }
    } else if (mode == 3u) {
        c = vec3<f32>(nan);
    }
    return vec4<f32>(post_output(c + keep), 1.0);
}";

impl Source {
    fn new(pattern: Pattern) -> Source {
        Source {
            pass: FullscreenPass::new("test source", SOURCE_WGSL, 48),
            pattern,
        }
    }
}

impl PostPass for Source {
    fn name(&self) -> &str {
        "source"
    }
    fn slot(&self) -> PostSlot {
        PostSlot::Hdr
    }
    fn record(&mut self, ctx: &mut PostContext<'_>) {
        let (a, b) = match self.pattern {
            Pattern::Flat(l) => ([l, l, l, 0.0], [0.0; 4]),
            Pattern::Ramp(lo, hi) => ([0.0, 0.0, 0.0, 1.0], [lo, hi, 0.0, 0.0]),
            Pattern::Poisoned(l) => ([l, l, l, 2.0], [0.0; 4]),
            Pattern::AllNan => ([0.0, 0.0, 0.0, 3.0], [0.0; 4]),
        };
        let mut words = [0u32; 12];
        for (i, v) in a.iter().chain(b.iter()).enumerate() {
            words[i] = v.to_bits();
        }
        words[8] = f32::NAN.to_bits();
        words[9] = f32::INFINITY.to_bits();
        self.pass.draw(ctx, bytemuck::cast_slice(&words));
    }
}

/// A renderer under the default lighting whose HDR image is `pattern`: no bloom.
fn source_renderer(ctx: &RenderContext, pattern: Pattern) -> Renderer {
    let mut r = renderer_on(ctx);
    r.post_mut().remove("bloom");
    r.post_mut().push(Box::new(Source::new(pattern)));
    r
}

fn set_pattern(r: &mut Renderer, pattern: Pattern) {
    r.post_mut()
        .get_mut::<Source>("source")
        .expect("source pass")
        .pattern = pattern;
}

/// Renders one frame (no capture reset) with time step `dt`.
fn frame(ctx: &RenderContext, r: &mut Renderer, dt: f32) {
    let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test frame"),
        size: wgpu::Extent3d {
            width: SIZE.0,
            height: SIZE.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    r.set_frame_time(dt);
    r.render(&view, &camera(), SIZE);
}

// ---- The metering model, from the lighting doc 3.9 ------------------------------------------

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Target EV100 for a flat grey `l` under the default lighting.
fn target_ev(l: f32, a: &AutoExposure) -> f32 {
    (REFERENCE_EV100 - METER_KEY_LOG2 + l.log2()).clamp(a.min_ev100, a.max_ev100)
}

/// The multiplier the meter settles on at EV100 `ev` (night bias and compensation included).
fn multiplier_at(ev: f32, a: &AutoExposure) -> f32 {
    let bias = -2.0 * (1.0 - smoothstep(0.0, 10.0, ev)) * (0.5 + 0.5 * a.night_adaptation);
    2f32.powf(REFERENCE_EV100 - ev + bias + a.compensation)
}

fn centre(px: &[u8]) -> [u8; 3] {
    let i = ((SIZE.1 / 2 * SIZE.0 + SIZE.0 / 2) * 4) as usize;
    [px[i], px[i + 1], px[i + 2]]
}

fn pixel(px: &[u8], w: u32, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * w + x) * 4) as usize;
    [px[i], px[i + 1], px[i + 2]]
}

// ---- Exposure -------------------------------------------------------------------------------

#[test]
fn exposure_converges_on_bright_and_dark_scenes() {
    let Some(ctx) = context() else { return };
    let mut r = source_renderer(&ctx, Pattern::Flat(1.0));
    if !r.auto_exposure_active() {
        eprintln!("no compute shaders; skipping");
        return;
    }
    let a = r.auto_exposure;
    assert!(a.enabled, "the default lighting meters");
    // Bright day, dim interior, and night (where the night bias keeps it darker than grey).
    let mut shown = Vec::new();
    for l in [0.3, 0.01, 0.0005] {
        set_pattern(&mut r, Pattern::Flat(l));
        let px = r.capture(&camera(), SIZE).expect("capture");
        let s = r.read_exposure().expect("metered");
        let ev = target_ev(l, &a);
        assert!(
            (s.ev100 - ev).abs() < 0.06,
            "L {l}: EV100 {} vs {ev}",
            s.ev100
        );
        let m = multiplier_at(ev, &a);
        assert!(
            (s.multiplier / m).log2().abs() < 0.06,
            "L {l}: multiplier {} vs {m}",
            s.multiplier
        );
        assert!(s.multiplier.is_finite());
        shown.push((l, centre(&px)));
    }
    // Without the night bias, bright and dark land on the same display grey: the meter maps
    // the scene's mid-tone to the key, whatever it was.
    let (bright, dark, night) = (shown[0].1, shown[1].1, shown[2].1);
    for c in 0..3 {
        assert!(
            (i32::from(bright[c]) - i32::from(dark[c])).abs() <= 3,
            "bright {bright:?} vs dark {dark:?}"
        );
    }
    assert!(
        night[1] + 10 < dark[1],
        "night {night:?} not darker than {dark:?}"
    );
    // And that grey is the key through the curve.
    let key = 2f32.powf(METER_KEY_LOG2);
    let want = (metallic_render::tonemap::linear_to_srgb(r.finish.curve.map([key; 3])[1]) * 255.0)
        .round() as i32;
    assert!(
        (i32::from(bright[1]) - want).abs() <= 3,
        "{bright:?} vs {want}"
    );
}

#[test]
fn adaptation_moves_toward_the_target_faster_for_light_than_dark() {
    let Some(ctx) = context() else { return };
    let mut r = source_renderer(&ctx, Pattern::Flat(0.01));
    if !r.auto_exposure_active() {
        eprintln!("no compute shaders; skipping");
        return;
    }
    let a = r.auto_exposure;
    let (dark, bright) = (target_ev(0.01, &a), target_ev(0.3, &a));
    r.capture(&camera(), SIZE).expect("capture");
    let ev = |r: &Renderer| r.read_exposure().expect("metered").ev100;
    assert!((ev(&r) - dark).abs() < 0.06);

    let dt = 1.0 / 30.0;
    let run = |r: &mut Renderer, seconds: f32| {
        for _ in 0..(seconds / dt).round() as usize {
            frame(&ctx, r, dt);
        }
        let now = ev(r);
        assert!(now.is_finite());
        now
    };

    set_pattern(&mut r, Pattern::Flat(0.3));
    let half = run(&mut r, 0.5);
    let up = (half - dark) / (bright - dark);
    // exp(-rate t): 1 - e^-1.5 = 0.78 of the way after half a second at 3/s.
    assert!(
        (up - (1.0 - (-1.5f32).exp())).abs() < 0.05,
        "toward light {up}"
    );
    let settled = run(&mut r, 3.0);
    assert!((settled - bright).abs() < 0.06, "{settled} vs {bright}");

    set_pattern(&mut r, Pattern::Flat(0.01));
    let half = run(&mut r, 0.5);
    let down = (bright - half) / (bright - dark);
    assert!(
        (down - (1.0 - (-0.5f32).exp())).abs() < 0.05,
        "toward dark {down}"
    );
    assert!(
        up > down + 0.2,
        "light {up} should adapt faster than dark {down}"
    );
    let settled = run(&mut r, 8.0);
    assert!((settled - dark).abs() < 0.06, "{settled} vs {dark}");

    // A cut jumps straight there.
    set_pattern(&mut r, Pattern::Flat(0.3));
    r.reset_exposure();
    frame(&ctx, &mut r, dt);
    assert!((ev(&r) - bright).abs() < 0.06);
}

#[test]
fn exposure_min_and_max_limit_the_meter() {
    let Some(ctx) = context() else { return };
    let mut r = source_renderer(&ctx, Pattern::Flat(50.0));
    if !r.auto_exposure_active() {
        eprintln!("no compute shaders; skipping");
        return;
    }
    r.capture(&camera(), SIZE).expect("capture");
    let s = r.read_exposure().expect("metered");
    assert!(
        (s.ev100 - 16.0).abs() < 1e-3,
        "clamped to ExposureMax: {}",
        s.ev100
    );
    assert!(s.target_ev100 <= 16.0);
    r.auto_exposure.max_ev100 = 20.0;
    r.capture(&camera(), SIZE).expect("capture");
    let s = r.read_exposure().expect("metered");
    let want = target_ev(50.0, &r.auto_exposure);
    assert!((s.ev100 - want).abs() < 0.06, "{} vs {want}", s.ev100);

    // 2^-10 meters as EV100 7.2 under the default lighting.
    set_pattern(&mut r, Pattern::Flat(2f32.powi(-10)));
    r.auto_exposure.min_ev100 = 10.0;
    r.capture(&camera(), SIZE).expect("capture");
    let s = r.read_exposure().expect("metered");
    assert!(
        (s.ev100 - 10.0).abs() < 1e-3,
        "clamped to ExposureMin: {}",
        s.ev100
    );
}

#[test]
fn the_default_lighting_meters_to_its_fixed_exposure() {
    // The meter's key is calibrated on the reference scene: auto exposure leaves Clear Day
    // where the fixed exposure (and the #149 review) put it, and every preset close to it.
    let Some(ctx) = context() else { return };
    let mut r = renderer_on(&ctx);
    if !r.auto_exposure_active() {
        eprintln!("no compute shaders; skipping");
        return;
    }
    for preset in &PRESETS {
        let mut world = reference_world();
        apply_preset(&mut world, preset).expect("apply");
        r.sync(&world);
        r.capture(&camera(), SIZE).expect("capture");
        let s = r.read_exposure().expect("metered");
        let limit = if preset.id == "clear-day" { 0.1 } else { 1.0 };
        assert!(
            s.multiplier.log2().abs() < limit,
            "{}: auto exposure moves it {} EV from the fixed exposure",
            preset.id,
            s.multiplier.log2()
        );
    }
}

#[test]
fn nan_and_infinite_light_cannot_poison_the_meter_or_the_image() {
    let Some(ctx) = context() else { return };
    let mut r = source_renderer(&ctx, Pattern::Flat(0.1));
    let metered = r.auto_exposure_active();
    r.capture(&camera(), SIZE).expect("capture");
    let clean = r.read_exposure();

    set_pattern(&mut r, Pattern::Poisoned(0.1));
    let px = r.capture(&camera(), SIZE).expect("capture");
    if metered {
        let (clean, s) = (clean.expect("metered"), r.read_exposure().expect("metered"));
        assert!(s.multiplier.is_finite() && s.ev100.is_finite());
        assert!(
            (s.multiplier / clean.multiplier).log2().abs() < 0.1,
            "NaN/inf moved the exposure: {} vs {}",
            s.multiplier,
            clean.multiplier
        );
    }
    let y = SIZE.1 / 2;
    let x_of = |u: f32| (u * SIZE.0 as f32) as u32;
    let nan = pixel(&px, SIZE.0, x_of(0.1), y);
    let pos_inf = pixel(&px, SIZE.0, x_of(0.26), y);
    let neg_inf = pixel(&px, SIZE.0, x_of(0.28), y);
    let grey = pixel(&px, SIZE.0, x_of(0.6), y);
    assert!(nan.iter().all(|&v| v <= 3), "NaN shows as {nan:?}");
    assert!(neg_inf.iter().all(|&v| v <= 3), "-inf shows as {neg_inf:?}");
    assert!(
        pos_inf.iter().all(|&v| v >= 245),
        "+inf shows as {pos_inf:?}"
    );
    assert!(
        grey[1] > 40 && grey[1] < 230,
        "valid light shows as {grey:?}"
    );

    // A frame of nothing but NaN holds the last exposure and renders black.
    if metered {
        let before = r.read_exposure().expect("metered");
        set_pattern(&mut r, Pattern::AllNan);
        frame(&ctx, &mut r, 1.0 / 30.0);
        let after = r.read_exposure().expect("metered");
        assert_eq!(before.multiplier, after.multiplier);
        let px = r.capture(&camera(), SIZE).expect("capture");
        let s = r.read_exposure().expect("metered");
        assert!(s.multiplier.is_finite() && s.multiplier > 0.0);
        assert!(px.chunks(4).all(|p| p[..3].iter().all(|&v| v <= 3)));
    }

    // Settings that are NaN or infinite are replaced before they reach the GPU.
    set_pattern(&mut r, Pattern::Flat(0.1));
    r.auto_exposure = AutoExposure {
        enabled: true,
        min_ev100: f32::NAN,
        max_ev100: f32::INFINITY,
        compensation: f32::NAN,
        night_adaptation: f32::NEG_INFINITY,
        speed_up: f32::NAN,
        speed_down: f32::INFINITY,
    };
    r.set_frame_time(f32::NAN);
    let px = r.capture(&camera(), SIZE).expect("capture");
    if metered {
        let s = r.read_exposure().expect("metered");
        assert!(s.multiplier.is_finite() && s.ev100.is_finite(), "{s:?}");
    }
    assert!(centre(&px)[1] > 20);
}

#[test]
fn auto_exposure_off_and_fixed_looks_keep_the_fixed_exposure() {
    let Some(ctx) = context() else { return };
    let mut r = renderer_on(&ctx);
    let world = reference_world();
    r.sync(&world);
    r.auto_exposure.enabled = false;
    let fixed = r.capture(&camera(), SIZE).expect("capture");

    for (prop_off, look) in [(true, ""), (false, "pixel-3d"), (false, "Retro 32-bit")] {
        let inputs = LightingInputs {
            auto_exposure: !prop_off,
            look: look.into(),
            ..LightingInputs::read(&world)
        };
        r.set_lighting(&inputs);
        assert!(!r.auto_exposure.enabled, "{look}: metering should be off");
        assert!(!r.auto_exposure_active());
        if look.is_empty() {
            assert_eq!(r.capture(&camera(), SIZE).expect("capture"), fixed);
        }
    }
}

#[test]
fn the_meter_runs_on_the_gl_backend() {
    let Ok(ctx) = RenderContext::new_headless_with_backends(wgpu::Backends::GL) else {
        eprintln!("no GL adapter; skipping");
        return;
    };
    let mut r = source_renderer(&ctx, Pattern::Flat(0.01));
    if !r.auto_exposure_active() {
        eprintln!("GL adapter without compute shaders; exposure stays fixed");
        return;
    }
    r.capture(&camera(), SIZE).expect("capture");
    let s = r.read_exposure().expect("metered");
    let want = target_ev(0.01, &r.auto_exposure);
    assert!((s.ev100 - want).abs() < 0.06, "{} vs {want}", s.ev100);
}

// ---- Tone curves ----------------------------------------------------------------------------

fn srgb(c: f64) -> f64 {
    if c <= 0.003_130_8 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

const REC709: [f64; 3] = [0.2126, 0.7152, 0.0722];

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// `cols` as the three columns of a matrix, times `v`.
fn times(cols: [[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|r| cols[0][r] * v[0] + cols[1][r] * v[1] + cols[2][r] * v[2])
}

/// AgX in its minimal polynomial form (see the module comment), in f64.
fn agx_reference(c: [f64; 3], punchy: bool) -> [f64; 3] {
    const INSET: [[f64; 3]; 3] = [
        [0.842479062253094, 0.0423282422610123, 0.0423756549057051],
        [0.0784335999999992, 0.878468636469772, 0.0784336],
        [0.0792237451477643, 0.0791661274605434, 0.879142973793104],
    ];
    const OUTSET: [[f64; 3]; 3] = [
        [1.19687900512017, -0.0528968517574562, -0.0529716355144438],
        [-0.0980208811401368, 1.15190312990417, -0.0980434501171241],
        [-0.0990297440797205, -0.0989611768448433, 1.15107367264116],
    ];
    let (lo, hi) = (-12.47393, 4.026069);
    let poly = |x: f64| {
        // 15.5 x^6 - 40.14 x^5 + 31.96 x^4 - 6.868 x^3 + 0.4298 x^2 + 0.1191 x - 0.00232
        let coeffs = [-0.00232, 0.1191, 0.4298, -6.868, 31.96, -40.14, 15.5];
        coeffs.iter().rev().fold(0.0, |acc, k| acc * x + k)
    };
    let inset = times(INSET, c.map(|v| v.max(1e-10)));
    let mut enc = inset.map(|v| poly((v.max(1e-10).log2().clamp(lo, hi) - lo) / (hi - lo)));
    if punchy {
        // ASC CDL: slope 1, offset 0, power 1.35, then saturation 1.4 around Rec. 709 luma.
        let sop = enc.map(|v| v.max(0.0).powf(1.35));
        let luma = dot(sop, REC709);
        enc = sop.map(|v| luma + 1.4 * (v - luma));
    }
    times(OUTSET, enc).map(|v| v.clamp(0.0, 1.0).powf(2.2))
}

/// Khronos PBR Neutral, from the specification, in f64.
fn pbr_neutral_reference(c: [f64; 3]) -> [f64; 3] {
    let start = 0.8 - 0.04;
    let desaturation = 0.15;
    let x = c[0].min(c[1]).min(c[2]);
    let offset = if x < 0.08 { x - 6.25 * x * x } else { 0.04 };
    let c = c.map(|v| v - offset);
    let peak = c[0].max(c[1]).max(c[2]);
    if peak < start {
        return c;
    }
    let d = 1.0 - start;
    let new_peak = 1.0 - d * d / (peak + d - start);
    let g = 1.0 - 1.0 / (desaturation * (peak - new_peak) + 1.0);
    c.map(|v| {
        let scaled = v * new_peak / peak;
        scaled * (1.0 - g) + new_peak * g
    })
}

/// Display light (0..1) of `curve` for scene light `c` (no pre-scale).
fn reference(curve: ToneCurve, c: [f64; 3]) -> Option<[f64; 3]> {
    let out = match curve {
        ToneCurve::Agx => agx_reference(c, false),
        ToneCurve::AgxPunchy => agx_reference(c, true),
        ToneCurve::Neutral => pbr_neutral_reference(c),
        ToneCurve::Clamp => c,
        _ => return None,
    };
    Some(out.map(|v| v.clamp(0.0, 1.0)))
}

const REFERENCED: [ToneCurve; 4] = [
    ToneCurve::AgxPunchy,
    ToneCurve::Agx,
    ToneCurve::Neutral,
    ToneCurve::Clamp,
];

const TINTS: [[f64; 3]; 3] = [[1.0, 1.0, 1.0], [1.0, 0.3, 0.05], [0.05, 0.2, 1.0]];

#[test]
fn cpu_curves_match_the_published_references() {
    for curve in REFERENCED {
        let s = f64::from(curve.prescale());
        let mut worst = 0.0f64;
        for i in 0..=160 {
            let v = 2f64.powf(-10.0 + 16.0 * f64::from(i) / 160.0);
            for tint in TINTS {
                let c = tint.map(|t| t * v);
                let want = reference(curve, c).expect("referenced");
                // `map` applies the pre-scale; undo it to compare the bare curve.
                let got = curve.map(c.map(|x| (x / s) as f32));
                for k in 0..3 {
                    worst = worst.max((f64::from(got[k]) - want[k]).abs());
                }
            }
        }
        assert!(worst < 2e-4, "{curve:?}: off the reference by {worst}");
    }
    // Spec properties of PBR Neutral: colours up to 0.76 only lose the 0.04 offset, and
    // highlights approach but never reach 1.
    let mid = pbr_neutral_reference([0.5, 0.3, 0.2]);
    assert!((mid[0] - 0.46).abs() < 1e-12 && (mid[2] - 0.16).abs() < 1e-12);
    assert!(pbr_neutral_reference([1e6; 3])[0] < 1.0);
}

#[test]
fn cpu_curves_are_safe_for_nan_and_infinity() {
    for curve in ToneCurve::ALL {
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, f32::MAX] {
            let out = curve.map([bad, 0.18, bad]);
            assert!(
                out.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
                "{curve:?} maps {bad} to {out:?}"
            );
        }
        assert!(curve.map([f32::NAN; 3]).iter().all(|&v| v < 0.01));
        assert!(curve.map([f32::INFINITY; 3]).iter().all(|&v| v > 0.95));
    }
}

#[test]
fn gpu_curves_match_the_published_references() {
    let Some(ctx) = context() else { return };
    let (lo, hi) = (-8.0f32, 6.0f32);
    let size = (256u32, 24u32);
    let mut r = source_renderer(&ctx, Pattern::Ramp(lo, hi));
    for curve in REFERENCED {
        r.set_lighting(&LightingInputs {
            auto_exposure: false,
            ..LightingInputs::default()
        });
        r.sky.exposure = 1.0;
        r.finish = FinishSettings {
            curve,
            dither: false,
            ..FinishSettings::default()
        };
        let px = r.capture(&camera(), size).expect("capture");
        let s = f64::from(curve.prescale());
        let mut worst = 0;
        for band in 0..3u32 {
            let y = band * 8 + 4;
            for x in 0..size.0 {
                let u = (f64::from(x) + 0.5) / f64::from(size.0);
                let v = 2f64.powf(f64::from(lo) + f64::from(hi - lo) * u);
                let c = TINTS[band as usize].map(|t| t * v * s);
                let want = reference(curve, c)
                    .expect("referenced")
                    .map(|d| (srgb(d) * 255.0).round() as i32);
                let got = pixel(&px, size.0, x, y);
                for k in 0..3 {
                    let e = (i32::from(got[k]) - want[k]).abs();
                    worst = worst.max(e);
                    assert!(
                        e <= 2,
                        "{curve:?} band {band} x {x}: got {got:?}, want {want:?}"
                    );
                }
            }
        }
        eprintln!("{curve:?}: worst {worst}/255");
    }
}

// ---- Looks ----------------------------------------------------------------------------------

#[test]
fn looks_pick_their_curve_and_exposure_mode() {
    use ExposureMode::{Auto, Display, Fixed};
    let cases = [
        ("", ToneCurve::AgxPunchy, Auto),
        ("realistic", ToneCurve::AgxPunchy, Auto),
        ("toon", ToneCurve::Neutral, Auto),
        ("Low-poly", ToneCurve::Neutral, Auto),
        ("pixel-3d", ToneCurve::Neutral, Fixed),
        ("Pixel 3D", ToneCurve::Neutral, Fixed),
        ("retro-32", ToneCurve::Clamp, Fixed),
        ("hd-2d", ToneCurve::AgxPunchy, Auto),
        ("2d-pixel", ToneCurve::Clamp, Display),
        ("2d-smooth", ToneCurve::Neutral, Display),
        ("1-bit", ToneCurve::Clamp, Fixed),
        ("looks/MyCustom", ToneCurve::AgxPunchy, Auto),
    ];
    for (look, curve, mode) in cases {
        let t = LookTone::for_look(look);
        assert_eq!((t.curve, t.exposure), (curve, mode), "{look}");
        let frame = evaluate(&LightingInputs {
            look: look.into(),
            ..LightingInputs::default()
        });
        assert_eq!(frame.finish.curve, curve, "{look}");
        assert_eq!(frame.auto_exposure.enabled, mode == Auto, "{look}");
        assert_eq!(frame.finish.curve_prescale, mode != Display, "{look}");
    }
    assert_eq!(LookTone::for_look("ink").compensation, 0.3);

    // TonemapperPreset overrides the Look; Default follows it.
    let with = |preset: &str, look: &str| {
        let mut world = metallic_core::World::new_with_services();
        let lighting = world.root("Lighting").expect("lighting");
        let grading = world
            .create("ColorGradingEffect", Some(lighting))
            .expect("grading");
        world
            .set(
                grading,
                "TonemapperPreset",
                metallic_core::Value::String(preset.into()),
            )
            .expect("preset");
        evaluate(&LightingInputs {
            look: look.into(),
            ..LightingInputs::read(&world)
        })
        .finish
        .curve
    };
    assert_eq!(with("Default", "toon"), ToneCurve::Neutral);
    assert_eq!(with("AgXPunchy", "toon"), ToneCurve::AgxPunchy);
    assert_eq!(with("Retro", ""), ToneCurve::Clamp);
    assert_eq!(with("Neutral", "hd-2d"), ToneCurve::Neutral);
}
