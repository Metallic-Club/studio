//! Pictures of the Properties panel and the Insert lists, for comparing a change by eye (#473).
//! They render with no window, on the wgpu adapter the test harness picks, so nothing pops up.
//!
//! ```text
//! METALLIC_PICTURES_DIR=out cargo test -p metallic_studio --test panel_pictures -j 4 -- --ignored
//! ```
use eframe::egui::{self, Color32, ColorImage, Event, Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use metallic_core::{InstanceId, Value, Vec3, World};
use metallic_studio::editor::EditorState;
use metallic_studio::editor_ui::EditorUi;
use metallic_studio::output::OutputPanel;
use metallic_studio::screenshot::save_png;

const SIZE: [f32; 2] = [1440.0, 900.0];

/// One instance of each class the pictures show, in a world with the services.
fn editor() -> (EditorUi, Vec<(&'static str, InstanceId)>) {
    let mut world = World::new_with_services();
    let ws = world.root("Workspace").unwrap();
    let model = world.create("Model", Some(ws)).unwrap();
    let part = world.create("Part", Some(model)).unwrap();
    world
        .set(part, "Position", Value::Vector3(Vec3::new(0.0, 2.0, 0.0)))
        .unwrap();
    let script = world.create("Script", Some(ws)).unwrap();
    world
        .set(script, "Source", Value::String("print('hi')\n".into()))
        .unwrap();
    let gui = world.create("ScreenGui", Some(ws)).unwrap();
    let label = world.create("TextLabel", Some(gui)).unwrap();
    let output = OutputPanel::new();
    let mut state = EditorState::new(
        world,
        std::path::Path::new("."),
        std::path::Path::new("unused.mscene"),
        "Pictures",
        output.log.sink(),
    );
    state.can_save = false;
    let shown = vec![
        ("part", part),
        ("model", model),
        ("script", script),
        ("text-label", label),
    ];
    (EditorUi::new(state, output), shown)
}

fn harness(ui: EditorUi) -> Harness<'static, EditorUi> {
    // The panels use the theme's bundled fonts, installed on the first pass (drawn empty).
    let mut themed = false;
    let mut h = Harness::builder()
        .with_size(egui::vec2(SIZE[0], SIZE[1]))
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
            ui,
        );
    h.run_steps(3);
    h
}

/// Saves the part of the frame inside `area` as `<dir>/<name>.png`.
fn save(h: &mut Harness<'static, EditorUi>, dir: &std::path::Path, name: &str, area: egui::Rect) {
    let frame = h.render().unwrap();
    let (x0, y0) = (area.left() as u32, area.top() as u32);
    let (w, h) = (area.width() as u32, area.height() as u32);
    let mut pixels = Vec::with_capacity((w * h) as usize);
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            let [r, g, b, a] = frame.get_pixel(x, y).0;
            pixels.push(Color32::from_rgba_premultiplied(r, g, b, a));
        }
    }
    let image = ColorImage::new([w as usize, h as usize], pixels);
    save_png(&dir.join(format!("{name}.png")), &image).unwrap();
}

#[test]
#[ignore = "writes pictures; needs METALLIC_PICTURES_DIR"]
fn capture_the_panels() {
    let dir = std::path::PathBuf::from(std::env::var("METALLIC_PICTURES_DIR").unwrap());
    let (ui, shown) = editor();
    let mut h = harness(ui);
    let whole = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SIZE[0], SIZE[1]));
    let properties =
        egui::Rect::from_min_max(egui::pos2(1030.0, 60.0), egui::pos2(SIZE[0], SIZE[1]));
    for (name, id) in shown {
        h.state_mut().state.selection.select_only(id);
        h.run_steps(3);
        save(&mut h, &dir, &format!("properties-{name}"), properties);
    }

    // The Insert menu of the menu bar, opened by clicking it.
    h.get_all_by_label("Insert")
        .min_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
        .unwrap()
        .click();
    h.run_steps(3);
    let menu = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(520.0, 560.0));
    save(&mut h, &dir, "insert-menu", menu);
    h.key_press(Key::Escape);
    h.run_steps(3);

    // Shift+A over the viewport, the empty query lists every class.
    let middle = egui::pos2(SIZE[0] * 0.45, SIZE[1] * 0.5);
    h.hover_at(middle);
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::SHIFT, Key::A);
    h.run_steps(3);
    save(&mut h, &dir, "insert-palette", whole);
    h.key_press(Key::Escape);
    h.run_steps(2);

    // Ctrl+K with ">insert": the Insert commands of the search palette.
    h.key_press_modifiers(Modifiers::COMMAND, Key::K);
    h.run_steps(2);
    h.event(Event::Text(">insert".into()));
    h.run_steps(3);
    save(&mut h, &dir, "command-palette-insert", whole);
}
