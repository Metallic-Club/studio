//! View > Debug views (#23): the renderer's whole-frame views (wireframe, normals, overdraw,
//! shadow cascades, the exposure light meter) and the physics debug draw of a play session
//! (collider shapes, contacts, velocities, touch volumes).
//!
//! The render view applies to the viewport in edit and play; the physics overlay needs the
//! simulation, so it shows while playing.
use eframe::egui::{self, RichText};
use metallic_render::DebugView;
use metallic_render::exposure::ExposureState;
use metallic_runtime::DebugDrawOptions;

use crate::theme;
use crate::viewport::Viewport;

/// One overlay's switch inside the options.
type Field = fn(&mut DebugDrawOptions) -> &mut bool;

/// The physics overlays, in menu order.
const PHYSICS: [(&str, Field); 4] = [
    ("Collision shapes", |o| &mut o.shapes),
    ("Contacts", |o| &mut o.contacts),
    ("Velocities", |o| &mut o.velocities),
    ("Touch volumes", |o| &mut o.touch_volumes),
];

/// The light meter line: adapted EV100 and the multiplier on the fixed exposure.
pub fn meter_text(e: &ExposureState) -> String {
    format!("EV100 {:.1}, exposure x{:.2}", e.ev100, e.multiplier)
}

/// The "Debug views" submenu. `physics` is the overlay to apply to the next play frames;
/// `playing` says whether a session runs; `exposure` is the meter's last reading, if any.
pub fn menu(
    ui: &mut egui::Ui,
    viewport: &mut Viewport,
    physics: &mut DebugDrawOptions,
    playing: bool,
    exposure: Option<ExposureState>,
) {
    ui.menu_button("Debug views", |ui| {
        heading(ui, "Render");
        let current = viewport.debug_view();
        for view in DebugView::ALL {
            if ui.radio(current == view, view.label()).clicked() {
                viewport.set_debug_view(view);
            }
        }
        if let Some(e) = exposure.filter(|_| viewport.debug_view() == DebugView::Exposure) {
            note(ui, &format!("Light meter: {}", meter_text(&e)));
        }
        ui.separator();
        heading(ui, "Physics");
        for (label, field) in PHYSICS {
            ui.checkbox(field(physics), label);
        }
        if !playing {
            note(ui, "Shows while playing.");
        }
    });
}

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).strong().color(theme::text_2()));
}

fn note(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).small().color(theme::text_3()));
}
