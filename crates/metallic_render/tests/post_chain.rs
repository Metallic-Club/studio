//! The HDR target, the finish pass and the post chain on a real (or software) adapter.
//! Skipped when the machine has none.
use metallic_render::glam::Vec3;
use metallic_render::{
    Camera, FinishSettings, FullscreenPass, LightingInputs, PostContext, PostPass, PostSlot,
    Quality, RenderContext, RenderSettings, Renderer, ToneCurve,
};

const SIZE: (u32, u32) = (64, 48);

/// A renderer whose whole view is a flat sky of linear `value` (no sun disc, no parts).
fn flat_sky(quality: Quality, value: f32) -> Option<Renderer> {
    Some(flat_sky_on(
        &RenderContext::new_headless().ok()?,
        quality,
        value,
    ))
}

/// [`flat_sky`] on a given device.
fn flat_sky_on(ctx: &RenderContext, quality: Quality, value: f32) -> Renderer {
    let mut r = Renderer::with_settings(
        ctx,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        RenderSettings { quality },
    );
    // These tests check the finish pass and the chain alone: no lighting curve, grade, fog or
    // bloom.
    r.set_lighting(&LightingInputs {
        atmosphere: None,
        ..LightingInputs::default()
    });
    r.finish = FinishSettings::default();
    r.post_mut().remove("bloom");
    r.sky.zenith = [value; 3];
    r.sky.horizon = [value; 3];
    r.sky.ground = [value; 3];
    r.sky.to_sun = Vec3::new(0.0, -1.0, 0.0);
    r
}

fn camera() -> Camera {
    Camera {
        position: Vec3::new(0.0, 5.0, 0.0),
        look_at: Vec3::new(0.0, 5.0, -10.0),
        fov_deg: 60.0,
    }
}

fn mean(px: &[u8], ch: usize) -> f32 {
    let p = px.as_chunks::<4>().0;
    p.iter().map(|p| f32::from(p[ch])).sum::<f32>() / p.len() as f32
}

#[test]
fn grey_lands_on_one_value_under_every_curve() {
    let Some(mut r) = flat_sky(Quality::Low, 0.18) else {
        return;
    };
    let mut first = None;
    for curve in ToneCurve::ALL {
        r.finish.curve = curve;
        let px = r.capture(&camera(), SIZE).expect("capture");
        let m = mean(&px, 1);
        let reference = *first.get_or_insert(m);
        assert!(
            (m - reference).abs() <= 1.0,
            "{curve:?}: mean {m} vs {reference}"
        );
        assert!((m - 141.0).abs() <= 1.5, "{curve:?}: grey at {m}");
    }
}

#[test]
fn dither_off_gives_a_flat_image_and_exposure_scales_light() {
    let Some(mut r) = flat_sky(Quality::High, 0.18) else {
        return;
    };
    r.finish.dither = false;
    let px = r.capture(&camera(), SIZE).expect("capture");
    let p = px.as_chunks::<4>().0;
    assert!(p.iter().all(|q| q == &p[0]), "flat sky is not flat");
    assert!(p.iter().all(|q| q[3] == 255));
    r.finish.exposure = 4.0;
    let brighter = r.capture(&camera(), SIZE).expect("capture");
    assert!(brighter[1] > px[1] + 40, "{} vs {}", brighter[1], px[1]);
}

/// The pre-#62 output for linear scene colour `c`, computed on the CPU: exposure, the
/// Narkowicz curve, sRGB encoding, no dither.
fn old_output(c: [f32; 3], exposure: f32) -> [u8; 3] {
    c.map(|v| {
        let x = (v * exposure).max(0.0);
        let mapped = ((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14)).clamp(0.0, 1.0);
        let encoded = if mapped <= 0.003_130_8 {
            mapped * 12.92
        } else {
            1.055 * mapped.powf(1.0 / 2.4) - 0.055
        };
        (encoded * 255.0).round() as u8
    })
}

/// With an empty chain and default finish settings, the image matches the old in-shader tone
/// mapping (CPU reference) within 1/255 across the range, in grey and in colour.
#[test]
fn empty_chain_matches_the_old_output() {
    let Some(mut r) = flat_sky(Quality::High, 0.18) else {
        return;
    };
    assert!(r.post().is_empty());
    r.finish.dither = false;
    let exposure = r.sky.exposure;
    let colours = [
        [0.002, 0.002, 0.002],
        [0.02, 0.02, 0.02],
        [0.18, 0.18, 0.18],
        [0.6, 0.6, 0.6],
        [1.5, 1.5, 1.5],
        [6.0, 6.0, 6.0],
        [0.9, 0.25, 0.04],
        [0.05, 0.3, 1.2],
        [2.0, 1.4, 0.2],
    ];
    for c in colours {
        r.sky.zenith = c;
        r.sky.horizon = c;
        r.sky.ground = c;
        let px = r.capture(&camera(), SIZE).expect("capture");
        let want = old_output(c, exposure);
        for p in px.as_chunks::<4>().0 {
            for ch in 0..3 {
                let d = (i32::from(p[ch]) - i32::from(want[ch])).abs();
                assert!(d <= 1, "{c:?}: got {:?}, old {want:?}", &p[..3]);
            }
        }
    }
    // Dither on (the default) stays within one step of the reference too.
    r.finish.dither = true;
    let c = [0.18; 3];
    r.sky.zenith = c;
    r.sky.horizon = c;
    r.sky.ground = c;
    let px = r.capture(&camera(), SIZE).expect("capture");
    let want = old_output(c, exposure);
    for p in px.as_chunks::<4>().0 {
        assert!((i32::from(p[1]) - i32::from(want[1])).abs() <= 1);
    }
}

#[test]
fn grade_controls_change_the_image() {
    let Some(mut r) = flat_sky(Quality::Low, 0.18) else {
        return;
    };
    r.sky.zenith = [0.3, 0.1, 0.05];
    r.sky.horizon = [0.3, 0.1, 0.05];
    r.sky.ground = [0.3, 0.1, 0.05];
    r.finish.dither = false;
    let base = r.capture(&camera(), SIZE).expect("capture");
    r.finish.saturation = -1.0;
    let grey = r.capture(&camera(), SIZE).expect("capture");
    assert!(
        (i32::from(grey[0]) - i32::from(grey[2])).abs() <= 1,
        "{grey:?}"
    );
    r.finish.saturation = 0.0;
    r.finish.tint = [0.0, 1.0, 1.0];
    let tinted = r.capture(&camera(), SIZE).expect("capture");
    assert!(tinted[0] < base[0] / 4, "tint kept red");
    r.finish.tint = [1.0; 3];
    r.finish.vignette = 1.0;
    let v = r.capture(&camera(), SIZE).expect("capture");
    let centre = ((SIZE.1 / 2 * SIZE.0 + SIZE.0 / 2) * 4) as usize;
    assert!(v[0] < base[0], "corner not darker");
    assert!(v[centre] + 2 >= base[centre], "centre darkened");
}

/// An HDR pass that replaces the image with linear `colour`.
struct Fill {
    pass: FullscreenPass,
    colour: [f32; 4],
    saw_depth: bool,
}

impl Fill {
    fn new(colour: [f32; 4]) -> Fill {
        let src = "struct P { c: vec4<f32> }
@group(0) @binding(3) var<uniform> params: P;
@fragment fn fs_main(in: PostIn) -> @location(0) vec4<f32> {
    let keep = textureSampleLevel(post_input, post_sampler, in.uv, 0.0).rgb * 0.0;
    return vec4<f32>(post_output(params.c.rgb + keep), 1.0);
}";
        Fill {
            pass: FullscreenPass::new("test fill", src, 16),
            colour,
            saw_depth: false,
        }
    }
}

impl PostPass for Fill {
    fn name(&self) -> &str {
        "fill"
    }
    fn slot(&self) -> PostSlot {
        PostSlot::Hdr
    }
    fn reads_depth(&self) -> bool {
        true
    }
    fn record(&mut self, ctx: &mut PostContext<'_>) {
        self.saw_depth = ctx.depth.is_some();
        let c = self.colour;
        self.pass.draw(ctx, bytemuck::cast_slice(&c));
    }
}

/// An LDR pass that inverts the encoded image (`invert`) or halves it.
struct Ldr {
    name: &'static str,
    pass: FullscreenPass,
}

impl Ldr {
    fn new(name: &'static str, expr: &str) -> Ldr {
        let src = format!(
            "@fragment fn fs_main(in: PostIn) -> @location(0) vec4<f32> {{
    let c = textureSampleLevel(post_input, post_sampler, in.uv, 0.0).rgb;
    return vec4<f32>(post_output({expr}), 1.0);
}}"
        );
        Ldr {
            name,
            pass: FullscreenPass::new(name, &src, 0),
        }
    }
}

impl PostPass for Ldr {
    fn name(&self) -> &str {
        self.name
    }
    fn slot(&self) -> PostSlot {
        PostSlot::Ldr
    }
    fn record(&mut self, ctx: &mut PostContext<'_>) {
        self.pass.draw(ctx, &[]);
    }
}

fn centre(px: &[u8]) -> [i32; 3] {
    let i = ((SIZE.1 / 2 * SIZE.0 + SIZE.0 / 2) * 4) as usize;
    [px[i], px[i + 1], px[i + 2]].map(i32::from)
}

fn near(a: [i32; 3], b: [i32; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() <= 2)
}

#[test]
fn passes_run_in_slot_order_around_the_finish_pass() {
    let Ok(ctx) = RenderContext::new_headless() else {
        return;
    };
    check_chain_order(&ctx);
}

fn check_chain_order(ctx: &RenderContext) {
    for quality in [Quality::Low, Quality::High] {
        let mut r = flat_sky_on(ctx, quality, 0.18);
        check_chain_order_on(&mut r, quality);
    }
}

/// The chain order checks on `r` (a flat 0.18 sky).
fn check_chain_order_on(r: &mut Renderer, quality: Quality) {
    r.sky.zenith = [0.18; 3];
    r.sky.horizon = [0.18; 3];
    r.sky.ground = [0.18; 3];
    r.finish.dither = true;
    // Linear red 1.0 through the default curve: Filmic(1.0) = 0.8037, sRGB 231.
    r.post_mut().push(Box::new(Fill::new([1.0, 0.0, 0.0, 0.0])));
    let red = centre(&r.capture(&camera(), SIZE).expect("capture"));
    assert!(near(red, [231, 0, 0]), "{quality:?} hdr fill: {red:?}");
    assert!(
        r.post_mut()
            .get_mut::<Fill>("fill")
            .expect("fill")
            .saw_depth,
        "depth not offered"
    );

    r.post_mut().push(Box::new(Ldr::new("invert", "1.0 - c")));
    let inv = centre(&r.capture(&camera(), SIZE).expect("capture"));
    assert!(near(inv, [24, 255, 255]), "{quality:?} invert: {inv:?}");

    // Two LDR passes: invert, then halve (ping-pong through both intermediates).
    r.post_mut().push(Box::new(Ldr::new("halve", "c * 0.5")));
    let half = centre(&r.capture(&camera(), SIZE).expect("capture"));
    assert!(near(half, [12, 128, 128]), "{quality:?} halve: {half:?}");
    assert_eq!(
        r.post().names(PostSlot::Ldr).collect::<Vec<_>>(),
        ["invert", "halve"]
    );

    r.post_mut().remove("invert");
    r.post_mut().remove("fill");
    let only_half = centre(&r.capture(&camera(), SIZE).expect("capture"));
    assert!(near(only_half, [70, 70, 70]), "{quality:?}: {only_half:?}");
}

/// The GL backend (when the machine has one) renders through the same HDR target, finish pass
/// and post chain: the flat grey matches the old output and the chain test passes, including a
/// pass that reads the multisampled depth.
#[test]
fn gl_backend_renders_the_hdr_chain() {
    let Ok(ctx) = RenderContext::new_headless_with_backends(wgpu::Backends::GL) else {
        eprintln!("no GL adapter, skipping");
        return;
    };
    let mut r = flat_sky_on(&ctx, Quality::High, 0.18);
    eprintln!(
        "GL adapter {}: HDR target {:?}",
        ctx.adapter_name,
        r.hdr_format()
    );
    r.finish.dither = false;
    let px = r.capture(&camera(), SIZE).expect("capture");
    let want = old_output([0.18; 3], r.sky.exposure);
    assert!(
        (i32::from(px[1]) - i32::from(want[1])).abs() <= 1,
        "{:?} vs {want:?}",
        &px[..3]
    );
    // A pass that reads depth (sampleable multisampled depth) and copies the image through.
    r.post_mut().push(Box::new(DepthCopy(FullscreenPass::new(
        "test depth copy",
        "@fragment fn fs_main(in: PostIn) -> @location(0) vec4<f32> {
    return vec4<f32>(post_output(textureLoad(post_input, vec2<i32>(in.clip.xy), 0).rgb), 1.0);
}",
        0,
    ))));
    let px = r.capture(&camera(), SIZE).expect("capture");
    assert!(
        (i32::from(px[1]) - i32::from(want[1])).abs() <= 1,
        "with depth read: {:?} vs {want:?}",
        &px[..3]
    );
    check_chain_order(&ctx);
}

/// Copies the HDR image through and asks for depth.
struct DepthCopy(FullscreenPass);

impl PostPass for DepthCopy {
    fn name(&self) -> &str {
        "depth copy"
    }
    fn slot(&self) -> PostSlot {
        PostSlot::Hdr
    }
    fn reads_depth(&self) -> bool {
        true
    }
    fn record(&mut self, ctx: &mut PostContext<'_>) {
        assert!(ctx.depth.is_some());
        self.0.draw(ctx, &[]);
    }
}

/// The 8-bit fallback scene target (adapters without float render targets) still renders the
/// chain: mid-grey within the fallback's quantisation, light above 1 clipped (not wrapped or
/// black), and passes still run.
#[test]
fn ldr_scene_target_fallback_renders() {
    let Ok(ctx) = RenderContext::new_headless() else {
        return;
    };
    for quality in [Quality::Low, Quality::High] {
        let mut r = flat_sky_on(&ctx, quality, 0.18);
        r.force_ldr_scene_target();
        assert_eq!(r.hdr_format(), metallic_render::post::LDR_FORMAT);
        r.finish.dither = false;
        let px = r.capture(&camera(), SIZE).expect("capture");
        let want = old_output([0.18; 3], r.sky.exposure);
        assert!(
            (i32::from(px[1]) - i32::from(want[1])).abs() <= 4,
            "{quality:?}: {:?} vs {want:?}",
            &px[..3]
        );
        // Linear 4.0 is stored as 1.0: the result is the curve at 1.0 x exposure.
        r.sky.zenith = [4.0; 3];
        r.sky.horizon = [4.0; 3];
        r.sky.ground = [4.0; 3];
        let px = r.capture(&camera(), SIZE).expect("capture");
        let clipped = old_output([1.0; 3], r.sky.exposure);
        assert!(
            (i32::from(px[1]) - i32::from(clipped[1])).abs() <= 2,
            "{quality:?}: {:?} vs {clipped:?}",
            &px[..3]
        );
        check_chain_order_on(&mut r, quality);
    }
}
