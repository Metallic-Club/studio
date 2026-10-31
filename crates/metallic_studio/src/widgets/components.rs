//! The components sheet (`--scene components`): every Studio widget in every state on one screen,
//! for design review. The live row reacts to the pointer and Tab like the real screens.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, Rect, RichText, Sense, Stroke, TextureHandle, Ui, vec2,
};

use crate::theme::{self, fonts, icons};
use crate::widgets::{Button, Plate, ShowAs, button::focus_ring, cut, keycap};

/// The states shown as columns, `None` being the live button.
const STATES: [(&str, Option<Option<ShowAs>>); 6] = [
    ("Rest", Some(None)),
    ("Hover", Some(Some(ShowAs::Hovered))),
    ("Pressed", Some(Some(ShowAs::Pressed))),
    ("Focus", Some(Some(ShowAs::Focused))),
    ("Disabled", None),
    ("Live", Some(None)),
];

/// Builds one row's button afresh for each state column.
type MakeButton = fn() -> Button<'static>;

const COLUMN: f32 = 212.0;
const LABEL: f32 = 72.0;

/// Draws the sheet. `art` is any texture to show the cut image clip with (the brand icon).
pub fn show(ui: &mut Ui, art: Option<&TextureHandle>) {
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(theme::BG_BASE)
                .inner_margin(egui::Margin::symmetric(48, 36)),
        )
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            ui.label(RichText::new("Components").font(fonts::display()));
            ui.add_space(6.0);
            ui.label(
                RichText::new("Buttons, keycaps, plates and the cut in every state.")
                    .font(fonts::body_lg())
                    .color(theme::TEXT_2),
            );
            ui.add_space(28.0);
            buttons(ui);
            ui.add_space(32.0);
            ui.horizontal_top(|ui| {
                ui.vertical(keycaps);
                ui.add_space(64.0);
                ui.vertical(plates);
                ui.add_space(64.0);
                ui.vertical(|ui| cuts(ui, art));
            });
        });
}

fn heading(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).font(fonts::heading()));
    ui.add_space(12.0);
}

fn caption(ui: &mut Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .font(fonts::small())
            .color(theme::TEXT_3),
    );
}

fn buttons(ui: &mut Ui) {
    heading(ui, "Buttons");
    ui.horizontal(|ui| {
        ui.add_space(LABEL);
        for (name, _) in STATES {
            ui.allocate_ui(vec2(COLUMN, 16.0), |ui| {
                ui.set_width(COLUMN);
                caption(ui, name);
            });
        }
    });
    ui.add_space(10.0);
    let rows: [(&str, MakeButton); 4] = [
        ("Primary", || Button::primary("Create game").keycap("Enter")),
        ("Secondary", || {
            Button::secondary("Open a game folder")
                .icon(icons::FOLDER_OPEN)
                .keycap("Ctrl O")
        }),
        ("Ghost", || Button::ghost("Settings").icon(icons::GEAR_SIX)),
        ("Danger", || {
            Button::danger("Remove from list").icon(icons::TRASH)
        }),
    ];
    for (name, make) in rows {
        ui.horizontal(|ui| {
            ui.allocate_ui(vec2(LABEL, 36.0), |ui| {
                ui.set_width(LABEL);
                ui.label(
                    RichText::new(name)
                        .font(fonts::strong())
                        .color(theme::TEXT_2),
                );
            });
            for (_, state) in STATES {
                ui.allocate_ui_with_layout(
                    vec2(COLUMN, 36.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_width(COLUMN);
                        match state {
                            Some(Some(s)) => ui.add(make().show_as(s)),
                            Some(None) => ui.add(make()),
                            None => ui.add_enabled(false, make()),
                        };
                    },
                );
            }
        });
        ui.add_space(16.0);
    }
}

fn keycaps(ui: &mut Ui) {
    heading(ui, "Keycaps");
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(
            RichText::new("Press")
                .font(fonts::body_lg())
                .color(theme::TEXT_2),
        );
        keycap(ui, "F5");
        ui.label(
            RichText::new("to try it.")
                .font(fonts::body_lg())
                .color(theme::TEXT_2),
        );
    });
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for k in ["Ctrl K", "Ctrl O", "Enter", "Esc"] {
            keycap(ui, k);
        }
    });
    ui.add_space(28.0);
    heading(ui, "Focus ring");
    let (rect, _) = ui.allocate_exact_size(vec2(160.0, 32.0), Sense::hover());
    ui.painter().rect_filled(rect, theme::R_SM, theme::BG_DEEP);
    ui.painter().rect_stroke(
        rect,
        theme::R_SM,
        Stroke::new(1.0, theme::FOCUS),
        egui::StrokeKind::Inside,
    );
    focus_ring(ui.painter(), rect, CornerRadius::same(theme::R_SM));
    ui.add_space(8.0);
    caption(ui, "2 px focus, 2 px outside");
}

fn plates(ui: &mut Ui) {
    heading(ui, "Plates");
    let size = vec2(150.0, 72.0);
    let items = [
        ("Panel", Plate::panel()),
        ("Raised", Plate::raised().radius(theme::R_MD)),
        ("Floating", Plate::floating()),
    ];
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        for (name, plate) in items {
            ui.vertical(|ui| {
                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                plate.paint(ui.painter(), rect);
                ui.add_space(8.0);
                caption(ui, name);
            });
        }
    });
}

fn cuts(ui: &mut Ui, art: Option<&TextureHandle>) {
    heading(ui, "The cut");
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        ui.vertical(|ui| {
            // A selected card: 2 px accent frame on the 12 px cut, the plate inside on ~11 px.
            let (rect, _) = ui.allocate_exact_size(vec2(150.0, 112.0), Sense::hover());
            let painter = ui.painter();
            let radius = CornerRadius::same(theme::R_MD + 2);
            cut::fill(painter, rect, radius, theme::CUT_MD + 2.0, theme::ACCENT);
            let inner = rect.shrink(2.0);
            let inner_radius = CornerRadius::same(theme::R_MD);
            let inner_cut = cut::inset_cut(theme::CUT_MD + 2.0, 2.0);
            cut::fill(painter, inner, inner_radius, inner_cut, theme::BG_RAISED);
            let art_rect = Rect::from_min_size(inner.min, vec2(inner.width(), 62.0));
            match art {
                Some(tex) => cut::paint_image(
                    painter,
                    art_rect,
                    CornerRadius {
                        nw: theme::R_MD,
                        ..CornerRadius::ZERO
                    },
                    inner_cut,
                    tex.id(),
                    Rect::from_min_max(egui::pos2(0.0, 0.3), egui::pos2(1.0, 0.7)),
                    Color32::WHITE,
                ),
                None => {
                    painter.rect_filled(art_rect, 0.0, theme::BG_ACTIVE);
                }
            }
            painter.text(
                egui::pos2(inner.left() + 12.0, art_rect.bottom() + 10.0),
                Align2::LEFT_TOP,
                icons::CHECK_CIRCLE,
                fonts::icon_fill(14.0),
                theme::ACCENT,
            );
            painter.text(
                egui::pos2(inner.left() + 32.0, art_rect.bottom() + 10.0),
                Align2::LEFT_TOP,
                "Selected",
                fonts::strong(),
                theme::TEXT_1,
            );
            ui.add_space(8.0);
            caption(ui, "cut-md frame, image clip");
        });
        ui.vertical(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(150.0, 112.0), Sense::hover());
            Plate::modal().paint(ui.painter(), rect);
            ui.add_space(8.0);
            caption(ui, "cut-lg, modal");
        });
    });
}
