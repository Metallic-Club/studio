//! The Input Actions panel on its own window (the editor docks it under View > Input actions).
//!
//! ```text
//! cargo run -p metallic_studio --example input_actions -- [scene.mscene] [--save]
//!     [--tab bindings|preview] [--mode keyboard|phone|tablet|gamepad] [--edit]
//!     [--add-controls] [--select <action>] [--screenshot <png>]
//! ```
//!
//! Without a scene it opens the Parkour template. Nothing is written unless `--save` is given;
//! then Ctrl+S saves the scene in place.

use std::path::PathBuf;

use eframe::egui;
use metallic_core::{World, load_scene, save_scene};
use metallic_runtime::input::default_character_controls;
use metallic_studio::input_actions::commands::{CommandSink, StepBuilder};
use metallic_studio::input_actions::model::context_snapshot;
use metallic_studio::input_actions::preview::PreviewAs;
use metallic_studio::input_actions::{InputActionsPanel, Tab};
use metallic_studio::screenshot::ScreenshotJob;
use metallic_studio::theme;

struct App {
    world: World,
    panel: InputActionsPanel,
    path: PathBuf,
    save: bool,
    shot: Option<ScreenshotJob>,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.save && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
            match save_scene(&self.world, &self.path) {
                Ok(()) => self.panel.mark_saved(&self.world),
                Err(e) => log::error!("save failed: {e}"),
            }
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new())
            .show(ui, |ui| self.panel.show(ui, &mut self.world));
        if let Some(job) = &mut self.shot {
            job.tick(&ctx);
        }
    }
}

fn main() -> eframe::Result {
    env_logger::init();
    let mut args = std::env::args().skip(1);
    let mut path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../templates/parkour/main.mscene");
    let (mut save, mut tab, mut mode, mut edit, mut add, mut select, mut shot) =
        (false, Tab::Bindings, None, false, false, None, None);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--save" => save = true,
            "--edit" => edit = true,
            "--add-controls" => add = true,
            "--tab" => {
                tab = if args.next().as_deref() == Some("preview") {
                    Tab::Preview
                } else {
                    Tab::Bindings
                }
            }
            "--mode" => {
                mode = match args.next().as_deref() {
                    Some("phone") => Some(PreviewAs::Phone),
                    Some("tablet") => Some(PreviewAs::Tablet),
                    Some("gamepad") => Some(PreviewAs::Gamepad),
                    _ => Some(PreviewAs::KeyboardMouse),
                }
            }
            "--select" => select = args.next(),
            "--screenshot" => shot = args.next().map(PathBuf::from),
            other => path = PathBuf::from(other),
        }
    }
    let mut world = match load_scene(&path) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("cannot open {}: {e}", path.display());
            std::process::exit(2);
        }
    };
    let mut panel = InputActionsPanel::new(&world);
    if add {
        let rs = world.root("ReplicatedStorage").expect("service");
        let mut b = StepBuilder::new(&mut world);
        b.create(rs, context_snapshot(&default_character_controls()))
            .expect("add");
        let _ = b.finish("Add CharacterControls").map(|s| {
            let mut h = metallic_studio::input_actions::History::new();
            h.record(s);
        });
        panel.mark_saved(&world);
    }
    panel.set_tab(tab);
    if let Some(m) = mode {
        panel.preview_state_mut().mode = m;
    }
    panel.preview_state_mut().editing_layout = edit;
    if let Some(name) = select {
        panel.select_action(&world, &name);
    }
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Input actions")
            .with_inner_size([1440.0, 880.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "Input actions",
        native,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx, theme::ThemeId::Graphite);
            Ok(Box::new(App {
                world,
                panel,
                path,
                save,
                shot: shot.map(ScreenshotJob::new),
            }))
        }),
    )
}
