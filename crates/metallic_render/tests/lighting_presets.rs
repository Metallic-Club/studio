//! Headless render tests for the lighting presets (#149): every preset renders the reference
//! scene inside luminance and colour bounds, and lighting edits reach the image.
mod support;

use metallic_core::Value;
use metallic_core::lighting::{PRESETS, apply_preset, find_preset};
use metallic_render::{Quality, RenderContext, RenderSettings, Renderer};
use support::reference_scene::{chroma, lumas, reference_camera, reference_world, stats};

const SIZE: (u32, u32) = (160, 90);

fn renderer() -> Option<Renderer> {
    let Ok(ctx) = RenderContext::new_headless() else {
        eprintln!("no GPU adapter; skipping");
        return None;
    };
    Some(Renderer::with_settings(
        &ctx,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        RenderSettings {
            quality: Quality::Low,
        },
    ))
}

#[test]
fn every_preset_renders_within_bounds() {
    let Some(mut r) = renderer() else { return };
    for preset in &PRESETS {
        let mut world = reference_world();
        apply_preset(&mut world, preset).expect("apply");
        r.sync(&world);
        let px = r.capture(&reference_camera(), SIZE).expect("capture");
        let (mean, p1, p99) = stats(&lumas(&px));
        let c = chroma(&px);
        let id = preset.id;
        // Night looks are darker, but nothing is black, blown out or flat.
        let night = matches!(id, "night-city" | "moonlit");
        let (lo, hi) = if night { (0.1, 0.45) } else { (0.3, 0.7) };
        assert!(mean > lo && mean < hi, "{id}: mean luma {mean}");
        assert!(p99 < 0.95, "{id}: highlights clip, p99 {p99}");
        assert!(p99 - p1 > 0.15, "{id}: no contrast, p1 {p1} p99 {p99}");
        assert!(p1 < 0.35, "{id}: no darks, p1 {p1}");
        // Colour: no preset is grey-scale or a single wash of colour.
        assert!(c > 0.01 && c < 0.45, "{id}: mean chroma {c}");
    }
}

#[test]
fn presets_differ_in_the_image() {
    let Some(mut r) = renderer() else { return };
    let mut images = Vec::new();
    for preset in &PRESETS {
        let mut world = reference_world();
        apply_preset(&mut world, preset).expect("apply");
        r.sync(&world);
        images.push((
            preset.id,
            r.capture(&reference_camera(), SIZE).expect("capture"),
        ));
    }
    for (i, (a, pa)) in images.iter().enumerate() {
        for (b, pb) in &images[i + 1..] {
            let diff: u64 = pa
                .iter()
                .zip(pb)
                .map(|(x, y)| u64::from(x.abs_diff(*y)))
                .sum();
            let mean = diff as f64 / pa.len() as f64;
            assert!(mean > 4.0, "{a} and {b} look the same ({mean})");
        }
    }
}

#[test]
fn lighting_edits_reach_the_image() {
    let Some(mut r) = renderer() else { return };
    let mut world = reference_world();
    r.sync(&world);
    let day = r.capture(&reference_camera(), SIZE).expect("capture");
    let lighting = world.root("Lighting").expect("lighting");
    world
        .set(lighting, "ClockTime", Value::Number(0.0))
        .expect("clock");
    r.sync(&world);
    let night = r.capture(&reference_camera(), SIZE).expect("capture");
    let (d, _, _) = stats(&lumas(&day));
    let (n, _, _) = stats(&lumas(&night));
    assert!(n < d - 0.15, "midnight {n} not darker than day {d}");

    // Back to the default preset gives back the same image.
    apply_preset(&mut world, find_preset("clear-day").expect("preset")).expect("apply");
    r.sync(&world);
    let again = r.capture(&reference_camera(), SIZE).expect("capture");
    assert_eq!(day, again);
}

#[test]
fn shadows_and_fog_switches_work() {
    let Some(mut r) = renderer() else { return };
    let mut world = reference_world();
    r.sync(&world);
    let base = r.capture(&reference_camera(), SIZE).expect("capture");
    let lighting = world.root("Lighting").expect("lighting");
    world
        .set(lighting, "GlobalShadows", Value::Bool(false))
        .expect("shadows");
    r.sync(&world);
    let unshadowed = r.capture(&reference_camera(), SIZE).expect("capture");
    assert_ne!(base, unshadowed, "GlobalShadows had no effect");
    world
        .set(lighting, "GlobalShadows", Value::Bool(true))
        .expect("shadows");
    world
        .set(lighting, "FogEnd", Value::Number(60.0))
        .expect("fog end");
    r.sync(&world);
    let fogged = r.capture(&reference_camera(), SIZE).expect("capture");
    assert!(
        chroma(&fogged) < chroma(&base),
        "FogEnd 60 did not wash the scene"
    );
}
