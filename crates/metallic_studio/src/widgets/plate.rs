//! Plates: the flat surfaces everything sits on (research doc section 3.4).
//!
//! A raised plate (level 2) is a fill with a 1 px `highlight` along the top and a 1 px `edge-low`
//! along the bottom, and no shadow. Floating (level 3) and modal (level 4) plates add the token
//! shadow. Any plate can carry the cut.

use eframe::egui::{Color32, CornerRadius, Painter, Pos2, Rect, Shadow, Shape, Stroke};

use crate::theme;
use crate::widgets::cut;

/// How a plate is painted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plate {
    /// Fill colour.
    pub fill: Color32,
    /// Corner radii (`ne` is ignored when there is a cut).
    pub radius: CornerRadius,
    /// The top-right chamfer in px (0 for none).
    pub cut: f32,
    /// Draw the 1 px `highlight` line along the top.
    pub highlight: bool,
    /// Draw the 1 px `edge-low` line along the bottom.
    pub edge_low: bool,
    /// Shadow cast under the plate (only levels 3 and 4 have one).
    pub shadow: Shadow,
}

impl Plate {
    /// Level 2: secondary buttons, cards, segments (`bg-raised`, `r-sm`, both edges).
    pub fn raised() -> Self {
        Self {
            fill: theme::bg_raised(),
            radius: CornerRadius::same(theme::R_SM),
            cut: 0.0,
            highlight: true,
            edge_low: true,
            shadow: Shadow::NONE,
        }
    }

    /// A panel-coloured surface with only the top highlight (cards at rest, the stage).
    pub fn panel() -> Self {
        Self {
            fill: theme::bg_panel(),
            radius: CornerRadius::same(theme::R_MD),
            edge_low: false,
            ..Self::raised()
        }
    }

    /// Level 3: menus, popovers, toasts, tooltips (`bg-hover`, `r-md`, the pop shadow).
    pub fn floating() -> Self {
        Self {
            fill: theme::bg_hover(),
            radius: CornerRadius::same(theme::R_MD),
            edge_low: false,
            shadow: theme::SHADOW_POP,
            ..Self::raised()
        }
    }

    /// Level 4: dialogs and the splash (`bg-panel`, `r-lg`, `cut-lg`, the modal shadow).
    pub fn modal() -> Self {
        Self {
            fill: theme::bg_panel(),
            radius: CornerRadius::same(theme::R_LG),
            cut: theme::CUT_LG,
            edge_low: false,
            shadow: theme::SHADOW_MODAL,
            ..Self::raised()
        }
    }

    /// Sets the fill.
    pub fn fill(mut self, fill: Color32) -> Self {
        self.fill = fill;
        self
    }

    /// Sets all corner radii.
    pub fn radius(mut self, radius: impl Into<CornerRadius>) -> Self {
        self.radius = radius.into();
        self
    }

    /// Sets the top-right chamfer.
    pub fn cut(mut self, cut: f32) -> Self {
        self.cut = cut;
        self
    }

    /// Paints the plate into `rect`.
    pub fn paint(&self, painter: &Painter, rect: Rect) {
        if self.shadow != Shadow::NONE {
            painter.add(self.shadow.as_shape(rect, self.radius));
        }
        if self.cut > 0.0 {
            cut::fill(painter, rect, self.radius, self.cut, self.fill);
        } else {
            painter.rect_filled(rect, self.radius, self.fill);
        }
        let (top, bottom) = edge_lines(rect, self.radius, self.cut);
        if self.highlight {
            painter.add(Shape::line_segment(
                top,
                Stroke::new(1.0, theme::highlight()),
            ));
        }
        if self.edge_low {
            painter.add(Shape::line_segment(
                bottom,
                Stroke::new(1.0, theme::edge_low()),
            ));
        }
    }
}

/// The 1 px top and bottom edge lines of a plate: half a pixel inside `rect`, running between the
/// corner arcs (and stopping where the cut starts).
pub fn edge_lines(rect: Rect, radius: CornerRadius, cut: f32) -> ([Pos2; 2], [Pos2; 2]) {
    let max = 0.5 * rect.width().min(rect.height()).max(0.0);
    let r = |v: u8| f32::from(v).min(max);
    let top_right = if cut > 0.0 {
        cut.min(max)
    } else {
        r(radius.ne)
    };
    let y_top = rect.top() + 0.5;
    let y_bottom = rect.bottom() - 0.5;
    (
        [
            Pos2::new(rect.left() + r(radius.nw), y_top),
            Pos2::new(rect.right() - top_right, y_top),
        ],
        [
            Pos2::new(rect.left() + r(radius.sw), y_bottom),
            Pos2::new(rect.right() - r(radius.se), y_bottom),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{pos2, vec2};

    #[test]
    fn edges_run_between_the_corners_and_stop_at_the_cut() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 32.0));
        let (top, bottom) = edge_lines(rect, CornerRadius::same(4), 0.0);
        assert_eq!(top, [pos2(4.0, 0.5), pos2(96.0, 0.5)]);
        assert_eq!(bottom, [pos2(4.0, 31.5), pos2(96.0, 31.5)]);
        let (top, _) = edge_lines(rect, CornerRadius::same(4), 14.0);
        assert_eq!(top[1], pos2(86.0, 0.5));
    }

    #[test]
    fn levels_follow_the_elevation_table() {
        assert!(Plate::raised().highlight && Plate::raised().edge_low);
        assert_eq!(Plate::raised().shadow, Shadow::NONE);
        assert_eq!(Plate::floating().shadow, theme::SHADOW_POP);
        assert_eq!(Plate::floating().fill, theme::bg_hover());
        assert_eq!(Plate::modal().shadow, theme::SHADOW_MODAL);
        assert_eq!(Plate::modal().cut, theme::CUT_LG);
    }
}
