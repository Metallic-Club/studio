//! Shading models from the material table (#63) on a real (or software) adapter. Skipped when the
//! machine has none. The tests take turns behind one lock: GL devices deadlock in parallel.
use metallic_core::{Color3, InstanceId, Value, Vec3 as V3, World};
use metallic_render::glam::Vec3;
use metallic_render::{
    Camera, ModelParams, Quality, RenderContext, RenderSettings, Renderer, Shading,
};
use std::sync::{Mutex, MutexGuard};

const SIZE: (u32, u32) = (320, 200);

fn gpu() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn add(w: &mut World, size: V3, pos: V3, color: Color3, material: &str, shape: &str) -> InstanceId {
    let ws = w.root("Workspace").expect("workspace");
    let id = w.create("Part", Some(ws)).expect("part");
    w.set(id, "Size", Value::Vector3(size)).expect("size");
    w.set(id, "Position", Value::Vector3(pos)).expect("pos");
    w.set(id, "Color", Value::Color3(color)).expect("color");
    w.set(id, "Material", Value::String(material.into()))
        .expect("material");
    w.set(id, "Shape", Value::String(shape.into()))
        .expect("shape");
    id
}

/// A grey floor and two balls: Glass (no procedural detail, opaque) on the left, Wood on the right.
fn scene() -> World {
    let mut w = World::new_with_services();
    let grey = Color3::new(150.0, 150.0, 150.0);
    add(
        &mut w,
        V3::new(80.0, 1.0, 80.0),
        V3::new(0.0, -0.5, 0.0),
        grey,
        "Concrete",
        "Block",
    );
    add(
        &mut w,
        V3::new(6.0, 6.0, 6.0),
        V3::new(-3.5, 3.0, 0.0),
        Color3::new(200.0, 80.0, 60.0),
        "Glass",
        "Ball",
    );
    add(
        &mut w,
        V3::new(6.0, 6.0, 6.0),
        V3::new(3.5, 3.0, 0.0),
        Color3::new(60.0, 120.0, 200.0),
        "Wood",
        "Ball",
    );
    w
}

fn camera() -> Camera {
    Camera {
        position: Vec3::new(0.0, 3.0, 13.0),
        look_at: Vec3::new(0.0, 3.0, 0.0),
        fov_deg: 50.0,
    }
}

fn renderer(ctx: &RenderContext) -> Renderer {
    let mut r = Renderer::with_settings(
        ctx,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        RenderSettings {
            quality: Quality::High,
        },
    );
    r.auto_exposure.enabled = false;
    r.finish.dither = false;
    r
}

fn pixel(px: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * SIZE.0 + x) * 4) as usize;
    [px[i], px[i + 1], px[i + 2]]
}

/// Pixels of a square around (cx, cy), half width `r`.
fn square(px: &[u8], cx: u32, cy: u32, r: u32) -> Vec<[u8; 3]> {
    let mut out = Vec::new();
    for y in cy - r..=cy + r {
        for x in cx - r..=cx + r {
            out.push(pixel(px, x, y));
        }
    }
    out
}

fn distinct(p: &[[u8; 3]]) -> usize {
    p.iter().collect::<std::collections::HashSet<_>>().len()
}

/// Share of pixels within 2 levels per channel of one of the `n` most common colours (fog
/// and filtering move single pixels by a level or two).
fn top_share(p: &[[u8; 3]], n: usize) -> f32 {
    let mut counts = std::collections::HashMap::new();
    for c in p {
        *counts.entry(*c).or_insert(0usize) += 1;
    }
    let mut v: Vec<([u8; 3], usize)> = counts.into_iter().collect();
    v.sort_unstable_by_key(|e| std::cmp::Reverse(e.1));
    let near = |a: &[u8; 3], b: &[u8; 3]| a.iter().zip(b).all(|(x, y)| x.abs_diff(*y) <= 2);
    let covered = p
        .iter()
        .filter(|c| v.iter().take(n).any(|(t, _)| near(c, t)))
        .count();
    covered as f32 / p.len() as f32
}

// Ball centres on screen (both balls are about 60 px wide at this camera).
const LEFT: (u32, u32) = (110, 100);
const RIGHT: (u32, u32) = (210, 100);

#[test]
fn unlit_rows_show_the_part_colour_and_other_rows_stay_lit() {
    let _gpu = gpu();
    let Ok(ctx) = RenderContext::new_headless() else {
        eprintln!("no graphics adapter, skipping");
        return;
    };
    let mut r = renderer(&ctx);
    r.sync(&scene());
    let lit = r.capture(&camera(), SIZE).expect("capture");
    r.set_shading(&Shading {
        default: ModelParams::LIT,
        materials: vec![("Glass".into(), ModelParams::UNLIT)],
        character: None,
    });
    let px = r.capture(&camera(), SIZE).expect("capture");
    // The Glass ball shows its own colour (200, 80, 60) all over, shade side included.
    for c in square(&px, LEFT.0, LEFT.1, 12) {
        for (got, want) in c.iter().zip([200u8, 80, 60]) {
            assert!(got.abs_diff(want) <= 3, "unlit pixel {c:?}");
        }
    }
    // The Wood ball keeps its row: unchanged to the byte.
    assert_eq!(
        square(&px, RIGHT.0, RIGHT.1, 12),
        square(&lit, RIGHT.0, RIGHT.1, 12)
    );
    // Back to the default shading gives the first image again.
    r.set_shading(&Shading::default());
    assert_eq!(r.capture(&camera(), SIZE).expect("capture"), lit);
}

#[test]
fn toon_bands_the_light_into_a_few_tones() {
    let _gpu = gpu();
    let Ok(ctx) = RenderContext::new_headless() else {
        eprintln!("no graphics adapter, skipping");
        return;
    };
    let mut r = renderer(&ctx);
    r.sync(&scene());
    let lit = square(
        &r.capture(&camera(), SIZE).expect("capture"),
        LEFT.0,
        LEFT.1,
        20,
    );
    // Flat ambient and no highlight, so each band is one colour.
    r.set_shading(&Shading::uniform(ModelParams {
        flatten: 1.0,
        spec_size: 0.0,
        ..ModelParams::TOON
    }));
    let toon = square(
        &r.capture(&camera(), SIZE).expect("capture"),
        LEFT.0,
        LEFT.1,
        20,
    );
    let (t, l) = (top_share(&toon, 3), top_share(&lit, 3));
    assert!(t > 0.8, "toon: 3 colours cover {t}");
    assert!(l < 0.3, "lit: 3 colours cover {l}");
}

#[test]
fn flat_gives_each_triangle_one_tone() {
    let _gpu = gpu();
    let Ok(ctx) = RenderContext::new_headless() else {
        eprintln!("no graphics adapter, skipping");
        return;
    };
    let mut r = renderer(&ctx);
    r.sync(&scene());
    let lit = square(
        &r.capture(&camera(), SIZE).expect("capture"),
        LEFT.0,
        LEFT.1,
        20,
    );
    r.set_shading(&Shading::uniform(ModelParams::FLAT));
    let flat = square(
        &r.capture(&camera(), SIZE).expect("capture"),
        LEFT.0,
        LEFT.1,
        20,
    );
    let (f, l) = (distinct(&flat), distinct(&lit));
    assert!(f * 3 < l, "flat {f} distinct colours, lit {l}");
}

#[test]
fn the_character_row_shades_standard_skinned_meshes() {
    use metallic_render::glam::Mat4;
    use metallic_render::metallic_asset::{Pose, SkinnedModel};
    use metallic_render::{SkinStyle, SkinnedInstance};
    use std::sync::Arc;

    let _gpu = gpu();
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
    let mut r = renderer(&ctx);
    r.sync(&World::new_with_services());
    let instance = |style| SkinnedInstance {
        model: &model,
        joints: &joints,
        slot_colors: &[],
        center: Vec3::new(0.0, 2.75, 0.0),
        height: 5.5,
        detail_scale: scale,
        style,
    };
    let cam = Camera {
        position: Vec3::new(0.0, 2.75, 4.0),
        look_at: Vec3::new(0.0, 2.75, 0.0),
        fov_deg: 60.0,
    };
    r.set_skinned(&[instance(SkinStyle::Standard)]);
    let lit = r.capture(&cam, SIZE).expect("capture");
    let toon_style = {
        r.set_skinned(&[instance(SkinStyle::Toon)]);
        r.capture(&cam, SIZE).expect("capture")
    };
    // Without a Character override the default model reaches standard characters; the
    // override alone does the same.
    r.set_skinned(&[instance(SkinStyle::Standard)]);
    r.set_shading(&Shading::uniform(ModelParams::UNLIT));
    let parts_unlit = r.capture(&cam, SIZE).expect("capture");
    r.set_shading(&Shading {
        character: Some(ModelParams::UNLIT),
        ..Shading::default()
    });
    let char_unlit = r.capture(&cam, SIZE).expect("capture");
    assert_ne!(parts_unlit, lit, "Unlit default reaches the Character row");
    assert_eq!(
        parts_unlit, char_unlit,
        "Character row alone gives the same figure"
    );
    // The toon style (#320) has its own pipeline and ignores the table.
    r.set_skinned(&[instance(SkinStyle::Toon)]);
    assert_eq!(r.capture(&cam, SIZE).expect("capture"), toon_style);
}
