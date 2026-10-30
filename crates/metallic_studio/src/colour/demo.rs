//! A stand-alone colour demo (`metallic-studio --scene colour`): one demo Part with a Properties
//! style `Color` row that opens the [`ColourPopover`]. It shows the preview/commit/cancel flow and
//! the undo names until the real Properties panel (#10) and preview overlay (#213) host it.

use eframe::egui::{self, Align2, FontId, Key, RichText, Sense, Stroke, StrokeKind, vec2};

use super::maths::Rgb8;
use super::palette::{Palette, PaletteStore, RecentColours};
use super::popover::{ColourEvent, ColourLibrary, ColourPopover};
use crate::theme;

/// The demo's starting colour (the spec's example).
pub const START: Rgb8 = Rgb8::new(0x52, 0x80, 0xCC);

/// The demo screen.
pub struct ColourDemo {
    colour: Rgb8,
    preview: Option<Rgb8>,
    hue_shift: Option<f32>,
    popover: ColourPopover,
    library: ColourLibrary,
    history: Vec<String>,
}

impl ColourDemo {
    /// Builds the demo. `persistent` reads and writes the user's recent colours; screenshots pass
    /// false so nothing personal can appear in a picture. Palettes stay in memory (no project).
    pub fn new(persistent: bool) -> Self {
        let recent = if persistent {
            RecentColours::load_default()
        } else {
            let mut r = RecentColours::load(None);
            for c in [
                0x7A7F87, 0x8A5CF0, 0xE2B23C, 0x1B2B3A, 0xF2F2F2, 0x3F8F4A, 0xC0392B, 0xA0603A,
                0xD9C8A9, 0x5280CC,
            ] {
                r.push(Rgb8::from_u32(c));
            }
            r
        };
        let castle = Palette {
            name: "Castle".into(),
            colours: [0x6B6B6B, 0x8A8378, 0xB5AC9C, 0x4A5560, 0x8C3B2E, 0xD9C8A9]
                .map(Rgb8::from_u32)
                .to_vec(),
            read_only: false,
        };
        Self {
            colour: START,
            preview: None,
            hue_shift: None,
            popover: ColourPopover::new("demo"),
            library: ColourLibrary {
                recent,
                palettes: PaletteStore::in_memory(vec![castle]),
                ..Default::default()
            },
            history: Vec::new(),
        }
    }

    /// Opens the popover next to `pos` (used by the screenshot scene).
    pub fn open_at(&mut self, pos: egui::Pos2) {
        self.popover.open(self.colour, 2, false, pos);
    }

    /// The Part colour after the last commit.
    pub fn colour(&self) -> Rgb8 {
        self.colour
    }

    /// Draws the demo; returns true when the user asks to go back.
    pub fn show(&mut self, ui: &mut egui::Ui) -> bool {
        let mut back = false;
        let mut open_at = None;
        egui::CentralPanel::default().show(ui, |ui| {
            ui.label(RichText::new("Colour popover demo").size(24.0).strong());
            ui.label(
                RichText::new(
                    "Stand-in for the Properties panel until the editor lands (#10, #213). \
                     Click the Color row or press C.",
                )
                .color(theme::TEXT_DIM),
            );
            ui.add_space(12.0);

            let shown = self.shown_colour();
            let (part, _) = ui.allocate_exact_size(vec2(260.0, 160.0), Sense::hover());
            let painter = ui.painter();
            painter.rect_filled(
                part,
                6.0,
                egui::Color32::from_rgb(shown.r, shown.g, shown.b),
            );
            painter.rect_stroke(
                part,
                6.0,
                Stroke::new(1.0, theme::BORDER),
                StrokeKind::Inside,
            );
            painter.text(
                part.left_bottom() + vec2(8.0, -8.0),
                Align2::LEFT_BOTTOM,
                if self.preview.is_some() || self.hue_shift.is_some() {
                    "Demo part (previewing)"
                } else {
                    "Demo part"
                },
                FontId::proportional(12.0),
                theme::TEXT,
            );
            ui.add_space(12.0);

            // A Properties-style row: label, swatch and the colour as R, G, B.
            let row = ui.horizontal(|ui| {
                ui.label(RichText::new("Color").color(theme::TEXT_DIM));
                ui.add_space(24.0);
                let text = format!("{}, {}, {}", shown.r, shown.g, shown.b);
                ui.add(
                    egui::Button::new(RichText::new(text).monospace()).min_size(vec2(160.0, 24.0)),
                )
            });
            let button = row.inner;
            let swatch =
                egui::Rect::from_min_size(button.rect.min + vec2(6.0, 6.0), vec2(12.0, 12.0));
            ui.painter().rect_filled(
                swatch,
                2.0,
                egui::Color32::from_rgb(shown.r, shown.g, shown.b),
            );
            let key_c = ui.input(|i| i.key_pressed(Key::C) && i.modifiers.is_none())
                && !ui.ctx().text_edit_focused()
                && !self.popover.is_open();
            if button.clicked() || key_c {
                open_at = Some(button.rect.right_top() + vec2(16.0, -120.0));
            }

            ui.add_space(16.0);
            ui.label(RichText::new("Undo history").strong());
            if self.history.is_empty() {
                ui.label(RichText::new("Nothing committed yet").color(theme::TEXT_DIM));
            }
            for name in self.history.iter().rev().take(8) {
                ui.label(RichText::new(name).monospace());
            }
            ui.add_space(16.0);
            back = ui.button("Back to start").clicked();
        });
        if let Some(pos) = open_at {
            self.popover.open(self.colour, 1, false, pos);
        }
        let ctx = ui.ctx().clone();
        for event in self.popover.show(&ctx, &mut self.library) {
            self.handle(&ctx, event);
        }
        back
    }

    fn shown_colour(&self) -> Rgb8 {
        match (self.hue_shift, self.preview) {
            (Some(d), _) => super::maths::shift_hue(self.colour, d),
            (None, Some(c)) => c,
            (None, None) => self.colour,
        }
    }

    fn handle(&mut self, ctx: &egui::Context, event: ColourEvent) {
        match event {
            ColourEvent::Preview(c) => {
                self.hue_shift = None;
                self.preview = Some(c);
            }
            ColourEvent::PreviewHueShift(d) => self.hue_shift = Some(d),
            ColourEvent::Commit { colour, undo_name } => {
                self.colour = colour;
                self.preview = None;
                self.hue_shift = None;
                self.history.push(undo_name);
            }
            ColourEvent::CommitHueShift { degrees, undo_name } => {
                self.colour = super::maths::shift_hue(self.colour, degrees);
                self.preview = None;
                self.hue_shift = None;
                self.history.push(undo_name);
            }
            ColourEvent::Cancel | ColourEvent::Closed => {
                self.preview = None;
                self.hue_shift = None;
            }
            // No viewport yet: every eyedropper click reads the Studio window's pixel.
            ColourEvent::EyedropperClick { pos, .. } => self.popover.pick_screen_pixel(ctx, pos),
        }
    }
}
