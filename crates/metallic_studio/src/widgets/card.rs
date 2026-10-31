//! Template cards and the art they show (research doc 3.7 and 4.2).
//!
//! A card is a `bg-panel` plate with the top highlight: 16:9 art, then the name (`strong`) and a
//! facts line (`small`, `text-3`). Hover lifts it to `bg-raised`. The selected card gets a 2 px
//! `accent` frame that follows the cut, a `bg-raised` plate inside it, and a check before the
//! name. Templates without a render get Theme art: sky, horizon and ground plus a glyph.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, Pos2, Rect, Response, Sense, Stroke, TextureHandle, Ui,
    Vec2, Widget, vec2,
};

use crate::theme::{self, fonts, icons};
use crate::widgets::{cut, truncated};

/// Height of the label under the art: 9 + 18 + 16 + 11.
pub const LABEL_HEIGHT: f32 = 54.0;
/// The selected card's frame width.
const FRAME: f32 = 2.0;

/// Theme art: the colours of a template's lighting as three flat bands (art, not chrome).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeArt {
    /// The sky band (top 58 %).
    pub sky: Color32,
    /// The horizon band (4 %).
    pub horizon: Color32,
    /// The ground band.
    pub ground: Color32,
    /// The glyph colour.
    pub ink: Color32,
}

impl ThemeArt {
    /// Neutral daylight, for templates that have no render of their own yet.
    pub const NEUTRAL: Self = Self {
        sky: Color32::from_rgb(185, 188, 198),
        horizon: Color32::from_rgb(214, 216, 222),
        ground: Color32::from_rgb(141, 144, 155),
        ink: Color32::from_rgb(44, 42, 52),
    };
}

/// What fills an art area.
#[derive(Clone, Copy)]
pub enum Art<'a> {
    /// A template render or game capture, cropped to cover.
    Image(&'a TextureHandle),
    /// Theme art with a Phosphor glyph in the middle.
    Theme(ThemeArt, char),
}

/// The part of a `texture`-sized image that covers `target` without stretching (centred crop).
pub fn cover_uv(texture: Vec2, target: Vec2) -> Rect {
    if texture.x <= 0.0 || texture.y <= 0.0 || target.x <= 0.0 || target.y <= 0.0 {
        return Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0));
    }
    let scale = (target.x / texture.x).max(target.y / texture.y);
    let visible = target / (texture * scale);
    Rect::from_center_size(egui::pos2(0.5, 0.5), visible)
}

/// Paints `art` into `rect` with the given corners and cut. `glyph_size` is the Theme art glyph.
pub fn paint_art(
    ui: &Ui,
    rect: Rect,
    radius: CornerRadius,
    cut_size: f32,
    art: Art<'_>,
    glyph_size: f32,
) {
    let painter = ui.painter();
    match art {
        Art::Image(tex) => {
            // A plate under the image keeps the clipped edges clean.
            cut::fill(painter, rect, radius, cut_size, theme::bg_raised());
            let uv = cover_uv(tex.size_vec2(), rect.size());
            cut::paint_image(
                painter,
                rect,
                radius,
                cut_size,
                tex.id(),
                uv,
                Color32::WHITE,
            );
        }
        Art::Theme(colours, glyph) => {
            cut::fill(painter, rect, radius, cut_size, colours.ground);
            let horizon_y = rect.top() + rect.height() * 0.58;
            let ground_y = rect.top() + rect.height() * 0.62;
            let sky = Rect::from_min_max(rect.min, egui::pos2(rect.right(), horizon_y));
            let top_only = CornerRadius {
                nw: radius.nw,
                ne: radius.ne,
                ..CornerRadius::ZERO
            };
            cut::fill(painter, sky, top_only, cut_size, colours.sky);
            painter.rect_filled(
                Rect::from_x_y_ranges(rect.x_range(), horizon_y..=ground_y),
                0.0,
                colours.horizon,
            );
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                glyph,
                fonts::icon(glyph_size),
                colours.ink,
            );
        }
    }
}

/// One starting-point card, added with `ui.add`. The caller sets the width; the selected frame
/// is drawn 2 px outside it, so selected and plain cards line up.
#[must_use = "add the card with `ui.add`"]
pub struct Card<'a> {
    width: f32,
    name: &'a str,
    facts: &'a str,
    art: Art<'a>,
    selected: bool,
}

impl<'a> Card<'a> {
    /// A card `width` wide showing `art`, `name` and `facts`.
    pub fn new(width: f32, name: &'a str, facts: &'a str, art: Art<'a>) -> Self {
        Self {
            width,
            name,
            facts,
            art,
            selected: false,
        }
    }

    /// Draws the selected look (accent frame, cut, check).
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// The card's height for a given width (16:9 art plus the label).
    pub fn height(width: f32) -> f32 {
        (width * 9.0 / 16.0).round() + LABEL_HEIGHT
    }
}

impl Widget for Card<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let size = vec2(self.width, Self::height(self.width));
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::ListBoxOption, true, self.selected, self.name)
        });
        if !ui.is_rect_visible(rect) {
            return response;
        }
        let hover = ui.ctx().animate_bool_with_time(
            response.id.with("hover"),
            response.hovered(),
            theme::t_fast(),
        );
        let painter = ui.painter();
        let radius = CornerRadius::same(theme::R_MD);
        // A selected card sits inside a 2 px accent frame (outside its rect) that shares the cut.
        let plate_cut = if self.selected {
            let outer_cut = theme::CUT_MD + FRAME;
            cut::fill(
                painter,
                rect.expand(FRAME),
                CornerRadius::same(theme::R_MD + FRAME as u8),
                outer_cut,
                theme::accent(),
            );
            cut::inset_cut(outer_cut, FRAME)
        } else {
            0.0
        };
        let fill = if self.selected {
            theme::bg_raised()
        } else {
            theme::bg_panel().lerp_to_gamma(theme::bg_raised(), hover)
        };
        cut::fill(painter, rect, radius, plate_cut, fill);

        let art_rect = Rect::from_min_size(
            rect.min,
            vec2(rect.width(), (self.width * 9.0 / 16.0).round()),
        );
        let art_radius = CornerRadius {
            nw: radius.nw,
            ne: radius.ne,
            ..CornerRadius::ZERO
        };
        paint_art(ui, art_rect, art_radius, plate_cut, self.art, 38.0);

        let label_left = rect.left() + 12.0;
        let label_top = art_rect.bottom() + 9.0;
        let mut name_x = label_left;
        if self.selected {
            painter.text(
                egui::pos2(name_x, label_top + 9.0),
                Align2::LEFT_CENTER,
                icons::CHECK_CIRCLE,
                fonts::icon_fill(14.0),
                theme::accent(),
            );
            name_x += 14.0 + 6.0;
        }
        let text_w = rect.right() - 12.0 - name_x;
        let name = truncated(painter, self.name, fonts::strong(), theme::text_1(), text_w);
        painter.galley(
            egui::pos2(name_x, label_top + 9.0 - name.size().y / 2.0),
            name,
            theme::text_1(),
        );
        let facts = truncated(
            painter,
            self.facts,
            fonts::small(),
            theme::text_3(),
            rect.right() - 12.0 - label_left,
        );
        painter.galley(
            egui::pos2(label_left, label_top + 18.0 + 8.0 - facts.size().y / 2.0),
            facts,
            theme::text_3(),
        );

        if response.has_focus() {
            focus_frame(painter, rect, self.selected);
        }
        response
    }
}

/// 2 px `focus`, 2 px outside the card's frame, following the cut on the selected one.
fn focus_frame(painter: &egui::Painter, rect: Rect, selected: bool) {
    let out = FRAME + 4.0;
    let radius = CornerRadius::same(theme::R_MD + out as u8);
    let cut_size = if selected {
        // Grow the cut so the ring's chamfer stays parallel to the card's.
        theme::CUT_MD + out * (2.0 - std::f32::consts::SQRT_2)
    } else {
        0.0
    };
    cut::frame(
        painter,
        rect.expand(out),
        radius,
        cut_size,
        Stroke::new(2.0, theme::focus()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_crops_the_long_side_and_keeps_the_centre() {
        // A 16:9 image in a square: full height, the middle 9/16 of the width.
        let uv = cover_uv(vec2(1600.0, 900.0), vec2(100.0, 100.0));
        assert!((uv.height() - 1.0).abs() < 1e-5);
        assert!((uv.width() - 0.5625).abs() < 1e-5);
        assert!((uv.center().x - 0.5).abs() < 1e-5);
        // Same aspect: the whole image.
        let uv = cover_uv(vec2(640.0, 360.0), vec2(320.0, 180.0));
        assert!((uv.width() - 1.0).abs() < 1e-5 && (uv.height() - 1.0).abs() < 1e-5);
        // Degenerate sizes never divide by zero.
        assert_eq!(cover_uv(Vec2::ZERO, vec2(10.0, 10.0)).width(), 1.0);
    }

    #[test]
    fn card_height_is_16_9_art_plus_the_label() {
        assert_eq!(Card::height(160.0), 90.0 + LABEL_HEIGHT);
    }

    #[test]
    fn theme_art_glyph_reads_on_its_sky() {
        let art = ThemeArt::NEUTRAL;
        assert!(theme::contrast_ratio(art.ink, art.sky) >= 3.0);
    }
}
