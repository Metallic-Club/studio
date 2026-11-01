//! A small HUD plate with the controls that matter most (Move, Jump, Run, Camera, Menu) in the
//! player's own bindings, drawn with the runtime's cached glyphs. It shows for a few seconds at the
//! start and again whenever a glyph changes (another device, a rebind, the glyph style), so a
//! rebinding is visible in play at once. It is the only controls row: games need no controls text
//! of their own (#480: Parkour's old text row sat under this plate).
use super::glyph_paint::{Ink, glyph_width, paint_glyph};
use super::theme;
use egui::{Align2, Color32, FontId, LayerId, Order, Rect, Shape, Stroke, pos2, vec2};
use metallic_runtime::input::glyph::slot_glyph;
use metallic_runtime::input::overrides::binding_device;
use metallic_runtime::input::{
    ActionId, CHARACTER_CONTROLS, ControlKind, DeviceKind, Glyph, InputRuntime, MENU_ACTION, Slot,
};

/// Seconds the plate stays.
const SHOW_SECONDS: f64 = 8.0;
/// Seconds it takes to fade out.
const FADE_SECONDS: f64 = 0.4;

/// The words on the plate, left to right; [`plate_actions`] gives the actions in the same order.
const LABELS: [&str; 5] = ["Move", "Jump", "Run", "Camera", "Menu"];

/// The controls hint.
#[derive(Debug, Default)]
pub struct ControlsHint {
    shown_at: Option<f64>,
    seen: Option<u64>,
    items: Vec<(Glyph, &'static str)>,
}

impl ControlsHint {
    /// A hint that shows at the first paint.
    pub fn new() -> Self {
        Self::default()
    }

    /// Paints the hint over the game (not while the menu is open, not on touch).
    pub fn paint(&mut self, ctx: &egui::Context, rt: &InputRuntime, now: f64) {
        let changes = rt.glyph_changes();
        if self.seen != Some(changes) {
            self.seen = Some(changes);
            self.shown_at = Some(now);
            self.items = plate_items(rt);
        }
        // On touch the overlay's buttons carry their own labels (#252).
        if rt.last_device() == DeviceKind::Touch {
            return;
        }
        let Some(at) = self.shown_at else { return };
        let age = now - at;
        if age > SHOW_SECONDS || self.items.is_empty() {
            return;
        }
        let alpha = ((SHOW_SECONDS - age) / FADE_SECONDS).clamp(0.0, 1.0) as f32;
        let p = ctx.layer_painter(LayerId::new(
            Order::Foreground,
            egui::Id::unique("metallic_hud"),
        ));
        let screen = ctx.content_rect();
        let h = 26.0;
        let mut items = Vec::with_capacity(self.items.len());
        let mut width = 20.0;
        for (g, label) in &self.items {
            let text =
                p.layout_no_wrap(label.to_string(), FontId::proportional(15.0), theme::TEXT_2);
            let gw = glyph_width(&p, g, h);
            width += gw + 8.0 + text.size().x + 20.0;
            items.push((g, gw, text));
        }
        let plate = Rect::from_min_size(
            pos2(screen.left() + 24.0, screen.bottom() - 24.0 - 48.0),
            vec2(width, 48.0),
        );
        let fill = Color32::from_rgba_unmultiplied(17, 16, 21, (224.0 * alpha) as u8);
        let cut = 12.0;
        p.add(Shape::convex_polygon(
            vec![
                plate.left_top(),
                pos2(plate.right() - cut, plate.top()),
                pos2(plate.right(), plate.top() + cut),
                plate.right_bottom(),
                plate.left_bottom(),
            ],
            fill,
            Stroke::NONE,
        ));
        let ink = Ink {
            line: theme::LINE_CONTROL.gamma_multiply(alpha),
            face: theme::BG_RAISED.gamma_multiply(alpha),
            fill: theme::TEXT_1.gamma_multiply(alpha),
            text: theme::TEXT_1.gamma_multiply(alpha),
        };
        let mut x = plate.left() + 20.0;
        for (g, gw, text) in items {
            let r = Rect::from_min_size(pos2(x, plate.center().y - h / 2.0), vec2(gw, h));
            paint_glyph(&p, r, g, ink);
            x += gw + 8.0;
            let size = text.size();
            p.galley(
                pos2(x, plate.center().y - size.y / 2.0),
                text,
                theme::TEXT_2.gamma_multiply(alpha),
            );
            x += size.x + 20.0;
            if g.toggle {
                p.text(
                    pos2(x - 16.0, plate.top() + 8.0),
                    Align2::RIGHT_TOP,
                    "toggle",
                    FontId::proportional(10.0),
                    theme::TEXT_3.gamma_multiply(alpha),
                );
            }
        }
    }
}

/// The plate's actions in [`LABELS`] order (`None` when the game has no such action).
fn plate_actions(rt: &InputRuntime) -> [Option<ActionId>; 5] {
    let character = |name: &str| rt.find_action(&format!("{CHARACTER_CONTROLS}/{name}"));
    [
        character("Move"),
        character("Jump"),
        character("Run"),
        character("Look"),
        rt.find_action(MENU_ACTION),
    ]
}

/// The plate's glyphs and words on the last-used device; actions the game lacks are left out.
/// Built once per glyph change, not per frame.
fn plate_items(rt: &InputRuntime) -> Vec<(Glyph, &'static str)> {
    plate_actions(rt)
        .into_iter()
        .zip(LABELS)
        .filter_map(|(id, label)| Some((plate_glyph(rt, id?)?, label)))
        .collect()
}

/// The glyph of an action's first binding on the last-used device, except that mouse movement
/// held behind a button (Look: the right button, "right-drag") shows that button: holding it is
/// the part to learn, the mouse moving is not.
fn plate_glyph(rt: &InputRuntime, id: ActionId) -> Option<Glyph> {
    let glyph = rt.glyph(id, None)?;
    let device = rt.last_device();
    let held = rt
        .bindings(id)
        .find(|b| binding_device(b) == Some(device))
        .filter(|b| b.key.is_some_and(|k| k.kind() == ControlKind::Delta))
        .and_then(|b| b.modifiers[0]);
    Some(match held {
        Some(button) => Glyph {
            toggle: glyph.toggle,
            ..slot_glyph(&Slot::Single(Some(button)), device, rt.gamepad_style())
        },
        None => glyph.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use metallic_runtime::input::glyph::{GlyphArt, MousePart};
    use std::path::Path;

    fn parkour() -> InputRuntime {
        let scene =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/parkour/main.mscene");
        let world = metallic_core::load_scene(&scene).expect("Parkour scene");
        InputRuntime::from_world(&world)
    }

    fn art(g: &Glyph) -> GlyphArt {
        g.parts[0].expect("bound").art
    }

    /// Parkour's controls make one plate with every key the old text row had (#480).
    #[test]
    fn parkour_gets_one_plate_with_move_and_camera() {
        let items = plate_items(&parkour());
        let labels: Vec<&str> = items.iter().map(|(_, l)| *l).collect();
        assert_eq!(labels, LABELS);
        let words: Vec<&str> = items.iter().map(|(g, _)| g.text.as_str()).collect();
        assert_eq!(
            words,
            ["W A S D", "Space", "Left Shift", "Right Click", "Esc"]
        );
        assert_eq!(
            art(&items[3].0),
            GlyphArt::Mouse(MousePart::Right),
            "Camera is right-drag"
        );
    }

    /// A pad shows its sticks: the held-button rule is for mouse movement only.
    #[test]
    fn gamepad_plate_shows_the_sticks() {
        let mut rt = parkour();
        rt.set_forced_device(Some(DeviceKind::Gamepad));
        let items = plate_items(&rt);
        assert_eq!(items.len(), LABELS.len());
        assert_eq!(items[0].0.text, "Left Stick");
        assert_eq!(items[3].0.text, "Right Stick");
        assert!(items.iter().all(|(g, _)| g.device == DeviceKind::Gamepad));
    }
}
