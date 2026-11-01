//! A headless picture of the style picker (#224) over the Parkour template, for design review
//! against `docs/design/building-tools/style-picker.png`. Renders with egui_kittest's wgpu
//! backend; the thumbnails come from a headless [`GpuThumbnails`]. Nothing is saved to the game.
//! The 3D view needs eframe's device, so it shows its flat fallback here.
//!
//! ```text
//! cargo run -p metallic_studio --example style_picker_shot -- <out.png> [hover]
//! ```
//! With `hover`, the Retro / PS1 tile is hovered so the preview banner shows.

use std::path::PathBuf;

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use metallic_core::{Color3, Value, Vec3};
use metallic_render::RenderContext;
use metallic_studio::editor_ui::EditorUi;
use metallic_studio::style_picker::{GpuThumbnails, OPEN_ID};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: style_picker_shot <out.png> [hover]")?;
    let hover = std::env::args().nth(2).is_some_and(|a| a == "hover");
    let template = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../templates/parkour");
    let mut editor = EditorUi::demo(&template).ok_or("the Parkour template did not open")?;
    let gpu = RenderContext::new_headless()?;
    editor
        .style_picker
        .set_thumbnail_source(Box::new(GpuThumbnails::from_context(&gpu)));
    // A small "Props" Model (crate, barrel, ball) like the mockup's, selected.
    let w = &mut editor.state.world;
    let ws = w.root("Workspace").ok_or("no Workspace")?;
    let model = w.create("Model", Some(ws))?;
    w.set(model, "Name", Value::String("Props".into()))?;
    for (name, shape, pos, size, colour) in [
        (
            "Crate",
            "Block",
            (0.0, 2.0, 0.0),
            (4.0, 4.0, 4.0),
            (184.0, 115.0, 64.0),
        ),
        (
            "Barrel",
            "Cylinder",
            (4.0, 2.5, 1.0),
            (5.0, 3.0, 3.0),
            (38.0, 115.0, 217.0),
        ),
        (
            "Ball",
            "Ball",
            (0.0, 5.0, 0.0),
            (2.0, 2.0, 2.0),
            (217.0, 26.0, 26.0),
        ),
    ] {
        let p = w.create("Part", Some(model))?;
        w.set(p, "Name", Value::String(name.into()))?;
        w.set(p, "Shape", Value::String(shape.into()))?;
        w.set(
            p,
            "Position",
            Value::Vector3(Vec3::new(pos.0, pos.1, pos.2)),
        )?;
        w.set(p, "Size", Value::Vector3(Vec3::new(size.0, size.1, size.2)))?;
        w.set(
            p,
            "Color",
            Value::Color3(Color3::new(colour.0, colour.1, colour.2)),
        )?;
        w.set(p, "Anchored", Value::Bool(true))?;
    }
    editor.state.selection.select_only(model);
    let mut themed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1000.0))
        .with_pixels_per_point(1.0)
        .with_accessibility_check(false)
        .wgpu()
        .build_ui_state(
            move |ui, editor: &mut EditorUi| {
                if !themed {
                    metallic_studio::theme::apply(
                        ui.ctx(),
                        metallic_studio::theme::ThemeId::Graphite,
                    );
                    themed = true;
                    ui.ctx().request_repaint();
                    return;
                }
                editor.ui(ui, None);
            },
            editor,
        );
    harness.run_steps(2);
    harness.state_mut().run_action(OPEN_ID);
    harness.run_steps(12);
    if hover {
        harness
            .get_by_label("Retro / PS1: wobbly vertices, warped textures, 15-bit colour")
            .hover();
        harness.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(200));
        harness.run_steps(3);
    }
    let image = harness.render()?;
    let (w, h) = (image.width() as usize, image.height() as usize);
    let colour = egui::ColorImage::from_rgba_unmultiplied([w, h], image.as_raw());
    metallic_studio::screenshot::save_png(&out, &colour)?;
    println!("wrote {}", out.display());
    Ok(())
}
