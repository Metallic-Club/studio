//! Renderer behaviour that needs a real (or software) adapter: transparent shadows and the MSAA
//! sample count. Skipped when the machine has none.
use metallic_core::{Value, Vec3 as V3, World};
use metallic_render::glam::Vec3;
use metallic_render::{Camera, Quality, RenderContext, RenderSettings, Renderer};

const SIZE: (u32, u32) = (160, 120);
/// Height of the slab whose shadow is measured.
const SLAB_Y: f64 = 25.0;

fn renderer(ctx: &RenderContext, quality: Quality) -> Renderer {
    Renderer::with_settings(
        ctx,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        RenderSettings { quality },
    )
}

/// A large ground plate and, when `slab` is given, a 30x1x30 slab at `SLAB_Y` with that
/// Transparency.
fn shadow_world(slab: Option<f64>) -> World {
    let mut w = World::new_with_services();
    let ws = w.root("Workspace").expect("workspace");
    let ground = w.create("Part", Some(ws)).expect("part");
    w.set(ground, "Size", Value::Vector3(V3::new(400.0, 1.0, 400.0)))
        .expect("size");
    w.set(ground, "Position", Value::Vector3(V3::new(0.0, -0.5, 0.0)))
        .expect("pos");
    if let Some(t) = slab {
        let p = w.create("Part", Some(ws)).expect("part");
        w.set(p, "Size", Value::Vector3(V3::new(30.0, 1.0, 30.0)))
            .expect("size");
        w.set(p, "Position", Value::Vector3(V3::new(0.0, SLAB_Y, 0.0)))
            .expect("pos");
        w.set(p, "Transparency", Value::Number(t))
            .expect("transparency");
    }
    w
}

/// Mean brightness (0..255) of the middle quarter of a frame.
fn centre_brightness(px: &[u8]) -> f32 {
    let (w, h) = SIZE;
    let mut sum = 0.0;
    let mut n = 0.0;
    for y in h * 3 / 8..h * 5 / 8 {
        for x in w * 3 / 8..w * 5 / 8 {
            let i = ((y * w + x) * 4) as usize;
            sum += (f32::from(px[i]) + f32::from(px[i + 1]) + f32::from(px[i + 2])) / 3.0;
            n += 1.0;
        }
    }
    sum / n
}

/// Renders the ground where the slab's shadow falls, from below the slab (so the slab itself is
/// out of view).
fn ground_under_shadow(ctx: &RenderContext, slab: Option<f64>) -> f32 {
    let mut r = renderer(ctx, Quality::Low);
    r.sync(&shadow_world(slab));
    let to_sun = r.sky.to_sun;
    assert!(to_sun.y > 0.2, "the default sun is up: {to_sun:?}");
    // Follow the sun ray from the slab's centre down to the ground.
    let hit = Vec3::ZERO.with_y(SLAB_Y as f32) - to_sun * (SLAB_Y as f32 / to_sun.y);
    let spot = hit.with_y(0.0);
    let cam = Camera {
        position: spot + Vec3::new(0.0, 8.0, 3.0),
        look_at: spot,
        fov_deg: 40.0,
    };
    centre_brightness(&r.capture(&cam, SIZE).expect("capture"))
}

/// Parts below full transparency cast shadows (as in Roblox); Transparency 1 casts none.
#[test]
fn transparent_parts_cast_shadows_until_fully_transparent() {
    let Ok(ctx) = RenderContext::new_headless() else {
        eprintln!("no graphics adapter, skipping");
        return;
    };
    let lit = ground_under_shadow(&ctx, None);
    let opaque = ground_under_shadow(&ctx, Some(0.0));
    let half = ground_under_shadow(&ctx, Some(0.5));
    let invisible = ground_under_shadow(&ctx, Some(1.0));
    eprintln!("lit {lit:.1}, opaque {opaque:.1}, half {half:.1}, invisible {invisible:.1}");
    assert!(opaque < lit * 0.8, "opaque slab casts no shadow");
    assert!(half < lit * 0.8, "Transparency 0.5 casts no shadow");
    assert!(
        (half - opaque).abs() < 2.0,
        "half {half} vs opaque {opaque}"
    );
    assert!(
        (invisible - lit).abs() < 1.0,
        "Transparency 1 casts a shadow"
    );
}

/// The MSAA sample count is the preset's, lowered to what the formats support, and a frame at
/// that count renders.
#[test]
fn msaa_sample_count_is_supported() {
    let Ok(ctx) = RenderContext::new_headless() else {
        eprintln!("no graphics adapter, skipping");
        return;
    };
    let mut r = renderer(&ctx, Quality::Low);
    assert_eq!(r.msaa_samples(), 1);
    r.set_settings(RenderSettings {
        quality: Quality::High,
    });
    let high = r.msaa_samples();
    assert!([1, 2, 4].contains(&high), "High uses {high}x");
    r.sync(&shadow_world(Some(0.5)));
    let cam = Camera::default();
    assert!(r.capture(&cam, SIZE).is_ok());
    // The 8-bit fallback target checks its own support and keeps a valid count.
    r.force_ldr_scene_target();
    assert!([1, 2, 4].contains(&r.msaa_samples()));
    assert!(r.capture(&cam, SIZE).is_ok());
}
