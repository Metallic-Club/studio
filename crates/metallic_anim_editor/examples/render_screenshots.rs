//! Renders the demo editor headlessly (egui_kittest + wgpu, no window) and saves PNGs.
//!
//! Run: `cargo run -p metallic_anim_editor --example render_screenshots -- <output dir>`
//! (default output: `docs/agents/40-anim`). Needs a GPU adapter or a software one (WARP on
//! Windows, lavapipe on Linux).

use egui_kittest::Harness;
use metallic_anim_editor::{DemoApp, EditorTab, KeyRef};
use std::path::PathBuf;

type Setup = fn(&mut DemoApp);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args()
        .nth(1)
        .map_or_else(|| PathBuf::from("docs/agents/40-anim"), PathBuf::from);
    std::fs::create_dir_all(&out)?;
    let shots: [(&str, Setup); 4] = [
        ("editor-dope-sheet.png", |app| {
            app.state.time = 0.72;
            app.state.active_track = Some(3);
            app.state.time_view.fit(0.0, 2.0, 1600.0 - 292.0 - 210.0);
            for key in 1..=3 {
                app.state.selection.insert(KeyRef { track: 3, key });
            }
        }),
        ("editor-curves.png", |app| {
            app.tab = EditorTab::Curves;
            app.state.time = 1.05;
            app.state.active_track = Some(3);
            app.state.time_view.fit(0.0, 2.0, 1600.0 - 292.0 - 56.0);
            app.state.selection.insert(KeyRef { track: 3, key: 2 });
            app.state.selection.insert(KeyRef { track: 3, key: 4 });
            app.state.frame_all_values();
        }),
        ("editor-walk.png", |app| {
            app.select_clip(1);
            app.state.time = 0.3;
            app.state.active_track = Some(4);
            app.state.time_view.fit(0.0, 1.0, 1600.0 - 292.0 - 210.0);
            app.state.selection.insert(KeyRef { track: 4, key: 1 });
        }),
        ("editor-recording.png", |app| {
            app.state.time = 1.4;
            app.state.recording = true;
            app.state.active_track = Some(1);
            app.state.time_view.fit(0.0, 2.0, 1600.0 - 292.0 - 210.0);
        }),
    ];
    for (name, setup) in shots {
        let mut app = DemoApp::new()?;
        setup(&mut app);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1600.0, 960.0))
            .with_pixels_per_point(1.0)
            .wgpu()
            .build_ui_state(|ui, app: &mut DemoApp| app.ui(ui), app);
        harness.run_steps(4);
        let image = harness.render()?;
        let path = out.join(name);
        image.save(&path)?;
        println!("wrote {}", path.display());
    }
    Ok(())
}
