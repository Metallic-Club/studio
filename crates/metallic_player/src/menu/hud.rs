//! A small HUD plate with the controls that matter most (Jump, Run, Menu) in the player's own
//! bindings, drawn with the runtime's cached glyphs. It shows for a few seconds at the start and
//! again whenever a glyph changes (another device, a rebind, the glyph style), so a rebinding is
//! visible in play at once.
use super::glyph_paint::{Ink, glyph_width, paint_glyph};
use super::theme;
use egui::{Align2, Color32, FontId, LayerId, Order, Rect, Shape, Stroke, pos2, vec2};
use metallic_runtime::input::{
    ActionId, CHARACTER_CONTROLS, DeviceKind, InputRuntime, MENU_ACTION,
};

/// Seconds the plate stays.
const SHOW_SECONDS: f64 = 8.0;
/// Seconds it takes to fade out.
const FADE_SECONDS: f64 = 0.4;

/// The controls hint.
#[derive(Debug, Default)]
pub struct ControlsHint {
    shown_at: Option<f64>,
    seen: Option<u64>,
    ids: [Option<ActionId>; 3],
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
            self.ids = [
                rt.find_action(&format!("{CHARACTER_CONTROLS}/Jump")),
                rt.find_action(&format!("{CHARACTER_CONTROLS}/Run")),
                rt.find_action(MENU_ACTION),
            ];
        }
        // On touch the overlay's buttons carry their own labels (#252).
        if rt.last_device() == DeviceKind::Touch {
            return;
        }
        let Some(at) = self.shown_at else { return };
        let age = now - at;
        if age > SHOW_SECONDS {
            return;
        }
        let alpha = ((SHOW_SECONDS - age) / FADE_SECONDS).clamp(0.0, 1.0) as f32;
        let p = ctx.layer_painter(LayerId::new(
            Order::Foreground,
            egui::Id::unique("metallic_hud"),
        ));
        let screen = ctx.content_rect();
        let labels = ["Jump", "Run", "Menu"];
        let h = 26.0;
        let mut items = Vec::with_capacity(3);
        let mut width = 20.0;
        for (id, label) in self.ids.iter().zip(labels) {
            let Some(g) = id.and_then(|id| rt.glyph(id, None)) else {
                continue;
            };
            let text =
                p.layout_no_wrap(label.to_string(), FontId::proportional(15.0), theme::TEXT_2);
            let gw = glyph_width(&p, g, h);
            width += gw + 8.0 + text.size().x + 20.0;
            items.push((g, gw, text));
        }
        if items.is_empty() {
            return;
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
