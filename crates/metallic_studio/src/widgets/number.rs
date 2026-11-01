//! Number fields: one display rule per kind of value, right-aligned in a fixed-width box, in the
//! interface font with tabular figures, so numbers in a column line up. Archivo's own figures are
//! proportional (a "1" is narrower than an "8") and egui cannot switch on the font's `tnum`
//! feature, so the field draws its text itself: every digit centred in a cell as wide as the
//! widest digit ([`tabular`]).
//!
//! Every kind rounds to its own number of decimals and drops trailing zeros, so a zero always
//! reads "0" and a value shows as much precision as the kind needs: studs to 0.001, degrees to
//! 0.01 (11.25° snaps stay exact), 0-1 fractions to 0.001, integers whole. A negative zero, or a
//! negative value that rounds to zero, shows as "0". While a field is being typed into it holds the
//! full value (to 0.000001), so clicking into a field and out again never rounds the value.

use eframe::egui::{
    self, Align, Align2, Color32, Layout, Response, Sense, TextStyle, Ui, UiBuilder, pos2, vec2,
};

/// What a number measures, which decides how it is shown and how fast it drags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberKind {
    /// Lengths and positions in studs (and other measures with no kind of their own).
    Studs,
    /// Angles in degrees.
    Degrees,
    /// A 0-1 fraction (Transparency, a UDim2 scale).
    Fraction,
    /// A whole number (ZIndex, a UDim2 offset in pixels, a text size).
    Integer,
}

impl NumberKind {
    /// Decimals shown at most.
    pub const fn decimals(self) -> usize {
        match self {
            NumberKind::Studs | NumberKind::Fraction => 3,
            NumberKind::Degrees => 2,
            NumberKind::Integer => 0,
        }
    }

    /// Change per point dragged.
    pub const fn speed(self) -> f64 {
        match self {
            NumberKind::Studs => 0.1,
            NumberKind::Degrees | NumberKind::Integer => 1.0,
            NumberKind::Fraction => 0.01,
        }
    }
}

/// `n` rounded to `decimals` (half away from zero), trailing zeros dropped, never "-0".
fn trimmed(n: f64, decimals: usize) -> String {
    if n.is_nan() {
        return "NaN".to_string();
    }
    if n.is_infinite() {
        return if n > 0.0 { "∞" } else { "-∞" }.to_string();
    }
    let scale = 10f64.powi(decimals as i32);
    let scaled = (n * scale).round();
    // Past about 1e292 the scaled value overflows; such a number has no decimals to show anyway.
    let r = if scaled.is_finite() {
        scaled / scale
    } else {
        n
    };
    let mut text = format!("{r:.decimals$}");
    if text.contains('.') {
        text.truncate(text.trim_end_matches('0').trim_end_matches('.').len());
    }
    if text == "-0" {
        text.remove(0);
    }
    text
}

/// How a number of `kind` is shown (see the module doc).
pub fn format(n: f64, kind: NumberKind) -> String {
    trimmed(n, kind.decimals())
}

/// The text a field starts with when it is clicked to type a value: the full value.
pub fn edit_text(n: f64) -> String {
    trimmed(n, 6)
}

/// Height of a number field: a button with body text, so the field fills exactly the space taken
/// for it. Labels beside fields take this height to share their centre line.
pub fn height(ui: &Ui) -> f32 {
    (ui.text_style_height(&TextStyle::Body) + 2.0 * ui.spacing().button_padding.y)
        .max(ui.spacing().interact_size.y)
}

/// A draggable, typeable number field `width` points wide with its text right-aligned. `parse`
/// reads typed text (Properties takes expressions, see [`crate::expr`]). An integer field rounds
/// what is typed. The response is `changed()` when the value changed.
pub fn field(
    ui: &mut Ui,
    value: &mut f64,
    kind: NumberKind,
    width: f32,
    parse: impl Fn(&str) -> Option<f64>,
) -> Response {
    let size = vec2(width, height(ui));
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    // The DragValue aligns its text (shown or typed) by the layout's main alignment.
    let layout = Layout::right_to_left(Align::Center).with_main_align(Align::Max);
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect).layout(layout));
    child.style_mut().drag_value_text_style = TextStyle::Body;
    // The DragValue takes the next id; while it has focus it is being typed into.
    let typing = child.memory(|m| m.has_focus(child.next_auto_id()));
    if !typing {
        // The shown value is drawn below with tabular figures; the button's own text is hidden.
        let w = &mut child.visuals_mut().widgets;
        for v in [
            &mut w.noninteractive,
            &mut w.inactive,
            &mut w.hovered,
            &mut w.active,
            &mut w.open,
        ] {
            v.fg_stroke.color = Color32::TRANSPARENT;
        }
    }
    let response = child.add(
        egui::DragValue::new(value)
            .speed(kind.speed())
            .max_decimals(kind.decimals())
            .min_size(size)
            .custom_formatter(move |n, _| {
                if typing {
                    edit_text(n)
                } else {
                    format(n, kind)
                }
            })
            .custom_parser(parse),
    );
    if kind == NumberKind::Integer {
        *value = value.round();
    }
    if !typing {
        let font = TextStyle::Body.resolve(ui.style());
        // The colour the button would have used (the child's painter still fades it when the
        // panel is disabled, as while playing).
        let color = ui.style().interact(&response).text_color();
        let advance = |c: char| ui.ctx().fonts_mut(|f| f.glyph_width(&font, c));
        let digit = ('0'..='9').map(advance).fold(0.0, f32::max);
        let right = response.rect.right() - ui.spacing().button_padding.x;
        let y = response.rect.center().y;
        for (c, x) in tabular(&format(*value, kind), digit, advance, right) {
            child
                .painter()
                .text(pos2(x, y), Align2::CENTER_CENTER, c, font.clone(), color);
        }
    }
    response
}

/// Lays `text` out right-aligned so it ends at `right`: each digit centred in a cell `digit` wide,
/// every other character as wide as `advance` says. Returns each character with its centre x.
fn tabular(text: &str, digit: f32, advance: impl Fn(char) -> f32, right: f32) -> Vec<(char, f32)> {
    let mut x = right;
    let mut out: Vec<(char, f32)> = text
        .chars()
        .rev()
        .map(|c| {
            let w = if c.is_ascii_digit() {
                digit
            } else {
                advance(c)
            };
            x -= w;
            (c, x + w / 2.0)
        })
        .collect();
    out.reverse();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn studs_show_up_to_three_decimals() {
        assert_eq!(format(0.0, NumberKind::Studs), "0");
        assert_eq!(format(4.0, NumberKind::Studs), "4");
        assert_eq!(format(0.5, NumberKind::Studs), "0.5");
        assert_eq!(format(0.125, NumberKind::Studs), "0.125");
        assert_eq!(format(-12.25, NumberKind::Studs), "-12.25");
        // Rounded, not cut: 1.23456 is 1.235, 2.0004 is 2.
        assert_eq!(format(1.23456, NumberKind::Studs), "1.235");
        assert_eq!(format(2.0004, NumberKind::Studs), "2");
        assert_eq!(format(2.0006, NumberKind::Studs), "2.001");
        assert_eq!(format(-2.0006, NumberKind::Studs), "-2.001");
    }

    #[test]
    fn degrees_keep_snap_steps_exact() {
        assert_eq!(format(0.0, NumberKind::Degrees), "0");
        assert_eq!(format(90.0, NumberKind::Degrees), "90");
        assert_eq!(format(11.25, NumberKind::Degrees), "11.25");
        assert_eq!(format(-22.5, NumberKind::Degrees), "-22.5");
        // Float noise from an orientation read back from a CFrame.
        assert_eq!(format(29.999_999_999_999_996, NumberKind::Degrees), "30");
        assert_eq!(format(-179.996, NumberKind::Degrees), "-180");
    }

    #[test]
    fn fractions_and_integers() {
        assert_eq!(format(0.0, NumberKind::Fraction), "0");
        assert_eq!(format(1.0, NumberKind::Fraction), "1");
        assert_eq!(format(0.25, NumberKind::Fraction), "0.25");
        assert_eq!(format(1.0 / 3.0, NumberKind::Fraction), "0.333");
        assert_eq!(format(0.0, NumberKind::Integer), "0");
        assert_eq!(format(24.0, NumberKind::Integer), "24");
        assert_eq!(format(-3.0, NumberKind::Integer), "-3");
        // Half away from zero, both ways.
        assert_eq!(format(2.5, NumberKind::Integer), "3");
        assert_eq!(format(-2.5, NumberKind::Integer), "-3");
        assert_eq!(format(2.4, NumberKind::Integer), "2");
    }

    #[test]
    fn negative_zero_shows_as_zero() {
        for kind in [
            NumberKind::Studs,
            NumberKind::Degrees,
            NumberKind::Fraction,
            NumberKind::Integer,
        ] {
            assert_eq!(format(-0.0, kind), "0", "{kind:?}");
            assert_eq!(format(-0.000_1, kind), "0", "{kind:?}");
        }
        assert_eq!(format(-0.4, NumberKind::Integer), "0");
        assert_eq!(edit_text(-0.0), "0");
    }

    #[test]
    fn large_values_print_in_full() {
        assert_eq!(format(2048.0, NumberKind::Studs), "2048");
        assert_eq!(format(123_456.789_4, NumberKind::Studs), "123456.789");
        assert_eq!(format(-1.0e9, NumberKind::Studs), "-1000000000");
        assert_eq!(format(1.0e15, NumberKind::Integer), "1000000000000000");
        assert_eq!(format(1.0e300, NumberKind::Studs).len(), 301);
        assert_eq!(format(f64::INFINITY, NumberKind::Studs), "∞");
        assert_eq!(format(f64::NEG_INFINITY, NumberKind::Degrees), "-∞");
        assert_eq!(format(f64::NAN, NumberKind::Fraction), "NaN");
    }

    #[test]
    fn typing_starts_from_the_full_value() {
        assert_eq!(edit_text(0.123_456_7), "0.123457");
        assert_eq!(edit_text(11.25), "11.25");
        assert_eq!(edit_text(4.0), "4");
    }

    #[test]
    fn digits_take_one_cell_each() {
        // A narrow "1" and a wide "8" take the same cell, so the points of "-1.25" and "-8.75"
        // line up, as do the digits after them; the sign and point keep their own widths.
        let advance = |c: char| if c == '1' { 4.0 } else { 3.0 };
        let a = tabular("-1.25", 7.0, advance, 100.0);
        let b = tabular("-8.75", 7.0, advance, 100.0);
        let x = |v: &[(char, f32)]| v.iter().map(|&(_, x)| x).collect::<Vec<_>>();
        assert_eq!(x(&a), x(&b));
        assert_eq!(x(&a), [74.5, 79.5, 84.5, 89.5, 96.5]);
        // Right-aligned: a shorter number ends at the same edge.
        assert_eq!(tabular("8", 7.0, advance, 100.0), [('8', 96.5)]);
    }
}
