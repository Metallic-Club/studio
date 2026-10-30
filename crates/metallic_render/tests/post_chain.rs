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
    let ctx = RenderContext::new_headless().ok()?;
    let mut r = Renderer::with_settings(
        &ctx,
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
    Some(r)
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
    for quality in [Quality::Low, Quality::High] {
        let Some(mut r) = flat_sky(quality, 0.18) else {
            return;
        };
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
}
