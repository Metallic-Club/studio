//! Post stack v1 (#65) on a real (or software) adapter: LUT grading, edge outlines and the
//! morphological anti-aliasing. Skipped when the machine has none. The tests take turns behind
//! one lock: GL devices deadlock in parallel.
use metallic_core::{Color3, Value, Vec3 as V3, World};
use metallic_render::glam::{Mat4, Vec3};
use metallic_render::metallic_asset::{Pose, SkinnedModel};
use metallic_render::{
    Camera, EDGE_PASS, EdgePass, LightingInputs, Lut3d, Quality, RenderContext, RenderSettings,
    Renderer, SMAA_PASS, SkinStyle, SkinnedInstance, SmaaPass,
};
use std::sync::{Arc, Mutex, MutexGuard};

const SIZE: (u32, u32) = (320, 200);

fn gpu() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn add(w: &mut World, size: [f64; 3], pos: [f64; 3], yaw: f64, color: [f64; 3]) {
    let ws = w.root("Workspace").expect("workspace");
    let id = w.create("Part", Some(ws)).expect("part");
    let v = |a: [f64; 3]| Value::Vector3(V3::new(a[0], a[1], a[2]));
    w.set(id, "Size", v(size)).expect("size");
    w.set(id, "Position", v(pos)).expect("pos");
    w.set(id, "Orientation", v([0.0, yaw, 0.0])).expect("yaw");
    w.set(
        id,
        "Color",
        Value::Color3(Color3::new(color[0], color[1], color[2])),
    )
    .expect("color");
    w.set(id, "Material", Value::String("SmoothPlastic".into()))
        .expect("material");
}

/// A large floor and a box turned 30 degrees, seen from above and to the side.
fn scene() -> World {
    let mut w = World::new_with_services();
    add(
        &mut w,
        [200.0, 1.0, 200.0],
        [0.0, -0.5, 0.0],
        0.0,
        [150.0; 3],
    );
    add(
        &mut w,
        [6.0, 6.0, 6.0],
        [0.0, 3.0, 0.0],
        30.0,
        [200.0, 120.0, 60.0],
    );
    w
}

fn camera() -> Camera {
    Camera {
        position: Vec3::new(10.0, 9.0, 16.0),
        look_at: Vec3::new(0.0, 2.0, 0.0),
        fov_deg: 55.0,
    }
}

fn renderer(ctx: &RenderContext, quality: Quality) -> Renderer {
    let mut r = Renderer::with_settings(
        ctx,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        RenderSettings { quality },
    );
    r.set_lighting(&LightingInputs {
        auto_exposure: false,
        ..LightingInputs::default()
    });
    r.finish.dither = false;
    r
}

fn edges(r: &mut Renderer) -> &mut EdgePass {
    r.post_mut()
        .get_mut::<EdgePass>(EDGE_PASS)
        .expect("edge pass")
}

fn px(p: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * SIZE.0 + x) * 4) as usize;
    [p[i], p[i + 1], p[i + 2]]
}

fn luma(c: [u8; 3]) -> f32 {
    0.2126 * c[0] as f32 + 0.7152 * c[1] as f32 + 0.0722 * c[2] as f32
}

fn changed(a: &[u8], b: &[u8]) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for y in 0..SIZE.1 {
        for x in 0..SIZE.0 {
            if px(a, x, y) != px(b, x, y) {
                out.push((x, y));
            }
        }
    }
    out
}

#[test]
fn lut_grades_display_colours_by_strength() {
    let _gpu = gpu();
    let Ok(ctx) = RenderContext::new_headless() else {
        eprintln!("no graphics adapter, skipping");
        return;
    };
    let mut r = renderer(&ctx, Quality::High);
    r.sync(&scene());
    let plain = r.capture(&camera(), SIZE).expect("capture");
    // An identity table changes nothing but filtering rounding.
    r.set_lut(Some(&Lut3d::identity(17)), 1.0);
    let ident = r.capture(&camera(), SIZE).expect("capture");
    let worst = plain
        .iter()
        .zip(&ident)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0);
    assert!(worst <= 1, "identity LUT moved a channel by {worst}");
    // Swap red and blue: the orange box turns blue, half way at strength 0.5.
    let swap = Lut3d::from_fn(17, |c| [c[2], c[1], c[0]]);
    r.set_lut(Some(&swap), 1.0);
    let full = r.capture(&camera(), SIZE).expect("capture");
    r.set_lut(None, 0.5);
    let half = r.capture(&camera(), SIZE).expect("capture");
    let (x, y) = (160, 110);
    let (p, f, h) = (px(&plain, x, y), px(&full, x, y), px(&half, x, y));
    assert!(p[0] > p[2] + 40, "box is orange: {p:?}");
    assert!(
        f[0].abs_diff(p[2]) <= 2 && f[2].abs_diff(p[0]) <= 2,
        "{p:?} -> {f:?}"
    );
    let mid = |a: u8, b: u8| ((a as i32 + b as i32) / 2) as u8;
    assert!(h[0].abs_diff(mid(p[0], p[2])) <= 2, "half: {h:?}");
    // Strength 0 turns it off.
    r.set_lut(None, 0.0);
    assert_eq!(r.capture(&camera(), SIZE).expect("capture"), plain);
}

#[test]
fn edges_outline_silhouettes_and_creases_but_not_planes_or_sky() {
    let _gpu = gpu();
    let Ok(ctx) = RenderContext::new_headless() else {
        eprintln!("no graphics adapter, skipping");
        return;
    };
    // MSAA 4x (High) and none (Low): the prepass is single-sampled either way.
    for quality in [Quality::High, Quality::Low] {
        let mut r = renderer(&ctx, quality);
        r.post_mut().remove(SMAA_PASS);
        r.post_mut().remove("bloom");
        r.sync(&scene());
        let off = r.capture(&camera(), SIZE).expect("capture");
        edges(&mut r).enabled = true;
        let on = r.capture(&camera(), SIZE).expect("capture");
        let diff = changed(&off, &on);
        // Lines exist, are darker, and stay a small share of the image.
        assert!(diff.len() > 150, "{quality:?}: {} line pixels", diff.len());
        assert!(diff.len() < 3000, "{quality:?}: {} line pixels", diff.len());
        for &(x, y) in &diff {
            assert!(
                luma(px(&on, x, y)) <= luma(px(&off, x, y)) + 0.5,
                "{quality:?}: line pixel at ({x}, {y}) got lighter"
            );
        }
        // The floor far from the box (bottom rows) and the sky (top rows) are untouched,
        // although the floor is seen at a grazing angle toward the horizon.
        for &(x, y) in &diff {
            assert!(y > 8 && y < SIZE.1 - 8, "{quality:?}: line at ({x}, {y})");
        }
        // Rebuilding targets (a resize) keeps working.
        let small = r.capture(&camera(), (160, 100)).expect("capture");
        assert_eq!(small.len(), 160 * 100 * 4);
    }
}

fn avatar_world_and_shot(
    r: &mut Renderer,
    model: &Arc<SkinnedModel>,
    joints: &[Mat4],
    style: SkinStyle,
) -> Vec<u8> {
    r.set_skinned(&[SkinnedInstance {
        model,
        joints,
        slot_colors: &[],
        center: Vec3::new(0.0, 2.75, 0.0),
        height: 5.5,
        detail_scale: 1.0 / 0.28,
        style,
    }]);
    let cam = Camera {
        position: Vec3::new(0.0, 3.0, 9.0),
        look_at: Vec3::new(0.0, 2.75, 0.0),
        fov_deg: 50.0,
    };
    r.capture(&cam, SIZE).expect("capture")
}

#[test]
fn toon_characters_keep_one_outline() {
    let _gpu = gpu();
    let Ok(ctx) = RenderContext::new_headless() else {
        eprintln!("no graphics adapter, skipping");
        return;
    };
    let glb = include_bytes!("../../../art/avatar/out/default_avatar.glb");
    let model = Arc::new(SkinnedModel::from_glb(glb).expect("avatar"));
    let mut joints = Vec::new();
    Pose::rest(&model.skeleton).skin_matrices(
        &model.skeleton,
        Mat4::from_scale(Vec3::splat(1.0 / 0.28)),
        &mut joints,
    );
    let mut r = renderer(&ctx, Quality::High);
    r.post_mut().remove("bloom");
    // The figure stands on a floor, in front of a wall.
    let mut w = World::new_with_services();
    add(&mut w, [60.0, 1.0, 60.0], [0.0, -0.5, 0.0], 0.0, [150.0; 3]);
    add(
        &mut w,
        [30.0, 20.0, 1.0],
        [0.0, 10.0, -6.0],
        0.0,
        [120.0, 140.0, 170.0],
    );
    r.sync(&w);
    let cam = Camera {
        position: Vec3::new(0.0, 3.0, 9.0),
        look_at: Vec3::new(0.0, 2.75, 0.0),
        fov_deg: 50.0,
    };
    r.set_skinned(&[]);
    let empty_off = r.capture(&cam, SIZE).expect("capture");
    let toon_off = avatar_world_and_shot(&mut r, &model, &joints, SkinStyle::Toon);
    let std_off = avatar_world_and_shot(&mut r, &model, &joints, SkinStyle::Standard);
    edges(&mut r).enabled = true;
    r.set_skinned(&[]);
    let empty_on = r.capture(&cam, SIZE).expect("capture");
    let toon_on = avatar_world_and_shot(&mut r, &model, &joints, SkinStyle::Toon);
    let std_on = avatar_world_and_shot(&mut r, &model, &joints, SkinStyle::Standard);
    // The toon figure keeps only its hull line: post edges add nothing on or around it, only
    // the floor-wall crease they draw without the figure too.
    let scene_lines = changed(&empty_off, &empty_on);
    assert!(!scene_lines.is_empty(), "the floor-wall crease");
    let toon_diff = changed(&toon_off, &toon_on);
    let std_diff = changed(&std_off, &std_on);
    let extra = |d: &[(u32, u32)]| d.iter().filter(|p| !scene_lines.contains(p)).count();
    assert_eq!(extra(&toon_diff), 0, "post edges on the toon figure");
    // A standard figure has no hull, so post edges outline it.
    assert!(extra(&std_diff) > 100, "standard: {}", extra(&std_diff));
}

#[test]
fn anti_aliasing_smooths_stair_steps_where_msaa_is_off() {
    let _gpu = gpu();
    let Ok(ctx) = RenderContext::new_headless() else {
        eprintln!("no graphics adapter, skipping");
        return;
    };
    let mut r = renderer(&ctx, Quality::Low);
    r.post_mut().remove("bloom");
    assert!(
        r.post_mut()
            .get_mut::<SmaaPass>(SMAA_PASS)
            .expect("aa pass")
            .enabled,
        "on by default without MSAA"
    );
    r.sync(&scene());
    let aa = r.capture(&camera(), SIZE).expect("capture");
    r.post_mut()
        .get_mut::<SmaaPass>(SMAA_PASS)
        .expect("aa")
        .enabled = false;
    let raw = r.capture(&camera(), SIZE).expect("capture");
    // The extra 8-bit LDR target rounds some pixels by one level; smoothing moves more.
    let diff: Vec<_> = changed(&raw, &aa)
        .into_iter()
        .filter(|&(x, y)| {
            let (a, b) = (px(&raw, x, y), px(&aa, x, y));
            (0..3).any(|i| a[i].abs_diff(b[i]) > 1)
        })
        .collect();
    assert!(diff.len() > 50, "only {} pixels smoothed", diff.len());
    assert!(diff.len() < 2500, "{} pixels changed", diff.len());
    // Smoothing only mixes neighbours: every changed pixel lies between its neighbours' range.
    for &(x, y) in &diff {
        if x == 0 || y == 0 || x + 1 == SIZE.0 || y + 1 == SIZE.1 {
            continue;
        }
        let around = [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1), (x, y)]
            .map(|(a, b)| luma(px(&raw, a, b)));
        let lo = around.iter().copied().fold(f32::MAX, f32::min);
        let hi = around.iter().copied().fold(f32::MIN, f32::max);
        let l = luma(px(&aa, x, y));
        assert!(
            l >= lo - 1.0 && l <= hi + 1.0,
            "({x}, {y}): {l} outside {lo}..{hi}"
        );
    }
    // Edges get softer: summed over the smoothed pixels, the largest step to any neighbour drops.
    let worst = |p: &[u8]| {
        let mut m = 0.0f32;
        for &(x, y) in &diff {
            if x == 0 || y == 0 || x + 1 == SIZE.0 || y + 1 == SIZE.1 {
                continue;
            }
            let l = luma(px(p, x, y));
            m += [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)]
                .map(|(a, b)| (luma(px(p, a, b)) - l).abs())
                .into_iter()
                .fold(0.0, f32::max);
        }
        m
    };
    let (soft, hard) = (worst(&aa), worst(&raw));
    assert!(
        soft < 0.8 * hard,
        "AA did not soften the edges: {soft} vs {hard}"
    );
    // High (MSAA 4x) leaves it off.
    let hi = renderer(&ctx, Quality::High);
    let mut hi = hi;
    assert!(
        !hi.post_mut()
            .get_mut::<SmaaPass>(SMAA_PASS)
            .expect("aa")
            .enabled
    );
}

#[test]
fn gpu_timing_measures_a_frame_where_supported() {
    let _gpu = gpu();
    let Ok(ctx) = RenderContext::new_headless() else {
        eprintln!("no graphics adapter, skipping");
        return;
    };
    let mut r = renderer(&ctx, Quality::High);
    r.sync(&scene());
    assert_eq!(r.read_gpu_time_ms(), None, "off by default");
    if !r.enable_gpu_timing() {
        eprintln!("no timestamp queries on {}, skipping", ctx.adapter_name);
        return;
    }
    assert_eq!(r.read_gpu_time_ms(), None, "no frame yet");
    r.capture(&camera(), SIZE).expect("capture");
    let ms = r.read_gpu_time_ms().expect("a timed frame");
    assert!(ms > 0.0 && ms < 1000.0, "{ms} ms");
    r.capture(&camera(), SIZE).expect("capture");
    assert!(r.read_gpu_time_ms().is_some());
}
