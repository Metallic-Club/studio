//! Debug views (#23) on a real adapter: every view draws something different from the normal
//! image, turning a view off restores it exactly, and debug shapes show up over the frame.
//! Without an adapter the tests print a note and pass.
mod support;

use metallic_render::glam::{Quat, Vec3};
use metallic_render::meshes::MeshKind;
use metallic_render::{
    DebugShape, DebugView, Quality, RenderContext, RenderSettings, Renderer, glam,
};
use support::reference_scene::{reference_camera, reference_world};

const SIZE: (u32, u32) = (160, 90);
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

fn renderer() -> Option<Renderer> {
    let Ok(ctx) = RenderContext::new_headless() else {
        eprintln!("no GPU adapter; skipping");
        return None;
    };
    let mut r = Renderer::with_settings(
        &ctx,
        FORMAT,
        RenderSettings {
            quality: Quality::High,
        },
    );
    r.sync(&reference_world());
    Some(r)
}

/// Mean absolute difference per channel, 0..255.
fn difference(a: &[u8], b: &[u8]) -> f32 {
    let sum: u64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| u64::from(x.abs_diff(*y)))
        .sum();
    sum as f32 / a.len() as f32
}

#[test]
fn every_view_changes_the_image_and_off_restores_it() {
    let Some(mut r) = renderer() else { return };
    let camera = reference_camera();
    let normal = r.capture(&camera, SIZE).expect("capture");
    for view in DebugView::ALL.into_iter().skip(1) {
        r.set_debug_view(view);
        let img = r.capture(&camera, SIZE).expect("capture");
        let d = difference(&normal, &img);
        assert!(
            d > 4.0,
            "{view:?} looks like the normal image (difference {d})"
        );
    }
    r.set_debug_view(DebugView::Off);
    let back = r.capture(&camera, SIZE).expect("capture");
    let d = difference(&normal, &back);
    assert!(d < 0.5, "turning the view off left a difference of {d}");
}

#[test]
fn debug_shapes_draw_over_the_frame_and_clear() {
    let Some(mut r) = renderer() else { return };
    let camera = reference_camera();
    let normal = r.capture(&camera, SIZE).expect("capture");
    let at = camera.look_at;
    let shapes = [
        DebugShape::oriented(
            MeshKind::Box,
            at,
            Quat::from_rotation_y(0.4),
            Vec3::splat(6.0),
            [0.9, 0.2, 0.2],
        ),
        DebugShape::oriented(
            MeshKind::Sphere,
            at + glam::Vec3::Y * 4.0,
            Quat::IDENTITY,
            Vec3::splat(3.0),
            [0.2, 0.9, 0.3],
        ),
    ];
    r.set_debug_shapes(&shapes);
    let with = r.capture(&camera, SIZE).expect("capture");
    assert!(
        difference(&normal, &with) > 0.3,
        "the shapes are not visible"
    );
    r.set_debug_shapes(&[]);
    let without = r.capture(&camera, SIZE).expect("capture");
    assert!(difference(&normal, &without) < 0.5);
}
