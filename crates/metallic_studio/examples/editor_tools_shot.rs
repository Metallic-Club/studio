//! A headless picture of the editor with the building tools and the Input Actions dock open
//! (design review for #319). Renders with egui_kittest's wgpu backend; nothing is saved to the
//! game. The 3D view itself needs eframe's device, so it shows its flat fallback here; the tool
//! overlays (array ghosts and the drag handle) are drawn over it.
//!
//! ```text
//! cargo run -p metallic_studio --example editor_tools_shot -- <out.png>
//! ```

use std::path::PathBuf;

use eframe::egui;
use egui_kittest::Harness;
use metallic_studio::editor_tools::Popover;
use metallic_studio::editor_ui::EditorUi;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: editor_tools_shot <out.png>")?;
    let template = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../templates/parkour");
    let editor = EditorUi::demo(&template).ok_or("the Parkour template did not open")?;
    editor.input_actions.set_open(true);
    let mut themed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 900.0))
        .with_pixels_per_point(1.0)
        // A picture only; the panels' own tests cover their labels.
        .with_accessibility_check(false)
        .wgpu()
        .build_ui_state(
            move |ui, editor: &mut EditorUi| {
                // Fonts and colours first; they take effect from the next pass.
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
    harness.state_mut().tools.state().popover = Some(Popover::Array);
    harness.run_steps(6);
    let image = harness.render()?;
    let (w, h) = (image.width() as usize, image.height() as usize);
    let colour = egui::ColorImage::from_rgba_unmultiplied([w, h], image.as_raw());
    metallic_studio::screenshot::save_png(&out, &colour)?;
    println!("wrote {}", out.display());
    Ok(())
}
