//! Shadow cascade choice on every backend the machine has (#426). The fragment `position.w`
//! builtin reads 1/w on Vulkan but w on DX12, so a cascade chosen from it lands in cascade 0 on
//! DX12 (and WARP) and far shadows vanish. The view depth now comes from the vertex stage; this
//! checks shadows on both sides of each cascade split, and that the backends agree.
use metallic_core::{Color3, Value, Vec3 as V3, World};
use metallic_render::glam::{Vec3, Vec4};
use metallic_render::shadows::{SHADOW_DISTANCE, cascade_splits};
use metallic_render::{
    Camera, LightingInputs, Quality, RenderContext, RenderSettings, Renderer, camera,
};
use std::sync::{Mutex, MutexGuard};

const SIZE: (u32, u32) = (480, 300);
/// Cascades at `Quality::High`.
const CASCADES: usize = 3;

fn gpu() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn add(w: &mut World, size: [f64; 3], pos: [f64; 3], color: [f64; 3]) {
    let ws = w.root("Workspace").expect("workspace");
    let id = w.create("Part", Some(ws)).expect("part");
    let v = |a: [f64; 3]| Value::Vector3(V3::new(a[0], a[1], a[2]));
    w.set(id, "Size", v(size)).expect("size");
    w.set(id, "Position", v(pos)).expect("pos");
    w.set(
        id,
        "Color",
        Value::Color3(Color3::new(color[0], color[1], color[2])),
    )
    .expect("color");
    w.set(id, "Material", Value::String("SmoothPlastic".into()))
        .expect("material");
}

/// Looking down the -Z axis from 10 studs up, 9 degrees below level.
fn cam() -> Camera {
    Camera {
        position: Vec3::new(0.0, 10.0, 0.0),
        look_at: Vec3::new(0.0, 0.0, -63.0),
        fov_deg: 50.0,
    }
}

/// Horizontal distances along -Z of floor points at the given view depths.
fn floor_distance(c: &Camera, view_depth: f32) -> f32 {
    let f = (c.look_at - c.position).normalize();
    // depth = dot((0, -10, -d), f) = -10 f.y - d f.z.
    (view_depth + c.position.y * f.y) / -f.z
}

/// Floor points to test: either side of each cascade split, and well inside the last cascade.
fn samples() -> Vec<f32> {
    let splits = cascade_splits(camera::NEAR, SHADOW_DISTANCE.min(camera::FAR), CASCADES);
    let mut depths = vec![splits[0] * 0.6];
    for s in &splits[..CASCADES - 1] {
        depths.push(s * 0.93);
        depths.push(s * 1.07);
    }
    depths.push((splits[CASCADES - 2] + splits[CASCADES - 1]) * 0.5);
    depths
}

/// A wide floor, and when `roofs` a roof 2 studs above the camera over every sample: high
/// enough not to hide the floor, low enough that its shadow covers the sample whatever the
/// sun's angle.
fn world(roofs: bool) -> World {
    let mut w = World::new_with_services();
    add(&mut w, [400.0, 1.0, 900.0], [0.0, -0.5, -440.0], [160.0; 3]);
    if roofs {
        let c = cam();
        for depth in samples() {
            let d = floor_distance(&c, depth) as f64;
            add(&mut w, [36.0, 1.0, 50.0], [0.0, 12.5, -d], [90.0; 3]);
        }
    }
    w
}

fn renderer(ctx: &RenderContext) -> Renderer {
    let mut r = Renderer::with_settings(
        ctx,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        RenderSettings {
            quality: Quality::High,
        },
    );
    light(&mut r);
    r.finish.dither = false;
    r
}

/// Noon sun only (no sky or ambient light), so a shadow reads as a clear drop. Applied after
/// each `sync`, which reads the world's `Lighting`.
fn light(r: &mut Renderer) {
    r.set_lighting(&LightingInputs {
        clock_time: 12.0,
        auto_exposure: false,
        ambient: [0.0; 3],
        outdoor_ambient: [0.0; 3],
        environment_diffuse: 0.0,
        environment_specular: 0.0,
        fog_start: 5000.0,
        fog_end: 10000.0,
        ..LightingInputs::default()
    });
}

fn luma(p: &[u8], (x, y): (u32, u32)) -> f32 {
    let i = ((y * SIZE.0 + x) * 4) as usize;
    0.2126 * p[i] as f32 + 0.7152 * p[i + 1] as f32 + 0.0722 * p[i + 2] as f32
}

/// The pixel showing floor point (0, 0, -d).
fn pixel(c: &Camera, d: f32) -> (u32, u32) {
    let clip = c.view_proj(SIZE.0 as f32 / SIZE.1 as f32) * Vec4::new(0.0, 0.0, -d, 1.0);
    let ndc = clip.truncate() / clip.w;
    let x = (ndc.x * 0.5 + 0.5) * SIZE.0 as f32;
    let y = (0.5 - ndc.y * 0.5) * SIZE.1 as f32;
    assert!(
        (0.0..SIZE.0 as f32).contains(&x) && (0.0..SIZE.1 as f32).contains(&y),
        "floor at {d} is off screen"
    );
    (x as u32, y as u32)
}

/// Shadow ratio (lit by roofs / open sky) at every sample, for one device.
fn ratios(ctx: &RenderContext) -> Vec<f32> {
    let c = cam();
    let mut r = renderer(ctx);
    r.sync(&world(false));
    light(&mut r);
    let open = r.capture(&c, SIZE).expect("capture");
    r.sync(&world(true));
    light(&mut r);
    let roofed = r.capture(&c, SIZE).expect("capture");
    samples()
        .into_iter()
        .map(|depth| {
            let p = pixel(&c, floor_distance(&c, depth));
            luma(&roofed, p) / luma(&open, p).max(1.0)
        })
        .collect()
}

#[test]
fn shadows_hold_across_cascade_splits_on_every_backend() {
    let _gpu = gpu();
    let mut devices: Vec<(String, RenderContext)> = Vec::new();
    for (name, backends) in [
        ("Vulkan", wgpu::Backends::VULKAN),
        ("DX12", wgpu::Backends::DX12),
        ("Metal", wgpu::Backends::METAL),
    ] {
        if let Ok(ctx) = RenderContext::new_headless_with_backends(backends) {
            devices.push((format!("{name} ({})", ctx.adapter_name), ctx));
        }
    }
    if let Ok(ctx) = RenderContext::new_headless_software() {
        devices.push((format!("software ({})", ctx.adapter_name), ctx));
    }
    if devices.is_empty() {
        eprintln!("no graphics adapter, skipping");
        return;
    }
    let depths = samples();
    let mut all = Vec::new();
    for (name, ctx) in &devices {
        let rs = ratios(ctx);
        eprintln!("{name}: {rs:?} at view depths {depths:?}");
        for (depth, ratio) in depths.iter().zip(&rs) {
            assert!(
                *ratio < 0.8,
                "{name}: floor at view depth {depth:.1} is not shadowed by its roof ({ratio:.2})"
            );
        }
        all.push((name, rs));
    }
    // Every backend picks the same cascades, so the shadow strength matches.
    let (first, base) = &all[0];
    for (name, rs) in &all[1..] {
        for ((depth, a), b) in depths.iter().zip(base).zip(rs) {
            assert!(
                (a - b).abs() < 0.1,
                "view depth {depth:.1}: {first} {a:.2} vs {name} {b:.2}"
            );
        }
    }
}
