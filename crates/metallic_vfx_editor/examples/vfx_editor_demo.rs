//! Runs the VFX editor in a window: `cargo run -p metallic_vfx_editor --example vfx_editor_demo`.

use metallic_vfx_editor::{VfxEditor, theme};

struct Demo {
    editor: VfxEditor,
}

impl eframe::App for Demo {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.editor.ui(ui);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let editor = VfxEditor::new()?;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Metallic VFX Editor")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([1100.0, 700.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "Metallic VFX Editor",
        options,
        Box::new(|cc| {
            if !theme::install_fonts(&cc.egui_ctx) {
                eprintln!("no system UI font found; text will not render");
            }
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(Demo { editor }))
        }),
    )?;
    Ok(())
}
