//! Standalone animation editor demo.
//!
//! Run: `cargo run -p metallic_anim_editor --example anim_editor_demo`

use metallic_anim_editor::DemoApp;

struct App {
    demo: DemoApp,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.demo.ui(ui);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut demo = DemoApp::new()?;
    demo.state.playing = true;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Metallic Studio - Animation")
            .with_inner_size([1600.0, 960.0])
            .with_min_inner_size([1000.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Metallic Animation Demo",
        options,
        Box::new(|_cc| Ok(Box::new(App { demo }))),
    )?;
    Ok(())
}
