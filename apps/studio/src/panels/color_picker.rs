//! The colour picker (#416), in Studio's style: a square of the colour's saturation and value (white at the top left,
//! the hue at the top right, black along the bottom), a vertical hue slider to its right, and under them a hex field
//! (`#rrggbb`) that a button switches to R, G and B fields (0 to 255). Everything is in sRGB, as the colour is shown.
//! The hue is kept while the picker is open, so a grey or black keeps the hue it was dragged from.

use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{Color32, CornerRadius, FontId, Id, Rect, Role, Sense, Stroke, TextEdit, Ui, WidgetInfo, pos2, vec2};

use crate::theme::current_colors;
use crate::theme::tokens::{size, space, text};

/// The square's side, in points.
const SQUARE: f32 = 176.0;
/// The hue slider's width.
const STRIP: f32 = 16.0;
/// Quads along each side of the square's mesh: the colours between vertices are interpolated, so a grid keeps the
/// gradient true.
const GRID: usize = 16;

/// A colour as hue (0..1, round), saturation and value (0..1), in sRGB.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hsv {
    /// Hue, 0 to 1 (red at 0 and 1).
    pub h: f32,
    /// Saturation.
    pub s: f32,
    /// Value.
    pub v: f32,
}

impl Hsv {
    /// `rgb`'s hue, saturation and value; a grey's hue is `hue`.
    pub fn of(rgb: [u8; 3], hue: f32) -> Self {
        let [r, g, b] = rgb.map(|c| f32::from(c) / 255.0);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let delta = max - min;
        let h = if delta <= 0.0 {
            hue
        } else if max == r {
            ((g - b) / delta).rem_euclid(6.0) / 6.0
        } else if max == g {
            ((b - r) / delta + 2.0) / 6.0
        } else {
            ((r - g) / delta + 4.0) / 6.0
        };
        let s = if max <= 0.0 { 0.0 } else { delta / max };
        Self { h, s, v: max }
    }

    /// The colour, 0 to 1 per channel.
    pub fn rgb(self) -> [f32; 3] {
        let h = self.h.rem_euclid(1.0) * 6.0;
        let c = self.v * self.s;
        let x = c * (1.0 - (h % 2.0 - 1.0).abs());
        let (r, g, b) = match h as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = self.v - c;
        [r + m, g + m, b + m]
    }

    /// The colour in 0-255 channels.
    pub fn rgb8(self) -> [u8; 3] {
        self.rgb().map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)
    }

    fn color(self) -> Color32 {
        let [r, g, b] = self.rgb8();
        Color32::from_rgb(r, g, b)
    }
}

/// `#rrggbb` for `rgb`.
pub fn hex(rgb: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

/// The colour `#rrggbb` (or `rrggbb`, or `#rgb`) names.
pub fn parse_hex(text: &str) -> Option<[u8; 3]> {
    let digits = text.trim().trim_start_matches('#');
    let digits: String = match digits.len() {
        3 => digits.chars().flat_map(|c| [c, c]).collect(),
        6 => digits.to_owned(),
        _ => return None,
    };
    let channel = |at: usize| u8::from_str_radix(digits.get(at..at + 2)?, 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

/// What the picker keeps while it is open: the colour as last picked, with its hue, and whether it shows R, G and B
/// rather than the hex.
#[derive(Clone, Copy, Debug)]
struct State {
    hsv: Hsv,
    rgb_fields: bool,
}

/// The picker, editing `rgb` (0-255 channels); `id` keeps its state. Returns whether the colour changed.
pub fn picker(ui: &mut Ui, id: Id, rgb: &mut [u8; 3]) -> bool {
    let colors = current_colors(ui.ctx());
    let p = colors.palette;
    let mut state = ui.data(|data| data.get_temp::<State>(id)).unwrap_or(State {
        hsv: Hsv::of(*rgb, 0.0),
        rgb_fields: false,
    });
    // Changed from outside (another selection, an undo): follow it, keeping the hue for a grey.
    if state.hsv.rgb8() != *rgb {
        state.hsv = Hsv::of(*rgb, state.hsv.h);
    }
    let before = state.hsv.rgb8();
    ui.spacing_mut().item_spacing = vec2(space::S3, space::S3);
    ui.horizontal(|ui| {
        square(ui, id, &mut state.hsv);
        hue_strip(ui, id, &mut state.hsv);
    });
    ui.horizontal(|ui| {
        let toggle = if state.rgb_fields { "Hex" } else { "RGB" };
        let field_width = SQUARE + STRIP + space::S3 - 44.0 - space::S3;
        if state.rgb_fields {
            let mut channels = state.hsv.rgb8();
            let mut changed = false;
            let each = (field_width - 2.0 * space::S2) / 3.0;
            ui.spacing_mut().item_spacing.x = space::S2;
            for (channel, name) in channels.iter_mut().zip(["R", "G", "B"]) {
                let response = ui.add_sized(
                    [each, size::CONTROL],
                    egui::DragValue::new(channel)
                        .range(0..=255)
                        .speed(1.0)
                        .prefix(format!("{name} ")),
                );
                response.widget_info(|| WidgetInfo::labeled(Role::SpinButton, true, name));
                changed |= response.changed();
            }
            ui.spacing_mut().item_spacing.x = space::S3;
            if changed {
                state.hsv = Hsv::of(channels, state.hsv.h);
            }
        } else {
            let field = id.with("hex");
            let shown = hex(state.hsv.rgb8());
            let mut typed = ui
                .data(|data| data.get_temp::<String>(field))
                .unwrap_or_else(|| shown.clone());
            let response = ui.add_sized(
                [field_width, size::CONTROL],
                TextEdit::singleline(&mut typed)
                    .id(field.with("field"))
                    .font(FontId::monospace(text::BODY))
                    .margin(vec2(space::S2, 2.0)),
            );
            response.widget_info(|| WidgetInfo::labeled(Role::TextInput, true, "Hex"));
            if response.has_focus() {
                ui.data_mut(|data| data.insert_temp(field, typed));
            } else {
                ui.data_mut(|data| data.remove::<String>(field));
                if response.lost_focus()
                    && let Some(picked) = parse_hex(&typed)
                {
                    state.hsv = Hsv::of(picked, state.hsv.h);
                }
            }
        }
        let button = ui.add_sized(
            [44.0, size::CONTROL],
            egui::Button::new(egui::RichText::new(toggle).size(text::SMALL).color(p.text))
                .fill(p.surface)
                .corner_radius(CornerRadius::same(4)),
        );
        let label = if state.rgb_fields { "Show hex" } else { "Show RGB" };
        button.widget_info(|| WidgetInfo::labeled(Role::Button, true, label));
        if button.clicked() {
            state.rgb_fields = !state.rgb_fields;
        }
    });
    ui.data_mut(|data| data.insert_temp(id, state));
    let after = state.hsv.rgb8();
    *rgb = after;
    after != before
}

/// The saturation and value square: white at the top left, the hue at the top right, black along the bottom, with a
/// ring where the colour is. A click or a drag picks.
fn square(ui: &mut Ui, id: Id, hsv: &mut Hsv) {
    let p = current_colors(ui.ctx()).palette;
    let (rect, response) = ui.allocate_exact_size(vec2(SQUARE, SQUARE), Sense::click_and_drag());
    let response = ui
        .interact(rect, id.with("square"), Sense::click_and_drag())
        .union(response);
    response.widget_info(|| WidgetInfo::labeled(Role::Slider, true, "Saturation and value"));
    if let Some(at) = response
        .interact_pointer_pos()
        .filter(|_| response.is_pointer_button_down_on())
    {
        hsv.s = ((at.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        hsv.v = (1.0 - (at.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
    }
    let mut mesh = Mesh::default();
    for row in 0..=GRID {
        for column in 0..=GRID {
            let (s, v) = (column as f32 / GRID as f32, 1.0 - row as f32 / GRID as f32);
            let at = pos2(
                rect.left() + rect.width() * column as f32 / GRID as f32,
                rect.top() + rect.height() * row as f32 / GRID as f32,
            );
            mesh.vertices.push(Vertex {
                pos: at,
                uv: WHITE_UV,
                color: Hsv { h: hsv.h, s, v }.color(),
            });
        }
    }
    grid_triangles(&mut mesh, GRID, GRID);
    ui.painter().add(mesh);
    ui.painter().rect_stroke(
        rect,
        CornerRadius::ZERO,
        Stroke::new(size::BORDER, p.border),
        egui::StrokeKind::Outside,
    );
    let at = pos2(
        rect.left() + hsv.s * rect.width(),
        rect.top() + (1.0 - hsv.v) * rect.height(),
    );
    let ring = if hsv.v > 0.5 { Color32::BLACK } else { Color32::WHITE };
    ui.painter().circle_stroke(at, 5.0, Stroke::new(1.5, ring));
}

/// The hue slider: the hues top to bottom, with a bar across where the colour's is. A click or a drag picks.
fn hue_strip(ui: &mut Ui, id: Id, hsv: &mut Hsv) {
    let p = current_colors(ui.ctx()).palette;
    let (rect, response) = ui.allocate_exact_size(vec2(STRIP, SQUARE), Sense::click_and_drag());
    let response = ui
        .interact(rect, id.with("hue"), Sense::click_and_drag())
        .union(response);
    response.widget_info(|| WidgetInfo::labeled(Role::Slider, true, "Hue"));
    if let Some(at) = response
        .interact_pointer_pos()
        .filter(|_| response.is_pointer_button_down_on())
    {
        hsv.h = ((at.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
        // A grey or black has no hue to show: picking one makes the colour show it.
        if hsv.s <= 0.0 || hsv.v <= 0.0 {
            hsv.s = hsv.s.max(1.0);
            hsv.v = hsv.v.max(1.0);
        }
    }
    let steps = 36;
    let mut mesh = Mesh::default();
    for row in 0..=steps {
        let h = row as f32 / steps as f32;
        let y = rect.top() + rect.height() * h;
        let color = Hsv { h, s: 1.0, v: 1.0 }.color();
        for x in [rect.left(), rect.right()] {
            mesh.vertices.push(Vertex {
                pos: pos2(x, y),
                uv: WHITE_UV,
                color,
            });
        }
    }
    grid_triangles(&mut mesh, steps, 1);
    ui.painter().add(mesh);
    ui.painter().rect_stroke(
        rect,
        CornerRadius::ZERO,
        Stroke::new(size::BORDER, p.border),
        egui::StrokeKind::Outside,
    );
    let y = rect.top() + hsv.h.clamp(0.0, 1.0) * rect.height();
    let bar = Rect::from_center_size(pos2(rect.center().x, y), vec2(STRIP + 4.0, 4.0));
    ui.painter().rect(
        bar,
        CornerRadius::same(2),
        Color32::WHITE,
        Stroke::new(1.0, Color32::BLACK),
        egui::StrokeKind::Outside,
    );
}

/// The two triangles of each quad of a grid of `rows` × `columns` quads whose vertices go row by row.
fn grid_triangles(mesh: &mut Mesh, rows: usize, columns: usize) {
    let width = columns as u32 + 1;
    for row in 0..rows as u32 {
        for column in 0..columns as u32 {
            let a = row * width + column;
            let b = a + 1;
            let c = a + width;
            let d = c + 1;
            mesh.add_triangle(a, b, c);
            mesh.add_triangle(b, d, c);
        }
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;

    #[test]
    fn hsv_and_hex_round_trip() {
        for rgb in [
            [255, 0, 0],
            [0, 255, 128],
            [99, 95, 98],
            [0, 0, 0],
            [255, 255, 255],
            [18, 52, 86],
        ] {
            assert_eq!(Hsv::of(rgb, 0.0).rgb8(), rgb, "{rgb:?}");
            assert_eq!(parse_hex(&hex(rgb)), Some(rgb));
        }
        assert_eq!(parse_hex("#F0a"), Some([0xff, 0x00, 0xaa]));
        assert_eq!(parse_hex("12345"), None);
        assert_eq!(parse_hex("#zzzzzz"), None);
        // A grey keeps the hue it is given.
        assert_eq!(Hsv::of([50, 50, 50], 0.3).h, 0.3);
    }

    fn harness(rgb: [u8; 3]) -> Harness<'static, [u8; 3]> {
        let mut harness = Harness::builder()
            .with_size(vec2(260.0, 260.0))
            .with_context_setup(crate::theme::fonts::install)
            .build_ui_state(
                |ui, rgb: &mut [u8; 3]| {
                    picker(ui, Id::unique("picker"), rgb);
                },
                rgb,
            );
        harness.run();
        harness
    }

    /// A click at `at`.
    fn press(harness: &mut Harness<'static, [u8; 3]>, at: egui::Pos2) {
        harness.drag_at(at);
        harness.run();
        harness.drop_at(at);
        harness.run();
    }

    #[test]
    fn the_square_the_hue_and_the_fields_each_pick() {
        let mut harness = harness([255, 0, 0]);
        // The square's top right is the hue, its bottom black.
        let square = harness
            .get_by_role_and_label(Role::Slider, "Saturation and value")
            .rect();
        press(&mut harness, square.right_top());
        harness.run();
        assert_eq!(*harness.state(), [255, 0, 0]);
        press(&mut harness, square.left_top());
        harness.run();
        assert_eq!(*harness.state(), [255, 255, 255], "the top left is white");
        press(&mut harness, square.right_top());
        harness.run();
        // Half way down the hue strip: cyan.
        let strip = harness.get_by_role_and_label(Role::Slider, "Hue").rect();
        press(&mut harness, strip.center());
        harness.run();
        assert_eq!(*harness.state(), [0, 255, 255]);
        // The hex field.
        harness.get_by_role_and_label(Role::TextInput, "Hex").click();
        harness.run();
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness
            .get_by_role_and_label(Role::TextInput, "Hex")
            .type_text("#123456");
        harness.key_press(egui::Key::Enter);
        harness.run();
        assert_eq!(*harness.state(), [0x12, 0x34, 0x56]);
        // Switched to R, G and B.
        harness.get_by_role_and_label(Role::Button, "Show RGB").click();
        harness.run();
        assert!(harness.query_by_role_and_label(Role::TextInput, "Hex").is_none());
        harness.get_by_role_and_label(Role::SpinButton, "R");
        harness.get_by_role_and_label(Role::Button, "Show hex");
    }
}
