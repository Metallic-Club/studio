//! Renders small scenes on a real (or software) adapter. Skipped when the machine has none.
use metallic_core::{Color3, Value, Vec3 as V3, World};
use metallic_render::glam::Vec3;
use metallic_render::{Camera, Quality, RenderContext, RenderSettings, Renderer};

const SIZE: (u32, u32) = (320, 200);

fn scene() -> World {
    let mut w = World::new_with_services();
    let ws = w.root("Workspace").expect("workspace");
    let ground = w.create("Part", Some(ws)).expect("part");
    w.set(ground, "Size", Value::Vector3(V3::new(200.0, 1.0, 200.0)))
        .expect("size");
    w.set(ground, "Position", Value::Vector3(V3::new(0.0, -0.5, 0.0)))
        .expect("pos");
    let cube = w.create("Part", Some(ws)).expect("part");
    w.set(cube, "Size", Value::Vector3(V3::new(6.0, 6.0, 6.0)))
        .expect("size");
    w.set(cube, "Position", Value::Vector3(V3::new(0.0, 3.0, 0.0)))
        .expect("pos");
    w.set(
        cube,
        "Color",
        Value::Color3(Color3::new(139.0, 108.0, 214.0)),
    )
    .expect("color");
    let glass = w.create("Part", Some(ws)).expect("part");
    w.set(glass, "Size", Value::Vector3(V3::new(8.0, 8.0, 1.0)))
        .expect("size");
    w.set(glass, "Position", Value::Vector3(V3::new(0.0, 4.0, 8.0)))
        .expect("pos");
    w.set(glass, "Transparency", Value::Number(0.5))
        .expect("transparency");
    w
}

fn camera() -> Camera {
    Camera {
        position: Vec3::new(14.0, 10.0, 22.0),
        look_at: Vec3::new(0.0, 3.0, 0.0),
        fov_deg: 60.0,
    }
}

fn render(quality: Quality) -> Option<Vec<u8>> {
    let ctx = RenderContext::new_headless().ok()?;
    let mut r = Renderer::with_settings(
        &ctx,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        RenderSettings { quality },
    );
    r.sync(&scene());
    assert_eq!(r.part_count(), 3);
    Some(r.capture(&camera(), SIZE).expect("capture"))
}

fn distinct_colours(px: &[u8]) -> usize {
    let mut seen = std::collections::HashSet::new();
    for p in px.as_chunks::<4>().0 {
        seen.insert([p[0], p[1], p[2]]);
    }
    seen.len()
}

#[test]
fn high_quality_frame_has_sky_ground_and_objects() {
    let Some(px) = render(Quality::High) else {
        eprintln!("no graphics adapter, skipping");
        return;
    };
    assert_eq!(px.len(), (SIZE.0 * SIZE.1 * 4) as usize);
    assert!(distinct_colours(&px) > 200, "frame looks flat");
    // Opaque alpha everywhere.
    assert!(px.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
    // Top row is sky, bottom row is ground: they must differ.
    let top = &px[0..3];
    let bottom = &px[px.len() - 4..px.len() - 1];
    assert_ne!(top, bottom);
}

#[test]
fn low_quality_renders_and_resizing_is_safe() {
    let Ok(ctx) = RenderContext::new_headless() else {
        eprintln!("no graphics adapter, skipping");
        return;
    };
    let mut r = Renderer::with_settings(
        &ctx,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        RenderSettings {
            quality: Quality::Low,
        },
    );
    r.sync(&scene());
    for size in [(320, 200), (97, 61), (640, 480), (320, 200)] {
        let px = r.capture(&camera(), size).expect("capture");
        assert_eq!(px.len(), (size.0 * size.1 * 4) as usize);
        assert!(distinct_colours(&px) > 50);
    }
    // A zero-sized target (minimised window) draws nothing and does not panic.
    let tex = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    r.render(&tex.create_view(&Default::default()), &camera(), (0, 0));
    // Switching quality at run time keeps working.
    r.set_settings(RenderSettings {
        quality: Quality::High,
    });
    let px = r.capture(&camera(), SIZE).expect("capture");
    assert!(distinct_colours(&px) > 200);
}

/// The default avatar drawn skinned in its rest pose: it covers the middle of the frame, the
/// close view uses LOD0 and a far view LOD2; clearing the instances removes it again.
#[test]
fn skinned_avatar_draws_and_picks_its_level_of_detail() {
    use metallic_render::glam::Mat4;
    use metallic_render::metallic_asset::{Pose, SkinnedModel};
    use metallic_render::{SkinStyle, SkinnedInstance};
    use std::sync::Arc;

    let Ok(ctx) = RenderContext::new_headless() else {
        eprintln!("no graphics adapter, skipping");
        return;
    };
    let glb = include_bytes!("../../../art/avatar/out/default_avatar.glb");
    let model = Arc::new(SkinnedModel::from_glb(glb).expect("avatar"));
    // Metres to studs; the avatar stands at the origin facing the camera (+Z).
    let scale = 1.0 / 0.28;
    let mut joints = Vec::new();
    Pose::rest(&model.skeleton).skin_matrices(
        &model.skeleton,
        Mat4::from_scale(Vec3::splat(scale)),
        &mut joints,
    );
    let mut r = Renderer::with_settings(
        &ctx,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        RenderSettings {
            quality: Quality::Low,
        },
    );
    let empty = World::new_with_services();
    r.sync(&empty);
    // Coverage is counted as changed pixels: keep the exposure fixed so the background does
    // not change with the metered brightness.
    r.auto_exposure.enabled = false;
    let close = Camera {
        position: Vec3::new(0.0, 2.75, 4.0),
        look_at: Vec3::new(0.0, 2.75, 0.0),
        fov_deg: 60.0,
    };
    let before = r.capture(&close, SIZE).expect("capture");
    let instance = SkinnedInstance {
        model: &model,
        joints: &joints,
        slot_colors: &[],
        center: Vec3::new(0.0, 2.75, 0.0),
        height: 5.5,
        detail_scale: scale,
        style: SkinStyle::Standard,
    };
    r.set_skinned(&[instance]);
    let with = r.capture(&close, SIZE).expect("capture");
    assert_eq!(r.skinned_lods(), [0]);
    let centre = |px: &[u8]| {
        let i = ((SIZE.1 / 2 * SIZE.0 + SIZE.0 / 2) * 4) as usize;
        [px[i], px[i + 1], px[i + 2]]
    };
    assert_ne!(centre(&before), centre(&with), "nothing drawn at the torso");
    let changed = before
        .as_chunks::<4>()
        .0
        .iter()
        .zip(with.as_chunks::<4>().0)
        .filter(|(a, b)| a != b)
        .count();
    let share = changed as f32 / (SIZE.0 * SIZE.1) as f32;
    assert!(
        (0.05..0.6).contains(&share),
        "avatar covers {share} of the frame"
    );

    let far = Camera {
        position: Vec3::new(0.0, 2.75, 200.0),
        ..close
    };
    r.capture(&far, SIZE).expect("capture");
    assert_eq!(r.skinned_lods(), [2]);

    r.set_skinned(&[]);
    let cleared = r.capture(&close, SIZE).expect("capture");
    assert_eq!(centre(&before), centre(&cleared));
}

/// The toon style draws a dark warm-brown ink outline around the avatar and shades it in warm
/// tones; the standard style draws neither.
#[test]
fn toon_avatar_has_ink_outline_and_warm_shading() {
    use metallic_render::glam::Mat4;
    use metallic_render::metallic_asset::{Pose, SkinnedModel};
    use metallic_render::{SkinStyle, SkinnedInstance};
    use std::sync::Arc;

    let Ok(ctx) = RenderContext::new_headless() else {
        eprintln!("no graphics adapter, skipping");
        return;
    };
    let glb = include_bytes!("../../../art/avatar/out/default_avatar.glb");
    let model = Arc::new(SkinnedModel::from_glb(glb).expect("avatar"));
    let scale = 1.0 / 0.28;
    let mut joints = Vec::new();
    Pose::rest(&model.skeleton).skin_matrices(
        &model.skeleton,
        Mat4::from_scale(Vec3::splat(scale)),
        &mut joints,
    );
    let mut r = Renderer::with_settings(
        &ctx,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        RenderSettings {
            quality: Quality::Low,
        },
    );
    r.sync(&World::new_with_services());
    // The default lighting's bloom lifts a 2 px dark line by about 15/255; this test checks the
    // ink colour itself.
    r.post_mut().remove("bloom");
    let cam = Camera {
        position: Vec3::new(0.0, 2.75, 9.0),
        look_at: Vec3::new(0.0, 2.75, 0.0),
        fov_deg: 50.0,
    };
    // Ink #5b3b33 within a small tolerance (dither and band edges).
    let is_ink = |p: &[u8; 4]| {
        (p[0] as i32 - 0x5b).abs() <= 12
            && (p[1] as i32 - 0x3b).abs() <= 12
            && (p[2] as i32 - 0x33).abs() <= 12
    };
    let mut shot = |style| {
        r.set_skinned(&[SkinnedInstance {
            model: &model,
            joints: &joints,
            slot_colors: &[],
            center: Vec3::new(0.0, 2.75, 0.0),
            height: 5.5,
            detail_scale: scale,
            style,
        }]);
        r.capture(&cam, SIZE).expect("capture")
    };
    let toon = shot(SkinStyle::Toon);
    let standard = shot(SkinStyle::Standard);
    let ink = |px: &[u8]| px.as_chunks::<4>().0.iter().filter(|p| is_ink(p)).count();
    let (toon_ink, standard_ink) = (ink(&toon), ink(&standard));
    assert!(toon_ink > 400, "toon outline pixels: {toon_ink}");
    assert!(
        standard_ink < toon_ink / 10,
        "standard has ink: {standard_ink}"
    );
    // The torso centre is warm: red above blue by a clear margin.
    let i = ((SIZE.1 / 2 * SIZE.0 + SIZE.0 / 2) * 4) as usize;
    let c = &toon[i..i + 3];
    assert!(
        c[0] as i32 - c[2] as i32 > 25,
        "torso colour {c:?} is not warm"
    );
}
