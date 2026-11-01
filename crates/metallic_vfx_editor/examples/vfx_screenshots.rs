//! Renders the editor and every preset headlessly (no window) to PNGs:
//! `cargo run -p metallic_vfx_editor --example vfx_screenshots -- <out-dir>`.
//!
//! Writes `editor-stack.png`, `editor-graph.png`, `presets.png` (every preset) and `styles.png` (a few
//! presets restyled to each art style).

use egui::{Pos2, Rect, Vec2};
use egui_kittest::Harness;
use metallic_vfx::{ArtStyle, SpawnModule, presets};
use metallic_vfx_editor::{Selection, Stage, VfxEditor, View, theme};
use std::path::{Path, PathBuf};

type Error = Box<dyn std::error::Error>;

fn setup(ctx: &egui::Context) {
    theme::apply(ctx);
}

fn save(img: &image::RgbaImage, path: &Path) -> Result<(), Error> {
    img.save(path)?;
    println!("wrote {}", path.display());
    Ok(())
}

/// A representative moment: one-shots shortly after their burst, loops once they have filled in.
fn moment(effect: &metallic_vfx::Effect) -> f32 {
    let burst_only = effect.emitters.iter().all(|e| {
        e.spawn
            .iter()
            .all(|s| matches!(s, SpawnModule::Burst { .. }))
    }) && !effect.emitters.is_empty();
    match effect.name.as_str() {
        "Explosion" => 0.32,
        "Shockwave" => 0.3,
        "Muzzle flash" => 0.05,
        "Hit impact" => 0.12,
        "Toon smear" => 0.12,
        "Crystal shatter" => 0.35,
        "Splash" => 0.3,
        "Footstep" => 0.25,
        "Dust puff" => 0.35,
        "Sword trail" => 0.42,
        _ if burst_only => 0.4,
        _ => 2.6,
    }
}

fn editor_shot(out: &Path, name: &str, view: View) -> Result<(), Error> {
    let mut editor = VfxEditor::new()?;
    editor.open_preset("Fire")?;
    editor.view = view;
    match view {
        View::Graph => editor.select(Selection::Emitter(0), Some((Stage::Update, 2))),
        _ => editor.select(Selection::Emitter(0), Some((Stage::Update, 0))),
    }
    editor.seek(2.4);
    editor.playing = false;
    let mut harness = Harness::builder()
        .with_size(Vec2::new(1440.0, 900.0))
        .with_pixels_per_point(1.0)
        .with_accessibility_check(false)
        .with_context_setup(setup)
        .wgpu()
        .build_ui_state(|ui, ed: &mut VfxEditor| ed.ui(ui), editor);
    harness.run_steps(4);
    let img = harness.render()?;
    save(&img, &out.join(name))
}

struct Sheet {
    cells: Vec<(String, VfxEditor)>,
    columns: usize,
    cell: Vec2,
}

fn sheet_ui(ui: &mut egui::Ui, sheet: &mut Sheet) {
    let painter = ui.painter().clone();
    painter.rect_filled(ui.max_rect(), 0.0, theme::color::bg_deep());
    let (cols, cell) = (sheet.columns, sheet.cell);
    for (i, (label, ed)) in sheet.cells.iter_mut().enumerate() {
        let x = (i % cols) as f32 * (cell.x + 6.0) + 6.0;
        let y = (i / cols) as f32 * (cell.y + 6.0) + 6.0;
        let r = Rect::from_min_size(Pos2::new(x, y), cell);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r));
        child.set_clip_rect(r);
        ed.preview_ui(&mut child, r);
        painter.text(
            r.left_top() + Vec2::new(10.0, 8.0),
            egui::Align2::LEFT_TOP,
            label.as_str(),
            theme::semibold(12.0),
            theme::color::text(),
        );
    }
}

fn render_sheet(out: &Path, name: &str, sheet: Sheet) -> Result<(), Error> {
    let rows = sheet.cells.len().div_ceil(sheet.columns);
    let size = Vec2::new(
        sheet.columns as f32 * (sheet.cell.x + 6.0) + 6.0,
        rows as f32 * (sheet.cell.y + 6.0) + 6.0,
    );
    let mut harness = Harness::builder()
        .with_size(size)
        .with_pixels_per_point(1.0)
        .with_accessibility_check(false)
        .with_context_setup(setup)
        .wgpu()
        .build_ui_state(sheet_ui, sheet);
    harness.run_steps(3);
    let img = harness.render()?;
    save(&img, &out.join(name))
}

fn prepared(name: &str, style: Option<ArtStyle>) -> Result<VfxEditor, Error> {
    let mut ed = VfxEditor::new()?;
    ed.open_preset(name)?;
    if style.is_some() {
        ed.set_style_view(style);
    }
    let t = moment(ed.effect());
    ed.seek(t);
    ed.playing = false;
    Ok(ed)
}

fn main() -> Result<(), Error> {
    let out = std::env::args()
        .nth(1)
        .map_or_else(|| PathBuf::from("docs/agents/41-vfx"), PathBuf::from);
    std::fs::create_dir_all(&out)?;
    let only = std::env::args().nth(2).unwrap_or_default();

    if only.is_empty() || only == "editor" {
        editor_shot(&out, "editor-stack.png", View::Stack)?;
        editor_shot(&out, "editor-graph.png", View::Graph)?;
    }
    if only.is_empty() || only == "presets" {
        let mut cells = Vec::new();
        for name in presets::names() {
            let ed = prepared(&name, None)?;
            cells.push((name, ed));
        }
        render_sheet(
            &out,
            "presets.png",
            Sheet {
                cells,
                columns: 6,
                cell: Vec2::new(236.0, 196.0),
            },
        )?;
    }
    if only.is_empty() || only == "styles" {
        let mut cells = Vec::new();
        for name in ["Fire", "Explosion", "Magic", "Sparks"] {
            for style in ArtStyle::ALL {
                let ed = prepared(name, Some(style))?;
                cells.push((format!("{name} · {}", style.label()), ed));
            }
        }
        render_sheet(
            &out,
            "styles.png",
            Sheet {
                cells,
                columns: 5,
                cell: Vec2::new(236.0, 196.0),
            },
        )?;
    }
    Ok(())
}
